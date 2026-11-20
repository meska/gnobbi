//! UI-local editable text; deliberately independent of backend transport.
use std::ops::Range;

/// Byte offset of the grapheme cluster before `cursor`: ZWJ, skin-tone, flag and keycap emoji move and delete as one.
pub fn prev_boundary(text: &str, cursor: usize) -> usize {
    use unicode_segmentation::UnicodeSegmentation as _;
    text[..cursor]
        .graphemes(true)
        .next_back()
        .map_or(0, |g| cursor - g.len())
}

/// Byte offset after the grapheme cluster that starts at `cursor`.
pub fn next_boundary(text: &str, cursor: usize) -> usize {
    use unicode_segmentation::UnicodeSegmentation as _;
    text[cursor..]
        .graphemes(true)
        .next()
        .map_or(cursor, |g| cursor + g.len())
}

fn utf16_byte(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (index, ch) in text.char_indices() {
        if units + ch.len_utf16() > offset {
            return index;
        }
        units += ch.len_utf16();
    }
    text.len()
}

#[derive(Clone)]
struct Snapshot {
    text: String,
    selected: Range<usize>,
}

#[derive(Default)]
pub struct Composer {
    text: String,
    selected: Range<usize>,
    marked: Option<Range<usize>>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

impl Composer {
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn selection(&self) -> Range<usize> {
        self.selected.clone()
    }
    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    /// Highlight spans for rendering: (range, selected, marked). Sorted and disjoint,
    /// clamped to the text and to char boundaries: gpui slices the text by these lengths.
    pub fn highlights(&self) -> Vec<(Range<usize>, bool, bool)> {
        let marked = self.marked.clone().unwrap_or(0..0);
        let mut cuts = vec![self.selected.start, self.selected.end, marked.start, marked.end];
        cuts.retain(|&c| self.text.is_char_boundary(c));
        cuts.sort_unstable();
        cuts.dedup();
        cuts.windows(2)
            .map(|w| {
                let (selected, marked) = (self.selected.contains(&w[0]), marked.contains(&w[0]));
                (w[0]..w[1], selected, marked)
            })
            .filter(|(_, selected, marked)| *selected || *marked)
            .collect()
    }

    pub fn unmark(&mut self) {
        self.marked = None;
    }
    pub fn from_utf16(&self, range: Range<usize>) -> Range<usize> {
        utf16_byte(&self.text, range.start)..utf16_byte(&self.text, range.end)
    }
    pub fn to_utf16(&self, range: Range<usize>) -> Range<usize> {
        self.text[..range.start].encode_utf16().count()
            ..self.text[..range.end].encode_utf16().count()
    }

    /// Range is UTF-8; the IME's new selection is UTF-16 relative to preedit.
    pub fn mark(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
    ) -> bool {
        let range = range
            .or(self.marked.clone())
            .unwrap_or(self.selected.clone());
        if !self.replace(Some(range.clone()), text) {
            return false;
        }
        self.marked = (!text.is_empty()).then_some(range.start..range.start + text.len());
        if let Some(selected) = selected {
            let byte_offset = |offset| utf16_byte(text, offset);
            self.selected = range.start + byte_offset(selected.start.min(selected.end))
                ..range.start + byte_offset(selected.end.max(selected.start));
        }
        true
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            selected: self.selected.clone(),
        }
    }
    fn restore(&mut self, state: Snapshot) {
        self.text = state.text;
        self.selected = state.selected;
        self.marked = None;
    }
    fn valid(&self, range: &Range<usize>) -> bool {
        range.start <= range.end
            && self.text.is_char_boundary(range.start)
            && self.text.is_char_boundary(range.end)
    }
    pub fn select(&mut self, range: Range<usize>) {
        if self.valid(&range) {
            self.selected = range;
        }
    }
    pub fn undo(&mut self) {
        if let Some(state) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.restore(state);
        }
    }
    pub fn redo(&mut self) {
        if let Some(state) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.restore(state);
        }
    }

    pub fn replace(&mut self, range: Option<Range<usize>>, text: &str) -> bool {
        let range = range
            .or(self.marked.clone())
            .unwrap_or(self.selected.clone());
        if !self.valid(&range) || text.len() > 65536 - (self.text.len() - range.len()) {
            return false;
        }
        if self.marked.is_none() {
            if self.undo.len() == 32 {
                self.undo.remove(0);
            }
            self.undo.push(self.snapshot());
        }
        self.redo.clear();
        self.marked = None;
        self.text.replace_range(range.clone(), text);
        let cursor = range.start + text.len();
        self.selected = cursor..cursor;
        true
    }

    pub fn enter(&mut self, shift: bool) -> Option<String> {
        if self.marked.is_some() {
            return None;
        }
        if shift {
            self.replace(None, "\n");
            return None;
        }
        if self.text.trim().is_empty() {
            return None;
        }
        self.selected = 0..0;
        self.undo.clear();
        self.redo.clear();
        Some(std::mem::take(&mut self.text))
    }
}

