#!/usr/bin/env bash
# Pubblica l'AppImage sul sito via SFTP (tmp + rename) e aggiorna latest.json per ultimo.
# Uso: publish-site.sh [--dry-run] <appimage> <build> <commit-sha>
#   (in alternativa env APPIMAGE, BUILD, COMMIT). Config: ~/.config/gnobbi-ci/publish.env
#   (override: GNOBBI_PUBLISH_ENV) con PUBLISH_HOST, PUBLISH_USER, PUBLISH_KEY, PUBLISH_DIR,
#   PUBLISH_URL_BASE. Versione testuale: VERSION, altrimenti Cargo.toml del workspace.
#   Il latest.json è firmato minisign e pubblicato come latest.json.minisig
#   (contratto: docs interno MANIFEST_SIGNING.md). Firma di default: chiave cifrata
#   SIGN_KEY con passphrase letta da SIGN_PASSPHRASE_FILE (file 0600, mai in argv/log);
#   sostituibile con SIGN_CMD (comando + argomenti) che riceve <manifest> <trusted-comment>
#   e deve scrivere <manifest>.minisig. Chiavi pubbliche: GNOBBI_UPDATE_KEYS, default
#   crates/matrix-desktop/update-keys.pub del repo.
# Modalità di test (niente upload): --sign <manifest> <build> firma e esce;
#   --verify <manifest> <minisig> verifica firma, trusted comment e scadenza, esce 0/3.
# Exit: 0 ok, 2 config/argomenti, 3 verifica fallita, 4 errore upload, 75 sito/host irraggiungibile.
set -euo pipefail

KEEP=3
VALID_DAYS=30
RENEW_DAYS=10
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
KEYS_FILE="${GNOBBI_UPDATE_KEYS:-$ROOT/crates/matrix-desktop/update-keys.pub}"
SIGN_KEY="${SIGN_KEY:-$HOME/gnobbi-update-keys/update-key.key}"
SIGN_PASSPHRASE_FILE="${SIGN_PASSPHRASE_FILE:-$HOME/gnobbi-update-keys/update-key.passphrase}"
ENV_FILE="${GNOBBI_PUBLISH_ENV:-${HOME}/.config/gnobbi-ci/publish.env}"
MODE=publish
DRY=0
case "${1:-}" in
  --sign|--verify) MODE="${1#--}"; shift ;;
  --dry-run) DRY=1; shift ;;
esac

die() { local rc=$1; shift; echo "publish-site: $*" >&2; exit "$rc"; }

# Firma i byte esatti di $1 con trusted comment $2, scrive $1.minisig (MANIFEST_SIGNING.md §3).
sign_manifest() { # <file> <trusted-comment>
  local f="$1" t="$2" rc=0
  if [[ -n "${SIGN_CMD:-}" ]]; then
    local -a sc; read -ra sc <<<"$SIGN_CMD"
    "${sc[@]}" "$f" "$t"
    [[ -s "$f.minisig" ]] || die 2 "SIGN_CMD non ha prodotto $f.minisig"
    return 0
  fi
  command -v minisign >/dev/null || die 2 "minisign mancante (o imposta SIGN_CMD)"
  [[ -f "$SIGN_KEY" ]] || die 2 "chiave privata assente: $SIGN_KEY (o imposta SIGN_CMD)"
  [[ -f "$SIGN_PASSPHRASE_FILE" ]] || die 2 "file passphrase assente: $SIGN_PASSPHRASE_FILE"
  [[ "$(stat -c %a "$SIGN_PASSPHRASE_FILE" 2>/dev/null || echo 0)" == 600 ]] ||
    die 2 "$SIGN_PASSPHRASE_FILE deve avere permessi 600"
  # La passphrase entra solo via stdin, mai in argv né nell'output.
  set +x
  minisign -S -s "$SIGN_KEY" -m "$f" -x "$f.minisig" -t "$t" <"$SIGN_PASSPHRASE_FILE" >/dev/null 2>&1 || rc=$?
  ((rc == 0)) || die 2 "firma minisign fallita (rc=$rc)"
}

