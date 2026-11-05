use matrix_desktop::Theme;

fn luminance(rgb: u32) -> f64 {
    let channel = |shift: u32| {
        let c = ((rgb >> shift) & 0xff_u32) as f64 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

fn contrast(a: u32, b: u32) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn placeholder_is_legible_but_less_prominent_than_text() {
    // Placeholder is an opaque token pre-composited against the actual surface.
    for theme in Theme::ALL {
        let t = theme.tokens();
        // The palette stores secondary at 80% opacity, pre-composited over surface.
        for shift in [0, 8, 16] {
            let secondary = (t.secondary >> shift) & 0xff;
            let surface = (t.surface >> shift) & 0xff;
            let expected = (secondary * 4 + surface + 2) / 5;
            let actual = (t.placeholder >> shift) & 0xff;
            assert_eq!(actual, expected, "{theme:?}: blended channel at {shift}");
        }
        let placeholder = contrast(t.placeholder, t.surface);
        let text = contrast(t.text, t.surface);
        assert!(
            placeholder >= 3.0,
            "{theme:?}: placeholder contrast {placeholder:.2}"
        );
        assert!(
            placeholder < text,
            "{theme:?}: placeholder {placeholder:.2}, text {text:.2}"
        );
    }
}