/// The `@query` being typed just before `cursor`: start of the `@` and the text after it.
pub fn mention_token(text: &str, cursor: usize) -> Option<(usize, &str)> {
    let head = text.get(..cursor)?;
    let at = head.rfind('@')?;
    let query = &head[at + 1..];
    let boundary = head[..at]
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace);
    (boundary && !query.contains(char::is_whitespace) && query.len() <= 64).then_some((at, query))
}

/// Up to six members for `query`: names (or user ids) starting with it first, then containing it.
pub fn mention_matches<'a>(
    members: &'a [matrix_engine::operations::Member],
    query: &str,
) -> Vec<&'a matrix_engine::operations::Member> {
    let query = query.to_lowercase();
    let rank = |m: &matrix_engine::operations::Member| {
        let name = m.name.trim_start_matches('@').to_lowercase();
        let id = m.user_id.trim_start_matches('@').to_lowercase();
        if name.starts_with(&query) || id.starts_with(&query) {
            Some(0)
        } else if name.contains(&query) || id.contains(&query) {
            Some(1)
        } else {
            None
        }
    };
    let mut ranked: Vec<_> = members
        .iter()
        .filter_map(|m| rank(m).map(|r| (r, m)))
        .collect();
    ranked.sort_by_key(|(r, m)| (*r, m.name.to_lowercase()));
    ranked.into_iter().take(6).map(|(_, m)| m).collect()
}

/// Emoji offered by the picker with the name read by screen readers (an `i18n` key, shown through `t`); no search or recents.
pub const EMOJI_SET: [(&str, &str); 32] = [
    ("Grinning face", "😀"),
    ("Beaming face", "😁"),
    ("Face with tears of joy", "😂"),
    ("Rolling on the floor laughing", "🤣"),
    ("Smiling face with smiling eyes", "😊"),
    ("Smiling face with heart eyes", "😍"),
    ("Face blowing a kiss", "😘"),
    ("Smiling face with sunglasses", "😎"),
    ("Thinking face", "🤔"),
    ("Grinning face with sweat", "😅"),
    ("Winking face", "😉"),
    ("Upside-down face", "🙃"),
    ("Crying face", "😢"),
    ("Loudly crying face", "😭"),
    ("Angry face", "😡"),
    ("Screaming face", "😱"),
    ("Thumbs up", "👍"),
    ("Thumbs down", "👎"),
    ("Clapping hands", "👏"),
    ("Folded hands", "🙏"),
    ("Flexed biceps", "💪"),
    ("Waving hand", "👋"),
    ("Handshake", "🤝"),
    ("Raising hands", "🙌"),
    ("Party popper", "🎉"),
    ("Fire", "🔥"),
    ("Sparkles", "✨"),
    ("Hundred points", "💯"),
    ("Check mark", "✅"),
    ("Cross mark", "❌"),
    ("Eyes", "👀"),
    ("Cup of coffee", "☕"),
];

#[cfg(test)]
mod mention_tests {
    use super::{mention_matches, mention_token};
    use matrix_engine::operations::Member;

    #[test]
    fn token_needs_a_word_boundary_and_no_space() {
        assert_eq!(mention_token("ciao @An", 8), Some((5, "An")));
        assert_eq!(mention_token("@", 1), Some((0, "")));
        assert_eq!(mention_token("mail@host", 9), None);
        assert_eq!(mention_token("@Anna ciao", 10), None);
        assert_eq!(mention_token("ciao", 4), None);
    }

