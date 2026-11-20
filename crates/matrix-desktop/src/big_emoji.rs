//! Telegram-style big emoji: a message made only of 1-3 emoji is drawn larger than the body text.

use unicode_segmentation::UnicodeSegmentation;

/// Text-size multiplier for a message of `n` emoji: 1 → 3×, 2 → 2.5×, 3 → 2×, more → none.
pub fn scale_for(n: usize) -> Option<f32> {
    match n {
        1 => Some(3.),
        2 => Some(2.5),
        3 => Some(2.),
        _ => None,
    }
}

/// Multiplier for the whole message body, or `None` when it is not emoji-only (after trimming) or has more than 3 emoji.
pub fn scale(text: &str) -> Option<f32> {
    let mut n = 0;
    for g in text.trim().graphemes(true) {
        if g.chars().all(char::is_whitespace) {
            continue; // spaces between emoji are allowed
        }
        if !is_emoji(g) {
            return None;
        }
        n += 1;
        if n > 3 {
            return None;
        }
    }
    scale_for(n)
}

/// One grapheme cluster that is a single emoji: pictograph (+ tone, FE0F, ZWJ chains, tags), flag, or keycap.
// shortcut: hand-written ranges (Emoji_Presentation plus the emoji blocks), no Unicode data crate; a supplementary-plane
// symbol that is text-by-default (e.g. U+1F321) is accepted without FE0F. Upgrade if such a message looks wrong.
fn is_emoji(g: &str) -> bool {
    let c: Vec<char> = g.chars().collect();
    let ri = |c: char| ('\u{1F1E6}'..='\u{1F1FF}').contains(&c);
    if c.len() == 2 && ri(c[0]) && ri(c[1]) {
        return true; // flag
    }
    // Keycap: bare 0-9 # * are NOT emoji; they only count with U+20E3.
    if c.last() == Some(&'\u{20E3}') {
        let base = c.len() - 1;
        let base = if c[base - 1] == '\u{FE0F}' { base - 1 } else { base };
        return base == 1 && matches!(c[0], '0'..='9' | '#' | '*');
    }
    let sel = c.contains(&'\u{FE0F}');
    // Every char is part of the sequence; the pieces between ZWJs must each start with a pictograph.
    let mut start = true;
    for &ch in &c {
        match ch {
            '\u{200D}' => {
                if start {
                    return false;
                }
                start = true;
            }
            '\u{FE0F}' | '\u{1F3FB}'..='\u{1F3FF}' | '\u{E0020}'..='\u{E007F}' => {
                if start {
                    return false;
                }
            }
            _ if pictograph(ch, sel) => start = false,
            _ => return false,
        }
    }
    !start
}

