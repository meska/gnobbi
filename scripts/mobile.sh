#!/bin/sh
# Mobile bridge build. Usage: scripts/mobile.sh bindings | android | ios
# Output under target/mobile/ (gitignored). Nothing here is part of the desktop CI.
#   bindings  Swift + Kotlin sources from the host cdylib (uniffi-bindgen, library mode)
#   android   libmatrix_mobile.so for arm64-v8a and x86_64 (needs cargo-ndk, ANDROID_NDK_HOME, rustup android targets)
#   ios       MatrixMobile.xcframework: device + simulator static libs (macOS with Xcode only)
# PROFILE (default dist) is the cargo profile of the android/ios libraries; CARGO_TARGET_DIR is honoured.
set -eu
cd "$(dirname "$0")/.."
target=${CARGO_TARGET_DIR:-target}
out=$target/mobile
profile=${PROFILE:-dist}
lib=matrix_mobile

bindings() {
  cargo build --locked -p matrix-mobile --features bindgen
  case $(uname) in Darwin) ext=dylib ;; *) ext=so ;; esac
  rm -rf "$out/swift" "$out/kotlin"
  for lang in swift kotlin; do
    cargo run --locked -q -p matrix-mobile --features bindgen --bin uniffi-bindgen -- \
      generate --no-format --library "$target/debug/lib$lib.$ext" --language $lang --out-dir "$out/$lang"
  done
}

android() {
  : "${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to an NDK (r28c tested)}"
  rm -rf "$out/android"
  cargo ndk --platform 24 -t arm64-v8a -t x86_64 -o "$out/android/jniLibs" \
    build --locked -p matrix-mobile --profile "$profile"
}

ios() {
  [ "$(uname)" = Darwin ] || { echo "ios needs macOS + Xcode" >&2; exit 1; }
  # Must match what the C parts (aws-lc, sqlite) are compiled for, or the link fails on ___chkstk_darwin.
  export IPHONEOS_DEPLOYMENT_TARGET=${IPHONEOS_DEPLOYMENT_TARGET:-16.0}
  bindings
  headers=$out/ios-headers
  rm -rf "$headers" "$out/MatrixMobile.xcframework"
  mkdir -p "$headers"
  cp "$out"/swift/*.h "$headers"/
  cp "$out"/swift/*.modulemap "$headers/module.modulemap"
  args=
  for t in aarch64-apple-ios aarch64-apple-ios-sim; do
    # Static lib only: the cdylib half would be linked here for nothing (and against the default iOS 10 floor).
    cargo rustc --locked --lib -p matrix-mobile --profile "$profile" --target $t --crate-type staticlib
    args="$args -library $target/$t/$profile/lib$lib.a -headers $headers"
  done
  # shellcheck disable=SC2086
  xcodebuild -create-xcframework $args -output "$out/MatrixMobile.xcframework"
}

case ${1:-} in
  bindings) bindings ;;
  android) android ;;
  ios) ios ;;
  *) echo "usage: $0 bindings|android|ios" >&2; exit 2 ;;
esac
