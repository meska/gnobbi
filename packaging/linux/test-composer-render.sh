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

# Rendered icon geometry (>=32x32, right of the text, centred within 1 physical px) and key activation: one process per test (GPUI runs once per process).
for scale in 1 1.5 2; do
    for name in painted_icons_are_32px_right_of_the_text_and_centred_in_the_bar painted_voice_is_32px_right_of_the_text_and_centred_in_the_bar painted_icons_stay_centred_in_a_bar_grown_by_many_lines; do
        GPUI_X11_SCALE_FACTOR="$scale" \
        xvfb-run -a -s '-screen 0 1920x1080x24' dbus-run-session -- \
            cargo test --locked -j4 -p matrix-desktop --features live --bin gnobbi \
            "composer_view::tests::$name" -- --ignored --exact --nocapture >"$log" 2>&1 || {
                cat "$log"
                exit 1
            }
        cat "$log"
        grep -Fq 'test result: ok. 1 passed; 0 failed;' "$log"
    done
done
