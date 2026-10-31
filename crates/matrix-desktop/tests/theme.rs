use matrix_desktop::{Accent, Theme, AVATAR_FALLBACK, ROOMS};

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
    for (theme, accent) in Theme::ALL.into_iter().flat_map(|t| Accent::ALL.map(|a| (t, a))) {
        let t = theme.tokens_with(accent);
        let theme = format!("{} / {}", theme.label(), accent.label());
        let mut check = |fg: &str, fgc: u32, bg: &str, bgc: u32| {
            let r = contrast(fgc, bgc);
            if r < 4.5 {
                failures.push(format!("{theme} {fg} {fgc:06x} / {bg} {bgc:06x}: {r:.2} < 4.5"));
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

/// sRGB hex -> (L, C, h°) in OKLCH (Ottosson).
fn oklch(color: u32) -> (f64, f64, f64) {
    let lin = |s: u32| {
        let v = f64::from((color >> s) & 255) / 255.;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (lin(16), lin(8), lin(0));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let (ll, a, bb) = (
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    );
    (ll, a.hypot(bb), bb.atan2(a).to_degrees().rem_euclid(360.))
}

/// ΔE in OKLab, scaled x100.
fn delta_e(a: u32, b: u32) -> f64 {
    let ab = |c: u32| {
        let (l, ch, h) = oklch(c);
        (l, ch * h.to_radians().cos(), ch * h.to_radians().sin())
    };
    let (x, y) = (ab(a), ab(b));
    100. * ((x.0 - y.0).powi(2) + (x.1 - y.1).powi(2) + (x.2 - y.2).powi(2)).sqrt()
}

#[test]
fn oklch_known_values() {
    let (l, c, _) = oklch(0xffffff);
    assert!((l - 1.).abs() < 1e-3 && c < 1e-3);
    let (_, _, h) = oklch(0x00ff00);
    assert!((h - 142.5).abs() < 1., "{h}");
}

/// Each of the 7 accent dots in each theme: accent text on every background it can sit on needs 4.5:1
/// (WCAG 1.4.3), and on_accent text on the accent fill needs 4.5:1.
#[test]
fn every_accent_dot_is_legible_on_green_backgrounds() {
    let mut failures = vec![];
    for theme in Theme::ALL {
        for accent in Accent::ALL {
            let t = theme.tokens_with(accent);
            for (b, bc) in [("surface", t.surface), ("timeline", t.timeline), ("outgoing", t.outgoing), ("selected", t.selected), ("input", t.input)] {
                let r = contrast(t.accent, bc);
                if r < 4.5 {
                    failures.push(format!("{} {} accent {:06x} / {b} {bc:06x}: {r:.2} < 4.5", theme.label(), accent.name(), t.accent));
                }
            }
            let r = contrast(t.on_accent, t.accent);
            if r < 4.5 {
                failures.push(format!("{} {} on_accent {:06x} / accent {:06x}: {r:.2} < 4.5", theme.label(), accent.name(), t.on_accent, t.accent));
            }
        }
    }
    assert!(failures.is_empty(), "accento illeggibile:\n{}", failures.join("\n"));
}

/// Disabled is an inactive fill (WCAG 1.4.3 exempts inactive components), so the documented
/// threshold is 3:1 for secondary text on it (the all-pairs test above already demands 4.5).
#[test]
fn disabled_keeps_secondary_text_at_3_to_1() {
    for theme in Theme::ALL {
        let t = theme.tokens();
        let r = contrast(t.secondary, t.disabled);
        assert!(r >= 3., "{} secondary {:06x} / disabled {:06x}: {r:.2} < 3", theme.label(), t.secondary, t.disabled);
    }
}

/// Green tint: hue 140-170° wherever the colour has chroma >= MIN_CHROMA. Below it OKLCH hue is
/// 8-bit quantisation noise (NIGHT/DAY near-neutrals, the pure black timeline), so those are exempt
/// from the hue check but must stay below it (no visible cast) — they cannot be a stray non-green tint.
#[test]
fn surfaces_are_green_tinted() {
    const MIN_CHROMA: f64 = 0.01;
    let mut failures = vec![];
    for theme in Theme::ALL {
        let t = theme.tokens();
        for (n, c) in [("surface", t.surface), ("timeline", t.timeline), ("border", t.border), ("input", t.input), ("disabled", t.disabled), ("selected", t.selected)] {
            let (l, ch, h) = oklch(c);
            if ch >= MIN_CHROMA && !(140.0..=170.0).contains(&h) {
                failures.push(format!("{} {n} {c:06x}: L {l:.3} C {ch:.4} h {h:.1} fuori 140-170", theme.label()));
            }
        }
    }
    assert!(failures.is_empty(), "tinta non verde:\n{}", failures.join("\n"));
}

/// Outgoing bubble vs received (= surface) and vs timeline: ΔE OKLab (x100) >= 5.
#[test]
fn outgoing_bubble_is_distinguishable() {
    let mut failures = vec![];
    for theme in Theme::ALL {
        let t = theme.tokens();
        for (n, c) in [("ricevuta(surface)", t.surface), ("timeline", t.timeline)] {
            let d = delta_e(t.outgoing, c);
            if d < 5. {
                failures.push(format!("{} outgoing {:06x} / {n} {c:06x}: ΔE {d:.1} < 5", theme.label(), t.outgoing));
            }
        }
    }
    assert!(failures.is_empty(), "bolle indistinguibili:\n{}", failures.join("\n"));
}

/// Received bubble vs timeline is weak by design (the bubble edge is carried by `border`): measured
/// ΔE Giorno 2.9 (#ffffff / #f4f6f5), Scuro 3.3. Floor 2.5 stops regressions; the target 5 (like outgoing) is NOT met.
#[test]
fn received_vs_timeline_has_a_floor() {
    for theme in Theme::ALL {
        let t = theme.tokens();
        let d = delta_e(t.surface, t.timeline);
        assert!(d >= 2.5, "{} ricevuta {:06x} / timeline {:06x}: ΔE {d:.1} < 2.5", theme.label(), t.surface, t.timeline);
    }
}
