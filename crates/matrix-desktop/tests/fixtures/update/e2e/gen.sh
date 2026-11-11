#!/bin/sh
# Fixtures for update/e2e_tests.rs: signed manifests + a fake artifact whose --self-test works.
# Throw-away test keys only; the secret keys are deleted on exit. Rerun => commit everything.
set -eu
cd "$(dirname "$0")"
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
for k in e2e other; do minisign -G -W -f -p "$k.pub" -s "$T/$k.key" >/dev/null; done
# build 42 = good; 43 = manifest hash wrong; 44 = manifest `size` smaller than the real file
printf '#!/bin/sh\necho "gnobbi build=42"\n' > artifact.sh
SIZE=$(wc -c < artifact.sh | tr -d ' ')
SHA=$(shasum -a 256 artifact.sh | cut -d' ' -f1)
BAD=$(printf x | shasum -a 256 | cut -d' ' -f1)
mk() { # name build sha size
  printf '{"schema":1,"version":"0.1.0","build":%s,"commit":"9f2c1d0a4b6e8c3a5d7f1b2c4e6a8d0f2b4c6e8a","released":"2026-11-05T18:40:00Z","valid_until":"2026-12-05T18:40:00Z","channel":"stable","artifacts":{"linux-x86_64-appimage":{"url":"https://gnobbi.meskatech.net/download/Gnobbi-x86_64-%s.AppImage","sha256":"%s","size":%s}}}\n' "$2" "$2" "$3" "$4" > "$1.json"
}
sign() { minisign -S -s "$T/$2.key" -m "$1.json" -x "${4:-$1.minisig}" -t "gnobbi build=$3" >/dev/null </dev/null; }
mk ok 42 "$SHA" "$SIZE";      sign ok e2e 42; sign ok other 42 ok.other.minisig
mk badhash 43 "$BAD" "$SIZE"; sign badhash e2e 43
mk small 44 "$SHA" 10;        sign small e2e 44
echo ok
