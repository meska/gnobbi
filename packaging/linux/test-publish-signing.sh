#!/usr/bin/env bash
# Test casi negativi/di bordo della firma del manifesto (publish-site.sh). Richiede GNU coreutils,
# minisign, jq. Chiave efimera, nessun accesso di rete (curl e sftp sono finti). Exit 0 = tutto ok.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PS="$ROOT/packaging/linux/publish-site.sh"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
PASSPHRASE="pw-$RANDOM$RANDOM-canary"
FAILS=0; LOG="$T/all.log"; : >"$LOG"
ok()  { echo "PASS  $1"; }
bad() { echo "FAIL  $1"; FAILS=$((FAILS + 1)); }
# expect <desc> <rc-atteso|nz> -- cmd...   (nz = qualunque exit != 0)
expect() { local d="$1" w="$2"; shift 3; "$@" >>"$LOG" 2>&1; local rc=$?
  if [[ "$w" == nz ]]; then ((rc != 0)) && ok "$d (rc=$rc)" || bad "$d: accettato"
  else ((rc == w)) && ok "$d (rc=$rc)" || bad "$d: rc=$rc atteso $w"; fi; }

mkkey() { printf '%s\n%s\n' "$3" "$3" | minisign -G -f -s "$1.key" -p "$1.pub" >/dev/null 2>&1
  printf '%s\n' "$3" >"$1.pass"; chmod 600 "$1.pass"; }
mkkey "$T/k1" _ "$PASSPHRASE"; mkkey "$T/k2" _ "$PASSPHRASE"
export GNOBBI_UPDATE_KEYS="$T/k1.pub" SIGN_KEY="$T/k1.key" SIGN_PASSPHRASE_FILE="$T/k1.pass"

gen() { # <file> <build> <valid_until> [sha] [size]
  jq -nc --argjson b "$2" --arg vu "$3" --arg s "${4:-$(printf 'a%.0s' {1..64})}" --argjson z "${5:-1000}" \
    '{schema:1,version:"0.0.0",build:$b,commit:"0000000000000000000000000000000000000000",
      released:"2026-01-01T00:00:00Z",valid_until:$vu,channel:"stable",
      artifacts:{"linux-x86_64-appimage":{url:"https://x.invalid/a",sha256:$s,size:$z}}}' >"$1"; }
FUT="$(date -u -d '+30 days' +%Y-%m-%dT%H:%M:%SZ)"
V() { "$PS" --verify "$@"; }

echo "== verifica: caso positivo e negativi =="
gen "$T/m.json" 7 "$FUT"; "$PS" --sign "$T/m.json" 7
expect "firma buona accettata" 0 -- V "$T/m.json" "$T/m.json.minisig"
# firma alterata di 1 byte (riga 2 = firma del file; riga 4 = firma del trusted comment)
for ln in 2 4; do
  cp "$T/m.json.minisig" "$T/s.minisig"
  python3 - "$T/s.minisig" "$ln" <<'PY'
import sys,re
p,ln=sys.argv[1],int(sys.argv[2])-1
L=open(p).read().split("\n"); s=L[ln]; i=len(s)//2
L[ln]=s[:i]+("B" if s[i]!="B" else "C")+s[i+1:]
open(p,"w").write("\n".join(L))
PY
  expect "1 byte alterato nella riga $ln della firma" nz -- V "$T/m.json" "$T/s.minisig"
