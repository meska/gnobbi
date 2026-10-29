# Gnobbi vendored gpui

- Version: 0.2.2 (Apache-2.0), official crates.io archive, extracted unchanged except below.
- Upstream source file modified: `src/platform/linux/wayland/client.rs`. `wl_keyboard::Event::Modifiers` and `Key` unwrapped `keymap_state`, which stays `None` until a `keymap` event arrives; a compositor (sway after `wtype`) may send `modifiers` first, which panicked at startup. Both events are now ignored while no keymap is known.
- Regression test: `crates/matrix-desktop/tests/wayland_keymap_order.rs` (fake compositor).
- `../gpui-0.2.2.patch` is the unified diff against the official archive. Drop this vendor copy when upstream fixes it.
