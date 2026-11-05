//! ```` ```mermaid ```` flowcharts as box drawings. A subset of 0.5.1's `@opencode/merman`:
//! `flowchart`/`graph` top-down charts whose edges each go one layer down, and left-to-right
//! chains. Other diagrams (sequence, state, gantt, gitGraph, timeline) and charts this layout
//! cannot draw are errors, and the block stays source.

use std::collections::HashMap;

use crate::text::width;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Down,
    Right,
}

struct Chart {
    direction: Direction,
    /// Node ids in order of first mention, with their labels.
    nodes: Vec<(String, String)>,
    edges: Vec<(usize, usize, Option<String>)>,
}

/// `A`, `A[Label]`, `A(Label)`, `A{Label}`, `A((Label))`, `A([Label])`: the id and its label.
fn node(text: &str) -> Option<(String, Option<String>)> {
    let text = text.trim();
    let end = text
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(text.len());
    let id = &text[..end];
    if id.is_empty() {
        return None;
    }
    let rest = text[end..].trim();
    if rest.is_empty() {
        return Some((id.to_string(), None));
    }
    let open: String = rest.chars().take_while(|c| "[({>".contains(*c)).collect();
    let close: String = rest
        .chars()
        .rev()
        .take_while(|c| "])}".contains(*c))
        .collect();
    if open.is_empty() || close.is_empty() || open.len() + close.len() > rest.len() {
        return None;
    }
    let label = rest[open.len()..rest.len() - close.len()]
        .trim()
        .trim_matches('"');
    Some((id.to_string(), Some(label.to_string())))
}

fn parse(source: &str) -> Result<Chart, String> {
    let mut lines = source
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("%%"));
    let header = lines.next().ok_or("empty")?;
    let mut words = header.split_whitespace();
    if !matches!(words.next(), Some("flowchart" | "graph")) {
        return Err("only flowcharts are drawn".into());
    }
    let direction = match words.next().unwrap_or("TD") {
        "TD" | "TB" => Direction::Down,
        "LR" => Direction::Right,
        other => return Err(format!("direction {other}")),
    };
    let mut chart = Chart {
        direction,
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut intern = |chart: &mut Chart, text: &str| -> Result<usize, String> {
        let (id, label) = node(text).ok_or_else(|| format!("cannot read node {text}"))?;
        let at = *index.entry(id.clone()).or_insert_with(|| {
            chart.nodes.push((id.clone(), id.clone()));
            chart.nodes.len() - 1
        });
        if let Some(label) = label {
            chart.nodes[at].1 = label;
        }
        Ok(at)
    };
    for line in lines {
        let line = line.trim_end_matches(';');
        let parts: Vec<&str> = line.split("-->").collect();
        if parts.len() == 1 {
            intern(&mut chart, line)?;
            continue;
        }
        let mut previous: Option<usize> = None;
        for part in parts {
            let (label, rest) = match part.trim().strip_prefix('|') {
                Some(after) => {
                    let (label, rest) = after.split_once('|').ok_or("unclosed edge label")?;
                    (Some(label.trim().to_string()), rest)
                }
                None => (None, part),
            };
            let at = intern(&mut chart, rest)?;
            if let Some(from) = previous {
                chart.edges.push((from, at, label));
            }
            previous = Some(at);
        }
    }
    if chart.nodes.is_empty() {
        return Err("no nodes".into());
    }
    Ok(chart)
}

fn boxed(label: &str) -> [String; 3] {
    let w = width(label);
    [
        format!("┌{}┐", "─".repeat(w + 2)),
        format!("│ {label} │"),
        format!("└{}┘", "─".repeat(w + 2)),
    ]
}

/// A chain left to right: every node has at most one edge in and one out, in order.
fn draw_right(chart: &Chart) -> Result<Vec<String>, String> {
    let n = chart.nodes.len();
    let chain = chart.edges.len() == n - 1
        && chart
            .edges
            .iter()
            .enumerate()
            .all(|(i, (a, b, _))| *a == i && *b == i + 1);
    if !chain {
        return Err("left-to-right charts are drawn only as chains".into());
    }
    let mut rows = [String::new(), String::new(), String::new()];
    for (i, (_, label)) in chart.nodes.iter().enumerate() {
        if i > 0 {
            let text = chart.edges[i - 1].2.clone().unwrap_or_default();
            let arrow = format!("─{text}─▶");
            let pad = " ".repeat(width(&arrow));
            rows[0].push_str(&pad);
            rows[1].push_str(&arrow);
            rows[2].push_str(&pad);
        }
        for (row, part) in rows.iter_mut().zip(boxed(label)) {
            row.push_str(&part);
        }
    }
    Ok(rows.to_vec())
}

