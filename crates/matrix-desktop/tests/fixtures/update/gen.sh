#!/bin/sh
# Regenerates the update-verify fixtures with the minisign CLI (>= 0.8). Test keys only:
# secret keys live in a temp dir that is deleted on exit; never commit them.
# Run from anywhere: sh gen.sh   (output replaces the committed files; keys change each run,
# so rerun only when a case is added, and commit everything together).
set -eu
cd "$(dirname "$0")"
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT

for k in current next other; do
  minisign -G -W -f -p "$k.pub" -s "$T/$k.key" >/dev/null
done
cat current.pub next.pub > keys.pub   # the keys file embedded in the client: [current, next]

# 1000 deterministic bytes standing in for the AppImage.
i=0; : > artifact.bin
while [ $i -lt 100 ]; do printf 'gnobbi-test\n' | head -c 10 >> artifact.bin; i=$((i+1)); done
SIZE=$(wc -c < artifact.bin | tr -d ' ')
SHA=$(shasum -a 256 artifact.bin | cut -d' ' -f1)

URL=https://gnobbi.meskatech.net/download/Gnobbi-x86_64-42.AppImage
# mk <name> [build] [valid_until] [url] [schema] [artifacts-json]
mk() {
  b=${2:-42}; v=${3:-2026-12-05T18:40:00Z}; u=${4:-$URL}; s=${5:-1}
  a=${6:-"{\"linux-x86_64-appimage\":{\"url\":\"$u\",\"sha256\":\"$SHA\",\"size\":$SIZE}}"}
  printf '{"schema":%s,"version":"0.1.0","build":%s,"commit":"9f2c1d0a4b6e8c3a5d7f1b2c4e6a8d0f2b4c6e8a","released":"2026-11-05T18:40:00Z","valid_until":"%s","channel":"stable","artifacts":%s}\n' \
    "$s" "$b" "$v" "$a" > "$1.json"
}
# sign <name> <key> [comment] [out]  -> <name>.minisig (default) signing <name>.json
sign() {
  minisign -S -s "$T/$2.key" -m "$1.json" -x "${4:-$1.minisig}" -t "${3:-gnobbi build=42}" >/dev/null < /dev/null
}

mk valid;                      sign valid current
mk lower 41;                   sign lower current "gnobbi build=41"
sign valid next "gnobbi build=42" valid.next.minisig     # rotation: next key signs
sign valid other "gnobbi build=42" valid.other.minisig   # unknown key
minisign -S -l -s "$T/current.key" -m valid.json -x valid.legacy.minisig -t "gnobbi build=42" >/dev/null </dev/null

mk expired 42 2026-11-01T00:00:00Z;                       sign expired current
mk schema2 42 2026-12-05T18:40:00Z "$URL" 2;              sign schema2 current
mk http 42 2026-12-05T18:40:00Z http://gnobbi.meskatech.net/download/G.AppImage; sign http current
mk otherhost 42 2026-12-05T18:40:00Z https://evil.example.org/download/G.AppImage; sign otherhost current
mk dotdot 42 2026-12-05T18:40:00Z https://gnobbi.meskatech.net/download/..;       sign dotdot current
mk percent 42 2026-12-05T18:40:00Z https://gnobbi.meskatech.net/download/G%2e.AppImage; sign percent current
mk noartifact 42 2026-12-05T18:40:00Z "$URL" 1 "{\"windows-x64-zip\":{\"url\":\"https://gnobbi.meskatech.net/download/G.zip\",\"sha256\":\"$SHA\",\"size\":$SIZE}}"; sign noartifact current
mk missing_sha 42 2026-12-05T18:40:00Z "$URL" 1 "{\"linux-x86_64-appimage\":{\"url\":\"$URL\",\"size\":$SIZE}}"; sign missing_sha current
printf 'this is not json\n' > notjson.json;                       sign notjson current
mk comment_mismatch;           sign comment_mismatch current "gnobbi build=43"
mk comment_format;             sign comment_format current "build=42"
mk comment_zero;               sign comment_zero current "gnobbi build=042"
cp current.pub floor.pub
cp noartifact.json floor-high.json
cp noartifact.minisig floor-high.minisig
echo ok
