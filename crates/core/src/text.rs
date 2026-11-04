//! String helpers for JavaScript-compatible lengths (UTF-16 code units) and ordering.

use std::cmp::Ordering;

pub fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// The longest prefix of `text` that is at most `units` UTF-16 code units long.
pub fn utf16_prefix(text: &str, units: usize) -> &str {
    let mut count = 0;
    for (index, ch) in text.char_indices() {
        count += ch.len_utf16();
        if count > units {
            return &text[..index];
        }
    }
    text
}

/// Approximates JavaScript's `localeCompare` (ICU root collation) for names: case-insensitive
/// first, then lowercase before uppercase. Exact ICU order for punctuation and non-ASCII letters
/// would need a collation library.
pub fn locale_compare(a: &str, b: &str) -> Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| b.cmp(a))
}
