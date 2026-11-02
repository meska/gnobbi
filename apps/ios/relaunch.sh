#!/bin/sh
# Real app kill + relaunch on the simulator: 1st launch signs in from GNOBBI_TEST_* , 2nd launch has no credentials and must restore.
# Usage: relaunch.sh synapse|tuwunel   (needs fx/fixture.json, fx/ca.pem). Screenshots in vm-out/.
set -eu
s=${1:?server}; B=chat.gnobbi.mobile.sample
eval "$(python3 - "$s" <<'PY'
import json,sys
j=json.load(open('fx/fixture.json'))['servers'][sys.argv[1]]
u,v=sorted(j['users'].items())[1]
print(f"export GNOBBI_TEST_HOMESERVER='{j['url']}' GNOBBI_TEST_USER='{u}' GNOBBI_TEST_PASSWORD='{v['password']}'")
PY
)"
app=DerivedData/Build/Products/Debug-iphonesimulator/GnobbiMobile.app
udid=$(xcrun simctl list devices available | grep "${SIM_NAME:-iPhone 17 Pro Max} (" | head -1 | grep -oE '[0-9A-F]{8}-[0-9A-F-]{27}')
xcrun simctl bootstatus "$udid" -b >/dev/null; mkdir -p vm-out
xcrun simctl uninstall "$udid" $B 2>/dev/null || true; xcrun simctl install "$udid" "$app"
SIMCTL_CHILD_GNOBBI_TEST_HOMESERVER=$GNOBBI_TEST_HOMESERVER SIMCTL_CHILD_GNOBBI_TEST_USER=$GNOBBI_TEST_USER \
SIMCTL_CHILD_GNOBBI_TEST_PASSWORD=$GNOBBI_TEST_PASSWORD SIMCTL_CHILD_GNOBBI_TEST_CA=$PWD/fx/ca.pem xcrun simctl launch "$udid" $B
sleep 25; xcrun simctl io "$udid" screenshot "vm-out/$s-1-login.png"
xcrun simctl terminate "$udid" $B
SIMCTL_CHILD_GNOBBI_TEST_CA=$PWD/fx/ca.pem xcrun simctl launch "$udid" $B   # no user/password: only the Keychain can restore
sleep 25; xcrun simctl io "$udid" screenshot "vm-out/$s-2-reopen.png"
xcrun simctl terminate "$udid" $B
