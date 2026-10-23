use matrix_desktop::LIGHT;

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
fn light_small_text_has_aa_contrast_on_message_and_panel_surfaces() {
    for foreground in [LIGHT.text, LIGHT.secondary, LIGHT.accent] {
        for background in [
            LIGHT.surface,
            LIGHT.outgoing,
            LIGHT.selected,
            LIGHT.timeline,
        ] {
            let ratio = contrast(foreground, background);
            assert!(
                ratio >= 4.5,
                "{foreground:06x}/{background:06x}: {ratio:.3} < 4.5"
            );
        }
    }
    assert!(contrast(0xffffff, LIGHT.accent) >= 4.5);
}

#[test]
fn dark_and_light_semantic_roles_keep_small_text_readable() {
    for t in [LIGHT, matrix_desktop::DARK] {
        for background in [
            t.surface,
            t.outgoing,
            t.selected,
            t.timeline,
            t.input,
            t.disabled,
            t.selection,
        ] {
            for foreground in [t.text, t.secondary, t.accent] {
                assert!(
                    contrast(foreground, background) >= 4.5,
                    "{foreground:06x}/{background:06x} below 4.5"
                );
            }
        }
        for background in [t.surface, t.outgoing, t.selected, t.timeline] {
            assert!(contrast(t.accent, background) >= 4.5);
            assert!(contrast(t.error, background) >= 4.5);
        }
        assert!(contrast(t.on_accent, t.accent) >= 4.5);
        for room in matrix_desktop::ROOMS {
            assert!(contrast(t.avatar_text, room.color) >= 4.5);
        }
    }
}
