use matrix_domain::ScopedTimelineId;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_SELECTION: AtomicU64 = AtomicU64::new(1);

/// A single message-body selection. Timestamps and adjacent bubbles are excluded.
pub struct TextSelection {
    generation: u64,
    id: Option<ScopedTimelineId>,
    text: String,
    anchor: usize,
    head: usize,
}

impl Default for TextSelection {
    fn default() -> Self {
        Self {
            generation: NEXT_SELECTION
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .expect("selection generation exhausted"),
            id: None,
            text: String::new(),
            anchor: 0,
            head: 0,
        }
    }
}

impl TextSelection {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn extend_current(
        &mut self,
        generation: u64,
        id: &ScopedTimelineId,
        text: &str,
        index: usize,
    ) {
        if generation == self.generation {
            self.extend(id, text, index);
        }
    }

    pub fn begin_current(
        &mut self,
        generation: u64,
        id: ScopedTimelineId,
        text: &str,
        index: usize,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
        self.begin(id, text, index);
        true
    }

    pub fn begin(&mut self, id: ScopedTimelineId, text: &str, index: usize) {
        self.clear();
        self.id = Some(id);
        self.text = text.to_owned();
        self.anchor = text.floor_char_boundary(index.min(text.len()));
        self.head = self.anchor;
    }

    pub fn extend(&mut self, id: &ScopedTimelineId, text: &str, index: usize) {
        if self.id.as_ref() == Some(id) && self.text == text {
            let selected = self.anchor != self.head;
            self.head = text.floor_char_boundary(index.min(text.len()));
            if selected && self.anchor == self.head {
                self.generation = Self::default().generation;
            }
        }
    }

    pub fn is_active(&self, id: &ScopedTimelineId, text: &str) -> bool {
        self.id.as_ref() == Some(id) && self.text == text
    }

    pub fn range(&self, id: &ScopedTimelineId, text: &str) -> Option<Range<usize>> {
        (self.id.as_ref() == Some(id) && self.text == text && self.anchor != self.head)
            .then(|| self.anchor.min(self.head)..self.anchor.max(self.head))
    }

    pub fn copy<'a>(&self, id: &ScopedTimelineId, text: &'a str) -> Option<&'a str> {
        text.get(self.range(id, text)?)
    }

    /// Home/End address the message body; arrows move by Unicode scalar, not bytes.
    pub fn key(&mut self, id: &ScopedTimelineId, text: &str, key: &str, shift: bool) -> bool {
        if self.id.as_ref() != Some(id) || self.text != text {
            return false;
        }
        let selected = self.anchor != self.head;
        if key == "select-all" {
            self.anchor = 0;
            self.head = text.len();
            return true;
        }
        if !shift && let Some(range) = self.range(id, text) {
            match key {
                "left" => {
                    self.anchor = range.start;
                    self.head = range.start;
                    self.generation = Self::default().generation;
                    return true;
                }
                "right" => {
                    self.anchor = range.end;
                    self.head = range.end;
                    self.generation = Self::default().generation;
                    return true;
                }
                _ => {}
            }
        }
        self.head = match key {
            "home" => 0,
            "end" => text.len(),
            "left" => text[..self.head]
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i),
            "right" => text[self.head..]
                .chars()
                .next()
                .map_or(text.len(), |c| self.head + c.len_utf8()),
            _ => return false,
        };
        if !shift {
            self.anchor = self.head;
        }
        if selected && self.anchor == self.head {
            self.generation = Self::default().generation;
        }
        true
    }

    pub fn retain_current<'a>(
        &mut self,
        bodies: impl IntoIterator<Item = (&'a ScopedTimelineId, &'a str)>,
    ) {
        if self.id.is_some()
            && !bodies
                .into_iter()
                .any(|(id, text)| self.is_active(id, text))
        {
            self.clear();
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}