# Verifica $1 contro $2 con tutte le chiavi di KEYS_FILE (max 2), poi trusted comment
# e scadenza (MANIFEST_SIGNING.md §5.4; rifiuti R3/R4/R5/R7).
verify_manifest() { # <manifest> <minisig>
  local m="$1" s="$2" pub b until ok=0 keys
  [[ -s "$m" && -s "$s" && -s "$KEYS_FILE" ]] || return 1
  mapfile -t keys < <(sed -n '2~2p' "$KEYS_FILE" | grep .)
  # più di 2 chiavi = file non previsto: rifiuto, non verifico un sottoinsieme
  ((${#keys[@]} >= 1 && ${#keys[@]} <= 2)) || return 1
  for pub in "${keys[@]}"; do
    minisign -V -q -P "$pub" -m "$m" -x "$s" 2>/dev/null && { ok=1; break; }
  done
  ((ok)) || return 1
  b="$(jq -er 'select(.schema==1) | select(.build|type=="number") |
       select(.valid_until|test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$")) |
       .build' "$m" 2>/dev/null)" || return 1
  grep -qxF "trusted comment: gnobbi build=$b" "$s" || return 1
  until="$(jq -r .valid_until "$m")"
  (( $(date -u -d "$until" +%s) > $(date -u +%s) )) || return 1
  return 0
}

if [[ "$MODE" == sign ]]; then
  M="${1:-}"; B="${2:-}"
  [[ -f "$M" && "$B" =~ ^[0-9]+$ ]] || die 2 "uso: publish-site.sh --sign <manifest> <build>"
  command -v jq >/dev/null || die 2 "jq mancante"
  sign_manifest "$M" "gnobbi build=$B"
  exit 0
fi
if [[ "$MODE" == verify ]]; then
  M="${1:-}"; S="${2:-}"
  [[ -f "$M" && -f "$S" ]] || die 2 "uso: publish-site.sh --verify <manifest> <minisig>"
  command -v jq >/dev/null || die 2 "jq mancante"
  verify_manifest "$M" "$S" || die 3 "verifica fallita: $M"
  exit 0
fi

APPIMAGE="${1:-${APPIMAGE:-}}"
BUILD="${2:-${BUILD:-}}"
COMMIT="${3:-${COMMIT:-}}"

[[ -f "$APPIMAGE" ]] || die 2 "AppImage non trovata (argomento 1)"
[[ "$BUILD" =~ ^[0-9]+$ ]] || die 2 "build number non intero (argomento 2)"
[[ "$COMMIT" =~ ^[0-9a-fA-F]{7,64}$ ]] || die 2 "commit SHA non valido (argomento 3)"
command -v jq >/dev/null || die 2 "jq mancante"

if [[ -f "$ENV_FILE" ]]; then
  # shellcheck disable=SC1090
  . "$ENV_FILE"
elif ((DRY)); then
  PUBLISH_URL_BASE="${PUBLISH_URL_BASE:-https://example.invalid/gnobbi}"
else
  die 2 "file di configurazione mancante: $ENV_FILE"
fi
if ((DRY)); then :; else
  for v in PUBLISH_HOST PUBLISH_USER PUBLISH_KEY PUBLISH_DIR PUBLISH_URL_BASE; do
    [[ -n "${!v:-}" ]] || die 2 "variabile $v mancante in $ENV_FILE"
  done
  [[ -f "$PUBLISH_KEY" ]] || die 2 "chiave PUBLISH_KEY non trovata"
  [[ "$(stat -c %a "$ENV_FILE")" == 600 ]] || die 2 "$ENV_FILE deve avere permessi 600"
fi
URL_BASE="${PUBLISH_URL_BASE%/}"

if [[ -z "${VERSION:-}" ]]; then
  root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
  VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -n1)"
  VERSION="${VERSION:-0.1.0}"
fi

sha256() { if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1; }
SHA="$(sha256 "$APPIMAGE")"
SIZE="$(wc -c <"$APPIMAGE" | tr -d ' ')"
NAME="Gnobbi-x86_64-$BUILD.AppImage"
STABLE="Gnobbi-x86_64.AppImage"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Manifesto compatto con \n finale (byte esatti che si firmeranno e caricheranno).
# valid_until = istante della firma + VALID_DAYS giorni (MANIFEST_SIGNING.md D5).
build_manifest() { # $1 = released (YYYY-MM-DDTHH:MM:SSZ)
  jq -nc --arg v "$VERSION" --argjson b "$BUILD" --arg c "$COMMIT" \
    --arg r "$1" --arg vu "$(date -u -d "+$VALID_DAYS days" +%Y-%m-%dT%H:%M:%SZ)" \
    --arg u "$URL_BASE/$NAME" --arg s "$SHA" --argjson z "$SIZE" \
    '{schema:1,version:$v,build:$b,commit:$c,released:$r,valid_until:$vu,channel:"stable",
      artifacts:{"linux-x86_64-appimage":{url:$u,sha256:$s,size:$z}}}' >"$TMP/latest.json"
}
build_manifest "$(date -u +%Y-%m-%dT%H:%M:%SZ)"

if ((DRY)); then cat "$TMP/latest.json"; exit 0; fi

fetch() { # url dest
  local rc=0
  curl -sSf --max-time 600 -o "$2" "$1" 2>/dev/null || rc=$?
  case $rc in
    0) ;;
    6|7|28|35|52|56) die 75 "sito pubblico irraggiungibile" ;;
    *) die 3 "download fallito (curl rc=$rc)" ;;
  esac
}

