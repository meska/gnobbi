#!/bin/sh
# Prints the macOS Info.plist (dmg, App Store/TestFlight and dev builds share it). The About panel shows
# "Version <version> (<build>)" and below it the copyright line "<commit> · <build date>".
# Usage: packaging/macos/info-plist.sh <bundle-id> <version> <build> [commit] [built]
#   build = integer, increasing (App Store Connect); default commit/built = HEAD short SHA / now (local time).
set -eu
d=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
bid=${1:?bundle id}; ver=${2:?version}; build=${3:?build}
commit=${4:-$(git -C "$d" rev-parse --short HEAD 2>/dev/null || echo unknown)}
built=${5:-$(date '+%d/%m/%Y %H:%M')}
case $build in ''|*[!0-9]*) echo "build must be an integer: $build" >&2; exit 1;; esac
sed -e "s|@BID@|$bid|" -e "s|@VERSION@|$ver|" -e "s|@BUILD@|$build|" -e "s|@COMMIT@|$commit|g" -e "s|@BUILT@|$built|" "$d/Info.plist.in"
