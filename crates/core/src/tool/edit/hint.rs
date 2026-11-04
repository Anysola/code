//! The hint appended to an oldString miss: newString is already there, or the file lines closest to
//! oldString, numbered like read output.

use std::sync::LazyLock;

use regex::Regex;

use super::matcher::{is_js_space, normalize};
use crate::text::{utf16_len, utf16_prefix};

const HINT_LINES: usize = 12;
const HINT_LINE_LENGTH: usize = 200;

static DISTINCT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\p{L}\p{N}]").expect("valid regex"));

/// Normalized, with whitespace runs collapsed to one space and the ends trimmed.
fn key(line: &str) -> String {
    let normalized = normalize(line);
    let mut result = String::with_capacity(normalized.len());
    let mut space = false;
    for c in normalized.trim_matches(is_js_space).chars() {
        if is_js_space(c) {
            space = true;
            continue;
        }
        if space {
            result.push(' ');
            space = false;
        }
        result.push(c);
    }
    result
}

/// Lines vote for where oldString starts when they match ignoring all whitespace; at least half of
/// oldString's distinctive lines must agree. Only the span between the first and last matching line
/// is shown, so padding oldString cannot reveal the lines after it.
pub(super) fn miss_hint(source: &str, old: &str, new: &str) -> Option<String> {
    let view = source.replace("\r\n", "\n");
    let replacement = new.replace("\r\n", "\n");
    if utf16_len(replacement.trim_matches(is_js_space)) >= 20
        && normalize(&view).contains(&normalize(&replacement))
    {
        return Some(
            "newString is already in the file, so this edit may already be applied.".into(),
        );
    }
    let lines: Vec<&str> = view.split('\n').collect();
    let keys: Vec<String> = lines.iter().map(|line| key(line)).collect();
    let mut positions: std::collections::HashMap<&str, Vec<usize>> = Default::default();
    for (index, value) in keys.iter().enumerate() {
        if DISTINCT.is_match(value) {
            positions.entry(value).or_default().push(index);
        }
    }
    let search: Vec<String> = old.replace("\r\n", "\n").split('\n').map(key).collect();
    // Insertion-ordered, so ties go to the first start that got a vote.
    let mut votes: Vec<(isize, usize)> = Vec::new();
    let mut wanted = 0;
    for (offset, value) in search.iter().enumerate() {
        if !DISTINCT.is_match(value) {
            continue;
        }
        wanted += 1;
        for &index in positions.get(value.as_str()).into_iter().flatten() {
            let start = index as isize - offset as isize;
            match votes.iter_mut().find(|(s, _)| *s == start) {
                Some((_, count)) => *count += 1,
                None => votes.push((start, 1)),
            }
        }
    }
    let (start, count) = votes.into_iter().fold(
        (0, 0),
        |best, vote| if vote.1 > best.1 { vote } else { best },
    );
    if count == 0 || count * 2 < wanted {
        return None;
    }
    let matched: Vec<usize> = search
        .iter()
        .enumerate()
        .filter_map(|(offset, value)| {
            let index = usize::try_from(start + offset as isize).ok()?;
            (!value.is_empty() && keys.get(index) == Some(value)).then_some(index)
        })
        .collect();
    let from = matched[0];
    let to = (matched[matched.len() - 1] + 1).min(from + HINT_LINES);
    let mut hint = vec!["The closest lines in the file:".to_string()];
    hint.extend(lines[from..to].iter().enumerate().map(|(index, line)| {
        let shown = if utf16_len(line) > HINT_LINE_LENGTH {
            format!("{}…", utf16_prefix(line, HINT_LINE_LENGTH))
        } else {
            line.to_string()
        };
        format!("{}: {shown}", from + index + 1)
    }));
    Some(hint.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closest_lines() {
        let source = "const x = 0\nconst y = 1\nconst z = 2\n";
        assert_eq!(
            miss_hint(source, "const  y = 1;", "const y = 3").as_deref(),
            None
        );
        assert_eq!(
            miss_hint(source, "const   y   =  1", "const y = 3").as_deref(),
            Some("The closest lines in the file:\n2: const y = 1")
        );
    }

    #[test]
    fn already_applied() {
        let source = "fn main() {\n    println!(\"hello world\");\n}\n";
        assert_eq!(
            miss_hint(source, "nothing", "    println!(\"hello world\");").as_deref(),
            Some("newString is already in the file, so this edit may already be applied.")
        );
        // Short newStrings never count as already applied.
        assert_eq!(miss_hint("abc\n", "zzz", "abc"), None);
    }

    #[test]
    fn needs_half_of_the_distinctive_lines() {
        let source = "alpha\nbeta\ngamma\n";
        assert_eq!(miss_hint(source, "alpha\nx1\nx2\nx3", "q"), None);
        assert_eq!(
            miss_hint(source, "alpha\nbeta\nx1\nx2", "q").as_deref(),
            Some("The closest lines in the file:\n1: alpha\n2: beta")
        );
    }

    #[test]
    fn long_lines_are_cut_and_span_is_capped() {
        let long = "w".repeat(250);
        let source = format!("{long}\n");
        assert_eq!(
            miss_hint(&source, &format!("  {long}"), "q").unwrap(),
            "The closest lines in the file:\n1: ".to_string() + &"w".repeat(200) + "…"
        );
        let source: String = (1..=20).map(|n| format!("line {n}\n")).collect();
        let old: String = (1..=20).map(|n| format!("line {n} \n")).collect();
        let hint = miss_hint(&source, &old, "q").unwrap();
        assert_eq!(hint.lines().count(), 13);
        assert!(hint.ends_with("\n12: line 12"));
    }
}
