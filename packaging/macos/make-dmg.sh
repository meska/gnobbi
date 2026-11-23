#!/bin/sh
# Builds Gnobbi.app and a dmg from the release binary (cargo build --locked --release -p matrix-desktop --features mac-vault).
# Usage: packaging/macos/make-dmg.sh <version> <output-dir>
set -eu
ver=${1:?version}; out=${2:?output dir}
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
bin="${CARGO_TARGET_DIR:-$root/target}/release/gnobbi"
[ -x "$bin" ] || { echo "build first: cargo build --locked --release -p matrix-desktop --features mac-vault" >&2; exit 1; }
stage=$(mktemp -d); A="$stage/Gnobbi.app"
mkdir -p "$A/Contents/MacOS" "$A/Contents/Resources" "$out"
cp "$bin" "$A/Contents/MacOS/Gnobbi"
cp "$root/packaging/macos/Gnobbi.icns" "$A/Contents/Resources/"
"$root/packaging/macos/info-plist.sh" net.meskatech.gnobbi.egui "$ver" "$(git -C "$root" rev-list --count HEAD)" > "$A/Contents/Info.plist"
# Ad-hoc signature only (arm64 needs one to run); no Developer ID, no notarization.
codesign --force --deep -s - "$A"
ln -s /Applications "$stage/Applications"
hdiutil create -volname Gnobbi -srcfolder "$stage" -ov -format UDZO "$out/Gnobbi-$ver-macos-arm64.dmg"
rm -rf "$stage"