# Solo sftp (nessuna shell remota); gli errori vanno nel log temporaneo, mai a video (conterrebbero l'host).
sftp_run() { # stdin = comandi batch
  local rc=0
  sftp -b - -q -i "$PUBLISH_KEY" -o BatchMode=yes -o IdentitiesOnly=yes \
    -o StrictHostKeyChecking=yes -o ConnectTimeout=20 \
    "$PUBLISH_USER@$PUBLISH_HOST" >"$TMP/sftp.out" 2>"$TMP/sftp.err" || rc=$?
  if ((rc == 255)); then die 75 "sito/host SFTP irraggiungibile o autenticazione rifiutata"; fi
  ((rc == 0)) || { sed -e "s/$PUBLISH_HOST/<host>/g" -e "s/$PUBLISH_USER/<user>/g" "$TMP/sftp.err" >&2; die 4 "comando SFTP fallito (rc=$rc)"; }
}
D="$PUBLISH_DIR"

# Anti-regressione SOLO contro un remoto con firma valida (MANIFEST_SIGNING.md §5.5):
# un latest.json remoto assente, non firmato o non verificabile vale come assente.
code="$(curl -sS --max-time 60 -o "$TMP/pre.json" -w '%{http_code}' "$URL_BASE/latest.json" 2>/dev/null)" || {
  rc=$?; [[ $rc =~ ^(6|7|28|35|52|56)$ ]] && die 75 "sito pubblico irraggiungibile"; die 3 "controllo latest.json fallito (curl rc=$rc)"; }
[[ "$code" == 200 || "$code" == 404 ]] || die 3 "controllo latest.json: HTTP $code"
REMOTE_SIGNED=0
sigcode="$(curl -sS --max-time 60 -o "$TMP/pre.minisig" -w '%{http_code}' "$URL_BASE/latest.json.minisig" 2>/dev/null)" || sigcode=000
if [[ "$code" == 200 && "$sigcode" == 200 ]] && verify_manifest "$TMP/pre.json" "$TMP/pre.minisig"; then
  REMOTE_SIGNED=1
else
  echo "publish-site: latest.json remoto assente o non verificabile, trattato come assente"
fi
if ((REMOTE_SIGNED)); then
  REMOTE_BUILD="$(jq -er '.build' "$TMP/pre.json")" || die 3 "latest.json remoto non valido"
  if ((REMOTE_BUILD > BUILD)); then
    echo "publish-site: build remota $REMOTE_BUILD più recente di $BUILD, nessuna azione"
    exit 0
  fi
  if ((REMOTE_BUILD == BUILD)); then
    rem_left=$(( ($(date -u -d "$(jq -r .valid_until "$TMP/pre.json")" +%s) - $(date -u +%s)) / 86400 ))
    if ((rem_left >= RENEW_DAYS)); then
      echo "publish-site: build $BUILD già pubblicata, valida per altri ${rem_left}g, nessuna azione"
      exit 0
    fi
    # Rinnovo (§5.6): stessa build, stessi artefatti e stesso released, solo nuovo valid_until.
    [[ "$(jq -r '.artifacts["linux-x86_64-appimage"].sha256' "$TMP/pre.json")" == "$SHA" ]] ||
      die 3 "rinnovo: artefatto remoto diverso da quello locale"
    build_manifest "$(jq -r .released "$TMP/pre.json")"
    sign_manifest "$TMP/latest.json" "gnobbi build=$BUILD"
    sftp_run <<EOF
