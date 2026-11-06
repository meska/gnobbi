//! Mention pills and highlighted rows (the MATRIX_MENZIONI_UI notes D5): readable in all four themes.
use matrix_desktop::Theme;

fn luminance(rgb: u32) -> f64 {
    let channel = |shift: u32| {
        let c = ((rgb >> shift) & 0xff_u32) as f64 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

fn contrast(a: u32, b: u32) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn pill_text_bar_and_row_secondary_text_are_legible_in_every_theme() {
    for theme in Theme::ALL {
        let t = theme.tokens();
        let own = contrast(t.surface, t.error);
        let other = contrast(t.text, t.border);
        let bar = contrast(t.error, t.timeline);
        let meta = contrast(t.secondary, t.mention_row);
        assert!(own >= 4.5, "{theme:?}: own pill {own:.2}");
        assert!(other >= 4.5, "{theme:?}: other pill {other:.2}");
        assert!(bar >= 3.0, "{theme:?}: bar {bar:.2}");
        assert!(meta >= 4.5, "{theme:?}: secondary on row {meta:.2}");
    }
}
