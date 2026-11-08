#!/bin/sh
# Validate or upload Gnobbi.pkg to App Store Connect (TestFlight internal testing only; no review submission).
# Usage: packaging/macos/upload-testflight.sh <Gnobbi.pkg> [validate|upload]
# App Store Connect API key from the local file ~/.config/gnobbi-ci/asc.env (0600, never in the repo):
#   ASC_KEY_ID=...  ASC_ISSUER_ID=...  ASC_KEY_FILE=/path/AuthKey_<id>.p8  [GNOBBI_BUILD=...]
# Exit 3 with "NOT UPLOADED" when that file is missing, so CI can say so without failing the build.
set -eu
PKG=${1:?pkg}; MODE=${2:-validate}
CONF="${GNOBBI_ASC_ENV:-$HOME/.config/gnobbi-ci/asc.env}"
[ -f "$CONF" ] || { echo "NOT UPLOADED: $CONF missing" >&2; exit 3; }
. "$CONF"
[ -n "${ASC_KEY_ID:-}" ] && [ -n "${ASC_ISSUER_ID:-}" ] && [ -f "${ASC_KEY_FILE:-/nonexistent}" ] || { echo "NOT UPLOADED: incomplete key configuration" >&2; exit 3; }
D="$HOME/.appstoreconnect/private_keys"; mkdir -p "$D"; chmod 700 "$D"
KEEP=0; [ -f "$D/AuthKey_$ASC_KEY_ID.p8" ] && KEEP=1
cleanup() { [ "$KEEP" = 1 ] || rm -f "$D/AuthKey_$ASC_KEY_ID.p8"; }; trap cleanup EXIT
[ "$KEEP" = 1 ] || ( umask 077; cp "$ASC_KEY_FILE" "$D/AuthKey_$ASC_KEY_ID.p8" )
ACT=--validate-app; [ "$MODE" = upload ] && ACT=--upload-package
BUILD="${GNOBBI_BUILD:-$(git rev-list --count HEAD 2>/dev/null || echo 0)}"
rc=0
xcrun altool $ACT -f "$PKG" -t macos --apple-id "${GNOBBI_APPLE_ID:-6826654528}" --bundle-id net.meskatech.gnobbi \
  --bundle-version "$BUILD" --bundle-short-version-string "${GNOBBI_VERSION:-0.1.0}" \
  --apiKey "$ASC_KEY_ID" --apiIssuer "$ASC_ISSUER_ID" 2>&1 || rc=$?
[ "$rc" = 0 ] || exit "$rc"
# What to Test: elenco commit visibili (crates/matrix-desktop|matrix-domain)
# dall'ultimo upload riuscito (marker locale --state, fallback ultimi 15), stessa chiave ASC
# di altool (nessun nuovo segreto). Best-effort: un errore qui non invalida l'upload.
if [ "$MODE" = upload ]; then
  state="${GNOBBI_TF_STATE:-$HOME/.config/gnobbi-ci/last-upload-sha}"
  export ASC_KEY_ID ASC_ISSUER_ID ASC_KEY_FILE
  python3 "$(dirname -- "$0")/asc-whatsnew.py" --app "${GNOBBI_APPLE_ID:-6826654528}" --build "$BUILD" --state "$state" \
    || echo "::warning::note TestFlight non impostate (rc=$?)"
  mkdir -p "$(dirname -- "$state")" 2>/dev/null || true
  git rev-parse HEAD > "$state" 2>/dev/null || true
fi
exit 0
