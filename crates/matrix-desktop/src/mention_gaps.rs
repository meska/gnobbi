//! Room for the avatar inside a user mention pill. GPUI draws the body as one styled text, so the
//! avatar's width is a gap of spaces put in the *drawn* string only; the message text (copy,
//! selection, accessibility, search) never holds it. This maps between the two.
use matrix_engine::rich_text::{MentionKind, MentionSpan};

/// An em space plus a hair: about 16 px at the 14 px body size, the avatar's width.
pub const AVATAR_GAP: &str = "\u{2003}\u{2009}";

/// Mentions that fit the text on character boundaries (a stale span is never sliced).
pub fn fitting<'a>(text: &'a str, mentions: &'a [MentionSpan]) -> impl Iterator<Item = &'a MentionSpan> {
    mentions.iter().filter(|m| {
        m.range.start < m.range.end
            && m.range.end <= text.len()
            && text.is_char_boundary(m.range.start)
            && text.is_char_boundary(m.range.end)
    })
}

/// The text as drawn, with the gaps, and where they went.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Gaps {
    /// Text indices the gaps sit in front of, ascending.
    inserts: Vec<usize>,
}

impl Gaps {
    pub fn new(text: &str, mentions: &[MentionSpan]) -> (Self, String) {
        let mut inserts: Vec<usize> = fitting(text, mentions)
            .filter(|m| matches!(m.kind, MentionKind::User { .. }))
            .map(|m| m.range.start)
            .collect();
        inserts.sort_unstable();
        inserts.dedup();
        let mut drawn = String::with_capacity(text.len() + inserts.len() * AVATAR_GAP.len());
        let mut last = 0;
        for &at in &inserts {
            drawn.push_str(&text[last..at]);
            drawn.push_str(AVATAR_GAP);
            last = at;
        }
        drawn.push_str(&text[last..]);
        (Self { inserts }, drawn)
    }

    /// Drawn index of text index `i`; a pill's start is after its own gap.
    pub fn start(&self, i: usize) -> usize {
        i + AVATAR_GAP.len() * self.inserts.iter().filter(|&&s| s <= i).count()
    }

    /// Drawn index of a range end: a gap that begins exactly there is not part of the range.
    pub fn end(&self, i: usize) -> usize {
        i + AVATAR_GAP.len() * self.inserts.iter().filter(|&&s| s < i).count()
    }

    /// Where the gap of the pill starting at text index `i` begins in the drawn string.
    pub fn gap_start(&self, i: usize) -> usize {
        self.start(i) - AVATAR_GAP.len()
    }

    /// Text index of drawn index `d`; inside a gap it is the pill's start.
    pub fn text_index(&self, d: usize) -> usize {
        let mut shift = 0;
        for &s in &self.inserts {
            let gap = s + shift;
            if d >= gap + AVATAR_GAP.len() {
                shift += AVATAR_GAP.len();
            } else if d > gap {
                return s;
            } else {
                break;
            }
        }
        d - shift
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_engine::history::HistorySender;

    fn user(range: std::ops::Range<usize>) -> MentionSpan {
        MentionSpan { range, kind: MentionKind::User { sender: HistorySender::new("@a:x", Some("Anna")), me: false } }
    }

    #[test]
    fn gaps_only_in_the_drawn_text_and_indices_map_both_ways() {
        let text = "ciao Anna e @room";
        let mentions = [user(5..9), MentionSpan { range: 12..17, kind: MentionKind::AtRoom }];
        let (gaps, drawn) = Gaps::new(text, &mentions);
        assert_eq!(drawn, format!("ciao {AVATAR_GAP}Anna e @room"));
        assert_eq!(gaps.gap_start(5), 5);
        assert_eq!(&drawn[gaps.start(5)..gaps.end(9)], "Anna");
        assert_eq!(&drawn[gaps.end(5)..gaps.start(5)], AVATAR_GAP);
        for i in (0..=text.len()).filter(|&i| text.is_char_boundary(i)) {
            assert_eq!(gaps.text_index(gaps.start(i)), i);
        }
        // A click on the avatar lands on the pill's first letter.
        assert_eq!(gaps.text_index(5 + 2), 5);
    }

    #[test]
    fn no_pills_no_change_and_stale_spans_are_ignored() {
        let (gaps, drawn) = Gaps::new("àb", &[user(1..2), user(0..99)]);
        assert_eq!((gaps, drawn.as_str()), (Gaps::default(), "àb"));
    }
}
