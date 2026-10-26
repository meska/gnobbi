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
xcrun altool $ACT -f "$PKG" -t macos --apple-id "${GNOBBI_APPLE_ID:-6826654528}" --bundle-id net.meskatech.gnobbi \
  --bundle-version "${GNOBBI_BUILD:-$(git rev-list --count HEAD)}" --bundle-short-version-string "${GNOBBI_VERSION:-0.1.0}" \
  --apiKey "$ASC_KEY_ID" --apiIssuer "$ASC_ISSUER_ID" 2>&1
