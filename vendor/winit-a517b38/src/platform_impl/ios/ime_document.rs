//! The text that `WinitView` shows UIKit through `UITextInput`.
//!
//! winit doesn't see the app's text, so the view reports the text typed since the caret last moved
//! for another reason (a tap, a field change, a reset), with the caret at its end and the marked
//! (pre-edit) text after it. That is enough for what the keyboard does next to the caret:
//! autocorrection replaces the word just typed, and a candidate replaces the marked text.
//!
//! Offsets are UTF-16 code units, as in `NSRange`. They keep growing across resets, so a range UIKit
//! computed before a reset never matches text typed after it. Text before `start` is unknown.

/// Text kept for replacements, in characters.
const KEPT_CHARS: usize = 256;

#[derive(Debug, Default)]
pub(super) struct ImeDocument {
    /// The text typed since the last reset; the caret is at its end.
    typed: String,
    /// The offset of `typed` in the document.
    start: usize,
    /// The composition after the caret.
    marked: String,
    /// The selection inside `marked`: location and length in UTF-16 units.
    marked_selection: (usize, usize),
}

impl ImeDocument {
    pub fn start(&self) -> usize {
        self.start
    }

    /// The caret's offset, which is also where marked text starts.
    pub fn caret(&self) -> usize {
        self.start + utf16_len(&self.typed)
    }

    /// The end of the marked text, or the caret.
    pub fn end(&self) -> usize {
        self.caret() + utf16_len(&self.marked)
    }

    pub fn is_empty(&self) -> bool {
        self.typed.is_empty() && self.marked.is_empty()
    }

    pub fn marked(&self) -> &str {
        &self.marked
    }

    /// Forget the text: the caret may have moved. The composition is dropped too.
    pub fn reset(&mut self) {
        self.start = self.end();
        self.typed.clear();
        self.marked.clear();
        self.marked_selection = (0, 0);
    }

    /// Text reached the app at the caret.
    pub fn insert(&mut self, text: &str) {
        self.typed.push_str(text);
        let excess = self.typed.chars().count().saturating_sub(KEPT_CHARS);
        if excess > 0 {
            let cut = self.typed.char_indices().nth(excess).map_or(self.typed.len(), |(i, _)| i);
            self.start += utf16_len(&self.typed[..cut]);
            self.typed.drain(..cut);
        }
    }

    /// The app deletes the character before the caret (Backspace). Text that may be a grapheme
    /// cluster (joiners, combining marks, flags, skin tones) isn't deleted by one scalar: forget it.
    pub fn delete_backward(&mut self) {
        let mut tail = self.typed.chars().rev();
        match (tail.next(), tail.next()) {
            (Some(last), previous) if !is_cluster_part(last) && !previous.is_some_and(is_cluster_part) => {
                self.typed.pop();
            },
            _ => self.reset(),
        }
    }

    /// Replace the marked text. `selection` is `NSRange` units inside `text`.
    pub fn set_marked(&mut self, text: String, selection: (usize, usize)) {
        let len = utf16_len(&text);
        let location = selection.0.min(len);
        self.marked_selection = (location, selection.1.min(len - location));
        self.marked = text;
    }

    /// The marked text is gone: committed or discarded by the caller.
    pub fn take_marked(&mut self) -> String {
        self.marked_selection = (0, 0);
        std::mem::take(&mut self.marked)
    }

    /// The marked text without its last character (Backspace during a composition).
    pub fn delete_marked_backward(&mut self) {
        self.marked.pop();
        let len = utf16_len(&self.marked);
        self.marked_selection = (len, 0);
    }

    /// The selection inside the marked text as UTF-8 byte offsets, as `Ime::Preedit` wants it.
    pub fn marked_selection_bytes(&self) -> (usize, usize) {
        let (location, length) = self.marked_selection;
        (utf16_to_byte(&self.marked, location), utf16_to_byte(&self.marked, location + length))
    }

    /// The selection to report: the caret, or the selection within the marked text.
    pub fn selected_range(&self) -> (usize, usize) {
        let (location, length) = self.marked_selection;
        if self.marked.is_empty() {
            (self.caret(), 0)
        } else {
            (self.caret() + location, length)
        }
    }

    /// Move the selection inside the marked text. Anything else would need the app's own text.
    pub fn select(&mut self, (location, length): (usize, usize)) {
        let caret = self.caret();
        if !self.marked.is_empty() && location >= caret && location + length <= self.end() {
            self.marked_selection = (location - caret, length);
        }
    }

