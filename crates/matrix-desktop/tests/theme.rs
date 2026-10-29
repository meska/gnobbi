use matrix_desktop::{Theme, AVATAR_FALLBACK, ROOMS};

fn luminance(color: u32) -> f64 {
    [16, 8, 0]
        .into_iter()
        .zip([0.2126, 0.7152, 0.0722])
        .map(|(shift, weight)| {
            let value = f64::from((color >> shift) & 255) / 255.;
            weight
                * if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
        })
        .sum()
}

fn contrast(a: u32, b: u32) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn contrast_known_values() {
    assert!((contrast(0x000000, 0xffffff) - 21.).abs() < 1e-9);
    assert!((contrast(0xffffff, 0x000000) - 21.).abs() < 1e-9);
    assert!((contrast(0x5288b5, 0x5288b5) - 1.).abs() < 1e-9);
}

#[test]
fn every_theme_meets_wcag_aa_on_all_required_pairs() {
    let mut failures = vec![];
    for theme in Theme::ALL {
        let t = theme.tokens();
        let mut check = |fg: &str, fgc: u32, bg: &str, bgc: u32| {
            let r = contrast(fgc, bgc);
            if r < 4.5 {
                failures.push(format!("{} {fg} {fgc:06x} / {bg} {bgc:06x}: {r:.2} < 4.5", theme.label()));
            }
        };
        let fgs = [("text", t.text), ("secondary", t.secondary), ("accent", t.accent)];
        let bgs = [
            ("surface", t.surface), ("timeline", t.timeline), ("outgoing", t.outgoing),
            ("selected", t.selected), ("input", t.input), ("disabled", t.disabled),
            ("selection", t.selection),
        ];
        for (f, fc) in fgs {
            for (b, bc) in bgs {
                check(f, fc, b, bc);
            }
        }
        for (b, bc) in bgs.iter().filter(|(b, _)| ["surface", "timeline", "outgoing", "selected", "input"].contains(b)) {
            check("error", t.error, b, *bc);
        }
        check("on_accent", t.on_accent, "accent", t.accent);
        for room in ROOMS {
            check("avatar_text", t.avatar_text, "ROOMS.color", room.color);
        }
        check("avatar_text", t.avatar_text, "AVATAR_FALLBACK", AVATAR_FALLBACK);
    }
    assert!(failures.is_empty(), "contrasto insufficiente:\n{}", failures.join("\n"));
}
