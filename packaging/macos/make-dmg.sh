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
cat > "$A/Contents/Info.plist" <<P
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleDevelopmentRegion</key><string>it</string><key>CFBundleLocalizations</key><array><string>it</string><string>en</string></array><key>CFBundleName</key><string>Gnobbi</string><key>CFBundleDisplayName</key><string>Gnobbi</string>
<key>CFBundleIdentifier</key><string>net.meskatech.gnobbi.egui</string><key>CFBundleExecutable</key><string>Gnobbi</string>
<key>CFBundleIconFile</key><string>Gnobbi</string><key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>$ver</string><key>CFBundleShortVersionString</key><string>$ver</string>
<key>NSHighResolutionCapable</key><true/><key>NSPrincipalClass</key><string>NSApplication</string>
<key>NSMicrophoneUsageDescription</key><string>Gnobbi usa il microfono per registrare le note vocali.</string>
</dict></plist>
P
# Ad-hoc signature only (arm64 needs one to run); no Developer ID, no notarization.
codesign --force --deep -s - "$A"
ln -s /Applications "$stage/Applications"
hdiutil create -volname Gnobbi -srcfolder "$stage" -ov -format UDZO "$out/Gnobbi-$ver-macos-arm64.dmg"
rm -rf "$stage"
