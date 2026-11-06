//! Flowcharts as box drawings, a subset of 0.5.1's `@opencode/merman`: `flowchart`/`graph`
//! top-down charts whose edges go down (an edge that skips layers passes through them as a
//! line), and left-to-right chains. Charts this layout cannot draw (edges that loop back) are
//! errors, and the block stays source. State diagrams (`state.rs`) are drawn with this layout.

use std::collections::HashMap;

use crate::text::width;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Direction {
    Down,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Shape {
    /// The label in a box.
    Box,
    /// The label alone, as a small glyph (`●`, `◉`, `◇`) edges reach without a box.
    Point,
    /// A line through a layer, for an edge that skips it (made while drawing).
    Pass,
}

#[derive(Debug, Clone)]
pub(super) struct Node {
    pub label: String,
    pub shape: Shape,
}

pub(super) struct Chart {
    pub direction: Direction,
    /// Nodes in order of first mention.
    pub nodes: Vec<Node>,
    pub edges: Vec<(usize, usize, Option<String>)>,
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
            chart.nodes.push(Node {
                label: id.clone(),
                shape: Shape::Box,
            });
            chart.nodes.len() - 1
        });
        if let Some(label) = label {
            chart.nodes[at].label = label;
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
    for (i, node) in chart.nodes.iter().enumerate() {
        if i > 0 {
            let text = chart.edges[i - 1].2.clone().unwrap_or_default();
            let arrow = format!("─{text}─▶");
            let pad = " ".repeat(width(&arrow));
            rows[0].push_str(&pad);
            rows[1].push_str(&arrow);
            rows[2].push_str(&pad);
        }
        let parts = match node.shape {
            Shape::Box => boxed(&node.label),
            _ => {
                let pad = " ".repeat(width(&node.label));
                [pad.clone(), node.label.clone(), pad]
            }
        };
        for (row, part) in rows.iter_mut().zip(parts) {
            row.push_str(&part);
        }
    }
    Ok(rows.iter().map(|r| r.trim_end().to_string()).collect())
}

/// A line cell's arms: up, down, left, right.
const LINES: [(char, u8); 11] = [
    ('│', 0b1100),
    ('─', 0b0011),
    ('┌', 0b0101),
    ('┐', 0b0110),
    ('└', 0b1001),
    ('┘', 0b1010),
    ('├', 0b1101),
    ('┤', 0b1110),
    ('┬', 0b0111),
    ('┴', 0b1011),
    ('┼', 0b1111),
];

fn arms(c: char) -> Option<u8> {
    LINES.iter().find(|(l, _)| *l == c).map(|(_, a)| *a)
}

/// Writes a line cell, joining it with a line already there (`┘` over `└` is `┴`).
fn put(canvas: &mut [Vec<char>], x: usize, y: usize, c: char) {
    if let Some(cell) = canvas.get_mut(y).and_then(|r| r.get_mut(x)) {
        *cell = match (arms(*cell), arms(c)) {
            (Some(old), Some(new)) => LINES
                .iter()
                .find(|(_, a)| *a == old | new)
                .map_or(c, |(l, _)| *l),
            _ => c,
        };
    }
}

