//! The Swift and Kotlin outputs must carry exactly the values of the generated Rust constants.
use matrix_domain::design::{LAYOUT, THEMES};

const SWIFT: &str = include_str!("../design/generated/DesignTokens.swift");
const KOTLIN: &str = include_str!("../design/generated/DesignTokens.kt");

/// Lines of the `enum <name> {` / `object <name> {` block that opens with `opener`.
fn block<'a>(src: &'a str, opener: &str) -> Vec<&'a str> {
    let start = src.find(opener).unwrap_or_else(|| panic!("missing block {opener}"));
    src[start..].lines().skip(1).take_while(|l| !l.starts_with("    }")).collect()
}

fn has(lines: &[&str], wanted: &str) {
    assert!(lines.iter().any(|l| l.trim() == wanted), "missing `{wanted}`");
}

#[test]
fn placeholder_contrast_is_readable_and_softer_than_text() {
    fn luminance(color: u32) -> f64 {
        let channel = |shift| {
            let value = f64::from((color >> shift) & 255u32) / 255.0;
            if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    }
    fn contrast(a: u32, b: u32) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
    for theme in THEMES {
        let p = theme.palette;
        let placeholder = contrast(p.placeholder, p.surface);
        assert!(placeholder >= 3.0, "{}: {placeholder}", theme.id);
        assert!(placeholder < contrast(p.text, p.surface), "{}", theme.id);
    }
}

#[test]
fn swift_and_kotlin_match_the_rust_constants_for_all_themes() {
    for th in THEMES {
        let name = format!("{}{}", th.id[..1].to_uppercase(), &th.id[1..]);
        let swift = block(SWIFT, &format!("    enum {name} {{"));
        let kotlin = block(KOTLIN, &format!("    object {name} {{"));
        has(&swift, &format!("static let label = \"{}\"", th.label));
        has(&kotlin, &format!("const val LABEL = \"{}\"", th.label));
        has(&swift, &format!("static let dark = {}", th.dark));
        has(&kotlin, &format!("const val DARK = {}", th.dark));
        let p = th.palette;
        for (swift_name, kotlin_name, v) in [
            ("surface", "SURFACE", p.surface), ("text", "TEXT", p.text),
            ("secondary", "SECONDARY", p.secondary), ("placeholder", "PLACEHOLDER", p.placeholder), ("accent", "ACCENT", p.accent),
            ("timeline", "TIMELINE", p.timeline), ("outgoing", "OUTGOING", p.outgoing),
            ("selected", "SELECTED", p.selected), ("border", "BORDER", p.border),
            ("input", "INPUT", p.input), ("disabled", "DISABLED", p.disabled),
            ("selection", "SELECTION", p.selection), ("error", "ERROR", p.error),
            ("onAccent", "ON_ACCENT", p.on_accent), ("avatarText", "AVATAR_TEXT", p.avatar_text),
        ] {
            has(&swift, &format!("static let {swift_name}: UInt32 = {v:#08x}"));
            has(&kotlin, &format!("const val {kotlin_name} = {v:#08x}L"));
        }
    }
    let top = |s: &str, l: &str| assert!(s.contains(l), "missing `{l}`");
    top(SWIFT, &format!("static let layoutSidebar: Double = {}", LAYOUT.sidebar));
    top(KOTLIN, &format!("const val LAYOUT_SIDEBAR = {:?}", LAYOUT.sidebar));
    top(SWIFT, "static let demoNotice = \"DEMO OFFLINE · nessun account connesso\"");
    top(KOTLIN, "const val SCRIM = 0x00000066L");
}
