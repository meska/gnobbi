#!/bin/sh
# Installs the debug APK on the running emulator/device, signs in against a test server, screenshots.
# Env: GNOBBI_TEST_HOMESERVER, GNOBBI_TEST_USER, GNOBBI_TEST_PASSWORD, GNOBBI_TEST_CA (CA pem path), optional GNOBBI_TEST_SEND.
# Usage: run-emulator.sh <label> [seconds]   (adb in PATH)
set -eu
label=${1:?label}; wait=${2:-30}
apk=$(dirname "$0")/app/build/outputs/apk/debug/app-debug.apk
pkg=chat.gnobbi.mobile.sample
adb install -r "$apk" >/dev/null
adb shell pm clear $pkg >/dev/null
# adb shell re-splits its arguments on the device: pass one string with single-quoted values (none may contain a quote).
adb shell "am start -W -n $pkg/chat.gnobbi.mobile.MainActivity --es homeserver '$GNOBBI_TEST_HOMESERVER' --es user '$GNOBBI_TEST_USER' --es password '$GNOBBI_TEST_PASSWORD' --es ca_b64 '$(base64 < "$GNOBBI_TEST_CA" | tr -d '\n')' --es send '${GNOBBI_TEST_SEND:-}'" >/dev/null
sleep "$wait"
adb exec-out screencap -p > "$label.png"
