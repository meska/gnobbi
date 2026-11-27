#!/bin/sh
# Renders the Info.plist and checks it is valid, registers the matrix: scheme and keeps the About panel fields.
set -eu
d=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
out=$(mktemp); trap 'rm -f "$out"' EXIT
"$d/info-plist.sh" net.meskatech.gnobbi 0.0.0 7 abc1234 '01/02/2026 03:04' > "$out"
plutil -lint "$out"
chk() { [ "$(plutil -extract "$1" raw "$out")" = "$2" ] || { echo "$1 != $2" >&2; exit 1; }; }
chk CFBundleURLTypes.0.CFBundleURLSchemes.0 matrix
chk CFBundleVersion 7; chk CFBundleShortVersionString 0.0.0; chk GnobbiCommit abc1234
chk GnobbiBuilt '01/02/2026 03:04'; chk NSHumanReadableCopyright '© 2026 Meskatech'
"$d/info-plist.sh" x 1 notint >/dev/null 2>&1 && { echo "non-integer build accepted" >&2; exit 1; }
echo "ok: Info.plist"