done
# manifesto alterato dopo la firma
mut() { jq -c "$1" "$T/m.json" >"$T/mut.json"; V "$T/mut.json" "$T/m.json.minisig"; }
expect "manifesto: sha256 alterato" nz -- mut '.artifacts["linux-x86_64-appimage"].sha256="b"+.artifacts["linux-x86_64-appimage"].sha256[1:]'
expect "manifesto: size alterata" nz -- mut '.artifacts["linux-x86_64-appimage"].size=1'
expect "manifesto: build alterata" nz -- mut '.build=99'
expect "manifesto: valid_until esteso" nz -- mut '.valid_until="2099-01-01T00:00:00Z"'
expect "manifesto: byte in coda (newline tolto)" nz -- bash -c "head -c -1 '$T/m.json' >'$T/nn.json'; '$PS' --verify '$T/nn.json' '$T/m.json.minisig'"
# firma di un'altra chiave / key id sconosciuto
SIGN_KEY="$T/k2.key" SIGN_PASSPHRASE_FILE="$T/k2.pass" "$PS" --sign "$T/m.json" 7 && cp "$T/m.json.minisig" "$T/k2.minisig"
"$PS" --sign "$T/m.json" 7
expect "firma di un'altra chiave (key id sconosciuto)" nz -- V "$T/m.json" "$T/k2.minisig"
cp "$T/m.json.minisig" "$T/id.minisig"; sed -i '2s/^\(..\)./\1A/' "$T/id.minisig"
expect "key id nell'header alterato" nz -- V "$T/m.json" "$T/id.minisig"
expect "chiave attendibile = k2, firma k1" nz -- env GNOBBI_UPDATE_KEYS="$T/k2.pub" "$PS" --verify "$T/m.json" "$T/m.json.minisig"
# file chiavi: 1 e 2 chiavi (firma dell'ultima) accettate; 3 chiavi rifiutate, anche con firma estranea
mkkey "$T/k3" _ "$PASSPHRASE"; mkkey "$T/k4" _ "$PASSPHRASE"
cat "$T/k2.pub" "$T/k1.pub" >"$T/two.pub"; cat "$T/k2.pub" "$T/k3.pub" "$T/k1.pub" >"$T/three.pub"
cp "$T/m.json.minisig" "$T/k1.minisig"
expect "2 chiavi, firma della seconda accettata" 0 -- env GNOBBI_UPDATE_KEYS="$T/two.pub" "$PS" --verify "$T/m.json" "$T/k1.minisig"
expect "3 chiavi, firma valida della terza rifiutata" 3 -- env GNOBBI_UPDATE_KEYS="$T/three.pub" "$PS" --verify "$T/m.json" "$T/k1.minisig"
SIGN_KEY="$T/k4.key" SIGN_PASSPHRASE_FILE="$T/k4.pass" "$PS" --sign "$T/m.json" 7 && cp "$T/m.json.minisig" "$T/k4.minisig"
cp "$T/k1.minisig" "$T/m.json.minisig"
cat "$T/k1.pub" "$T/k2.pub" "$T/k3.pub" >"$T/three.pub"
expect "3 chiavi, firma estranea rifiutata" 3 -- env GNOBBI_UPDATE_KEYS="$T/three.pub" "$PS" --verify "$T/m.json" "$T/k4.minisig"
# firma mancante / vuota / manifesto vuoto
expect "firma mancante (404)" nz -- V "$T/m.json" "$T/nope.minisig"
: >"$T/empty.minisig"
expect "firma vuota" nz -- V "$T/m.json" "$T/empty.minisig"
printf '' >"$T/e.json"; expect "manifesto vuoto" nz -- V "$T/e.json" "$T/m.json.minisig"
# contratto: scaduto, trusted comment di altra build, schema, formato date
gen "$T/x.json" 7 2020-01-01T00:00:00Z; "$PS" --sign "$T/x.json" 7
expect "valid_until scaduto" 3 -- V "$T/x.json" "$T/x.json.minisig"
gen "$T/b.json" 8 "$FUT"; "$PS" --sign "$T/b.json" 7
expect "trusted comment build != manifest.build" 3 -- V "$T/b.json" "$T/b.json.minisig"
jq -c '.schema=2' "$T/m.json" >"$T/s2.json"; "$PS" --sign "$T/s2.json" 7
expect "schema != 1 (firmato bene)" 3 -- V "$T/s2.json" "$T/s2.json.minisig"
jq -c '.valid_until="2099-01-01"' "$T/m.json" >"$T/d.json"; "$PS" --sign "$T/d.json" 7
expect "valid_until senza formato RFC3339 Z" 3 -- V "$T/d.json" "$T/d.json.minisig"
jq -c '.build="7"' "$T/m.json" >"$T/bs.json"; "$PS" --sign "$T/bs.json" 7
expect "build stringa invece di numero" 3 -- V "$T/bs.json" "$T/bs.json.minisig"

