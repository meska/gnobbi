#!/bin/sh
# The merged manifest of the `play` flavor must not declare REQUEST_INSTALL_PACKAGES (Play policy); the `direct` one must (self-update).
# Needs the same environment as a normal build (JDK, ANDROID_HOME, artifacts/android/jniLibs only for the real build, not for this).
set -eu
cd "$(dirname "$0")"
./gradlew --no-daemon -q :app:processPlayReleaseManifest :app:processDirectReleaseManifest
merged() { find app/build/intermediates -path "*/$1/*" -name AndroidManifest.xml | grep -i "merged_manifest" | head -1; }
play=$(merged playRelease); direct=$(merged directRelease)
[ -n "$play" ] && [ -n "$direct" ] || { echo 'merged manifests not found' >&2; exit 1; }
perm=android.permission.REQUEST_INSTALL_PACKAGES
if grep -q "$perm" "$play"; then echo "FAIL: $perm in the play manifest ($play)" >&2; exit 1; fi
grep -q "$perm" "$direct" || { echo "FAIL: $perm missing from the direct manifest ($direct)" >&2; exit 1; }
echo "ok: play without $perm, direct with it"
