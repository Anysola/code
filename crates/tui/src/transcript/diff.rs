//! Unified patches drawn as numbered diff rows, unified or side by side, wrapped by word.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

use super::highlight;
use super::markdown::{Line, plain, wrap_spans};
use crate::text::width;
use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Context,
    Removed,
    Added,
}

struct Row {
    kind: Kind,
    old: Option<usize>,
    new: Option<usize>,
    text: String,
}

struct Hunk {
    header: String,
    rows: Vec<Row>,
}

fn parse(patch: &str) -> Vec<Hunk> {
    let mut hunks: Vec<Hunk> = Vec::new();
    let (mut old, mut new) = (0, 0);
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("@@") {
            let start = |sign: char| {
                rest.split_whitespace()
                    .find_map(|t| t.strip_prefix(sign))
                    .and_then(|t| t.split(',').next()?.parse().ok())
                    .unwrap_or(1)
            };
            old = start('-');
            new = start('+');
            hunks.push(Hunk {
                header: format!("@@{}", rest.split("@@").next().unwrap_or_default())
                    .trim_end()
                    .to_string()
                    + " @@",
                rows: Vec::new(),
            });
            continue;
        }
        let Some(hunk) = hunks.last_mut() else {
            continue;
        };
        let (kind, text) = match line.chars().next() {
            Some('+') => (Kind::Added, &line[1..]),
            Some('-') => (Kind::Removed, &line[1..]),
            Some(' ') => (Kind::Context, &line[1..]),
            Some('\\') => continue,
            None => (Kind::Context, ""),
            _ => continue,
        };
        let row = Row {
            kind,
            old: (kind != Kind::Added).then_some(old),
            new: (kind != Kind::Removed).then_some(new),
            text: text.to_string(),
        };
        if kind != Kind::Added {
            old += 1;
        }
        if kind != Kind::Removed {
            new += 1;
        }
        hunk.rows.push(row);
    }
    hunks
}

pub struct Options<'a> {
    pub width: usize,
    pub split: bool,
    pub filetype: &'a str,
    /// Long lines wrap by word (`diffs.wrap`); otherwise they are cut at the edge.
    pub wrap: bool,
    /// Line-number columns; 0 sizes them from the patch.
    pub digits: usize,
}

/// Added-only patches longer than this lay out in chunks (`patch-diff.tsx:24-25`).
pub const VIRTUAL_MIN_LINES: usize = 3000;
pub const VIRTUAL_CHUNK_LINES: usize = 128;

/// A new file's patch cut into `size`-line chunks, each a patch of its own with the file's
/// header; `None` unless the whole patch is one `@@ -0,0 +1,N @@` hunk of `+` lines.
pub fn split_added(patch: &str, size: usize) -> Option<Vec<String>> {
    let at = patch.find("@@ -0,0 +1,")?;
    if at > 0 && !patch[..at].ends_with('\n') {
        return None;
    }
    let header_end = patch[at..].find('\n')? + at;
    let count: usize = patch[at + 11..header_end].split(' ').next()?.parse().ok()?;
    let body = patch[header_end + 1..]
        .strip_suffix('\n')
        .unwrap_or(&patch[header_end + 1..]);
    let mut lines: Vec<&str> = body.split('\n').collect();
    let marker = lines
        .last()
        .filter(|l| l.starts_with("\\ No newline at end of file"))
        .copied();
    if marker.is_some() {
        lines.pop();
    }
    if lines.len() != count || lines.iter().any(|l| !l.starts_with('+')) {
        return None;
    }
    let prefix = &patch[..at];
    Some(
        lines
            .chunks(size)
            .enumerate()
            .map(|(index, slice)| {
                let start = index * size;
                let tail = match marker {
                    Some(m) if start + size >= count => format!("\n{m}"),
                    _ => String::new(),
                };
                format!(
                    "{prefix}@@ -0,0 +{},{} @@\n{}{tail}",
                    start + 1,
                    slice.len(),
                    slice.join("\n")
                )
            })
            .collect(),
    )
}

/// The chunks of a patch the viewer virtualizes (added-only, more than 3000 lines).
pub fn virtual_chunks(patch: &str) -> Option<Vec<String>> {
    split_added(patch, VIRTUAL_CHUNK_LINES).filter(|chunks| {
        (chunks.len() - 1) * VIRTUAL_CHUNK_LINES + counts(chunks.last().unwrap()).1
            > VIRTUAL_MIN_LINES
    })
}

struct Colors {
    text: Color,
    bg: [Color; 3],
    sign: [Color; 3],
    number: Color,
    number_bg: [Color; 3],
    hunk: Color,
}

