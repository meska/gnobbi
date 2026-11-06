//! Message-text matching shared by the room search service and the highlight in the timeline.
use std::ops::Range;

/// A term shorter than this is not searched.
pub const MIN_CHARS: usize = 2;
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

/// Case- and accent-insensitive: NFKD, combining marks dropped, lowercase.
fn fold(c: char) -> impl Iterator<Item = char> {
    std::iter::once(c).nfkd().filter(|c| !is_combining_mark(*c)).flat_map(char::to_lowercase)
}

/// Byte ranges of `term` («contains») in `text`, measured on `text` itself.
pub fn find_ranges(text: &str, term: &str) -> Vec<Range<usize>> {
    let needle: String = term.chars().flat_map(fold).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let (mut folded, mut origin) = (String::new(), Vec::<(usize, usize)>::new());
    for (start, c) in text.char_indices() {
        for f in fold(c) {
            folded.push(f);
            origin.extend(std::iter::repeat_n((start, start + c.len_utf8()), f.len_utf8()));
        }
    }
    folded.match_indices(&needle).map(|(at, m)| origin[at].0..origin[at + m.len() - 1].1).collect()
}