/// Top-down by layers; every edge must go down. An edge that skips layers passes through each
/// of them as a one-cell `Pass` node.
fn draw_down(chart: &Chart) -> Result<Vec<String>, String> {
    let n = chart.nodes.len();
    let mut layer = vec![0usize; n];
    for _ in 0..n {
        for (a, b, _) in &chart.edges {
            layer[*b] = layer[*b].max(layer[*a] + 1);
        }
    }
    if chart.edges.iter().any(|(a, b, _)| layer[*b] <= layer[*a]) {
        return Err("edges that loop back are not drawn".into());
    }
    let mut nodes = chart.nodes.clone();
    let mut edges: Vec<(usize, usize, Option<String>)> = Vec::new();
    for (a, b, label) in &chart.edges {
        let (mut from, mut label) = (*a, label.clone());
        for l in layer[*a] + 1..layer[*b] {
            nodes.push(Node {
                label: String::new(),
                shape: Shape::Pass,
            });
            layer.push(l);
            edges.push((from, nodes.len() - 1, label.take()));
            from = nodes.len() - 1;
        }
        edges.push((from, *b, label));
    }
    let n = nodes.len();
    let has_in: Vec<bool> = (0..n).map(|i| edges.iter().any(|e| e.1 == i)).collect();
    let has_out: Vec<bool> = (0..n).map(|i| edges.iter().any(|e| e.0 == i)).collect();
    let layers = layer.iter().max().copied().unwrap_or(0) + 1;
    let gap = 2;
    let mut x = vec![0usize; n];
    let mut widths = vec![0usize; n];
    let mut total = 0;
    let mut rows_of: Vec<Vec<usize>> = vec![Vec::new(); layers];
    for i in 0..n {
        rows_of[layer[i]].push(i);
        widths[i] = match nodes[i].shape {
            Shape::Box => width(&nodes[i].label) + 4,
            Shape::Point => width(&nodes[i].label).max(1),
            Shape::Pass => 1,
        };
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
        let center = x[i] + widths[i] / 2;
        match nodes[i].shape {
            Shape::Box => {
                for (dy, row) in boxed(&nodes[i].label).iter().enumerate() {
                    for (dx, c) in row.chars().enumerate() {
                        canvas[y + dy][x[i] + dx] = c;
                    }
                }
            }
            Shape::Point => {
                // Reached from above: the glyph takes the top row and a line runs down from it;
                // a source only: the glyph sits on the bottom row, next to its edges.
                let at = if has_in[i] { y } else { y + 2 };
                for (dx, c) in nodes[i].label.chars().enumerate() {
                    canvas[at][x[i] + dx] = c;
                }
                if has_out[i] {
                    for row in canvas.iter_mut().take(y + 3).skip(at + 1) {
                        row[center] = '│';
                    }
                }
            }
            Shape::Pass => {
                for row in canvas.iter_mut().skip(y).take(3) {
                    row[center] = '│';
                }
            }
        }
    }
    for (a, b, label) in &edges {
        let from = x[*a] + widths[*a] / 2;
        let to = x[*b] + widths[*b] / 2;
        let top = layer[*a] * (3 + band) + 3;
        if nodes[*a].shape == Shape::Box {
            put(&mut canvas, from, top - 1, '┬');
        }
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
        let head = if nodes[*b].shape == Shape::Pass {
            '│'
        } else {
            '▼'
        };
        put(&mut canvas, to, top + 2, head);
        if let Some(label) = label {
            // Beside a straight line, or beside the head of a bent one (fanned-out edges share
            // their first rows). The label may run past the boxes' width.
            let (y, at) = if from == to {
                (top, from + 2)
            } else {
                (top + 2, to + 2)
            };
            let row = &mut canvas[y];
            if row.len() < at + label.chars().count() {
                row.resize(at + label.chars().count(), ' ');
            }
            for (i, c) in label.chars().enumerate() {
                if row[at + i] == ' ' {
                    row[at + i] = c;
                }
            }
        }
    }
    let mut rows: Vec<String> = canvas
        .into_iter()
        .map(|r| r.into_iter().collect::<String>().trim_end().to_string())
        .collect();
    // A source point on the first layer or an end point on the last leaves blank rows.
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    let blank = rows.iter().take_while(|r| r.is_empty()).count();
    Ok(rows.split_off(blank))
}

/// The chart's rows in its direction, or why it cannot be drawn.
pub(super) fn draw(chart: &Chart) -> Result<Vec<String>, String> {
    match chart.direction {
        Direction::Down => draw_down(chart),
        Direction::Right => draw_right(chart),
    }
}

/// The drawing's rows, or why the chart cannot be drawn (the block then stays source).
pub fn render(source: &str) -> Result<Vec<String>, String> {
    draw(&parse(source)?)
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
    fn edges_that_skip_layers_pass_through() {
        assert_eq!(
            render("flowchart TD\nA --> B\nB --> C\nA --> C").unwrap(),
            vec![
                " ┌───┐",
                " │ A │",
                " └─┬─┘",
                "   │",
                "  ┌┴───┐",
                "  ▼    │",
                "┌───┐  │",
                "│ B │  │",
                "└─┬─┘  │",
                "  │    │",
                "  └┬───┘",
                "   ▼",
                " ┌───┐",
                " │ C │",
                " └───┘",
            ]
        );
    }

    #[test]
    fn cycles_fall_back() {
        assert!(render("flowchart TD\nA --> B\nB --> A").is_err());
    }
}
