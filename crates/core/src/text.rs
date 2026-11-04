//! String helpers for JavaScript-compatible lengths (UTF-16 code units).

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
