# Design tokens

`tokens.toml` is the only place where colours, layout sizes, spacing and shared Italian strings are defined.
Regenerate after editing (Python >= 3.11, run from anywhere):

    python3 crates/matrix-domain/design/generate.py

Outputs (committed): `../src/design.rs` (Rust), `generated/DesignTokens.kt`.
Mobile apps must use the generated files and never define colours or measures by hand.

Tests: `matrix-desktop/tests/design_tokens.rs` (generated values == desktop `Theme::tokens()` for all 4 themes),
`matrix-domain/tests/design_outputs.rs` (Swift/Kotlin carry the same values as the Rust constants).

The `placeholder` palette colour is secondary text blended at 80% over the theme surface; it is used for the composer on that surface.

## Still inside matrix-desktop, to move into the domain for mobile

- `DesignTokens`, `LIGHT/DARK/DAY/NIGHT`, `SCRIM`, `AVATAR_FALLBACK` (lib.rs) and `Theme::label/is_dark` (preferences.rs): colours and layout now come from `matrix_domain::design` (desktop builds `LIGHT/DAY/DARK/NIGHT` from the generated palettes); `SCRIM`, `AVATAR_FALLBACK` and the labels are still copies guarded by the identity test.
- Spacing/radii/type scale: literals in the views (`px(12.)`, `rounded(..)`, `text_size(..)`), only `row_text::ROW_PAD_Y/TEXT_GAP_PX` are named. Not extracted: no source of truth to copy without inventing values.
- Density: does not exist in the desktop yet.
- Fonts: `UI_FONT/SERIF_FONT/MONO_FONT` are per-OS desktop families; mobile uses system fonts.
- Italian strings: only theme labels; the rest are literals in views plus the `live`-gated consts of `signin_form.rs` (COPY_LABEL, RECOVERY_*, SETUP_*), the labels of `Preferences`-adjacent enums (icon choices, etc.) and `group_label` in sidebar.rs.
- Pure logic worth moving: `Theme` (names/parse), `Preferences`, `Sidebar`/`Navigation`/`group_label`, `Composer`/mentions, `links`, `drafts`, `history` demo types, `bubble_stamp`, `row_text`, `scrollbar` geometry (GPUI-free, but in the desktop crate with GPUI/cpal deps).
