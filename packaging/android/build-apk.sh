#!/bin/sh
# Debug APK. The cdylib is built on the Linux CT first:
#   ANDROID_NDK_HOME=/opt/android-ndk-r28c cargo ndk -P 26 -t arm64-v8a -o artifacts/android/jniLibs rustc --locked -p matrix-desktop --example gnobbi --features android-vault --crate-type cdylib
# Then: packaging/android/build-apk.sh  (or ./gradlew assembleDebug in packaging/android) -> artifacts/android/gnobbi-debug.apk
# Needs the Android SDK (ANDROID_HOME) and a JDK 17-21.
set -eu
cd "$(dirname "$0")"
# Firebase client config (PA-1), kept out of the repo: GOOGLE_SERVICES_JSON=/path/google-services.json
[ -z "${GOOGLE_SERVICES_JSON:-}" ] || cp "$GOOGLE_SERVICES_JSON" app/google-services.json
./gradlew assembleDebug
mkdir -p ../../artifacts/android
cp app/build/outputs/apk/debug/app-debug.apk ../../artifacts/android/gnobbi-debug.apk
ls -la ../../artifacts/android/gnobbi-debug.apk
