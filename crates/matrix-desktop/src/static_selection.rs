//! Mouse selection over static text (GPUI has no primitive for it): pure index maths, no GPUI types.
use std::ops::Range;

/// One selection over one text; it only answers for the text it was made on.
#[derive(Default)]
pub struct StaticSelection {
    text: String,
    anchor: usize,
    head: usize,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl StaticSelection {
    /// Mouse down: 1 click = caret, 2 = word, 3 or more = line (newline-delimited).
    pub fn press(&mut self, text: &str, index: usize, clicks: usize) {
        let at = text.floor_char_boundary(index.min(text.len()));
        self.text = text.to_owned();
        (self.anchor, self.head) = match clicks {
            0 | 1 => (at, at),
            2 => {
                let start = text[..at].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(at, |(i, _)| i);
                let end = at + text[at..].char_indices().take_while(|(_, c)| is_word(*c)).last().map_or(0, |(i, c)| i + c.len_utf8());
                (start, end)
            }
            _ => (
                text[..at].rfind('\n').map_or(0, |i| i + 1),
                text[at..].find('\n').map_or(text.len(), |i| at + i),
            ),
        };
    }

    pub fn drag(&mut self, text: &str, index: usize) {
        if self.text == text {
            self.head = text.floor_char_boundary(index.min(text.len()));
        }
    }

    pub fn range(&self, text: &str) -> Option<Range<usize>> {
        (self.text == text && self.anchor != self.head)
            .then(|| self.anchor.min(self.head)..self.anchor.max(self.head))
    }

    pub fn copy<'a>(&self, text: &'a str) -> Option<&'a str> {
        text.get(self.range(text)?)
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Splits `len` bytes into runs by link / selection membership, so both can be drawn
/// without overlapping highlight ranges. Runs with neither are left out.
pub fn highlight_runs(
    len: usize,
    links: &[Range<usize>],
    selection: Option<Range<usize>>,
) -> Vec<(Range<usize>, bool, bool)> {
    let mut cuts = vec![0, len];
    for r in links.iter().chain(&selection) {
        cuts.extend([r.start.min(len), r.end.min(len)]);
    }
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .filter_map(|w| {
            let run = w[0]..w[1];
            let link = links.iter().any(|l| l.start <= run.start && run.end <= l.end);
            let sel = selection.as_ref().is_some_and(|s| s.start <= run.start && run.end <= s.end);
            (link || sel).then_some((run, link, sel))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_selects_either_direction_and_copies() {
        let t = "ciao mondo";
        let mut s = StaticSelection::default();
        s.press(t, 8, 1);
        assert_eq!(s.range(t), None);
        s.drag(t, 5);
        assert_eq!(s.copy(t), Some("mon"));
        s.drag(t, 8);
        assert_eq!(s.range(t), None);
    }

    #[test]
    fn double_click_word_triple_click_line() {
        let t = "ciao à_b mondo\nsecond";
        let mut s = StaticSelection::default();
        s.press(t, 6, 2);
        assert_eq!(s.copy(t), Some("à_b"));
        s.press(t, 2, 2);
        assert_eq!(s.copy(t), Some("ciao"));
        s.press(t, 16, 3);
        assert_eq!(s.copy(t), Some("second"));
        s.press(t, 3, 3);
        assert_eq!(s.copy(t), Some("ciao à_b mondo"));
    }

    #[test]
    fn double_click_on_space_selects_nothing_and_changed_text_drops_it() {
        let t = "a  b";
        let mut s = StaticSelection::default();
        s.press(t, 2, 2);
        assert_eq!(s.range(t), None);
        s.press(t, 0, 3);
        assert_eq!(s.copy(t), Some("a  b"));
        assert_eq!(s.range("altro"), None);
        s.drag("altro", 1);
        assert_eq!(s.range(t), Some(0..4));
    }

    #[test]
    fn index_inside_a_char_snaps_to_boundary() {
        let t = "àb";
        let mut s = StaticSelection::default();
        s.press(t, 1, 1);
        s.drag(t, 99);
        assert_eq!(s.copy(t), Some("àb"));
    }

    #[test]
    fn runs_split_link_and_selection_without_overlap() {
        let runs = highlight_runs(10, &[2..6], Some(4..8));
        assert_eq!(
            runs,
            vec![(2..4, true, false), (4..6, true, true), (6..8, false, true)]
        );
        assert!(highlight_runs(5, &[], None).is_empty());
    }
}