echo "== comando di firma sostituito (SIGN_CMD) =="
cat >"$T/fakesign" <<EOF
#!/usr/bin/env bash
printf '%s\n%s\n' "\$1" "\$2" >"$T/fakesign.args"
printf '%s\n' "$PASSPHRASE" | minisign -S -s "$T/k1.key" -m "\$1" -x "\$1.minisig" -t "\$2" >/dev/null 2>&1
EOF
chmod +x "$T/fakesign"; gen "$T/c.json" 9 "$FUT"
expect "SIGN_CMD senza chiave/passphrase reali" 0 -- env SIGN_CMD="$T/fakesign" SIGN_KEY=/nonexistent SIGN_PASSPHRASE_FILE=/nonexistent "$PS" --sign "$T/c.json" 9
expect "firma da SIGN_CMD verificabile" 0 -- V "$T/c.json" "$T/c.json.minisig"
[[ "$(sed -n 2p "$T/fakesign.args")" == "gnobbi build=9" ]] && ok "SIGN_CMD riceve <manifest> <trusted-comment>" || bad "argomenti SIGN_CMD"
printf '#!/bin/sh\nexit 0\n' >"$T/nosign"; chmod +x "$T/nosign"
cp "$T/c.json" "$T/c2.json"; rm -f "$T/c2.json.minisig"
expect "SIGN_CMD che non scrive .minisig rifiutato" 2 -- env SIGN_CMD="$T/nosign" "$PS" --sign "$T/c2.json" 9
expect "passphrase file con permessi 644 rifiutato" 2 -- bash -c "chmod 644 '$T/k1.pass'; '$PS' --sign '$T/c2.json' 9; rc=\$?; chmod 600 '$T/k1.pass'; exit \$rc"

echo "== pubblicazione con curl/sftp finti: anti-regressione e rinnovo =="
SITE="$T/site"; BIN="$T/bin"; mkdir -p "$SITE" "$BIN"
cat >"$BIN/curl" <<'EOF'
#!/usr/bin/env bash
out=""; wfmt=""; url=""
while (($#)); do case "$1" in -o) out="$2"; shift 2;; -w) wfmt="$2"; shift 2;; -*) shift;; *) url="$1"; shift;; esac; done
f="$SITE/$(basename "$url")"
if [[ -f "$f" ]]; then cp "$f" "$out"; code=200; else : >"$out"; code=404; fi
[[ -n "$wfmt" ]] && { printf '%s' "$code"; exit 0; }
[[ $code == 200 ]] || exit 22
EOF
cat >"$BIN/sftp" <<'EOF'
#!/usr/bin/env bash
while IFS= read -r line; do eval "set -- $line"; cmd="$1"; shift
  case "$cmd" in
    put) cp "$1" "$2"; [[ -n "${CORRUPT_SIG:-}" && "$2" == *latest.json.minisig.tmp ]] && sed -i '2{ /^.\{20\}Z/s/^\(.\{20\}\)./\1A/; t; s/^\(.\{20\}\)./\1Z/; }' "$2";;
    rename) mv "$1" "$2";; rm) rm -f "$1";; ls) ls -1 "$SITE";;
  esac; done
EOF
chmod +x "$BIN/curl" "$BIN/sftp"
printf 'img' >"$T/app.AppImage"; : >"$T/pk"
cat >"$T/pub.env" <<EOF
PUBLISH_HOST=h; PUBLISH_USER=u; PUBLISH_KEY=$T/pk; PUBLISH_DIR=$SITE; PUBLISH_URL_BASE=https://site.invalid/g
EOF
chmod 600 "$T/pub.env"
pub() { PATH="$BIN:$PATH" SITE="$SITE" GNOBBI_PUBLISH_ENV="$T/pub.env" "$PS" "$T/app.AppImage" "$1" abcdef1; }
reset() { rm -rf "$SITE"; mkdir -p "$SITE"; }
remote() { gen "$SITE/latest.json" "$1" "$2"; "$PS" --sign "$SITE/latest.json" "$1"; }
bld() { jq -r .build "$SITE/latest.json"; }

