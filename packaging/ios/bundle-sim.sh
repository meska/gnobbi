#!/bin/sh
# iOS spike (simulator only): builds gnobbi-egui and wraps it in Gnobbi.app. No Xcode project: winit's run() calls UIApplicationMain itself.
# Usage (macOS with Rust + Xcode): packaging/ios/bundle-sim.sh [target-dir] [debug|dist]   then: xcrun simctl install <udid> <target-dir>/ios/Gnobbi.app
set -eu
t=${1:-target}; prof=${2:-debug}; triple=aarch64-apple-ios-sim; here=$(cd "$(dirname "$0")" && pwd)
flag=; out=debug; [ "$prof" = debug ] || { flag="--profile $prof"; out=$prof; }
# The entitlements go in a __TEXT section: the simulator reads them from there, and a codesign --entitlements signature is refused at launch.
CARGO_TARGET_DIR=$t cargo rustc --locked $flag -p matrix-desktop-egui --bin gnobbi-egui --target $triple --features ios-vault -- \
  -C link-arg=-Wl,-sectcreate,__TEXT,__entitlements,"$here/sim.entitlements"
app=$t/ios/Gnobbi.app; rm -rf "$app"; mkdir -p "$app"
cp "$t/$triple/$out/gnobbi-egui" "$app/"; cp "$here/Info.plist" "$app/Info.plist"
# shortcut: SDK version lowered to 26.0 because from the iOS 27 SDK UIKit traps an app without UIScene lifecycle (winit 0.30 has none); fix by adopting UIScene in winit.
vtool -set-build-version iossim 17.0 26.0 -replace -output "$app/gnobbi-egui" "$app/gnobbi-egui"
codesign -f -s - "$app/gnobbi-egui"
echo "$app"
