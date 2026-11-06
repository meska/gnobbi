#!/bin/sh
# Runs the vault/session XCTests against apps/fixture (fx/fixture.json + fx/ca.pem). Usage: run-tests.sh synapse|tuwunel
# Simulator: run it inside the macOS test VM (~/.hermes/scripts/vm_test.sh). Result bundle: vm-out/<server>.xcresult
set -eu
s=${1:?synapse|tuwunel}
export TEST_RUNNER_GNOBBI_FIXTURE=$PWD/fx/fixture.json TEST_RUNNER_GNOBBI_CA=$PWD/fx/ca.pem TEST_RUNNER_GNOBBI_SERVER=$s
mkdir -p vm-out; rm -rf vm-out/$s.xcresult
xcodebuild test -project GnobbiMobile.xcodeproj -scheme GnobbiMobile -configuration Debug \
  -destination "platform=iOS Simulator,name=${SIM_NAME:-iPhone 17 Pro}" -derivedDataPath DerivedData ARCHS=arm64 \
  CODE_SIGNING_ALLOWED=YES CODE_SIGN_IDENTITY=- -resultBundlePath vm-out/$s.xcresult
