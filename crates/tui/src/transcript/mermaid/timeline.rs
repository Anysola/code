//! `timeline` as a vertical spine: section and period labels right-aligned on the left, a
//! fading join, `●` per period and its events to the right (merman's terminal layout).

use super::{Grid, lines, strip_comment, unquote};
use crate::text::width;

enum Entry {
    Section(String),
    Period(String, Vec<String>),
}

struct Timeline {
    title: Option<String>,
    entries: Vec<Entry>,
}

const JOIN: usize = 3;
const SPINE_OFFSET: usize = JOIN + 1;
const EVENT_OFFSET: usize = 3;

/// Splits on `:` followed by whitespace or the end, outside a leading quote.
fn fields(value: &str) -> Vec<String> {
    let chars: Vec<char> = value.chars().collect();
    let mut out = Vec::new();
    let mut quote: Option<char> = None;
    let mut start = 0;
    for (i, &c) in chars.iter().enumerate() {
        if c == '"' || c == '\'' {
            if quote == Some(c) {
                quote = None;
            } else if quote.is_none()
                && chars[start..i].iter().collect::<String>().trim().is_empty()
            {
                quote = Some(c);
            }
            continue;
        }
        let next = chars.get(i + 1);
        if c != ':' || quote.is_some() || next.is_some_and(|n| !n.is_whitespace()) {
            continue;
        }
        out.push(chars[start..i].iter().collect());
        start = i + 1;
    }
    out.push(chars[start..].iter().collect());
    out
}

fn events(fields: &[String]) -> Result<Vec<String>, String> {
    let events: Vec<String> = fields.iter().map(|f| unquote(f)).collect();
    if events.iter().any(String::is_empty) {
        return Err("Timeline event cannot be empty".into());
    }
    Ok(events)
}

fn parse(source: &str) -> Result<Timeline, String> {
    let mut timeline = Timeline {
        title: None,
        entries: Vec::new(),
    };
    let mut in_description = false;
    let mut header = false;
    for raw in source.lines() {
        let line = strip_comment(raw);
        if in_description {
            in_description = line != "}";
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("accdescr") && line.ends_with('{') {
            in_description = true;
            continue;
        }
        if line.is_empty()
            || line.starts_with('#')
            || lower.starts_with("acctitle")
            || lower.starts_with("accdescr")
        {
            continue;
        }
        if !header {
            header = lower == "timeline" || lower == "timeline lr" || lower == "timeline td";
            if !header {
                return Err("Timeline header is required".into());
            }
            continue;
        }
        if let Some(rest) = keyword(line, "title") {
            timeline.title = Some(unquote(rest.ok_or("Timeline title cannot be empty")?));
            continue;
        }
        if let Some(rest) = keyword(line, "section") {
            let label = unquote(rest.ok_or("Timeline section cannot be empty")?);
            timeline.entries.push(Entry::Section(label));
            continue;
        }
        if let Some(rest) = line.strip_prefix(':') {
            let Some(Entry::Period(_, list)) = timeline.entries.last_mut() else {
                return Err("Timeline continuation requires a preceding period".into());
            };
            list.extend(events(&fields(rest))?);
            continue;
        }
        let mut parts = fields(line);
        let period = unquote(&parts.remove(0));
        if period.is_empty() {
            return Err("Timeline period cannot be empty".into());
        }
        let list = if parts.is_empty() {
            Vec::new()
        } else {
            events(&parts)?
        };
        timeline.entries.push(Entry::Period(period, list));
    }
    Ok(timeline)
}

/// `title Foo` → `Some(Some("Foo"))`, bare `title` → `Some(None)`, other lines → `None`.
fn keyword<'a>(line: &'a str, word: &str) -> Option<Option<&'a str>> {
    let head = line.get(..word.len())?;
    if !head.eq_ignore_ascii_case(word) {
        return None;
    }
    let rest = &line[word.len()..];
    if rest.is_empty() {
        return Some(None);
    }
    rest.starts_with(char::is_whitespace)
        .then(|| Some(rest.trim()).filter(|r| !r.is_empty()))
}