fn put(canvas: &mut [Vec<char>], x: usize, y: usize, c: char) {
    if let Some(cell) = canvas.get_mut(y).and_then(|r| r.get_mut(x)) {
        *cell = match (*cell, c) {
            ('─', '│') | ('│', '─') => '┼',
            ('┴', '│') | ('│', '┴') => '┼',
            ('┬', '│') | ('│', '┬') => '┼',
            (_, c) => c,
        };
    }
}

/// Top-down by layers; each edge must go exactly one layer down.
fn draw_down(chart: &Chart) -> Result<Vec<String>, String> {
    let n = chart.nodes.len();
    let mut layer = vec![0usize; n];
    for _ in 0..n {
        for (a, b, _) in &chart.edges {
            layer[*b] = layer[*b].max(layer[*a] + 1);
        }
    }
    if chart
        .edges
        .iter()
        .any(|(a, b, _)| layer[*b] != layer[*a] + 1)
    {
        return Err("edges that skip layers or loop back are not drawn".into());
    }
    let layers = layer.iter().max().copied().unwrap_or(0) + 1;
    let gap = 2;
    let mut x = vec![0usize; n];
    let mut widths = vec![0usize; n];
    let mut total = 0;
    let mut rows_of: Vec<Vec<usize>> = vec![Vec::new(); layers];
    for i in 0..n {
        rows_of[layer[i]].push(i);
        widths[i] = width(&chart.nodes[i].1) + 4;
    }
    for row in &rows_of {
        let w: usize =
            row.iter().map(|i| widths[*i]).sum::<usize>() + gap * row.len().saturating_sub(1);
        total = total.max(w);
    }
    for row in &rows_of {
        let w: usize =
            row.iter().map(|i| widths[*i]).sum::<usize>() + gap * row.len().saturating_sub(1);
        let mut at = (total - w) / 2;
        for i in row {
            x[*i] = at;
            at += widths[*i] + gap;
        }
    }
    let band = 3;
    let height = layers * 3 + (layers - 1) * band;
    let mut canvas = vec![vec![' '; total]; height];
    for i in 0..n {
        let y = layer[i] * (3 + band);
        for (dy, row) in boxed(&chart.nodes[i].1).iter().enumerate() {
            for (dx, c) in row.chars().enumerate() {
                canvas[y + dy][x[i] + dx] = c;
            }
        }
    }
    for (a, b, label) in &chart.edges {
        let from = x[*a] + widths[*a] / 2;
        let to = x[*b] + widths[*b] / 2;
        let top = layer[*a] * (3 + band) + 3;
        put(&mut canvas, from, top - 1, '┬');
        put(&mut canvas, from, top, '│');
        let (left, right) = (from.min(to), from.max(to));
        for cx in left..=right {
            if cx != from && cx != to {
                put(&mut canvas, cx, top + 1, '─');
            }
        }
        if from == to {
            put(&mut canvas, from, top + 1, '│');
        } else {
            put(
                &mut canvas,
                from,
                top + 1,
                if to > from { '└' } else { '┘' },
            );
            put(&mut canvas, to, top + 1, if to > from { '┐' } else { '┌' });
        }
        put(&mut canvas, to, top + 2, '▼');
        if let Some(label) = label {
            for (i, c) in label.chars().enumerate() {
                let cx = from + 2 + i;
                if canvas[top].get(cx) == Some(&' ') {
                    canvas[top][cx] = c;
                }
            }
        }
    }
    Ok(canvas
        .into_iter()
        .map(|r| r.into_iter().collect::<String>().trim_end().to_string())
        .collect())
}

/// The drawing's rows, or why the chart cannot be drawn (the block then stays source).
pub fn render(source: &str) -> Result<Vec<String>, String> {
    let chart = parse(source)?;
    match chart.direction {
        Direction::Down => draw_down(&chart),
        Direction::Right => draw_right(&chart),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_down_chart() {
        let rows = render("flowchart TD\n  A[Start] --> B{Check}\n  B -->|yes| C[Done]").unwrap();
        assert_eq!(
            rows,
            vec![
                "┌───────┐",
                "│ Start │",
                "└───┬───┘",
                "    │",
                "    │",
                "    ▼",
                "┌───────┐",
                "│ Check │",
                "└───┬───┘",
                "    │ yes",
                "    │",
                "    ▼",
                "┌──────┐",
                "│ Done │",
                "└──────┘",
            ]
        );
    }

    #[test]
    fn chains_left_to_right() {
        assert_eq!(
            render("graph LR\nA[a] --> B[b]").unwrap(),
            vec!["┌───┐   ┌───┐", "│ a │──▶│ b │", "└───┘   └───┘"]
        );
    }

    #[test]
    fn other_diagrams_fall_back() {
        assert!(render("sequenceDiagram\nA->>B: hi").is_err());
        assert!(render("flowchart TD\nA --> B\nB --> A").is_err());
    }
}
