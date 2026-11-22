#!/bin/sh
# Debug APK from the cdylib built on the Linux CT:
#   CT_SRC=$PWD the ct script 'ANDROID_NDK_HOME=/opt/android-ndk-r28c cargo ndk -P 26 -t arm64-v8a -o artifacts/android/jniLibs rustc -p matrix-desktop --example gnobbi --crate-type cdylib'
#   packaging/android/build-apk.sh            # -> artifacts/android/gnobbi-debug.apk (needs the Android SDK build-tools and a JDK)
# NativeActivity has no Java code, so aapt2 + zipalign + apksigner are enough (no Gradle).
set -eu
cd "$(dirname "$0")/../.."
SDK=${ANDROID_HOME:-/opt/homebrew/share/android-commandlinetools}
BT=$(ls -d "$SDK"/build-tools/* | sort -V | tail -1)
JAR=$(ls -d "$SDK"/platforms/android-* | sort -V | tail -1)/android.jar
OUT=artifacts/android
KS=${GNOBBI_DEBUG_KEYSTORE:-$HOME/.android/debug.keystore}
[ -f "$KS" ] || keytool -genkeypair -keystore "$KS" -storepass android -keypass android -alias androiddebugkey -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=Android Debug,O=Android,C=US"
rm -f "$OUT/base.apk" "$OUT/aligned.apk" "$OUT/gnobbi-debug.apk"
"$BT/aapt2" link -o "$OUT/base.apk" -I "$JAR" --manifest packaging/android/AndroidManifest.xml --min-sdk-version 26 --target-sdk-version 35
# cargo-ndk writes jniLibs/<abi>/, an APK wants lib/<abi>/
rm -rf "$OUT/lib" && mkdir "$OUT/lib" && cp -R "$OUT"/jniLibs/* "$OUT/lib/"
(cd "$OUT" && zip -q -r base.apk lib)
"$BT/zipalign" -p -f 4 "$OUT/base.apk" "$OUT/aligned.apk"
"$BT/apksigner" sign --ks "$KS" --ks-pass pass:android --key-pass pass:android --out "$OUT/gnobbi-debug.apk" "$OUT/aligned.apk"
rm -rf "$OUT/base.apk" "$OUT/aligned.apk" "$OUT/lib"
ls -la "$OUT/gnobbi-debug.apk"
