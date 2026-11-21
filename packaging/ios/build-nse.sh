#!/bin/sh
# Unsigned GnobbiNSE.appex (notification service extension): Rust core (crates/gnobbi-nse) + thin Swift shell (packaging/ios/nse).
# No Xcode project (like the app: packaging/ios/bundle-*.sh). Signing is a separate step; bundle-*.sh embeds it when GNOBBI_NSE_APPEX is set.
# Usage (macOS with Rust + Xcode): packaging/ios/build-nse.sh [target-dir] [device|sim]   -> target-dir/ios-nse/<device|sim>/GnobbiNSE.appex
# Env: GNOBBI_NSE_ENT (sim only: entitlements plist put in a __TEXT section, the way the simulator reads them), GNOBBI_BUILD (CFBundleVersion, default = commit count), GNOBBI_VERSION (default 1.0): must equal the app's, the signer checks.
set -eu
here=$(cd "$(dirname "$0")" && pwd); root=$(cd "$here/../.." && pwd)
t=${1:-target}; kind=${2:-device}
build=${GNOBBI_BUILD:-$(git -C "$root" rev-list --count HEAD)}; version=${GNOBBI_VERSION:-1.0}
case $kind in
  device) triple=aarch64-apple-ios; sdk=iphoneos; swift_target=arm64-apple-ios17.0; platform=iPhoneOS;;
  sim) triple=aarch64-apple-ios-sim; sdk=iphonesimulator; swift_target=arm64-apple-ios17.0-simulator; platform=iPhoneSimulator;;
  *) echo "kind: device|sim" >&2; exit 1;;
esac
# same deployment target as the app (the rustc default lacks ___chkstk_darwin for aws-lc)
IPHONEOS_DEPLOYMENT_TARGET=17.0 CARGO_TARGET_DIR=$t cargo build --locked --profile dist-ios -p gnobbi-nse --target $triple
out=$t/ios-nse/$kind; appex=$out/GnobbiNSE.appex; rm -rf "$out"; mkdir -p "$appex"
entflags=; [ -z "${GNOBBI_NSE_ENT:-}" ] || entflags="-Xlinker -sectcreate -Xlinker __TEXT -Xlinker __entitlements -Xlinker $GNOBBI_NSE_ENT"
sdkpath=$(xcrun --sdk $sdk --show-sdk-path)
xcrun swiftc -O -target $swift_target -sdk "$sdkpath" -module-name GnobbiNSE -parse-as-library -application-extension \
  -import-objc-header "$here/nse/gnobbi_nse.h" "$here/nse/NotificationService.swift" \
  -Xlinker -e -Xlinker _NSExtensionMain -Xlinker -dead_strip $entflags -Xlinker "$t/$triple/dist-ios/libgnobbi_nse.a" \
  -framework UserNotifications -framework Security -framework CoreFoundation -framework SystemConfiguration -lc++ -lresolv \
  -o "$appex/GnobbiNSE"
strip -x "$appex/GnobbiNSE"
cp "$here/nse/Info.plist" "$appex/Info.plist"
pb=/usr/libexec/PlistBuddy; p=$appex/Info.plist
$pb -c "Set :CFBundleVersion $build" -c "Set :CFBundleShortVersionString $version" -c "Set :CFBundleSupportedPlatforms:0 $platform" "$p"
# The simulator host is the sim build of the app (a different id): an extension id must extend its host's.
[ $kind = sim ] && $pb -c "Set :CFBundleIdentifier net.meskatech.gnobbi.egui.ios.nse" "$p"
# DT* keys that App Store Connect reads from the build toolchain (device builds only).
if [ $kind = device ]; then
  sdkv=$(xcrun --sdk $sdk --show-sdk-version); sdkb=$(xcrun --sdk $sdk --show-sdk-build-version); xb=$(xcodebuild -version | sed -n 's/^Build version //p')
  for kv in "DTPlatformName=$sdk" "DTSDKName=$sdk$sdkv" "DTPlatformVersion=$sdkv" "DTSDKBuild=$sdkb" "DTPlatformBuild=$sdkb" "DTXcodeBuild=$xb" "DTCompiler=com.apple.compilers.llvm.clang.1_0"; do
    $pb -c "Add :${kv%%=*} string ${kv#*=}" "$p"; done
fi
plutil -lint "$p" >/dev/null; lipo -archs "$appex/GnobbiNSE" | grep -q arm64
echo "$appex ($version, build $build, $(du -k "$appex/GnobbiNSE" | cut -f1) KB)"
