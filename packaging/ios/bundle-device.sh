#!/bin/sh
# Unsigned Gnobbi.app for a real iPhone (aarch64-apple-ios, dist-ios profile, current SDK). Signing/IPA is a separate step.
# Usage (macOS with Rust + Xcode): packaging/ios/bundle-device.sh [target-dir]   -> target-dir/ios-device/Gnobbi.app
# Env: GNOBBI_BUILD (CFBundleVersion, default = commit count), GNOBBI_COMMIT (short SHA compiled into the app; set it when the tree has no .git), GNOBBI_VERSION (default 1.0)
set -eu
here=$(cd "$(dirname "$0")" && pwd); root=$(cd "$here/../.." && pwd)
t=${1:-target}; triple=aarch64-apple-ios
build=${GNOBBI_BUILD:-$(git -C "$root" rev-list --count HEAD)}; version=${GNOBBI_VERSION:-1.0}
# deployment target = MinimumOSVersion: the rustc default (10.0) lacks ___chkstk_darwin for aws-lc
IPHONEOS_DEPLOYMENT_TARGET=17.0 CARGO_TARGET_DIR=$t cargo build --locked --profile dist-ios -p matrix-desktop --bin gnobbi --target $triple --features ios-vault
app=$t/ios-device/Gnobbi.app; rm -rf "$app" "$t/ios-device/work"; mkdir -p "$app" "$t/ios-device/work/Assets.xcassets/AppIcon.appiconset"
cp "$t/$triple/dist-ios/gnobbi" "$app/gnobbi"; cp "$here/Info-device.plist" "$app/Info.plist"
w=$t/ios-device/work
# One 1024 opaque icon: Xcode 15+ derives every size from it (single-size AppIcon).
cp "$root/packaging/store/apple-1024.png" "$w/Assets.xcassets/AppIcon.appiconset/icon.png"
echo '{"images":[{"filename":"icon.png","idiom":"universal","platform":"ios","size":"1024x1024"}],"info":{"author":"xcode","version":1}}' > "$w/Assets.xcassets/AppIcon.appiconset/Contents.json"
echo '{"info":{"author":"xcode","version":1}}' > "$w/Assets.xcassets/Contents.json"
xcrun actool "$w/Assets.xcassets" --compile "$app" --platform iphoneos --minimum-deployment-target 17.0 --app-icon AppIcon \
  --target-device iphone --output-partial-info-plist "$w/icon.plist" >/dev/null
pb=/usr/libexec/PlistBuddy; p=$app/Info.plist
$pb -c "Set :CFBundleVersion $build" -c "Set :CFBundleShortVersionString $version" "$p"
$pb -c "Merge $w/icon.plist" "$p"
# DT* keys that App Store Connect reads from the build toolchain.
sdk=$(xcrun --sdk iphoneos --show-sdk-version); sdkbuild=$(xcrun --sdk iphoneos --show-sdk-build-version); xb=$(xcodebuild -version | sed -n 's/^Build version //p')
for kv in "DTPlatformName=iphoneos" "DTSDKName=iphoneos$sdk" "DTPlatformVersion=$sdk" "DTSDKBuild=$sdkbuild" "DTPlatformBuild=$sdkbuild" "DTXcodeBuild=$xb" "DTCompiler=com.apple.compilers.llvm.clang.1_0"; do
  $pb -c "Add :${kv%%=*} string ${kv#*=}" "$p"; done
plutil -lint "$p" >/dev/null; lipo -archs "$app/gnobbi" | grep -q arm64
# The separately built NSE is copied unsigned; signing is done inside-out later.
if [ -n "${GNOBBI_NSE_APPEX:-}" ]; then
  [ -d "$GNOBBI_NSE_APPEX" ] || { echo 'NSE bundle not found' >&2; exit 1; }
  mkdir -p "$app/PlugIns"
  cp -R "$GNOBBI_NSE_APPEX" "$app/PlugIns/GnobbiNSE.appex"
fi
echo "$app ($version, build $build)"
