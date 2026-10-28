//! UI-local editable text; deliberately independent of backend transport.
use std::ops::Range;

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

/// Emoji offered by the picker with the name read by screen readers; no search or recents.
pub const EMOJI_SET: [(&str, &str); 32] = [
    ("Faccina sorridente", "😀"),
    ("Faccina raggiante", "😁"),
    ("Faccina che ride fino alle lacrime", "😂"),
    ("Faccina che rotola dalle risate", "🤣"),
    ("Faccina con occhi sorridenti", "😊"),
    ("Faccina con occhi a cuore", "😍"),
    ("Faccina che manda un bacio", "😘"),
    ("Faccina con occhiali da sole", "😎"),
    ("Faccina pensierosa", "🤔"),
    ("Faccina sorridente con sudore", "😅"),
    ("Faccina che fa l'occhiolino", "😉"),
    ("Faccina capovolta", "🙃"),
    ("Faccina triste", "😢"),
    ("Faccina che piange forte", "😭"),
    ("Faccina arrabbiata", "😡"),
    ("Faccina spaventata", "😱"),
    ("Pollice in su", "👍"),
    ("Pollice in giù", "👎"),
    ("Applauso", "👏"),
    ("Mani giunte", "🙏"),
    ("Bicipite", "💪"),
    ("Mano che saluta", "👋"),
    ("Stretta di mano", "🤝"),
    ("Mani alzate", "🙌"),
    ("Festa", "🎉"),
    ("Fuoco", "🔥"),
    ("Scintille", "✨"),
    ("Cento punti", "💯"),
    ("Segno di spunta", "✅"),
    ("Croce rossa", "❌"),
    ("Occhi", "👀"),
    ("Tazza di caffè", "☕"),
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
}