pub fn render(source: &str) -> Result<Vec<String>, String> {
    let timeline = parse(source)?;
    let mut left = 0;
    let mut right = 0;
    let mut body = 0;
    for entry in &timeline.entries {
        match entry {
            Entry::Section(label) => {
                let rows = lines(label);
                body += rows.len() + 1;
                left = rows.iter().map(|l| width(l)).fold(left, usize::max);
            }
            Entry::Period(period, events) => {
                let rows = lines(period);
                let event_rows: usize = events.iter().map(|e| lines(e).len()).sum();
                body += rows.len().max(event_rows) + 1;
                left = rows.iter().map(|l| width(l)).fold(left, usize::max);
                for event in events {
                    right = lines(event)
                        .iter()
                        .map(|l| width(l))
                        .fold(right, usize::max);
                }
            }
        }
    }
    let title = timeline.title.as_deref().map(lines).unwrap_or_default();
    let empty = timeline.entries.is_empty();
    let body_width = if empty {
        0
    } else {
        left + SPINE_OFFSET + EVENT_OFFSET + right + 1
    };
    let title_width = title.iter().map(|l| width(l)).max().unwrap_or(0);
    let total = body_width.max(title_width);
    let title_height = if title.is_empty() {
        0
    } else {
        title.len() + usize::from(!empty)
    };
    if total == 0 {
        return Err("Timeline is empty".into());
    }
    let mut grid = Grid::new(total, title_height + body);
    for (i, line) in title.iter().enumerate() {
        grid.text((total - width(line)) / 2, i, line);
    }
    let spine = left + SPINE_OFFSET;
    let mut y = title_height;
    let mut started = false;
    for entry in &timeline.entries {
        match entry {
            Entry::Section(label) => {
                let rows = lines(label);
                for (i, line) in rows.iter().enumerate() {
                    grid.text(left - width(line), y + i, line);
                    if i > 0 {
                        grid.set(spine, y + i, '│');
                    }
                }
                join(&mut grid, left, y);
                grid.set(spine, y, if started { '┤' } else { '┐' });
                grid.set(spine, y + rows.len(), '│');
                started = true;
                y += rows.len() + 1;
            }
            Entry::Period(period, events) => {
                let rows = lines(period);
                let event_rows: Vec<Vec<String>> = events.iter().map(|e| lines(e)).collect();
                let height = rows.len().max(event_rows.iter().map(Vec::len).sum());
                for row in 0..=height {
                    grid.set(spine, y + row, '│');
                }
                started = true;
                grid.set(spine, y, '●');
                for (i, line) in rows.iter().enumerate() {
                    grid.text(left - width(line), y + i, line);
                }
                join(&mut grid, left, y);
                let mut event_y = y;
                for event in &event_rows {
                    for (i, line) in event.iter().enumerate() {
                        grid.text(spine + EVENT_OFFSET, event_y + i, line);
                    }
                    event_y += event.len();
                }
                y += height + 1;
            }
        }
    }
    Ok(grid.rows())
}

fn join(grid: &mut Grid, x: usize, y: usize) {
    for i in 0..JOIN {
        grid.set(x + i + 1, y, '─');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// merman's own expectation (`timeline/diagram.test.ts`).
    #[test]
    fn vertical_spine_like_merman() {
        let rows = render(
            "timeline\n  title Product &amp;<br/>Platform\n  section Foundation<br/>phase\n  2024 : Prototype<br/>ready : First release\n       : Scale &#x2265; 10k",
        )
        .unwrap();
        assert_eq!(
            rows,
            [
                "           Product &",
                "           Platform",
                "",
                "Foundation ───┐",
                "     phase    │",
                "              │",
                "      2024 ───●  Prototype",
                "              │  ready",
                "              │  First release",
                "              │  Scale ≥ 10k",
                "              │",
            ]
        );
    }

    #[test]
    fn keeps_colons_inside_events() {
        let timeline =
            parse("timeline\n  2024 : https://example.com : event:detail : next event").unwrap();
        let Entry::Period(_, events) = &timeline.entries[0] else {
            panic!()
        };
        assert_eq!(
            events,
            &["https://example.com", "event:detail", "next event"]
        );
        assert!(render("timeline\n  : orphan").is_err());
    }
}
