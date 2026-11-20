#!/bin/sh
# Mac App Store / TestFlight package of Gnobbi from the dist binary
# (cargo build --locked -p matrix-desktop --features mac-vault --profile dist).
# Needs, in the login keychain of the user running it (a GUI-session user, e.g. the CI LaunchAgent):
#   "Apple Distribution", "3rd Party Mac Developer Installer" identities and a Mac App Store provisioning
#   profile named "Gnobbi Mac App Store" in ~/Library/MobileDevice/Provisioning Profiles.
# Usage: packaging/macos/build-testflight.sh [output-dir]       -> output-dir/Gnobbi.app (signed, sandboxed) and Gnobbi.pkg
# Env: GNOBBI_BUILD (integer, default = commit count), GNOBBI_VERSION (default 0.1.0)
set -eu
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
OUT=${1:-$root/artifacts/macos}
TEAM=T4RJLVSR79; BID=net.meskatech.gnobbi; APPID="$TEAM.$BID"
VERSION=${GNOBBI_VERSION:-0.1.0}
BUILD=${GNOBBI_BUILD:-$(git -C "$root" rev-list --count HEAD)}
bin="${CARGO_TARGET_DIR:-$root/target}/dist/gnobbi"
[ -x "$bin" ] || { echo "build first: cargo build --locked -p matrix-desktop --features mac-vault --profile dist" >&2; exit 1; }
lipo -archs "$bin" | grep -q arm64 || { echo "binary is not arm64" >&2; exit 1; }
PROFILE=$(grep -l -a "Gnobbi Mac App Store" "$HOME/Library/MobileDevice/Provisioning Profiles/"*.provisionprofile 2>/dev/null | head -1 || true)
[ -n "$PROFILE" ] || { echo "provisioning profile 'Gnobbi Mac App Store' missing" >&2; exit 2; }
rm -rf "$OUT"; mkdir -p "$OUT"
APP="$OUT/Gnobbi.app"; mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$bin" "$APP/Contents/MacOS/Gnobbi"
cat > "$APP/Contents/Info.plist" <<P
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Gnobbi</string>
<key>CFBundleDisplayName</key><string>Gnobbi</string>
<key>CFBundleIdentifier</key><string>$BID</string>
<key>CFBundleExecutable</key><string>Gnobbi</string>
<key>CFBundleIconFile</key><string>Gnobbi</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>$BUILD</string>
<key>CFBundleShortVersionString</key><string>$VERSION</string>
<key>LSMinimumSystemVersion</key><string>12.0</string>
<key>LSApplicationCategoryType</key><string>public.app-category.social-networking</string>
<key>ITSAppUsesNonExemptEncryption</key><false/>
<key>NSHighResolutionCapable</key><true/>
<key>NSMicrophoneUsageDescription</key><string>Gnobbi usa il microfono per registrare le note vocali.</string>
<key>CFBundleURLTypes</key><array><dict>
<key>CFBundleURLName</key><string>Matrix</string>
<key>CFBundleURLSchemes</key><array><string>matrix</string></array>
</dict></array>
<key>NSPrincipalClass</key><string>NSApplication</string>
</dict></plist>
P
cp "$PROFILE" "$APP/Contents/embedded.provisionprofile"
cp "$root/packaging/macos/Gnobbi.icns" "$APP/Contents/Resources/Gnobbi.icns"
cp "$root/THIRD_PARTY_NOTICES.md" "$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$APP/Contents/Resources/"
cat > "$OUT/app.entitlements" <<E2
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>com.apple.security.app-sandbox</key><true/>
<key>com.apple.security.files.user-selected.read-write</key><true/>
<key>com.apple.security.network.client</key><true/>
<key>com.apple.security.device.audio-input</key><true/>
<key>com.apple.application-identifier</key><string>$APPID</string>
<key>com.apple.developer.team-identifier</key><string>$TEAM</string>
<key>keychain-access-groups</key><array><string>$APPID</string></array>
</dict></plist>
E2
# a restrictive umask (service accounts) would leave root-only files in the bundle: App Store Connect rejects it (90255)
chmod -R u+rwX,go+rX "$APP"
codesign --force --timestamp -s "Apple Distribution" --entitlements "$OUT/app.entitlements" "$APP"
codesign --verify --strict --deep --verbose=2 "$APP"
productbuild --component "$APP" /Applications --sign "3rd Party Mac Developer Installer" "$OUT/Gnobbi.pkg"
echo "built Gnobbi $VERSION ($BUILD): $OUT/Gnobbi.pkg"
