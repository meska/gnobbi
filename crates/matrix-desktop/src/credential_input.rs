//! Single-line credential buffer. Protected values never export; no undo history or formatting.

/// Native input-protocol projection. The field kind is fixed at construction;
/// protected inputs never expose their contents through paint/surrounding-text.
pub struct LoginInput {
    buffer: CredentialInput,
    protected: bool,
    cursor: usize,
    marked: Option<std::ops::Range<usize>>,
}
impl LoginInput {
    pub fn secret() -> Self {
        Self {
            buffer: CredentialInput::default(),
            protected: true,
            cursor: 0,
            marked: None,
        }
    }
    /// Public text that may span lines (room topic): newlines are kept, never trimmed.
    pub fn multiline() -> Self {
        let mut input = Self::public();
        input.buffer.multiline = true;
        input
    }
    pub fn is_multiline(&self) -> bool {
        self.buffer.multiline
    }
    pub fn public() -> Self {
        Self {
            buffer: CredentialInput::default(),
            protected: false,
            cursor: 0,
            marked: None,
        }
    }
    pub fn replace(&mut self, range: Option<std::ops::Range<usize>>, text: &str) -> bool {
        let changed = self
            .buffer
            .replace(range.or_else(|| self.marked.clone()), text);
        if changed {
            self.cursor = self.buffer.selection.end;
            self.marked = None;
        }
        changed
    }
    pub fn mark(
        &mut self,
        range: Option<std::ops::Range<usize>>,
        text: &str,
        selected: Option<std::ops::Range<usize>>,
    ) -> bool {
        if let Some(selected) = &selected {
            let boundary = |target| {
                let mut offset = 0;
                for c in text.chars() {
                    if offset == target {
                        return true;
                    }
                    offset += c.len_utf16();
                }
                offset == target
            };
            if selected.start > selected.end || !boundary(selected.start) || !boundary(selected.end)
            {
                return false;
            }
        }
        let start = range
            .as_ref()
            .or(self.marked.as_ref())
            .map_or(self.buffer.selection.start, |range| range.start);
        if !self.replace(range, text) {
            return false;
        }
        self.marked = Some(start..start + text.encode_utf16().count());
        if let Some(selected) = selected {
            self.buffer
                .select_utf16(start + selected.start..start + selected.end);
            self.cursor = self.buffer.selection.end;
        }
        true
    }
    pub fn unmark(&mut self) {
        self.marked = None;
    }
    pub fn marked_utf16(&self) -> Option<std::ops::Range<usize>> {
        self.marked.clone()
    }
    pub fn display_text(&self) -> String {
        if self.protected {
            self.buffer.masked_text()
        } else {
            self.buffer.value.to_string()
        }
    }
    pub fn text_for_range(&self, range: std::ops::Range<usize>) -> Option<String> {
        let display = self.display_text();
        fn byte(text: &str, target: usize) -> Option<usize> {
            let mut offset = 0;
            for (i, c) in text.char_indices() {
                if offset == target {
                    return Some(i);
                }
                offset += c.len_utf16();
            }
            (offset == target).then_some(text.len())
        }
        display
            .get(byte(&display, range.start)?..byte(&display, range.end)?)
            .map(str::to_owned)
    }
    pub fn selection_utf16(&self) -> std::ops::Range<usize> {
        self.buffer.selection_utf16()
    }
    /// Selected text for the clipboard; protected or composing inputs never export.
    pub fn exportable_selection(&self) -> Option<String> {
        let range = self.buffer.selection_utf16();
        if self.protected || self.marked.is_some() || range.is_empty() {
            return None;
        }
        self.text_for_range(range)
    }
    pub fn edit(&mut self, key: &str, control: bool, shift: bool) -> bool {
        if self.marked.is_some() {
            return false;
        }
        let end = self.buffer.value.encode_utf16().count();
        if control {
            if key != "a" {
                return false;
            }
            self.buffer.select_utf16(0..end);
            self.cursor = end;
            return true;
        }
        let selected = self.buffer.selection_utf16();
        let before = (0..self.cursor)
            .rev()
            .find(|i| self.buffer.byte_offset(*i).is_some())
            .unwrap_or(0);
        let after = ((self.cursor + 1)..=end)
            .find(|i| self.buffer.byte_offset(*i).is_some())
            .unwrap_or(end);
        match key {
            "backspace" | "delete" => {
                if selected.is_empty() {
                    self.buffer.select_utf16(if key == "backspace" {
                        before..self.cursor
                    } else {
                        self.cursor..after
                    });
                }
                self.replace(None, "");
            }
            "home" | "end" | "left" | "right" => {
                let next = match key {
                    "home" | "end" => self.line_edge(key == "end"),
                    "left" if !selected.is_empty() && !shift => selected.start,
                    "right" if !selected.is_empty() && !shift => selected.end,
                    "left" => before,
                    _ => after,
                };
                self.move_to(next, shift);
            }
            _ => return false,
        }
        true
    }
    /// Moves the caret to a UTF-16 offset, extending the selection when `shift` is held.
    pub fn move_to(&mut self, next: usize, shift: bool) {
        let selected = self.buffer.selection_utf16();
        let anchor = if !shift {
            next
        } else if self.cursor == selected.start {
            selected.end
        } else {
            selected.start
        };
        self.buffer.select_utf16(anchor.min(next)..anchor.max(next));
        self.cursor = next;
    }
    /// Click selection at a UTF-16 offset: 1 caret (Shift extends), 2 word, 3+ line.
    /// Protected text never reveals its structure: a multi-click selects all of it.
    pub fn click_at(&mut self, at: usize, clicks: usize, shift: bool) {
        if self.marked.is_some() {
            return;
        }
        let end = self.buffer.value.encode_utf16().count();
        let at = at.min(end);
        let range = match clicks {
            0 | 1 => return self.move_to(at, shift),
            _ if self.protected => 0..end,
            2 => word_range(&self.buffer.value, at),
            _ => line_range(&self.buffer.value, at, self.buffer.multiline),
        };
        self.buffer.select_utf16(range.clone());
        self.cursor = range.end;
    }
    /// Start or end of the caret's line (the whole text when single-line).
    fn line_edge(&self, end: bool) -> usize {
        let mut line = (0, 0);
        let mut offset = 0;
        for c in self.buffer.value.chars() {
            if c == '\n' && offset >= self.cursor {
                break;
            }
            offset += c.len_utf16();
            if c == '\n' {
                line.0 = offset;
            }
            line.1 = offset;
        }
        match (self.buffer.multiline, end) {
            (false, false) => 0,
            (false, true) => self.buffer.value.encode_utf16().count(),
            (true, false) => line.0,
            // The loop stops before the '\n' that ends the line, or at the end of the text.
            (true, true) => line.1,
        }
    }
    pub fn take(&mut self) -> zeroize::Zeroizing<String> {
        self.cursor = 0;
        self.marked = None;
        self.buffer.take_for_authentication()
    }
}
use std::ops::Range;
use zeroize::Zeroizing;

