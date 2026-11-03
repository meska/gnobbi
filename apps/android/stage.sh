#!/bin/sh
# Copies the generated mobile bridge (scripts/mobile.sh bindings + android) and the design tokens into the app.
# Usage: apps/android/stage.sh [target-dir]   (default: target); then: ./gradlew assembleDebug
set -eu
cd "$(dirname "$0")"
t=${1:-../../target}/mobile
m=app/src/main
rm -rf $m/jniLibs $m/kotlin/uniffi $m/kotlin/chat/gnobbi/design
mkdir -p $m/kotlin/chat/gnobbi/design
cp -R "$t/android/jniLibs" $m/jniLibs
cp -R "$t/kotlin/uniffi" $m/kotlin/uniffi
cp ../../crates/matrix-domain/design/generated/DesignTokens.kt $m/kotlin/chat/gnobbi/design/
