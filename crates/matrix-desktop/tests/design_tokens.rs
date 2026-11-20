//! The generated tokens (matrix-domain, from design/tokens.toml) must equal what the desktop uses today.
use matrix_desktop::i18n::{self, Preference};
use matrix_desktop::{AVATAR_FALLBACK, DEMO_NOTICE, DesignTokens, SCRIM, Theme, row_text};
use matrix_domain::design as gen_;

fn same(id: &str, d: DesignTokens, g: gen_::Palette) {
    let pairs = [
        ("surface", d.surface, g.surface), ("text", d.text, g.text),
        ("secondary", d.secondary, g.secondary), ("placeholder", d.placeholder, g.placeholder), ("accent", d.accent, g.accent),
        ("timeline", d.timeline, g.timeline), ("outgoing", d.outgoing, g.outgoing),
        ("selected", d.selected, g.selected), ("border", d.border, g.border),
        ("input", d.input, g.input), ("disabled", d.disabled, g.disabled),
        ("selection", d.selection, g.selection), ("mention_row", d.mention_row, g.mention_row), ("error", d.error, g.error),
        ("on_accent", d.on_accent, g.on_accent), ("avatar_text", d.avatar_text, g.avatar_text),
    ];
    for (name, desktop, generated) in pairs {
        assert_eq!(desktop, generated, "theme {id}, token {name}");
    }
}

#[test]
fn generated_tokens_equal_the_desktop_for_all_four_themes() {
    // design/tokens.toml carries the Italian labels; the desktop labels are translated.
    i18n::apply(Preference::Italian, None);
    assert_eq!(gen_::THEMES.len(), Theme::ALL.len());
    for (theme, g) in Theme::ALL.into_iter().zip(gen_::THEMES) {
        same(g.id, theme.tokens(), g.palette);
        assert_eq!(theme.label(), g.label, "label of {}", g.id);
        assert_eq!(theme.is_dark(), g.dark, "dark flag of {}", g.id);
        let d = theme.tokens();
        let l = gen_::LAYOUT;
        assert_eq!((d.width, d.height, d.sidebar, d.header, d.bottom_bar),
                   (l.width, l.height, l.sidebar, l.header, l.bottom_bar), "layout of {}", g.id);
    }
    assert_eq!(SCRIM, gen_::SCRIM);
    assert_eq!(AVATAR_FALLBACK, gen_::AVATAR_FALLBACK);
    assert_eq!(DEMO_NOTICE, gen_::DEMO_NOTICE);
    assert_eq!(row_text::ROW_PAD_Y, gen_::SPACING_LIST_ROW_PAD_Y);
    assert_eq!(row_text::TEXT_GAP_PX, gen_::SPACING_LIST_TEXT_GAP);
}