#[derive(Default)]
pub struct CredentialInput {
    value: Zeroizing<String>,
    selection: Range<usize>,
    multiline: bool,
}
impl CredentialInput {
    /// Range and selection are UTF-16 offsets, matching the native input protocol.
    pub fn replace(&mut self, range: Option<Range<usize>>, text: &str) -> bool {
        let range = range.unwrap_or_else(|| self.selection.clone());
        let text = &if self.multiline { text.replace("\r\n", "\n").replace('\r', "\n") } else { text.to_owned() };
        let max = if self.multiline { 32768 } else { 4096 };
        let Some(start) = self.byte_offset(range.start) else {
            return false;
        };
        let Some(end) = self.byte_offset(range.end) else {
            return false;
        };
        if start > end
            || text.len() > max
            || text.chars().any(|c| c.is_control() && !(self.multiline && c == '\n'))
            || self.value.len() - (end - start) + text.len() > max
        {
            return false;
        }
        // Allocate once: growing a String mid-edit could leave a nonzeroized
        // old allocation outside the Zeroizing owner's final buffer.
        let mut next = Zeroizing::new(String::with_capacity(
            self.value.len() - (end - start) + text.len(),
        ));
        next.push_str(&self.value[..start]);
        next.push_str(text);
        next.push_str(&self.value[end..]);
        self.value = next;
        let caret = range.start + text.encode_utf16().count();
        self.selection = caret..caret;
        true
    }

    pub fn select_utf16(&mut self, range: Range<usize>) -> bool {
        if range.start > range.end
            || self.byte_offset(range.start).is_none()
            || self.byte_offset(range.end).is_none()
        {
            return false;
        }
        self.selection = range;
        true
    }

    pub fn masked_text(&self) -> String {
        "•".repeat(self.value.encode_utf16().count())
    }
    pub fn selection_utf16(&self) -> Range<usize> {
        self.selection.clone()
    }

    pub fn take_for_authentication(&mut self) -> Zeroizing<String> {
        self.selection = 0..0;
        std::mem::take(&mut self.value)
    }

    fn byte_offset(&self, utf16: usize) -> Option<usize> {
        let mut offset = 0;
        for (byte, c) in self.value.char_indices() {
            if offset == utf16 {
                return Some(byte);
            }
            offset += c.len_utf16();
        }
        (offset == utf16).then_some(self.value.len())
    }
}

#[cfg(test)]
mod multiline_tests {
    use super::LoginInput;

