//! The UI diff attached to edit and write: jsdiff's `createTwoFilesPatch` (4 lines of context,
//! `Index:` header) and `diffLines` addition/deletion counts. jsdiff's Myers variant is ported
//! rather than using `similar`, whose tie-breaking puts insertions first.

use serde::Serialize;
use serde_json::Value;

const CONTEXT: usize = 4;
const UNDERLINE: &str = "===================================================================";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Status {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct FileDiff {
    pub file: String,
    pub patch: String,
    pub status: Status,
    pub additions: usize,
    pub deletions: usize,
}

impl FileDiff {
    pub fn new(file: &str, before: &str, after: &str, status: Status) -> Self {
        let changes = changes(before, after);
        let count = |kind| {
            changes
                .iter()
                .filter(|c| c.kind == kind)
                .map(|c| c.lines.len())
                .sum()
        };
        FileDiff {
            file: file.to_string(),
            patch: patch(file, &changes),
            status,
            additions: count(Kind::Added),
            deletions: count(Kind::Removed),
        }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("a file diff serializes")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Equal,
    Removed,
    Added,
}

struct Change<'a> {
    kind: Kind,
    /// Line tokens, each keeping its own newline.
    lines: Vec<&'a str>,
}

/// jsdiff's line tokens: each line with its own newline.
fn tokens(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

#[derive(Clone, Copy)]
struct Component {
    kind: Kind,
    count: usize,
    previous: Option<usize>,
}

/// A furthest-reaching path on one diagonal: its old-side position and last component.
#[derive(Clone, Copy)]
struct Reach {
    old_pos: isize,
    last: Option<usize>,
}

/// jsdiff's `diffLines`: Myers' algorithm that, on a tie, extends the path furthest along the old
/// text, which lists removals before additions.
fn changes<'a>(before: &'a str, after: &'a str) -> Vec<Change<'a>> {
    let (old, new) = (tokens(before), tokens(after));
    let (old_len, new_len) = (old.len() as isize, new.len() as isize);
    let mut components: Vec<Component> = Vec::new();
    // Follows the diagonal while tokens are equal; returns the new-side position.
    let extract_common = |components: &mut Vec<Component>, reach: &mut Reach, diagonal: isize| {
        let (mut old_pos, mut new_pos) = (reach.old_pos, reach.old_pos - diagonal);
        let start = old_pos;
        while new_pos + 1 < new_len
            && old_pos + 1 < old_len
            && old[(old_pos + 1) as usize] == new[(new_pos + 1) as usize]
        {
            old_pos += 1;
            new_pos += 1;
        }
        if old_pos > start {
            components.push(Component {
                kind: Kind::Equal,
                count: (old_pos - start) as usize,
                previous: reach.last,
            });
            reach.last = Some(components.len() - 1);
        }
        reach.old_pos = old_pos;
        new_pos
    };
    let extend = |components: &mut Vec<Component>, reach: Reach, kind: Kind| {
        let last = reach.last.map(|i| components[i]);
        let component = match last {
            Some(last) if last.kind == kind => Component {
                count: last.count + 1,
                ..last
            },
            _ => Component {
                kind,
                count: 1,
                previous: reach.last,
            },
        };
        components.push(component);
        Reach {
            old_pos: reach.old_pos + isize::from(kind == Kind::Removed),
            last: Some(components.len() - 1),
        }
    };

    let max_edit = old_len + new_len;
    let offset = max_edit + 1;
    let mut best: Vec<Option<Reach>> = vec![None; (2 * offset + 1) as usize];
    let slot = |diagonal: isize| (diagonal + offset) as usize;
    let mut first = Reach {
        old_pos: -1,
        last: None,
    };
    let new_pos = extract_common(&mut components, &mut first, 0);
    let mut done = (first.old_pos + 1 >= old_len && new_pos + 1 >= new_len).then_some(first.last);
    best[slot(0)] = Some(first);
    let (mut min_diagonal, mut max_diagonal) = (isize::MIN, isize::MAX);
    let mut edit_length = 1;
    while done.is_none() && edit_length <= max_edit {
        let mut diagonal = min_diagonal.max(-edit_length);
        while diagonal <= max_diagonal.min(edit_length) {
            let remove = best[slot(diagonal - 1)].take();
            let add = best[slot(diagonal + 1)];
            let can_add = add.is_some_and(|a| (0..new_len).contains(&(a.old_pos - diagonal)));
            let can_remove = remove.is_some_and(|r| r.old_pos + 1 < old_len);
            let mut reach = match (remove, add) {
                (Some(r), Some(a)) if can_remove && !(can_add && r.old_pos < a.old_pos) => {
                    extend(&mut components, r, Kind::Removed)
                }
                (Some(r), None) if can_remove => extend(&mut components, r, Kind::Removed),
                (_, Some(a)) if can_add => extend(&mut components, a, Kind::Added),
                _ => {
                    best[slot(diagonal)] = None;
                    diagonal += 2;
                    continue;
                }
            };
            let new_pos = extract_common(&mut components, &mut reach, diagonal);
            if reach.old_pos + 1 >= old_len && new_pos + 1 >= new_len {
                done = Some(reach.last);
                break;
            }
            best[slot(diagonal)] = Some(reach);
            if reach.old_pos + 1 >= old_len {
                max_diagonal = max_diagonal.min(diagonal - 1);
            }
            if new_pos + 1 >= new_len {
                min_diagonal = min_diagonal.max(diagonal + 1);
            }
            diagonal += 2;
        }
        edit_length += 1;
    }

    let mut path = Vec::new();
    let mut cursor = done.flatten();
    while let Some(index) = cursor {
        path.push(components[index]);
        cursor = components[index].previous;
    }
    let (mut old_pos, mut new_pos) = (0, 0);
    path.iter()
        .rev()
        .map(|component| {
            let (side, pos) = match component.kind {
                Kind::Removed => (&old, &mut old_pos),
                _ => (&new, &mut new_pos),
            };
            let lines = side[*pos..*pos + component.count].to_vec();
            *pos += component.count;
            if component.kind == Kind::Equal {
                old_pos += component.count;
            }
            Change {
                kind: component.kind,
                lines,
            }
        })
        .collect()
}

struct Hunk {
    old_start: usize,
    old_lines: usize,
    new_start: usize,
    new_lines: usize,
    lines: Vec<String>,
}

/// A port of jsdiff's `structuredPatch` + `formatPatch` with identical file names and no headers.
fn patch(file: &str, changes: &[Change]) -> String {
    let header = [
        format!("Index: {file}"),
        UNDERLINE.to_string(),
        format!("--- {file}"),
        format!("+++ {file}"),
    ];
    let mut out = header.join("\n") + "\n";
    out.push_str(&format_hunks(changes, CONTEXT));
    out
}

/// Only the `@@` hunks of jsdiff's patch between two texts, with `context` lines around changes.
pub(crate) fn hunks_text(before: &str, after: &str, context: usize) -> String {
    format_hunks(&changes(before, after), context)
}

fn format_hunks(changes: &[Change], context: usize) -> String {
    let mut out = Vec::new();
    for hunk in hunks(changes, context) {
        let old_start = hunk.old_start - usize::from(hunk.old_lines == 0);
        let new_start = hunk.new_start - usize::from(hunk.new_lines == 0);
        out.push(format!(
            "@@ -{old_start},{} +{new_start},{} @@",
            hunk.old_lines, hunk.new_lines
        ));
        for line in hunk.lines {
            match line.strip_suffix('\n') {
                Some(line) => out.push(line.to_string()),
                None => {
                    out.push(line);
                    out.push("\\ No newline at end of file".to_string());
                }
            }
        }
    }
    out.iter().map(|l| format!("{l}\n")).collect()
}

fn hunks(changes: &[Change], context_size: usize) -> Vec<Hunk> {
    let sentinel = Change {
        kind: Kind::Equal,
        lines: Vec::new(),
    };
    let all: Vec<&Change> = changes.iter().chain(std::iter::once(&sentinel)).collect();
    let context = |lines: &[&str]| lines.iter().map(|l| format!(" {l}")).collect::<Vec<_>>();
    let mut hunks = Vec::new();
    let (mut old_range_start, mut new_range_start) = (0, 0);
    let mut range: Vec<String> = Vec::new();
    let (mut old_line, mut new_line) = (1, 1);
    for (i, current) in all.iter().enumerate() {
        let lines = &current.lines;
        if current.kind != Kind::Equal {
            if old_range_start == 0 {
                old_range_start = old_line;
                new_range_start = new_line;
                if let Some(prev) = i.checked_sub(1).map(|p| all[p]) {
                    range = context(&prev.lines[prev.lines.len().saturating_sub(context_size)..]);
                    old_range_start -= range.len();
                    new_range_start -= range.len();
                }
            }
            let sign = if current.kind == Kind::Added {
                '+'
            } else {
                '-'
            };
            range.extend(lines.iter().map(|l| format!("{sign}{l}")));
            if current.kind == Kind::Added {
                new_line += lines.len();
            } else {
                old_line += lines.len();
            }
            continue;
        }
        if old_range_start != 0 {
            if lines.len() <= context_size * 2 && i + 2 < all.len() {
                range.extend(context(lines));
            } else {
                let size = lines.len().min(context_size);
                range.extend(context(&lines[..size]));
                hunks.push(Hunk {
                    old_start: old_range_start,
                    old_lines: old_line - old_range_start + size,
                    new_start: new_range_start,
                    new_lines: new_line - new_range_start + size,
                    lines: std::mem::take(&mut range),
                });
                old_range_start = 0;
                new_range_start = 0;
            }
        }
        old_line += lines.len();
        new_line += lines.len();
    }
    hunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_change() {
        let diff = FileDiff::new("notes.md", "a\nb\nc\n", "a\nB\nc\n", Status::Modified);
        assert_eq!(
            diff.patch,
            "Index: notes.md\n===================================================================\n--- notes.md\n+++ notes.md\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n"
        );
        assert_eq!((diff.additions, diff.deletions), (1, 1));
        assert_eq!(
            diff.to_value().to_string(),
            format!(
                "{{\"file\":\"notes.md\",\"patch\":{},\"status\":\"modified\",\"additions\":1,\"deletions\":1}}",
                serde_json::to_string(&diff.patch).unwrap()
            )
        );
    }

    #[test]
    fn created_file_without_trailing_newline() {
        let diff = FileDiff::new("n.txt", "", "hello", Status::Added);
        assert_eq!(
            diff.patch,
            "Index: n.txt\n===================================================================\n--- n.txt\n+++ n.txt\n@@ -0,0 +1,1 @@\n+hello\n\\ No newline at end of file\n"
        );
        assert_eq!((diff.additions, diff.deletions), (1, 0));
    }

    #[test]
    fn last_line_gains_newline() {
        let diff = FileDiff::new("f", "x\ny", "x\ny\n", Status::Modified);
        assert_eq!(
            diff.patch,
            "Index: f\n===================================================================\n--- f\n+++ f\n@@ -1,2 +1,2 @@\n x\n-y\n\\ No newline at end of file\n+y\n"
        );
    }

    #[test]
    fn distant_changes_make_two_hunks_and_near_ones_merge() {
        let before: String = (1..=20).map(|n| format!("{n}\n")).collect();
        let swap = |changes: &[(usize, &str)]| {
            let mut lines: Vec<String> = (1..=20).map(|n| format!("{n}\n")).collect();
            for (n, text) in changes {
                lines[n - 1] = format!("{text}\n");
            }
            lines.concat()
        };
        let far = swap(&[(2, "two"), (19, "nineteen")]);
        let patch = FileDiff::new("f", &before, &far, Status::Modified).patch;
        assert!(patch.contains("@@ -1,6 +1,6 @@\n 1\n-2\n+two\n 3\n 4\n 5\n 6\n"));
        assert!(patch.contains("@@ -15,6 +15,6 @@\n 15\n 16\n 17\n 18\n-19\n+nineteen\n 20\n"));
        let near = swap(&[(2, "two"), (10, "ten")]);
        let patch = FileDiff::new("f", &before, &near, Status::Modified).patch;
        assert!(patch.contains("@@ -1,14 +1,14 @@\n"));
        assert_eq!(patch.matches("@@ -").count(), 1);
    }

    #[test]
    fn identical_texts_have_no_hunks() {
        assert_eq!(
            FileDiff::new("f", "a\n", "a\n", Status::Modified).patch,
            "Index: f\n===================================================================\n--- f\n+++ f\n"
        );
    }
    /// Patches and counts produced by jsdiff 8.0.4 (0.5.1's `diff`) for the same inputs.
    const JSDIFF: &str = r#"[["a\nb\nc\n", "a\nB\nc\n", "Index: f\n===================================================================\n--- f\n+++ f\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n", [1, 1]], ["", "hello", "Index: f\n===================================================================\n--- f\n+++ f\n@@ -0,0 +1,1 @@\n+hello\n\\ No newline at end of file\n", [1, 0]], ["x\ny", "x\ny\n", "Index: f\n===================================================================\n--- f\n+++ f\n@@ -1,2 +1,2 @@\n x\n-y\n\\ No newline at end of file\n+y\n", [1, 1]], ["a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n", "a\nX\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\nY\nt\n", "Index: f\n===================================================================\n--- f\n+++ f\n@@ -1,6 +1,6 @@\n a\n-b\n+X\n c\n d\n e\n f\n@@ -15,6 +15,6 @@\n o\n p\n q\n r\n-s\n+Y\n t\n", [2, 2]], ["a\nb\na\nb\na\n", "b\na\nb\na\nb\n", "Index: f\n===================================================================\n--- f\n+++ f\n@@ -1,5 +1,5 @@\n-a\n b\n a\n b\n a\n+b\n", [1, 1]], ["one\r\ntwo\r\n", "one\r\nnew\r\ntwo\r\n", "Index: f\n===================================================================\n--- f\n+++ f\n@@ -1,2 +1,3 @@\n one\r\n+new\r\n two\r\n", [1, 0]], ["1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n", "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n", "Index: f\n===================================================================\n--- f\n+++ f\n@@ -7,4 +7,5 @@\n 7\n 8\n 9\n 10\n+11\n", [1, 0]], ["x\n", "", "Index: f\n===================================================================\n--- f\n+++ f\n@@ -1,1 +0,0 @@\n-x\n", [0, 1]], ["a\nb\nc\nd\n", "a\nc\nb\nd\n", "Index: f\n===================================================================\n--- f\n+++ f\n@@ -1,4 +1,4 @@\n a\n-b\n c\n+b\n d\n", [1, 1]]]"#;

    #[test]
    fn matches_jsdiff() {
        let cases: Vec<(String, String, String, (usize, usize))> =
            serde_json::from_str(JSDIFF).unwrap();
        for (before, after, patch, (additions, deletions)) in cases {
            let diff = FileDiff::new("f", &before, &after, Status::Modified);
            assert_eq!(diff.patch, patch, "{before:?} -> {after:?}");
            assert_eq!((diff.additions, diff.deletions), (additions, deletions));
        }
    }
}