reset; expect "sito vuoto: pubblica build 5" 0 -- pub 5
V "$SITE/latest.json" "$SITE/latest.json.minisig" && [[ "$(bld)" == 5 ]] && ok "manifesto pubblicato firmato, build 5" || bad "manifesto pubblicato"
cp "$SITE/latest.json" "$T/before.json"
remote 6 "$FUT"; cp "$SITE/latest.json" "$T/before.json"
expect "remoto build 6 > 5: nessuna azione" 0 -- pub 5
cmp -s "$SITE/latest.json" "$T/before.json" && ok "build inferiore non sovrascrive il remoto" || bad "build inferiore ha sovrascritto"
remote 5 "$FUT"; cp "$SITE/latest.json" "$T/before.json"
expect "remoto build 5 uguale e valido: nessuna azione" 0 -- pub 5
cmp -s "$SITE/latest.json" "$T/before.json" && ok "build uguale non riscritta" || bad "build uguale riscritta"
reset; gen "$SITE/latest.json" 99 "$FUT"
expect "remoto build 99 SENZA firma (404 minisig): trattato come assente" 0 -- pub 5
[[ "$(bld)" == 5 ]] && ok "manifesto non firmato non blocca la pubblicazione" || bad "manifesto non firmato ha vinto"
reset; remote 99 2020-01-01T00:00:00Z
expect "remoto 99 firmato ma scaduto: trattato come assente" 0 -- pub 5
[[ "$(bld)" == 5 ]] && ok "manifesto remoto scaduto non blocca" || bad "remoto scaduto ha vinto"
reset; gen "$SITE/latest.json" 5 "$(date -u -d '+3 days' +%Y-%m-%dT%H:%M:%SZ)" "$(sha256sum "$T/app.AppImage" | cut -d' ' -f1)" 3
"$PS" --sign "$SITE/latest.json" 5; printf 'img' >"$SITE/Gnobbi-x86_64-5.AppImage"
expect "stessa build con meno di 10 giorni: rinnovo" 0 -- pub 5
V "$SITE/latest.json" "$SITE/latest.json.minisig" && [[ "$(jq -r .valid_until "$SITE/latest.json")" > "$(date -u -d '+20 days' +%Y-%m-%dT%H:%M:%SZ)" ]] && ok "valid_until rinnovato e firmato" || bad "rinnovo"
reset; expect "firma pubblicata corrotta: verifica post-upload rifiuta" 3 -- env CORRUPT_SIG=1 PATH="$BIN:$PATH" SITE="$SITE" GNOBBI_PUBLISH_ENV="$T/pub.env" "$PS" "$T/app.AppImage" 5 abcdef1
reset; expect "firma di un'altra chiave pubblicata: verifica rifiuta" 3 -- env GNOBBI_UPDATE_KEYS="$T/k2.pub" PATH="$BIN:$PATH" SITE="$SITE" GNOBBI_PUBLISH_ENV="$T/pub.env" "$PS" "$T/app.AppImage" 5 abcdef1

echo "== segreti =="
! grep -rqF "$PASSPHRASE" "$LOG" && ok "passphrase assente dall'output dei comandi" || bad "passphrase nell'output"
! grep -q 'secret key' "$LOG" && ok "nessuna chiave privata nell'output" || bad "chiave privata nell'output"
# minisign cifra la chiave privata con header base64 'RWRTY0Iy' (algoritmo Ed + scrypt)
if git -C "$ROOT" rev-parse >/dev/null 2>&1; then
  ! git -C "$ROOT" grep -qE 'RWRTY0Iy|untrusted comment: minisign encrypted secret key' $(git -C "$ROOT" rev-list --all | head -500) -- . ':!packaging/linux/test-publish-signing.sh' 2>/dev/null \
    && ok "nessuna chiave privata minisign nel repo (storia inclusa)" || bad "chiave privata nel repo"
  ! git -C "$ROOT" grep -qiE '(SIGN_)?PASSPHRASE *= *[^ $"]' -- packaging .gitea ':!packaging/linux/test-publish-signing.sh' 2>/dev/null \
    && ok "nessuna passphrase letterale nel repo" || bad "passphrase letterale nel repo"
fi
ps_argv="$(grep -n 'minisign -S' "$PS")"; [[ "$ps_argv" != *"$PASSPHRASE"* && "$ps_argv" == *'<"$SIGN_PASSPHRASE_FILE"'* ]] && ok "passphrase solo via stdin" || bad "passphrase in argv"

echo; ((FAILS == 0)) && { echo "TUTTI I CASI OK"; exit 0; } || { echo "$FAILS FALLITI"; exit 1; }
