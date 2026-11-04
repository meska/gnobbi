#!/bin/sh
# Copies the generated mobile bridge (scripts/mobile.sh ios) and the design tokens next to the project.
# Usage: apps/ios/stage.sh [target-dir]   (default: target); then: xcodegen && xcodebuild ...
set -eu
cd "$(dirname "$0")"
t=${1:-../../target}/mobile
rm -rf Generated && mkdir -p Generated/swift
cp -R "$t/MatrixMobile.xcframework" Generated/
cp "$t/swift/matrix_mobile.swift" ../../crates/matrix-domain/design/generated/DesignTokens.swift Generated/swift/