fn colors(theme: &Theme) -> Colors {
    Colors {
        text: theme.fg("text.base"),
        bg: [
            theme.fg("diff.background.context"),
            theme.fg("diff.background.removed"),
            theme.fg("diff.background.added"),
        ],
        sign: [
            theme.fg("diff.text.context"),
            theme.fg("diff.highlight.removed"),
            theme.fg("diff.highlight.added"),
        ],
        number: theme.fg("diff.lineNumber.text"),
        number_bg: [
            theme.fg("diff.background.context"),
            theme.fg("diff.lineNumber.background.removed"),
            theme.fg("diff.lineNumber.background.added"),
        ],
        hunk: theme.fg("diff.text.hunkHeader"),
    }
}

fn index(kind: Kind) -> usize {
    match kind {
        Kind::Context => 0,
        Kind::Removed => 1,
        Kind::Added => 2,
    }
}

/// One side's lines for a row: gutter, sign and wrapped code filling `room` columns.
fn side(
    row: Option<&Row>,
    number_width: usize,
    room: usize,
    filetype: &str,
    wrap: bool,
    theme: &Theme,
    c: &Colors,
) -> Vec<Line> {
    let Some(row) = row else {
        return vec![vec![Span::styled(
            " ".repeat(room),
            Style::new().bg(c.bg[0]),
        )]];
    };
    let k = index(row.kind);
    let number = match row.kind {
        Kind::Removed => row.old,
        _ => row.new,
    }
    .map_or(String::new(), |n| n.to_string());
    let sign = match row.kind {
        Kind::Context => " ",
        Kind::Removed => "-",
        Kind::Added => "+",
    };
    let gutter = 1 + number_width + 3;
    let code_room = room.saturating_sub(gutter).max(1);
    let code: Line = highlight::highlight(&row.text, filetype, theme)
        .into_iter()
        .next()
        .unwrap_or_default()
        .into_iter()
        .map(|s| {
            let style = s.style.bg(c.bg[k]);
            Span::styled(
                s.content.into_owned(),
                if filetype.is_empty() {
                    style.fg(c.text)
                } else {
                    style
                },
            )
        })
        .collect();
    let wrapped = if wrap {
        wrap_spans(&code, code_room)
    } else {
        vec![clip_spans(code, code_room)]
    };
    wrapped
        .into_iter()
        .enumerate()
        .map(|(i, mut line)| {
            let used = width(&plain(&line));
            let mut out: Line = if i == 0 {
                vec![
                    Span::styled(
                        format!(" {number:>number_width$}"),
                        Style::new().fg(c.number).bg(c.number_bg[k]),
                    ),
                    Span::styled(format!(" {sign} "), Style::new().fg(c.sign[k]).bg(c.bg[k])),
                ]
            } else {
                vec![
                    Span::styled(
                        " ".repeat(1 + number_width),
                        Style::new().bg(c.number_bg[k]),
                    ),
                    Span::styled("   ", Style::new().bg(c.bg[k])),
                ]
            };
            out.append(&mut line);
            out.push(Span::styled(
                " ".repeat(code_room.saturating_sub(used)),
                Style::new().bg(c.bg[k]),
            ));
            out
        })
        .collect()
}

/// The patch as rows exactly `options.width` wide.
/// The rows where each hunk starts in `render`'s output: the first at 0, the others at their
/// `@@` separators.
pub fn hunk_rows(lines: &[Line]) -> Vec<usize> {
    let mut rows = vec![0];
    rows.extend(lines.iter().enumerate().skip(1).filter_map(|(i, line)| {
        line.first()
            .is_some_and(|span| span.content.starts_with(" @@"))
            .then_some(i)
    }));
    rows
}

pub fn render(patch: &str, options: &Options, theme: &Theme) -> Vec<Line> {
    let hunks = parse(patch);
    let c = colors(theme);
    let number_width = hunks
        .iter()
        .flat_map(|h| &h.rows)
        .flat_map(|r| [r.old, r.new])
        .flatten()
        .max()
        .unwrap_or(1)
        .to_string()
        .len()
        .max(options.digits);
    let mut out = Vec::new();
    for (i, hunk) in hunks.iter().enumerate() {
        if i > 0 {
            let text = format!(" {}", hunk.header);
            out.push(vec![Span::styled(
                crate::text::pad_right(&text, options.width),
                Style::new().fg(c.hunk).bg(c.number_bg[0]),
            )]);
        }
        if !options.split {
            for row in &hunk.rows {
                out.extend(side(
                    Some(row),
                    number_width,
                    options.width,
                    options.filetype,
                    options.wrap,
                    theme,
                    &c,
                ));
            }
            continue;
        }
        let left_width = (options.width / 2 + 1).min(options.width);
        let right_width = options.width - left_width;
        let mut rows = hunk.rows.iter().peekable();
        while let Some(row) = rows.next() {
            let (left, right): (Vec<&Row>, Vec<&Row>) = if row.kind == Kind::Context {
                (vec![row], vec![row])
            } else {
                let mut removed = Vec::new();
                let mut added = Vec::new();
                let mut current = Some(row);
                while let Some(r) = current {
                    match r.kind {
                        Kind::Removed => removed.push(r),
                        Kind::Added => added.push(r),
                        Kind::Context => break,
                    }
                    current = rows.next_if(|n| n.kind != Kind::Context);
                }
                (removed, added)
            };
            for n in 0..left.len().max(right.len()) {
                let l = side(
                    left.get(n).copied(),
                    number_width,
                    left_width,
                    options.filetype,
                    options.wrap,
                    theme,
                    &c,
                );
                let r = side(
                    right.get(n).copied(),
                    number_width,
                    right_width,
                    options.filetype,
                    options.wrap,
                    theme,
                    &c,
                );
                for line in 0..l.len().max(r.len()) {
                    let mut joined = l.get(line).cloned().unwrap_or_else(|| {
                        vec![Span::styled(
                            " ".repeat(left_width),
                            Style::new().bg(c.bg[0]),
                        )]
                    });
                    joined.extend(r.get(line).cloned().unwrap_or_else(|| {
                        vec![Span::styled(
                            " ".repeat(right_width),
                            Style::new().bg(c.bg[0]),
                        )]
                    }));
                    out.push(joined);
                }
            }
        }
    }
    out
}

