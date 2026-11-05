#!/bin/sh
set -eu

# Requires xvfb-run, xauth, dbus-run-session and the standard GPUI build libraries.
command -v xvfb-run >/dev/null
command -v dbus-run-session >/dev/null
command -v xauth >/dev/null

export MATRIX_DESKTOP_ISOLATED_RESIZE=1
unset WAYLAND_DISPLAY
log=$(mktemp)
trap 'rm -f "$log"' EXIT HUP INT TERM
for scale in 1 1.5 2; do
    for state in empty text multiline attachment; do
        attachment=0
        [ "$state" != attachment ] || attachment=1
        COMPOSER_TEST_STATE="$state" COMPOSER_TEST_SCALE="$scale" \
        COMPOSER_TEST_ATTACHMENT="$attachment" GPUI_X11_SCALE_FACTOR="$scale" \
        xvfb-run -a -s '-screen 0 1920x1080x24' dbus-run-session -- \
            cargo test --locked -j4 -p matrix-desktop --features live --bin gnobbi \
            composer_view::tests::painted_composer_padding_and_height -- --ignored --exact --nocapture >"$log" 2>&1 || {
                cat "$log"
                exit 1
            }
        cat "$log"
        grep -Fq 'test result: ok. 1 passed; 0 failed;' "$log"
    done
done
