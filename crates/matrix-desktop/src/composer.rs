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