/// `-N` removed and `+N` added line counts of a patch.
pub fn counts(patch: &str) -> (usize, usize) {
    let rows: Vec<Row> = parse(patch).into_iter().flat_map(|h| h.rows).collect();
    (
        rows.iter().filter(|r| r.kind == Kind::Removed).count(),
        rows.iter().filter(|r| r.kind == Kind::Added).count(),
    )
}

/// The part of `line` that fits `room` columns.
fn clip_spans(line: Line, room: usize) -> Line {
    let mut used = 0;
    let mut out = Vec::new();
    for span in line {
        let left = room.saturating_sub(used);
        if left == 0 {
            break;
        }
        let text = crate::text::clip(&span.content, left);
        used += width(&text);
        out.push(Span::styled(text, span.style));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Catalog, Mode};

    const PATCH: &str = "Index: hello.txt\n===\n--- hello.txt\n+++ hello.txt\n@@ -1,1 +1,1 @@\n-Helo wrold\n+Hello world\n";

    fn text(split: bool, width: usize) -> Vec<String> {
        let theme = Catalog::bundled().load("anysola", Mode::Dark).unwrap();
        let options = Options {
            width,
            split,
            filetype: "",
            wrap: true,
            digits: 0,
        };
        render(PATCH, &options, &theme)
            .iter()
            .map(|l| plain(l).trim_end().to_string())
            .collect()
    }

    #[test]
    fn hunk_rows_follow_separators() {
        let theme = Catalog::bundled().load("anysola", Mode::Dark).unwrap();
        let patch = "--- a\n+++ a\n@@ -1,1 +1,1 @@\n-a\n+b\n@@ -9,1 +9,1 @@\n-c\n+d\n";
        let options = Options {
            width: 40,
            split: false,
            filetype: "",
            wrap: true,
            digits: 0,
        };
        assert_eq!(hunk_rows(&render(patch, &options, &theme)), [0, 2]);
    }

    #[test]
    fn unified_and_split() {
        assert_eq!(text(false, 40), [" 1 - Helo wrold", " 1 + Hello world"]);
        let split = text(true, 116);
        assert_eq!(split.len(), 1);
        assert_eq!(split[0].find("1 +"), Some(60));
    }

    #[test]
    fn no_wrap_cuts_at_the_edge() {
        let theme = Catalog::bundled().load("anysola", Mode::Dark).unwrap();
        let patch = "--- a\n+++ a\n@@ -1,1 +1,1 @@\n-a\n+one two three four five six seven\n";
        let rows = |wrap| {
            let options = Options {
                width: 20,
                split: false,
                filetype: "",
                wrap,
                digits: 0,
            };
            render(patch, &options, &theme).len()
        };
        assert!(rows(true) > rows(false));
        assert_eq!(rows(false), 2);
    }

    #[test]
    fn added_patches_split_into_chunks() {
        let patch =
            "--- /dev/null\n+++ b/a\n@@ -0,0 +1,3 @@\n+a\n+b\n+c\n\\ No newline at end of file\n";
        let chunks = split_added(patch, 2).unwrap();
        assert_eq!(chunks[0], "--- /dev/null\n+++ b/a\n@@ -0,0 +1,2 @@\n+a\n+b");
        assert_eq!(
            chunks[1],
            "--- /dev/null\n+++ b/a\n@@ -0,0 +3,1 @@\n+c\n\\ No newline at end of file"
        );
        assert!(split_added("@@ -1,2 +1,2 @@\n-a\n+b\n", 2).is_none());
        assert!(virtual_chunks(patch).is_none());
    }
}
