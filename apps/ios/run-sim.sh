#!/bin/sh
# Installs the debug build on a fresh simulator, signs in against a test server and takes a screenshot.
# Env: GNOBBI_TEST_HOMESERVER, GNOBBI_TEST_USER, GNOBBI_TEST_PASSWORD, GNOBBI_TEST_CA (path of the CA pem),
#      optional GNOBBI_TEST_SEND (text sent once to the first room). Usage: run-sim.sh <label> [seconds]
set -eu
label=${1:?label}; wait=${2:-25}
app=$(find DerivedData/Build/Products/Debug-iphonesimulator -maxdepth 1 -name '*.app' | head -1)
dev=${SIM_NAME:-iPhone 17 Pro}
udid=$(xcrun simctl list devices available | awk -v d="$dev" 'index($0,d" (")==5{gsub(/[()]/,"",$(NF-1)); print $(NF-1); exit}')
xcrun simctl bootstatus "$udid" -b >/dev/null
xcrun simctl uninstall "$udid" chat.gnobbi.mobile.sample 2>/dev/null || true
xcrun simctl install "$udid" "$app"
for v in GNOBBI_TEST_HOMESERVER GNOBBI_TEST_USER GNOBBI_TEST_PASSWORD GNOBBI_TEST_CA GNOBBI_TEST_SEND; do
  eval "val=\${$v:-}"; [ -n "$val" ] && export "SIMCTL_CHILD_$v=$val"
done
xcrun simctl launch "$udid" chat.gnobbi.mobile.sample
sleep "$wait"
mkdir -p vm-out
xcrun simctl io "$udid" screenshot "vm-out/$label.png"