    #[test]
    fn topic_keeps_lines_and_single_line_still_rejects_them() {
        let mut topic = LoginInput::multiline();
        assert!(topic.replace(None, "uno\r\ndue\rtre"));
        assert_eq!(topic.display_text(), "uno\ndue\ntre");
        assert!(topic.replace(None, &"x".repeat(5000)), "longer than a single-line field");
        assert!(!LoginInput::public().replace(None, "a\nb"));
        assert!(!topic.replace(None, "a\tb"), "tabs stay rejected");
    }

    #[test]
    fn three_lines_round_trip_and_size_cap() {
        let mut topic = LoginInput::multiline();
        assert!(topic.replace(None, "  uno \ndue\ntre"));
        assert_eq!(topic.display_text(), "  uno \ndue\ntre", "leading and inner spaces untouched");
        assert!(topic.replace(Some(0..topic.display_text().encode_utf16().count()), &"x".repeat(32768)));
        assert!(!topic.replace(None, "y"), "32 KB is the cap");
        let mut other = LoginInput::multiline();
        assert!(!other.replace(None, &"x".repeat(32769)));
    }

    #[test]
    fn home_end_stay_on_the_line() {
        let mut topic = LoginInput::multiline();
        topic.replace(None, "ab\ncde\nf");
        topic.move_to(5, false); // between d and e
        topic.edit("home", false, false);
        assert_eq!(topic.selection_utf16(), 3..3);
        topic.edit("end", false, false);
        assert_eq!(topic.selection_utf16(), 6..6);
        topic.move_to(7, false);
        topic.edit("end", false, true);
        assert_eq!(topic.selection_utf16(), 7..8);
    }
}

/// UTF-16 range of the word (letters/digits/`_`), blank run or symbol run around `at`.
pub fn word_range(text: &str, at: usize) -> std::ops::Range<usize> {
    let class = |c: char| match c {
        c if c.is_alphanumeric() || c == '_' => 0,
        c if c.is_whitespace() => 1,
        _ => 2,
    };
    let mut spans = Vec::new(); // (start, end, class) per char
    let mut offset = 0;
    for c in text.chars() {
        spans.push((offset, offset + c.len_utf16(), class(c)));
        offset += c.len_utf16();
    }
    // At the very end, or between two chars, the char to the right wins; at the end the last one.
    let Some(i) = spans.iter().position(|s| at < s.1).or(spans.len().checked_sub(1)) else {
        return 0..0;
    };
    let kind = spans[i].2;
    let mut a = i;
    while a > 0 && spans[a - 1].2 == kind {
        a -= 1;
    }
    let mut b = i;
    while b + 1 < spans.len() && spans[b + 1].2 == kind {
        b += 1;
    }
    spans[a].0..spans[b].1
}

/// UTF-16 range of the line around `at` (the whole text when single-line), without its `\n`.
pub fn line_range(text: &str, at: usize, multiline: bool) -> std::ops::Range<usize> {
    let total = text.encode_utf16().count();
    if !multiline {
        return 0..total;
    }
    let (mut start, mut offset) = (0, 0);
    for c in text.chars() {
        if c == '\n' {
            if offset >= at {
                return start..offset;
            }
            start = offset + 1;
        }
        offset += c.len_utf16();
    }
    start..total
}

#[cfg(test)]
mod click_tests {
    use super::*;

    #[test]
    fn words_blanks_and_symbols() {
        let t = "ciao mondo, @x_1";
        assert_eq!(word_range(t, 0), 0..4);
        assert_eq!(word_range(t, 4), 4..5, "boundary: char to the right");
        assert_eq!(word_range(t, 7), 5..10);
        assert_eq!(word_range(t, 10), 10..11);
        assert_eq!(word_range(t, 16), 13..16, "end of text: last word");
        assert_eq!(word_range("", 0), 0..0);
        assert_eq!(word_range("a😀b", 1), 1..3, "UTF-16 pairs stay whole");
    }

    #[test]
    fn lines() {
        let t = "uno\ndue\n\ntre";
        assert_eq!(line_range(t, 1, true), 0..3);
        assert_eq!(line_range(t, 3, true), 0..3);
        assert_eq!(line_range(t, 4, true), 4..7);
        assert_eq!(line_range(t, 8, true), 8..8);
        assert_eq!(line_range(t, 12, true), 9..12);
        assert_eq!(line_range(t, 5, false), 0..12);
    }

    #[test]
    fn click_counts_and_shift() {
        let mut i = LoginInput::public();
        i.replace(None, "ciao mondo");
        i.click_at(7, 2, false);
        assert_eq!(i.selection_utf16(), 5..10);
        i.click_at(1, 3, false);
        assert_eq!(i.selection_utf16(), 0..10);
        i.click_at(2, 1, false);
        assert_eq!(i.selection_utf16(), 2..2);
        i.click_at(6, 1, true);
        assert_eq!(i.selection_utf16(), 2..6);
        let mut s = LoginInput::secret();
        s.replace(None, "pa ss");
        s.click_at(1, 2, false);
        assert_eq!(s.selection_utf16(), 0..5);
    }
}