    /// The known text (typed, then marked) in `range`, with the range it actually covers.
    pub fn substring(&self, (location, length): (usize, usize)) -> Option<(String, (usize, usize))> {
        let from = location.max(self.start);
        let to = location.saturating_add(length).min(self.end());
        if from >= to {
            return None;
        }
        let all = format!("{}{}", self.typed, self.marked);
        let from_byte = byte_offset(&all, self.start, from);
        let to_byte = byte_offset(&all, self.start, to);
        let text = &all[from_byte..to_byte];
        let from = self.start + utf16_len(&all[..from_byte]);
        Some((text.to_owned(), (from, utf16_len(text))))
    }

    /// Apply `replaceRange:withText:`: the replaced text is removed from the document and the
    /// marked text is dropped. Returns how many bytes before the caret the app has to delete, or
    /// `None` for a range the document can't answer.
    ///
    /// Only a range that reaches the caret is honoured: one that ends before it would need the app
    /// to delete text behind the caret, and one that starts before `start` is text we never saw.
    pub fn replace(&mut self, (location, length): (usize, usize)) -> Option<usize> {
        let caret = self.caret();
        let end = location.saturating_add(length);
        if location < self.start || location > self.end() || end < caret {
            return None;
        }
        self.marked.clear();
        self.marked_selection = (0, 0);
        if location >= caret {
            return Some(0);
        }
        let from = byte_offset(&self.typed, self.start, location);
        Some(self.typed.split_off(from).len())
    }
}

/// The byte offset in `text` (which starts at document offset `start`) of a document offset,
/// moved back to a character boundary.
fn byte_offset(text: &str, start: usize, offset: usize) -> usize {
    let mut units = start;
    for (index, c) in text.char_indices() {
        units += c.len_utf16();
        if units > offset {
            return index;
        }
    }
    text.len()
}

fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// UTF-8 byte offset in `text` of a UTF-16 offset, clamped to the text.
pub(super) fn utf16_to_byte(text: &str, offset: usize) -> usize {
    byte_offset(text, 0, offset)
}

