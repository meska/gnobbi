# Gnobbi vendored gpui

- Version: 0.2.2 (Apache-2.0).
- Official archive: https://static.crates.io/crates/gpui/gpui-0.2.2.crate
- Archive SHA-256: `979b45cfa6ec723b6f42330915a1b3769b930d02b2d505f9697f8ca602bee707`.
- Upstream source file modified: `src/platform/linux/wayland/client.rs`. `wl_keyboard::Event::Modifiers` and `Key` unwrapped `keymap_state`, which stays `None` until a `keymap` event arrives; a compositor (sway after `wtype`) may send `modifiers` first, which panicked at startup. Both events are now ignored while no keymap is known, and the `Modifiers` arm no longer unwraps at all (two locals, as in the upstream PR).
- Upstream: issues zed-industries/zed#64660 and #64959; fix PR zed-industries/zed#64970 (open on 2026-10-30).
- Removal: drop this vendor directory, `../gpui-0.2.2.patch` and the `[patch.crates-io]` entry once that PR is merged and released in a gpui version we bump to.
- Regression test: `crates/matrix-desktop/tests/wayland_keymap_order.rs` (fake compositor).
- `../gpui-0.2.2.patch` is generated using `diff -ruN` from the extracted official archive to this directory, excluding the added distribution file `GNOBBI-PATCH.md`. All other upstream files remain byte-identical.
