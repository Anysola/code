//! Finding oldString in a file and building each replacement, in 0.5.1's four tiers: an endings-only
//! edit matches the raw bytes; every other edit matches the LF view (exact, then typographically
//! normalized, then line by line ignoring trailing whitespace) and maps offsets back to the file.

use std::ops::Range;

const CRLF: &str = "\r\n";

/// JavaScript's `\s` / `trim()` set, which differs from Rust's (it has U+FEFF, not U+0085).
pub(super) fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{B}' | '\u{C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

fn normalize_char(c: char) -> char {
    match c {
        '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
        '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
        '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
        '\u{A0}' | '\u{2002}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}' => ' ',
        c => c,
    }
}

/// Typographic quotes, dashes and spaces folded to ASCII, one char for one char.
pub(super) fn normalize(text: &str) -> String {
    text.chars().map(normalize_char).collect()
}

/// [`normalize`] plus, for every byte offset of the result (and its end), the offset in `text`.
fn normalize_mapped(text: &str) -> (String, Vec<usize>) {
    let mut normalized = String::with_capacity(text.len());
    let mut offsets = Vec::with_capacity(text.len() + 1);
    for (index, c) in text.char_indices() {
        let folded = normalize_char(c);
        normalized.push(folded);
        offsets.extend(std::iter::repeat_n(index, folded.len_utf8()));
    }
    offsets.push(text.len());
    (normalized, offsets)
}

fn line_key(line: &str) -> String {
    normalize(line.trim_end_matches(is_js_space))
}

/// Non-overlapping occurrences, left to right. `search` is never empty.
fn occurrences(content: &str, search: &str) -> Vec<Range<usize>> {
    content
        .match_indices(search)
        .map(|(start, found)| start..start + found.len())
        .collect()
}

/// Line-wise matching: every line equal after trimming trailing whitespace and normalizing.
fn line_occurrences(content: &str, search: &str) -> Vec<Range<usize>> {
    struct Line<'a> {
        start: usize,
        end: usize,
        text: &'a str,
        content_end: usize,
        newline: bool,
    }
    let trailing_newline = search.ends_with('\n');
    let mut expected: Vec<String> = search.split('\n').map(line_key).collect();
    if trailing_newline {
        expected.pop();
    }
    let mut start = 0;
    let lines: Vec<Line> = content
        .split_inclusive('\n')
        .map(|segment| {
            let newline = segment.ends_with('\n');
            let text = segment.strip_suffix('\n').unwrap_or(segment);
            let line = Line {
                start,
                end: start + segment.len(),
                text,
                content_end: start + text.len() - usize::from(text.ends_with('\r')),
                newline,
            };
            start += segment.len();
            line
        })
        .collect();
    let mut result: Vec<Range<usize>> = Vec::new();
    for window in lines.windows(expected.len()) {
        let equal = window
            .iter()
            .zip(&expected)
            .all(|(line, key)| line_key(line.text) == *key);
        let last = &window[window.len() - 1];
        if !equal || (trailing_newline && !last.newline) {
            continue;
        }
        let candidate = window[0].start..if trailing_newline {
            last.end
        } else {
            last.content_end
        };
        if !result
            .iter()
            .any(|m| m.end > candidate.start && m.start < candidate.end)
        {
            result.push(candidate);
        }
    }
    result
}