put "$TMP/latest.json.minisig" "$D/latest.json.minisig.tmp"
rename "$D/latest.json.minisig.tmp" "$D/latest.json.minisig"
put "$TMP/latest.json" "$D/latest.json.tmp"
rename "$D/latest.json.tmp" "$D/latest.json"
EOF
    fetch "$URL_BASE/latest.json" "$TMP/remote.json"
    fetch "$URL_BASE/latest.json.minisig" "$TMP/remote.minisig"
    verify_manifest "$TMP/remote.json" "$TMP/remote.minisig" || die 3 "rinnovo: verifica della firma pubblicata fallita"
    jq -e --argjson b "$BUILD" '.build==$b' "$TMP/remote.json" >/dev/null || die 3 "rinnovo: build remota diversa"
    echo "publish-site: valid_until della build $BUILD rinnovato (scadeva tra ${rem_left}g)"
    exit 0
  fi
fi

# Pubblicazione nuova: artefatti → firma → latest.json (§5.3).
sign_manifest "$TMP/latest.json" "gnobbi build=$BUILD"
# Il rename SFTP di OpenSSH usa posix-rename (sovrascrive in modo atomico) se il server lo espone.
sftp_run <<EOF
put "$APPIMAGE" "$D/$NAME.tmp"
rename "$D/$NAME.tmp" "$D/$NAME"
put "$APPIMAGE" "$D/$STABLE.tmp"
rename "$D/$STABLE.tmp" "$D/$STABLE"
put "$TMP/latest.json.minisig" "$D/latest.json.minisig.tmp"
rename "$D/latest.json.minisig.tmp" "$D/latest.json.minisig"
put "$TMP/latest.json" "$D/latest.json.tmp"
rename "$D/latest.json.tmp" "$D/latest.json"
EOF

# Retention: solo file che combaciano esattamente col pattern gestito (e i .tmp orfani di run cancellati); mai quello appena pubblicato.
echo "ls -1 \"$D\"" | sftp_run
mapfile -t OLD < <(sed 's#.*/##' "$TMP/sftp.out" | grep -E '^Gnobbi-x86_64-[0-9]+\.AppImage$' |
  sed -E 's/^Gnobbi-x86_64-([0-9]+)\.AppImage$/\1/' | sort -n | head -n -"$KEEP" || true)
mapfile -t TMPS < <(sed 's#.*/##' "$TMP/sftp.out" | grep -E '^Gnobbi-x86_64-[0-9]+\.AppImage\.tmp$' || true)
RM=()
for b in "${OLD[@]}"; do
  if [[ "$b" != "$BUILD" ]]; then RM+=("Gnobbi-x86_64-$b.AppImage"); fi
done
for t in "${TMPS[@]}"; do
  if [[ "$t" != "$NAME.tmp" ]]; then RM+=("$t"); fi
done
if ((${#RM[@]})); then
  for f in "${RM[@]}"; do echo "rm \"$D/$f\""; done | sftp_run
fi

# Verifica sull'URL pubblico: firma prima, poi contenuto e artefatti (MANIFEST_SIGNING.md §5.4).
fetch "$URL_BASE/latest.json" "$TMP/remote.json"
fetch "$URL_BASE/latest.json.minisig" "$TMP/remote.minisig"
verify_manifest "$TMP/remote.json" "$TMP/remote.minisig" ||
  die 3 "firma del latest.json pubblicato non valida (o scaduta)"
jq -e --argjson b "$BUILD" '.schema==1 and .build==$b' "$TMP/remote.json" >/dev/null ||
  die 3 "latest.json pubblicato non valido o build diversa"
M_URL="$(jq -r '.artifacts["linux-x86_64-appimage"].url' "$TMP/remote.json")"
M_SHA="$(jq -r '.artifacts["linux-x86_64-appimage"].sha256' "$TMP/remote.json")"
fetch "$M_URL" "$TMP/dl.AppImage"
[[ "$(sha256 "$TMP/dl.AppImage")" == "$M_SHA" && "$M_SHA" == "$SHA" ]] || die 3 "SHA-256 dell'AppImage versionata diverso dal manifesto"
fetch "$URL_BASE/$STABLE" "$TMP/dl2.AppImage"
[[ "$(sha256 "$TMP/dl2.AppImage")" == "$M_SHA" ]] || die 3 "SHA-256 del link stabile diverso dal manifesto"
echo "publish-site: build $BUILD pubblicata e verificata"