/// `sel`: the cluster carries U+FE0F, which makes the text-default BMP symbols (©, ❤, ☺, ✂…) emoji.
fn pictograph(c: char, sel: bool) -> bool {
    let u = c as u32;
    if (0x1F300..=0x1FAFF).contains(&u) || matches!(u, 0x1F004 | 0x1F0CF | 0x1F18E | 0x1F191..=0x1F19A | 0x1F201 | 0x1F21A | 0x1F22F | 0x1F232..=0x1F23A | 0x1F250 | 0x1F251) {
        return true;
    }
    // BMP: default emoji presentation (Unicode Emoji_Presentation=Yes) …
    let default = matches!(u, 0x231A | 0x231B | 0x23E9..=0x23EC | 0x23F0 | 0x23F3 | 0x25FD | 0x25FE | 0x2614 | 0x2615 | 0x2648..=0x2653 | 0x267F | 0x2693 | 0x26A1 | 0x26AA | 0x26AB | 0x26BD | 0x26BE | 0x26C4 | 0x26C5 | 0x26CE | 0x26D4 | 0x26EA | 0x26F2 | 0x26F3 | 0x26F5 | 0x26FA | 0x26FD | 0x2705 | 0x270A | 0x270B | 0x2728 | 0x274C | 0x274E | 0x2753..=0x2755 | 0x2757 | 0x2795..=0x2797 | 0x27B0 | 0x27BF | 0x2B1B | 0x2B1C | 0x2B50 | 0x2B55);
    // … or a text-default symbol that needs FE0F (© ® ™ stay plain text without it).
    let text_default = matches!(u, 0xA9 | 0xAE | 0x203C | 0x2049 | 0x2122 | 0x2139 | 0x2194..=0x2199 | 0x21A9 | 0x21AA | 0x2328 | 0x23CF | 0x23ED..=0x23EF | 0x23F1 | 0x23F2 | 0x23F8..=0x23FA | 0x24C2 | 0x25AA | 0x25AB | 0x25B6 | 0x25C0 | 0x25FB..=0x25FC | 0x2600..=0x2604 | 0x260E | 0x2611 | 0x2618 | 0x261D | 0x2620 | 0x2622 | 0x2623 | 0x2626 | 0x262A | 0x262E | 0x262F | 0x2638..=0x263A | 0x2640 | 0x2642 | 0x265F | 0x2660 | 0x2663 | 0x2665 | 0x2666 | 0x2668 | 0x267B | 0x267E | 0x2692 | 0x2694..=0x2697 | 0x2699 | 0x269B | 0x269C | 0x26A0 | 0x26A7 | 0x26B0 | 0x26B1 | 0x26C8 | 0x26CF | 0x26D1 | 0x26D3 | 0x26E9 | 0x26F0 | 0x26F1 | 0x26F4 | 0x26F7..=0x26F9 | 0x2702 | 0x2708 | 0x2709 | 0x270C | 0x270D | 0x270F | 0x2712 | 0x2714 | 0x2716 | 0x271D | 0x2721 | 0x2733 | 0x2734 | 0x2744 | 0x2747 | 0x2763 | 0x2764 | 0x27A1 | 0x2934 | 0x2935 | 0x2B05..=0x2B07 | 0x3030 | 0x303D | 0x3297 | 0x3299);
    default || (sel && text_default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_factors() {
        assert_eq!(scale("😀"), Some(3.));
        assert_eq!(scale("😀😀"), Some(2.5));
        assert_eq!(scale("😀 😀 😀"), Some(2.));
        assert_eq!(scale("😀😀😀😀"), None);
        assert_eq!(scale_for(0), None);
        assert_eq!(scale_for(4), None);
    }

    #[test]
    fn trims_and_allows_spaces() {
        assert_eq!(scale("  \n😀\n "), Some(3.));
        assert_eq!(scale("😀 \u{a0} 😀"), Some(2.5));
    }

    #[test]
    fn sequences_are_one_emoji() {
        assert_eq!(scale("👨‍👩‍👧"), Some(3.)); // ZWJ family
        assert_eq!(scale("👍🏽"), Some(3.)); // skin tone
        assert_eq!(scale("🇮🇹"), Some(3.)); // flag
        assert_eq!(scale("🇮🇹🇩🇪"), Some(2.5));
        assert_eq!(scale("1️⃣"), Some(3.)); // keycap
        assert_eq!(scale("❤️"), Some(3.)); // FE0F
        assert_eq!(scale("🏴\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}"), Some(3.)); // tag sequence (England)
        assert_eq!(scale("👨‍👩‍👧👍🏽🇮🇹"), Some(2.));
    }

    #[test]
    fn rejects_everything_else() {
        assert_eq!(scale(""), None);
        assert_eq!(scale("   \n"), None);
        assert_eq!(scale("1"), None);
        assert_eq!(scale("#"), None);
        assert_eq!(scale("*"), None);
        assert_eq!(scale("12"), None);
        assert_eq!(scale("©"), None);
        assert_eq!(scale("®"), None);
        assert_eq!(scale("™"), None);
        assert_eq!(scale("❤"), None); // text default without FE0F
        assert_eq!(scale("ciao"), None);
        assert_eq!(scale("😀 ciao"), None);
        assert_eq!(scale("😀 https://a.it"), None);
        assert_eq!(scale("🇮"), None); // lone regional indicator
        assert_eq!(scale("\u{200D}😀"), None);
        // Other scripts next to emoji never count as emoji-only.
        assert_eq!(scale("مرحبا 😀"), None);
        assert_eq!(scale("שלום"), None);
        assert_eq!(scale("😀שלום"), None);
        assert_eq!(scale("你好"), None);
        assert_eq!(scale("😀 こんにちは"), None);
    }

    #[test]
    fn symbols_with_selector_count() {
        assert_eq!(scale("©️"), Some(3.));
        assert_eq!(scale("✅"), Some(3.));
    }
}
