#!/usr/bin/env bash
# Pubblica l'AppImage sul sito via SFTP (tmp + rename) e aggiorna latest.json per ultimo.
# Uso: publish-site.sh [--dry-run] <appimage> <build> <commit-sha>
#   (in alternativa env APPIMAGE, BUILD, COMMIT). Config: ~/.config/gnobbi-ci/publish.env
#   (override: GNOBBI_PUBLISH_ENV) con PUBLISH_HOST, PUBLISH_USER, PUBLISH_KEY, PUBLISH_DIR,
#   PUBLISH_URL_BASE. Versione testuale: VERSION, altrimenti Cargo.toml del workspace.
# Exit: 0 ok, 2 config/argomenti, 3 verifica fallita, 4 errore upload, 75 sito/host irraggiungibile.
set -euo pipefail

KEEP=3
ENV_FILE="${GNOBBI_PUBLISH_ENV:-${HOME}/.config/gnobbi-ci/publish.env}"
DRY=0
if [[ "${1:-}" == "--dry-run" ]]; then DRY=1; shift; fi
APPIMAGE="${1:-${APPIMAGE:-}}"
BUILD="${2:-${BUILD:-}}"
COMMIT="${3:-${COMMIT:-}}"

die() { local rc=$1; shift; echo "publish-site: $*" >&2; exit "$rc"; }

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
jq -n --arg v "$VERSION" --argjson b "$BUILD" --arg c "$COMMIT" \
  --arg r "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --arg u "$URL_BASE/$NAME" \
  --arg s "$SHA" --argjson z "$SIZE" \
  '{schema:1,version:$v,build:$b,commit:$c,released:$r,channel:"stable",
    artifacts:{"linux-x86_64-appimage":{url:$u,sha256:$s,size:$z}}}' >"$TMP/latest.json"

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

# Mai far regredire il manifesto: se il sito ha già questa build o una successiva, non si fa nulla.
code="$(curl -sS --max-time 60 -o "$TMP/pre.json" -w '%{http_code}' "$URL_BASE/latest.json" 2>/dev/null)" || {
  rc=$?; [[ $rc =~ ^(6|7|28|35|52|56)$ ]] && die 75 "sito pubblico irraggiungibile"; die 3 "controllo latest.json fallito (curl rc=$rc)"; }
if [[ "$code" == 200 ]]; then
  REMOTE_BUILD="$(jq -er '.build | select(type=="number")' "$TMP/pre.json")" || die 3 "latest.json remoto non valido"
  if ((REMOTE_BUILD >= BUILD)); then
    echo "publish-site: build $BUILD già pubblicata o superata (remota: $REMOTE_BUILD), nessuna azione"
    exit 0
  fi
elif [[ "$code" != 404 ]]; then
  die 3 "controllo latest.json: HTTP $code"
fi

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
# Il rename SFTP di OpenSSH usa posix-rename (sovrascrive in modo atomico) se il server lo espone.
sftp_run <<EOF
put "$APPIMAGE" "$D/$NAME.tmp"
rename "$D/$NAME.tmp" "$D/$NAME"
put "$APPIMAGE" "$D/$STABLE.tmp"
rename "$D/$STABLE.tmp" "$D/$STABLE"
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

# Verifica sull'URL pubblico.
fetch "$URL_BASE/latest.json" "$TMP/remote.json"
jq -e --argjson b "$BUILD" '.schema==1 and .build==$b' "$TMP/remote.json" >/dev/null ||
  die 3 "latest.json pubblicato non valido o build diversa"
M_URL="$(jq -r '.artifacts["linux-x86_64-appimage"].url' "$TMP/remote.json")"
M_SHA="$(jq -r '.artifacts["linux-x86_64-appimage"].sha256' "$TMP/remote.json")"
fetch "$M_URL" "$TMP/dl.AppImage"
[[ "$(sha256 "$TMP/dl.AppImage")" == "$M_SHA" && "$M_SHA" == "$SHA" ]] || die 3 "SHA-256 dell'AppImage versionata diverso dal manifesto"
fetch "$URL_BASE/$STABLE" "$TMP/dl2.AppImage"
[[ "$(sha256 "$TMP/dl2.AppImage")" == "$M_SHA" ]] || die 3 "SHA-256 del link stabile diverso dal manifesto"
echo "publish-site: build $BUILD pubblicata e verificata"
