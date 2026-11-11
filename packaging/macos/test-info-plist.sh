#!/bin/sh
# Renders the Info.plist heredoc of build-testflight.sh and checks it is valid and registers the matrix: scheme.
set -eu
d=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
BID=net.meskatech.gnobbi BUILD=1 VERSION=0.0.0
out=$(mktemp)
trap 'rm -f "$out"' EXIT
eval "cat <<P
$(awk '/^<\?xml/{on=1} /^P$/{exit} on{print}' "$d/build-testflight.sh")
P" > "$out"
plutil -lint "$out"
[ "$(plutil -extract CFBundleURLTypes.0.CFBundleURLSchemes.0 raw "$out")" = matrix ] || { echo "matrix scheme missing" >&2; exit 1; }
echo "ok: Info.plist registers matrix:"
