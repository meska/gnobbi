//! One-line list text: shared by the chat list rows (and any other list that must not wrap).
//! The row height comes from its content, so every row has the same height.

pub const AVATAR_PX: f32 = 46.;
pub const ROW_PAD_Y: f32 = 12.;
pub const NAME_PX: f32 = 15.;
pub const NAME_LINE_PX: f32 = 20.;
pub const PREVIEW_PX: f32 = 12.;
pub const PREVIEW_LINE_PX: f32 = 16.;
pub const TEXT_GAP_PX: f32 = 4.;
/// Name row (badge or time sits inside it, never taller) + gap + preview row.
pub const TEXT_BLOCK_PX: f32 = NAME_LINE_PX + TEXT_GAP_PX + PREVIEW_LINE_PX;
pub const ROW_HEIGHT_PX: f32 = ROW_PAD_Y * 2. + if TEXT_BLOCK_PX > AVATAR_PX { TEXT_BLOCK_PX } else { AVATAR_PX };

/// Line breaks and runs of whitespace become single spaces, so the text fits one ellipsized line.
pub fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines the row paints: both texts are forced to one line; an empty one still reserves its line.
    fn content_height(name: &str, preview: &str) -> f32 {
        let lines = |t: &str| one_line(t).lines().count().max(1) as f32;
        let (n, p) = (lines(name), lines(preview));
        assert_eq!((n, p), (1., 1.));
        NAME_LINE_PX * n + TEXT_GAP_PX + PREVIEW_LINE_PX * p
    }

    #[test]
    fn content_never_taller_than_the_row() {
        let long = "x".repeat(300);
        let cases: [(&str, &str, &str); 6] = [
            (&long, "ciao", "3"),                                // long name, no spaces
            ("Claudio Martini 😀", "ok 😀\n\ngrazie\r\n👍", "1"), // emoji and multiline preview
            ("", "ciao", ""),                                    // empty name
            ("Claudio", "", ""),                                 // empty preview
            ("Claudio", "ciao", "999"),                          // three-digit badge
            ("漢字漢字漢字漢字漢字漢字", "ＷＩＤＥ　ｗｉｄｅ　漢字\u{2028}ok", "12"), // wide chars, selected + unread
        ];
        for (name, preview, _badge) in cases {
            assert!(content_height(name, preview) <= ROW_HEIGHT_PX - 2. * ROW_PAD_Y, "{name:?}/{preview:?}");
            assert!(!one_line(preview).contains(['\n', '\r', '\u{2028}']));
        }
        assert!(AVATAR_PX + 2. * ROW_PAD_Y <= ROW_HEIGHT_PX);
    }

    #[test]
    fn one_line_collapses_breaks() {
        assert_eq!(one_line("a\n\n b\r\nc "), "a b c");
        assert_eq!(one_line(""), "");
    }
}