/// The dominant line ending of `text`, or `fallback` on a tie.
fn line_ending(text: &str, fallback: &'static str) -> &'static str {
    let crlf = text.matches(CRLF).count();
    let lf = text.matches('\n').count() - crlf;
    match crlf.cmp(&lf) {
        std::cmp::Ordering::Equal => fallback,
        std::cmp::Ordering::Greater => CRLF,
        std::cmp::Ordering::Less => "\n",
    }
}

/// Joins the replacement's lines with endings from the matched region: a line that keeps or
/// replaces a region line reuses that line's ending; any other line takes `fallback`.
fn with_region_endings(region: &str, replacement: &str, fallback: &str) -> String {
    let pieces: Vec<&str> = region.split('\n').collect();
    let before: Vec<String> = pieces
        .iter()
        .enumerate()
        .map(|(index, piece)| {
            let line = if index + 1 < pieces.len() {
                piece.strip_suffix('\r').unwrap_or(piece)
            } else {
                piece
            };
            line_key(line)
        })
        .collect();
    let endings: Vec<&str> = pieces[..pieces.len() - 1]
        .iter()
        .map(|piece| if piece.ends_with('\r') { CRLF } else { "\n" })
        .collect();
    let after: Vec<&str> = replacement.split('\n').collect();
    let keys: Vec<String> = after.iter().map(|line| line_key(line)).collect();
    let source = align_lines(&before, &keys);
    let mut result = String::with_capacity(replacement.len());
    for (index, line) in after.iter().enumerate() {
        result.push_str(line);
        if index + 1 < after.len() {
            let ending = source[index].and_then(|i| endings.get(i).copied());
            result.push_str(ending.unwrap_or(fallback));
        }
    }
    result
}

/// For each line of `after`, the `before` line it keeps (longest common subsequence) or, between
/// kept lines, the one it takes the place of.
fn align_lines(before: &[String], after: &[String]) -> Vec<Option<usize>> {
    let width = after.len() + 1;
    let mut table = vec![0u32; (before.len() + 1) * width];
    for i in (0..before.len()).rev() {
        for j in (0..after.len()).rev() {
            table[i * width + j] = if before[i] == after[j] {
                table[(i + 1) * width + j + 1] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }
    // Lines between two kept lines pair up in order: the k-th new line replaces the k-th old one.
    fn flush(
        result: &mut [Option<usize>],
        gap_before: &mut Vec<usize>,
        gap_after: &mut Vec<usize>,
    ) {
        for (k, j) in gap_after.drain(..).enumerate() {
            result[j] = gap_before.get(k).copied();
        }
        gap_before.clear();
    }
    let mut result = vec![None; after.len()];
    let (mut gap_before, mut gap_after) = (Vec::new(), Vec::new());
    let (mut i, mut j) = (0, 0);
    while i < before.len() && j < after.len() {
        if before[i] == after[j] {
            flush(&mut result, &mut gap_before, &mut gap_after);
            result[j] = Some(i);
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            gap_before.push(i);
            i += 1;
        } else {
            gap_after.push(j);
            j += 1;
        }
    }
    gap_before.extend(i..before.len());
    gap_after.extend(j..after.len());
    flush(&mut result, &mut gap_before, &mut gap_after);
    result
}

enum Replacement<'a> {
    /// An endings-only edit: newString as given.
    Verbatim(&'a str),
    /// newString in the LF view, re-joined with the region's endings.
    Region {
        text: String,
        file_ending: &'static str,
    },
}

pub(super) struct Matches<'a> {
    /// Byte ranges in the source, sorted and disjoint.
    pub ranges: Vec<Range<usize>>,
    replacement: Replacement<'a>,
}

impl Matches<'_> {
    /// `source` with its first `count` matches replaced.
    pub fn apply(&self, source: &str, count: usize) -> String {
        let mut result = String::with_capacity(source.len());
        let mut last = 0;
        for range in self.ranges.iter().take(count) {
            result.push_str(&source[last..range.start]);
            match &self.replacement {
                Replacement::Verbatim(text) => result.push_str(text),
                Replacement::Region { text, file_ending } => {
                    let region = &source[range.clone()];
                    let fallback = line_ending(region, file_ending);
                    result.push_str(&with_region_endings(region, text, fallback));
                }
            }
            last = range.end;
        }
        result.push_str(&source[last..]);
        result
    }
}

/// Strings that differ only in `\r` change line endings and apply to the exact bytes.
pub(super) fn find<'a>(source: &str, old: &str, new: &'a str) -> Matches<'a> {
    if old.replace('\r', "") == new.replace('\r', "") {
        return Matches {
            ranges: occurrences(source, old),
            replacement: Replacement::Verbatim(new),
        };
    }
    let view = source.replace(CRLF, "\n");
    let search = old.replace(CRLF, "\n");
    let mut ranges = occurrences(&view, &search);
    if ranges.is_empty() {
        let (normalized, offsets) = normalize_mapped(&view);
        ranges = occurrences(&normalized, &normalize(&search))
            .into_iter()
            .map(|r| offsets[r.start]..offsets[r.end])
            .collect();
    }
    if ranges.is_empty() {
        ranges = line_occurrences(&view, &search);
    }
    // View offset of each `\r` dropped from the source.
    let removed: Vec<usize> = source
        .match_indices(CRLF)
        .enumerate()
        .map(|(k, (index, _))| index - k)
        .collect();
    let to_source = |offset: usize| offset + removed.partition_point(|&p| p < offset);
    Matches {
        ranges: ranges
            .into_iter()
            .map(|r| to_source(r.start)..to_source(r.end))
            .collect(),
        replacement: Replacement::Region {
            text: new.replace(CRLF, "\n"),
            file_ending: line_ending(source, "\n"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(source: &str, old: &str, new: &str, all: bool) -> Option<String> {
        let matches = find(source, old, new);
        if matches.ranges.is_empty() {
            return None;
        }
        Some(matches.apply(source, if all { usize::MAX } else { 1 }))
    }

    #[test]
    fn exact_tier() {
        assert_eq!(edit("a b a", "a", "c", true).unwrap(), "c b c");
        assert_eq!(edit("aaaa", "aa", "b", true).unwrap(), "bb");
        assert_eq!(find("x", "y", "z").ranges, Vec::<Range<usize>>::new());
    }

    #[test]
    fn crlf_file_keeps_crlf() {
        assert_eq!(
            edit("one\r\ntwo\r\nthree\r\n", "two\n", "TWO\nextra\n", false).unwrap(),
            "one\r\nTWO\r\nextra\r\nthree\r\n"
        );
        assert_eq!(
            edit("a\r\nb\r\n", "a\nb", "x\ny\nz", false).unwrap(),
            "x\r\ny\r\nz\r\n"
        );
    }

    #[test]
    fn mixed_endings_keep_context_bytes() {
        let source = "a\r\nb\nc\r\n";
        assert_eq!(
            edit(source, "a\nb\nc", "a\nB\nc", false).unwrap(),
            "a\r\nB\nc\r\n"
        );
    }

    #[test]
    fn inserted_lines_use_dominant_ending_ties_to_lf() {
        // A one-line region has no ending; the file has one CRLF and one LF: tie -> "\n".
        assert_eq!(
            edit("x\r\ny\nz", "z", "z1\nz2", false).unwrap(),
            "x\r\ny\nz1\nz2"
        );
        assert_eq!(
            edit("x\r\ny\r\nz", "z", "z1\nz2", false).unwrap(),
            "x\r\ny\r\nz1\r\nz2"
        );
    }

    #[test]
    fn endings_only_edit_applies_bytes() {
        assert_eq!(edit("a\nb\n", "a\n", "a\r\n", false).unwrap(), "a\r\nb\n");
        assert!(find("a\r\nb", "a\n", "a\r\n").ranges.is_empty());
    }

    #[test]
    fn typographic_tier_maps_offsets() {
        let source = "let z = \u{201C}hi\u{201D}\nnext";
        assert_eq!(
            edit(source, "let z = \"hi\"", "let z = \"bye\"", false).unwrap(),
            "let z = \"bye\"\nnext"
        );
        let source = "a\u{2014}b\u{A0}c d";
        assert_eq!(edit(source, "b c", "X", false).unwrap(), "a\u{2014}X d");
    }

    #[test]
    fn line_tier_ignores_trailing_whitespace() {
        assert_eq!(
            edit("foo   \nbar", "foo\nbar", "baz", false).unwrap(),
            "baz"
        );
        assert_eq!(
            edit("foo \r\nbar\r\n", "foo\nbar\n", "x\n", false).unwrap(),
            "x\r\n"
        );
        // A trailing newline in oldString needs one after the last matched line.
        assert!(find("foo  \nbar", "foo\nbar\n", "x").ranges.is_empty());
        // Without one, the match stops before the last line's `\r`.
        assert_eq!(find("a \nb\r", "a\nb", "x").ranges, vec![0..4]);
    }

    #[test]
    fn line_tier_drops_overlaps() {
        assert_eq!(find("x \nx \nx \n", "x\nx", "y").ranges, vec![0..5]);
    }

    #[test]
    fn js_whitespace_set() {
        assert!(is_js_space('\u{FEFF}'));
        assert!(!is_js_space('\u{85}'));
        assert_eq!(line_key("a\u{FEFF} \t"), "a");
    }

    #[test]
    fn alignment() {
        let keys = |s: &str| s.split(',').map(String::from).collect::<Vec<_>>();
        assert_eq!(
            align_lines(&keys("a,b,c"), &keys("a,x,y,c")),
            vec![Some(0), Some(1), None, Some(2)]
        );
        assert_eq!(align_lines(&keys("a"), &keys("b,a")), vec![None, Some(0)]);
    }
}