    #[test]
    fn matches_rank_prefix_before_contains_and_cap_at_six() {
        let m = |id: &str, name: &str| Member {
            user_id: id.into(),
            name: name.into(),
            avatar: None,
            role: Default::default(),
        };
        let members = vec![m("@b:x", "Sabrina"), m("@a:x", "Bruno"), m("@c:x", "@c:x")];
        let names: Vec<_> = mention_matches(&members, "br")
            .iter()
            .map(|m| m.name.as_str())
            .collect();
        assert_eq!(names, ["Bruno", "Sabrina"]);
        assert_eq!(mention_matches(&members, "C").len(), 1);
        let many: Vec<_> = (0..9)
            .map(|i| m(&format!("@u{i}:x"), &format!("U{i}")))
            .collect();
        assert_eq!(mention_matches(&many, "").len(), 6);
    }

    #[test]
    fn matches_are_identical_with_and_without_avatars() {
        let m = |id: &str, name: &str, avatar: Option<&str>| Member {
            user_id: id.into(),
            name: name.into(),
            avatar: avatar.map(Into::into),
            role: Default::default(),
        };
        let with = vec![
            m("@b:x", "Sabrina", Some("mxc://x/b")),
            m("@a:x", "Bruno", Some("")),
            m("@c:x", "@c:x", None),
            m("@d:x", "Brunella", Some("mxc://x/d")),
            m("@e:x", "anna", None),
        ];
        let without: Vec<_> = with.iter().map(|x| Member { avatar: None, ..x.clone() }).collect();
        for query in ["", "br", "BR", "a", "c", "x", "@", "zzz", "sabri"] {
            let ids = |list: &[Member]| -> Vec<String> {
                mention_matches(list, query).iter().map(|x| x.user_id.clone()).collect()
            };
            assert_eq!(ids(&with), ids(&without), "query {query:?}");
        }
        let many: Vec<_> = (0..9).map(|i| m(&format!("@u{i}:x"), &format!("U{i}"), Some("mxc://x/u"))).collect();
        assert_eq!(mention_matches(&many, "").len(), 6, "cap of six holds with avatars");
        // The result still carries the avatar so the list can draw it.
        assert_eq!(mention_matches(&with, "sabri")[0].avatar.as_deref(), Some("mxc://x/b"));
    }
}

#[cfg(test)]
mod cluster_tests {
    #[test]
    fn emoji_sequences_move_and_delete_as_one_cluster() {
        for seq in [
            "👨\u{200d}👩\u{200d}👧",
            "👍🏽",
            "🇮🇹",
            "1\u{fe0f}\u{20e3}",
            "❤\u{fe0f}",
            "è",
        ] {
            let text = format!("a{seq}b");
            let after = 1 + seq.len();
            assert_eq!(super::next_boundary(&text, 1), after, "{seq:?} forward");
            assert_eq!(super::prev_boundary(&text, after), 1, "{seq:?} backward");
        }
        assert_eq!(super::prev_boundary("", 0), 0);
        assert_eq!(super::next_boundary("a", 1), 1);
    }
}

#[cfg(test)]
mod highlight_tests {
    use super::Composer;

    /// gpui turns highlights into consecutive text runs: they must be sorted,
    /// disjoint and on char boundaries, or its layout slices the text and aborts.
    fn assert_valid(c: &Composer) {
        let text = c.text();
        let mut end = 0;
        for (r, _, _) in c.highlights() {
            assert!(r.start >= end && r.start < r.end && r.end <= text.len(), "{r:?} after {end}");
            assert!(text.is_char_boundary(r.start) && text.is_char_boundary(r.end));
            end = r.end;
        }
    }

    #[test]
    fn caret_selection_inside_preedit_gives_disjoint_runs() {
        let mut c = Composer::default();
        c.replace(None, "ciao ");
        // IME preedit "èà" with the selection covering its first char only.
        c.mark(None, "èà", Some(0..1));
        assert_valid(&c);
        // Selection that precedes the marked span.
        c.mark(None, "ìò", Some(0..2));
        c.select(0..2);
        assert_valid(&c);
    }
}