/// A scalar that joins a grapheme cluster: combining marks, joiner, variation selectors, regional
/// indicators, skin tone modifiers, keycap/enclosing marks (U+20E3) and emoji tags (flag sequences).
fn is_cluster_part(c: char) -> bool {
    matches!(c, '\u{300}'..='\u{36F}' | '\u{200D}' | '\u{20D0}'..='\u{20FF}' | '\u{FE00}'..='\u{FE0F}' | '\u{1F1E6}'..='\u{1F1FF}' | '\u{1F3FB}'..='\u{1F3FF}' | '\u{E0020}'..='\u{E007F}')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> ImeDocument {
        let mut document = ImeDocument::default();
        document.insert(text);
        document
    }

    #[test]
    fn autocorrect_replaces_the_word_before_the_caret() {
        let mut document = typed("see teh");
        assert_eq!(document.substring((4, 3)), Some(("teh".to_owned(), (4, 3))));
        assert_eq!(document.replace((4, 3)), Some(3));
        document.insert("the ");
        assert_eq!(document.substring((0, 20)), Some(("see the ".to_owned(), (0, 8))));
    }

    #[test]
    fn a_candidate_replaces_the_marked_text() {
        let mut document = typed("a");
        document.set_marked("ni".to_owned(), (2, 0));
        assert_eq!(document.selected_range(), (3, 0));
        assert_eq!(document.end(), 3);
        // The keyboard replaces exactly the marked range: nothing typed is deleted.
        assert_eq!(document.replace((1, 2)), Some(0));
        assert_eq!(document.marked(), "");
        document.insert("你");
        assert_eq!(document.caret(), 2);
    }

    #[test]
    fn a_range_over_typed_and_marked_text_deletes_the_typed_part() {
        let mut document = typed("e");
        document.set_marked("n".to_owned(), (1, 0));
        assert_eq!(document.replace((0, 2)), Some(1));
        assert_eq!(document.caret(), 0);
    }

    #[test]
    fn ranges_the_document_cannot_answer_are_ignored() {
        let mut document = typed("abc");
        for range in [(0, 1), (1, 1), (2, 0), (9, 1)] {
            assert_eq!(document.replace(range), None, "{range:?}");
        }
        document.reset();
        document.insert("x");
        // A range from before the reset.
        assert_eq!(document.replace((2, 1)), None);
        assert_eq!(document.substring((0, 3)), None);
        assert_eq!(document.replace((3, 1)), Some(1));
    }

    #[test]
    fn offsets_count_utf16_units_and_bytes_are_exact() {
        let mut document = typed("a😀b");
        assert_eq!(document.caret(), 4);
        // A range starting inside the surrogate pair takes the whole character.
        assert_eq!(document.replace((2, 2)), Some("😀b".len()));
        assert_eq!(document.caret(), 1);
    }

    #[test]
    fn marked_selection_is_in_utf16_units() {
        let mut document = ImeDocument::default();
        document.set_marked("😀a".to_owned(), (2, 9));
        assert_eq!(document.selected_range(), (2, 1));
        assert_eq!(utf16_to_byte("😀a", 2), 4);
        assert_eq!(document.marked_selection_bytes(), (4, 5));
        document.select((0, 3));
        assert_eq!(document.selected_range(), (0, 3));
        document.select((5, 1));
        assert_eq!(document.selected_range(), (0, 3));
    }

    #[test]
    fn old_text_is_dropped_but_offsets_stay() {
        let mut document = typed(&"a".repeat(KEPT_CHARS));
        document.insert("bc");
        assert_eq!(document.caret(), KEPT_CHARS + 2);
        assert_eq!(document.substring((0, 2)), None);
        assert_eq!(document.replace((KEPT_CHARS, 2)), Some(2));
    }

    #[test]
    fn backspace_keeps_the_document_in_step() {
        let mut document = typed("ab");
        document.delete_backward();
        assert_eq!(document.substring((0, 5)), Some(("a".to_owned(), (0, 1))));
        // A cluster isn't one scalar for the app: forget instead of guessing.
        let mut document = typed("a👍🏽");
        document.delete_backward();
        assert_eq!(document.substring((0, 9)), None);
        assert_eq!(document.caret(), 5);
        document.insert("é");
        document.delete_backward();
        assert_eq!(document.caret(), 5);
    }

    #[test]
    fn backspace_in_a_composition_edits_only_the_marked_text() {
        let mut document = typed("ab");
        document.set_marked("ni".to_owned(), (2, 0));
        document.delete_marked_backward();
        assert_eq!(document.marked(), "n");
        assert_eq!(document.take_marked(), "n");
        assert_eq!(document.substring((0, 9)), Some(("ab".to_owned(), (0, 2))));
    }

    #[test]
    fn reset_drops_composition_and_text() {
        let mut document = typed("ab");
        document.set_marked("c".to_owned(), (1, 0));
        document.reset();
        assert_eq!((document.start(), document.caret(), document.end()), (3, 3, 3));
        assert_eq!(document.substring((0, 9)), None);
    }
    #[test]
    fn backspace_after_keycap_tags_zwj_and_flags_forgets_the_text() {
        // egui deletes the whole cluster; a single scalar here would put the document out of step.
        for cluster in ["1\u{FE0F}\u{20E3}", "#\u{20E3}", "\u{1F3F4}\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}", "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}", "\u{1F1EE}\u{1F1F9}", "\u{1F44B}\u{1F3FD}"] {
            let mut document = typed(&format!("a{cluster}"));
            let caret = document.caret();
            document.delete_backward();
            assert_eq!(document.substring((0, 99)), None, "{cluster:?}");
            assert_eq!(document.caret(), caret, "{cluster:?}");
        }
    }

    #[test]
    fn backspace_on_a_lone_emoji_next_to_cjk_and_rtl_text_removes_one_scalar() {
        for text in ["\u{4F60}\u{597D}\u{1F44D}", "\u{5E9}\u{5DC}\u{5D5}\u{5DD}\u{1F44D}", "\u{645}\u{631}\u{62D}\u{628}\u{627}\u{1F44D}"] {
            let mut document = typed(text);
            document.delete_backward();
            let kept = text.trim_end_matches('\u{1F44D}');
            assert_eq!(document.substring((0, 99)).map(|(t, _)| t), Some(kept.to_owned()), "{text:?}");
        }
    }

    #[test]
    fn utf16_offsets_over_zwj_family_next_to_cjk_and_rtl() {
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        for lead in ["\u{4F60}\u{597D}", "\u{5E9}\u{5DC}"] {
            let mut document = typed(&format!("{lead}{family}"));
            assert_eq!(document.caret(), 2 + 8); // 3 surrogate pairs + 2 joiners
            // A range that starts inside the first surrogate pair takes the whole emoji.
            assert_eq!(document.replace((3, 7)), Some(family.len()));
            assert_eq!(document.substring((0, 99)).map(|(t, _)| t), Some(lead.to_owned()));
        }
        assert_eq!(utf16_to_byte(&format!("\u{4F60}{family}"), 1), 3);
        assert_eq!(utf16_to_byte("1\u{FE0F}\u{20E3}", 2), 4);
    }
}
