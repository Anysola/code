//! `sequenceDiagram` after merman's terminal layout (`sequence/parser.ts`, `placement.ts`,
//! `drawing.ts`): participants and lifelines, messages with their arrow heads, self-messages as
//! loops, notes, `alt`/`else`/`loop` frames, `box` groups, activations and autonumbering. The
//! transcript uses the compact form (no header rule, labels inline when they fit).

use std::collections::HashMap;

use super::{Grid, strip_comment, unquote};
use crate::text::width;

const MIN_GAP: i32 = 18;
const NOTE_PADDING: usize = 2;
const GROUP_PADDING: i32 = 2;
const OVERHANG: i32 = 3;
const FADE: i32 = 5;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Head {
    Filled,
    Open,
    Cross,
    Async,
}

#[derive(Debug, Clone)]
struct Message {
    from: String,
    to: String,
    label: String,
    head: Head,
    number: Option<i64>,
    activate: Option<String>,
    deactivate: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Side {
    Over,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Fragment {
    Alt,
    Else,
    Loop,
    End,
}

#[derive(Debug, Clone)]
enum Step {
    Message(Message),
    Note {
        over: Vec<String>,
        label: String,
        side: Side,
    },
    Activation {
        participant: String,
        active: bool,
    },
    Fragment(Fragment, String),
}

#[derive(Debug, Default)]
struct Diagram {
    /// `(id, label)` in order of appearance.
    participants: Vec<(String, String)>,
    steps: Vec<Step>,
    /// `(label, participant ids)` of `box` groups.
    groups: Vec<(String, Vec<String>)>,
}

impl Diagram {
    fn ensure(&mut self, id: &str, label: Option<String>) {
        match self.participants.iter_mut().find(|(p, _)| p == id) {
            Some(existing) => {
                if let Some(label) = label {
                    existing.1 = label;
                }
            }
            None => self
                .participants
                .push((id.to_string(), label.unwrap_or_else(|| id.to_string()))),
        }
    }
}

const OPERATORS: [&str; 8] = ["-->>", "->>", "--x", "-x", "--)", "-)", "-->", "->"];

/// `from <op>[+-] to: label`, the operator being the first one found after at least one
/// character, as merman's lazy pattern reads it.
fn message(line: &str) -> Option<(String, &'static str, Option<char>, String, String)> {
    let (at, op) = line.char_indices().skip(1).find_map(|(i, _)| {
        OPERATORS
            .iter()
            .find(|op| line[i..].starts_with(**op))
            .map(|op| (i, *op))
    })?;
    let from = line[..at].trim();
    let mut rest = &line[at + op.len()..];
    let marker = rest.chars().next().filter(|c| *c == '+' || *c == '-');
    if let Some(m) = marker {
        rest = &rest[m.len_utf8()..];
    }
    let rest = rest.trim_start();
    let colon = rest.char_indices().skip(1).find(|(_, c)| *c == ':')?.0;
    let to = rest[..colon].trim();
    if from.is_empty() || to.is_empty() {
        return None;
    }
    let label = rest[colon + 1..].trim();
    Some((from.into(), op, marker, to.into(), label.into()))
}

fn parse(source: &str) -> Result<Diagram, String> {
    let mut diagram = Diagram::default();
    let mut blocks: Vec<Fragment> = Vec::new();
    // `None` marks a `box`; groups pair with them by depth.
    let mut open: Vec<Option<Fragment>> = Vec::new();
    let mut group_stack: Vec<usize> = Vec::new();
    let mut number: Option<i64> = None;
    let mut increment = 1;
    let mut header = false;
    for raw in source.lines() {
        let line = strip_comment(raw);
        if line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if !header {
            if lower != "sequencediagram" {
                return Err("sequenceDiagram header is required".into());
            }
            header = true;
            continue;
        }
        if lower == "sequencediagram" {
            continue;
        }
        if line.contains("<<->>") || line.contains("<<-->>") {
            return Err(format!("unsupported sequence line: {line}"));
        }
        let (word, rest) = line
            .split_once(char::is_whitespace)
            .map_or((line, ""), |(w, r)| (w, r.trim()));
        let word = word.to_ascii_lowercase();
        let group = group_stack.last().copied();
        let add_to_group = |diagram: &mut Diagram, id: &str| {
            if let Some(g) = group
                && !diagram.groups[g].1.iter().any(|p| p == id)
            {
                diagram.groups[g].1.push(id.to_string());
            }
        };
        match word.as_str() {
            "autonumber" => {
                let numbers: Vec<i64> = rest
                    .split_whitespace()
                    .filter_map(|n| n.parse().ok())
                    .collect();
                number = Some(numbers.first().copied().unwrap_or(1));
                increment = numbers.get(1).copied().unwrap_or(1);
            }
            "box" => {
                diagram.groups.push((box_label(rest), Vec::new()));
                group_stack.push(diagram.groups.len() - 1);
                open.push(None);
            }
            "participant" | "actor" => {
                let (id, label) = match rest.split_once(" as ") {
                    Some((id, label)) => (unquote(id), Some(unquote(label))),
                    None => (unquote(rest), None),
                };
                if id.is_empty() || id.contains(char::is_whitespace) {
                    return Err(format!("unsupported sequence line: {line}"));
                }
                diagram.ensure(&id, Some(label.unwrap_or_else(|| id.clone())));
                add_to_group(&mut diagram, &id);
            }
            "note" => {
                let lower_rest = rest.to_ascii_lowercase();
                let (side, after) = if let Some(r) = lower_rest.strip_prefix("over") {
                    (Side::Over, &rest[rest.len() - r.len()..])
                } else if lower_rest.starts_with("left of") {
                    (Side::Left, &rest[7..])
                } else if lower_rest.starts_with("right of") {
                    (Side::Right, &rest[8..])
                } else {
                    return Err(format!("unsupported sequence line: {line}"));
                };
                let (who, label) = after
                    .split_once(':')
                    .ok_or_else(|| format!("unsupported sequence line: {line}"))?;
                let over: Vec<String> = who
                    .split(',')
                    .map(unquote)
                    .filter(|p| !p.is_empty())
                    .collect();
                for participant in &over {
                    diagram.ensure(participant, None);
                }
                diagram.steps.push(Step::Note {
                    over,
                    label: unquote(label),
                    side,
                });
            }
            "activate" | "deactivate" if !rest.is_empty() => {
                let participant = unquote(rest);
                diagram.ensure(&participant, None);
                diagram.steps.push(Step::Activation {
                    participant,
                    active: word == "activate",
                });
            }
            "alt" | "loop" if !rest.is_empty() => {
                let kind = if word == "alt" {
                    Fragment::Alt
                } else {
                    Fragment::Loop
                };
                blocks.push(kind);
                open.push(Some(kind));
                diagram.steps.push(Step::Fragment(kind, unquote(rest)));
            }
            "else" => {
                if open.last() != Some(&Some(Fragment::Alt)) {
                    return Err("Unexpected \"else\" without an open \"alt\" block".into());
                }
                diagram
                    .steps
                    .push(Step::Fragment(Fragment::Else, unquote(rest)));
            }
            "end" if rest.is_empty() => match open.pop() {
                None => return Err("Unexpected \"end\" without an open block".into()),
                Some(None) => {
                    group_stack.pop();
                }
                Some(Some(_)) => {
                    blocks.pop();
                    diagram
                        .steps
                        .push(Step::Fragment(Fragment::End, String::new()));
                }
            },
            _ => {
                let Some((from, op, marker, to, label)) = message(line) else {
                    return Err(format!("unsupported sequence line: {line}"));
                };
                if OPERATORS.iter().any(|o| from.contains(o) || to.contains(o)) {
                    return Err(format!("unsupported sequence line: {line}"));
                }
                let (from, to) = (unquote(&from), unquote(&to));
                diagram.ensure(&from, None);
                diagram.ensure(&to, None);
                add_to_group(&mut diagram, &from);
                add_to_group(&mut diagram, &to);
                let head = if op.ends_with('x') {
                    Head::Cross
                } else if op.ends_with(')') {
                    Head::Async
                } else if op.ends_with(">>") {
                    Head::Filled
                } else {
                    Head::Open
                };
                let current = number;
                if let Some(n) = number.as_mut() {
                    *n += increment;
                }
                diagram.steps.push(Step::Message(Message {
                    activate: (marker == Some('+')).then(|| to.clone()),
                    deactivate: (marker == Some('-')).then(|| from.clone()),
                    from,
                    to,
                    label: unquote(&label),
                    head,
                    number: current,
                }));
            }
        }
    }
    if let Some(kind) = open.last() {
        let name = match kind {
            None => "box",
            Some(Fragment::Loop) => "loop",
            _ => "alt",
        };
        return Err(format!("Unclosed {name} block; expected \"end\""));
    }
    if diagram.participants.is_empty() {
        return Err("sequence diagram is empty".into());
    }
    Ok(diagram)
}

/// A `box` label without a leading colour token.
fn box_label(value: &str) -> String {
    let raw = value.trim();
    if raw.len() >= 2 && (raw.starts_with('"') || raw.starts_with('\'')) {
        return unquote(raw);
    }
    let label = unquote(raw);
    let (token, rest) = if label.to_ascii_lowercase().starts_with("rgb") && label.contains(')') {
        let close = label.find(')').unwrap_or(0);
        (&label[..=close], label[close + 1..].trim())
    } else {
        label
            .split_once(char::is_whitespace)
            .map_or((label.as_str(), ""), |(t, r)| (t, r.trim()))
    };
    let colour = token.eq_ignore_ascii_case("transparent")
        || token.starts_with('#')
        || token.to_ascii_lowercase().starts_with("rgb")
        || COLOURS.contains(&token.to_ascii_lowercase().as_str());
    if colour { unquote(rest) } else { label }
}

const COLOURS: [&str; 28] = [
    "black",
    "white",
    "red",
    "green",
    "blue",
    "yellow",
    "cyan",
    "magenta",
    "silver",
    "gray",
    "grey",
    "maroon",
    "olive",
    "lime",
    "aqua",
    "teal",
    "navy",
    "fuchsia",
    "purple",
    "orange",
    "brightblack",
    "brightred",
    "brightgreen",
    "brightblue",
    "brightyellow",
    "brightcyan",
    "brightmagenta",
    "brightwhite",
];

/// Label lines: `<br>` spellings and a literal `\n` break them.
fn label_lines(label: &str) -> Vec<String> {
    let mut text = label.replace("\\n", "\n");
    for br in [
        "<br />", "<br/>", "<br>", "<BR />", "<BR/>", "<BR>", "<br >", "<br / >",
    ] {
        text = text.replace(br, "\n");
    }
    text.split('\n').map(|l| l.trim_end().to_string()).collect()
}

fn lines_width(lines: &[String]) -> i32 {
    lines.iter().map(|l| width(l) as i32).max().unwrap_or(0)
}

fn note_lines(label: &str) -> Vec<String> {
    let lines = label_lines(label);
    let w = lines_width(&lines) as usize;
    let pad = " ".repeat(NOTE_PADDING);
    lines
        .iter()
        .map(|l| format!("{pad}{l}{}{pad}", " ".repeat(w - width(l))))
        .collect()
}

fn message_text(m: &Message) -> String {
    match m.number {
        Some(n) => format!("{n}. {}", m.label),
        None => m.label.clone(),
    }
}

fn header_width(label: &str, compact: bool) -> i32 {
    let w = lines_width(&label_lines(label));
    if compact { w } else { w.max(3) }
}

fn self_loop_width(lines: &[String]) -> i32 {
    10.max(lines_width(lines) + 4)
}

fn fragment_label(kind: Fragment, label: &str) -> String {
    match kind {
        Fragment::End => String::new(),
        Fragment::Loop => format!(" ↻ loop: {label} "),
        Fragment::Alt => format!(" alt: {label} "),
        Fragment::Else => format!(" else: {label} "),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Bounds {
    left: i32,
    right: i32,
}

impl Bounds {
    fn expand(&mut self, left: i32, right: i32) {
        self.left = self.left.min(left);
        self.right = self.right.max(right);
    }
}

struct Layout<'a> {
    diagram: &'a Diagram,
    index: HashMap<&'a str, usize>,
    compact: bool,
    centers: Vec<i32>,
}

impl Layout<'_> {
    fn at(&self, id: &str) -> Option<usize> {
        self.index.get(id).copied()
    }

    fn inline(&self, m: &Message, lines: &[String], from: i32, to: i32) -> Option<String> {
        if !self.compact || m.from == m.to || lines.len() != 1 {
            return None;
        }
        let label = format!(" {} ", lines[0]);
        (width(&label) as i32 <= (to - from).abs() - 3).then_some(label)
    }

    fn step_height(&self, step: &Step) -> i32 {
        match step {
            Step::Note { label, .. } => note_lines(label).len() as i32 + 2,
            Step::Activation { .. } => 0,
            Step::Fragment(..) => 2,
            Step::Message(m) => {
                let lines = label_lines(&message_text(m));
                if let (Some(f), Some(t)) = (self.at(&m.from), self.at(&m.to))
                    && self
                        .inline(m, &lines, self.centers[f], self.centers[t])
                        .is_some()
                {
                    return 2;
                }
                lines.len() as i32 + if m.from == m.to { 3 } else { 2 }
            }
        }
    }

    fn note_x(&self, over: &[String], label: &str, side: Side) -> Option<(i32, Vec<String>)> {
        let indexes: Vec<usize> = over.iter().filter_map(|p| self.at(p)).collect();
        let (lo, hi) = (*indexes.iter().min()?, *indexes.iter().max()?);
        let center = (self.centers[lo] + self.centers[hi]).div_euclid(2);
        let lines = note_lines(label);
        let w = lines_width(&lines);
        let x = match side {
            Side::Left => center - w - 2,
            Side::Right => center + 2,
            Side::Over => center - w / 2,
        };
        Some((x, lines))
    }

    fn content(&self, step: &Step) -> Option<Bounds> {
        match step {
            Step::Message(m) => {
                let (f, t) = (self.at(&m.from)?, self.at(&m.to)?);
                let (fx, tx) = (self.centers[f], self.centers[t]);
                let lines = label_lines(&message_text(m));
                if f == t {
                    return Some(Bounds {
                        left: fx,
                        right: fx + self_loop_width(&lines),
                    });
                }
                let (left, right) = (fx.min(tx), fx.max(tx));
                let label_left = left + 2;
                Some(Bounds {
                    left: left.min(label_left),
                    right: right.max(label_left + lines_width(&lines) - 1),
                })
            }
            Step::Note { over, label, side } => {
                let (x, lines) = self.note_x(over, label, *side)?;
                Some(Bounds {
                    left: x,
                    right: x + lines_width(&lines) - 1,
                })
            }
            _ => None,
        }
    }

    fn step_participants(&self, step: &Step) -> Vec<usize> {
        match step {
            Step::Message(m) => [&m.from, &m.to].iter().filter_map(|p| self.at(p)).collect(),
            Step::Note { over, .. } => over.iter().filter_map(|p| self.at(p)).collect(),
            _ => Vec::new(),
        }
    }

    fn header(&self, i: usize) -> (i32, i32) {
        let w = header_width(&self.diagram.participants[i].1, self.compact);
        let left = self.centers[i] - w / 2;
        (left, left + w - 1)
    }

    /// Group ranges `(label, first, last)` over participant indexes.
    fn ranges(&self) -> Vec<(String, usize, usize)> {
        self.diagram
            .groups
            .iter()
            .filter_map(|(label, ids)| {
                let indexes: Vec<usize> = ids.iter().filter_map(|p| self.at(p)).collect();
                Some((
                    label.clone(),
                    *indexes.iter().min()?,
                    *indexes.iter().max()?,
                ))
            })
            .collect()
    }

    fn group_bounds(&self) -> Vec<(String, Bounds)> {
        self.ranges()
            .into_iter()
            .map(|(label, first, last)| {
                let mut b = Bounds {
                    left: self.centers[first],
                    right: self.centers[last],
                };
                for i in first..=last {
                    let (l, r) = self.header(i);
                    b.expand(l, r);
                }
                for step in &self.diagram.steps {
                    let indexes = self.step_participants(step);
                    let inside =
                        !indexes.is_empty() && indexes.iter().all(|i| (first..=last).contains(i));
                    if let (true, Some(c)) = (inside, self.content(step)) {
                        b.expand(c.left, c.right);
                    }
                }
                let text = if label.is_empty() {
                    String::new()
                } else {
                    format!(" {label} ")
                };
                let (mut left, mut right) = (b.left - GROUP_PADDING, b.right + GROUP_PADDING);
                let extra = (width(&text) as i32 + 4 - (right - left + 1)).max(0);
                left -= extra / 2;
                right += extra - extra / 2;
                (text, Bounds { left, right })
            })
            .collect()
    }

    fn content_bounds(&self) -> Bounds {
        let mut b = Bounds { left: 0, right: 0 };
        for i in 0..self.centers.len() {
            let (l, r) = self.header(i);
            b.expand(l, r);
        }
        for step in &self.diagram.steps {
            if let Some(c) = self.content(step) {
                b.expand(c.left, c.right);
            }
        }
        b
    }

    fn frame(&self, kind: Fragment, label: &str, depth: i32) -> Bounds {
        let left = self.centers[0] - OVERHANG + depth;
        let right = (self.centers[self.centers.len() - 1] + OVERHANG - depth)
            .max(left + 2 + width(&fragment_label(kind, label)) as i32 + 1);
        Bounds { left, right }
    }

    /// Each opening fragment's frame, widened for what it contains.
    fn frames(&self) -> HashMap<usize, Bounds> {
        let mut done = HashMap::new();
        let mut active: Vec<(usize, Bounds)> = Vec::new();
        for (index, step) in self.diagram.steps.iter().enumerate() {
            let Step::Fragment(kind, label) = step else {
                if let Some(c) = self.content(step) {
                    for (_, frame) in &mut active {
                        frame.expand(c.left - 1, c.right + 1);
                    }
                }
                continue;
            };
            let bounds = self.frame(*kind, label, active.len() as i32);
            if matches!(kind, Fragment::Alt | Fragment::Loop) {
                active.push((index, bounds));
                continue;
            }
            let Some(frame) = active.last_mut() else {
                continue;
            };
            frame.1.expand(bounds.left, bounds.right);
            if *kind != Fragment::End {
                continue;
            }
            let (start, bounds) = active.pop().expect("checked above");
            done.insert(start, bounds);
            if let Some(parent) = active.last_mut() {
                parent.1.expand(bounds.left - 1, bounds.right + 1);
            }
        }
        for (start, bounds) in active {
            done.insert(start, bounds);
        }
        done
    }

    fn resolve_centers(&mut self) {
        let d = self.diagram;
        let n = d.participants.len();
        let hw = |i: usize| header_width(&d.participants[i].1, self.compact);
        let mut gaps: Vec<i32> = (0..n.saturating_sub(1))
            .map(|i| MIN_GAP.max((hw(i) + 1) / 2 + (hw(i + 1) + 1) / 2 + 6))
            .collect();
        for step in &d.steps {
            match step {
                Step::Message(m) => {
                    let (Some(f), Some(t)) = (self.at(&m.from), self.at(&m.to)) else {
                        continue;
                    };
                    let lines = label_lines(&message_text(m));
                    if f == t {
                        if f + 1 < n {
                            let next = lines_width(&label_lines(&d.participants[f + 1].1));
                            gaps[f] = gaps[f].max(self_loop_width(&lines) + (next + 1) / 2 + 2);
                        }
                    } else if f.abs_diff(t) == 1 {
                        let g = f.min(t);
                        gaps[g] = gaps[g].max(lines_width(&lines) + 6);
                    }
                }
                Step::Note { over, label, side } => {
                    let indexes: Vec<usize> = over.iter().filter_map(|p| self.at(p)).collect();
                    let need = lines_width(&note_lines(label)) + 4;
                    if indexes.len() == 1 && *side != Side::Over {
                        let g = if *side == Side::Left {
                            indexes[0].checked_sub(1)
                        } else {
                            Some(indexes[0])
                        };
                        if let Some(g) = g.filter(|g| *g < gaps.len()) {
                            gaps[g] = gaps[g].max(need);
                        }
                    } else if indexes.len() == 2 && indexes[0].abs_diff(indexes[1]) == 1 {
                        let g = indexes[0].min(indexes[1]);
                        gaps[g] = gaps[g].max(need);
                    }
                }
                _ => {}
            }
        }
        let mut centers = vec![1.max(hw(0) / 2)];
        for gap in gaps {
            centers.push(centers[centers.len() - 1] + gap);
        }
        self.centers = centers;
    }

    /// Moves participants right of a group's edge so the group frame clears them.
    fn separate_groups(&mut self) {
        let ranges = self.ranges();
        for boundary in 0..self.centers.len().saturating_sub(1) {
            let groups = self.group_bounds();
            let mut left_right = self.header(boundary).1;
            let mut right_left = self.header(boundary + 1).0;
            let mut borders = false;
            for (i, (_, first, last)) in ranges.iter().enumerate() {
                if *last == boundary {
                    left_right = left_right.max(groups[i].1.right);
                    borders = true;
                }
                if *first == boundary + 1 {
                    right_left = right_left.min(groups[i].1.left);
                    borders = true;
                }
            }
            let shift = left_right + GROUP_PADDING + 1 - right_left;
            if borders && shift > 0 {
                for center in &mut self.centers[boundary + 1..] {
                    *center += shift;
                }
            }
        }
    }
}

const CORNERS: [char; 4] = ['╭', '╮', '╰', '╯'];

/// A grid that accepts any coordinate and drops what falls outside.
struct Canvas {
    grid: Grid,
    width: i32,
    height: i32,
}

impl Canvas {
    fn set(&mut self, x: i32, y: i32, c: char) {
        if (0..self.width).contains(&x) && (0..self.height).contains(&y) {
            self.grid.set(x as usize, y as usize, c);
        }
    }

    fn get(&self, x: i32, y: i32) -> char {
        self.grid
            .0
            .get(y.max(0) as usize)
            .and_then(|row| row.get(x.max(0) as usize))
            .copied()
            .unwrap_or(' ')
    }

    fn text(&mut self, x: i32, y: i32, text: &str) {
        if (0..self.height).contains(&y) {
            self.grid.text(x.max(0) as usize, y as usize, text);
        }
    }
}

fn head_char(head: Head, rightward: bool) -> char {
    match (head, rightward) {
        (Head::Open, true) => '>',
        (Head::Open, false) => '<',
        (Head::Cross, _) => '✕',
        (Head::Async, true) => ')',
        (Head::Async, false) => '(',
        (Head::Filled, true) => '►',
        (Head::Filled, false) => '◄',
    }
}

fn draw(diagram: &Diagram, compact: bool) -> Vec<String> {
    let mut layout = Layout {
        diagram,
        index: diagram
            .participants
            .iter()
            .enumerate()
            .map(|(i, (id, _))| (id.as_str(), i))
            .collect(),
        compact,
        centers: Vec::new(),
    };
    layout.resolve_centers();
    layout.separate_groups();
    let fragments = |l: &Layout| {
        let mut b = Bounds { left: 0, right: 0 };
        for f in l.frames().values() {
            b.expand(f.left, f.right);
        }
        b
    };
    let groups = layout.group_bounds();
    let overflow = groups
        .iter()
        .map(|(_, b)| b.left)
        .chain([layout.content_bounds().left, fragments(&layout).left, 0])
        .min()
        .unwrap_or(0);
    if overflow < 0 {
        for center in &mut layout.centers {
            *center -= overflow;
        }
    }
    let groups = layout.group_bounds();
    let content = layout.content_bounds();
    let frames = layout.frames();
    let fragment_bounds = fragments(&layout);
    let has_groups = !groups.is_empty();
    let label_height = diagram
        .participants
        .iter()
        .map(|(_, l)| label_lines(l).len())
        .max()
        .unwrap_or(1) as i32;
    let header_y = i32::from(has_groups);
    let rule_y = header_y
        + if compact {
            label_height - 1
        } else {
            label_height
        };
    let lifeline_start = rule_y + 1;
    let step_start = lifeline_start + 1;
    let width = groups
        .iter()
        .map(|(_, b)| b.right + 1)
        .chain([content.right + 1, fragment_bounds.right + 1])
        .max()
        .unwrap_or(1);
    let base: i32 = step_start
        + diagram
            .steps
            .iter()
            .map(|s| layout.step_height(s))
            .sum::<i32>();
    let height = if has_groups {
        5.max(base + 1)
    } else {
        3.max(base)
    };
    let lifeline_end = if has_groups { height - 2 } else { height - 1 };
    let mut c = Canvas {
        grid: Grid::new(width as usize, height as usize),
        width,
        height,
    };
    for (label, b) in &groups {
        for x in b.left..=b.right {
            c.set(x, 0, '─');
            c.set(x, height - 1, '─');
        }
        c.set(b.left, 0, '╭');
        c.set(b.right, 0, '╮');
        c.set(b.left, height - 1, '╰');
        c.set(b.right, height - 1, '╯');
        for y in 1..height - 1 {
            for x in [b.left, b.right] {
                if matches!(c.get(x, y), ' ' | '│') {
                    c.set(x, y, '│');
                }
            }
        }
        if !label.is_empty() {
            c.text(b.left + 2, 0, label);
        }
    }
    for (i, (_, label)) in diagram.participants.iter().enumerate() {
        let center = layout.centers[i];
        for (n, line) in label_lines(label).iter().enumerate() {
            c.text(center - width_of(line) / 2, header_y + n as i32, line);
        }
        if !compact {
            let (left, right) = layout.header(i);
            for x in left..=right {
                c.set(x, rule_y, '─');
            }
            c.set(center, rule_y, '┬');
        }
        for y in lifeline_start..=lifeline_end {
            c.set(center, y, '│');
        }
    }
    let mut activations: Vec<(i32, i32, i32)> = Vec::new();
    let mut active: HashMap<String, Vec<i32>> = HashMap::new();
    let mut last_event: HashMap<String, i32> = HashMap::new();
    let close = |active: &mut HashMap<String, Vec<i32>>,
                 who: &str,
                 y: i32,
                 out: &mut Vec<(i32, i32, i32)>| {
        if let (Some(start), Some(i)) = (active.get_mut(who).and_then(Vec::pop), layout.at(who)) {
            out.push((layout.centers[i], start, y));
        }
    };
    let mut y = step_start;
    let mut open_frames: Vec<(Bounds, i32)> = Vec::new();
    for (index, step) in diagram.steps.iter().enumerate() {
        let h = layout.step_height(step);
        match step {
            Step::Activation {
                participant,
                active: on,
            } => {
                let event = last_event
                    .get(participant)
                    .copied()
                    .unwrap_or(y)
                    .min(lifeline_end);
                if *on {
                    active.entry(participant.clone()).or_default().push(event);
                } else {
                    close(&mut active, participant, event, &mut activations);
                }
                continue;
            }
            Step::Note { over, label, side } => {
                if let Some((x, lines)) = layout.note_x(over, label, *side) {
                    for (n, line) in lines.iter().enumerate() {
                        c.text(x, y + 1 + n as i32, line);
                    }
                }
            }
            Step::Fragment(kind, label) => {
                let mut bounds = frames
                    .get(&index)
                    .copied()
                    .unwrap_or_else(|| layout.frame(*kind, label, 0));
                if matches!(kind, Fragment::Alt | Fragment::Loop) {
                    open_frames.push((bounds, y));
                } else if let Some(frame) = open_frames.last_mut() {
                    bounds = frame.0;
                    for wy in frame.1 + 1..y {
                        c.set(bounds.left, wy, '│');
                        c.set(bounds.right, wy, '│');
                    }
                    if *kind == Fragment::End {
                        open_frames.pop();
                    } else {
                        frame.1 = y;
                    }
                }
                for x in bounds.left..=bounds.right {
                    c.set(x, y, '─');
                }
                let (l, r) = match kind {
                    Fragment::Alt | Fragment::Loop => (CORNERS[0], CORNERS[1]),
                    Fragment::Else => ('├', '┤'),
                    Fragment::End => (CORNERS[2], CORNERS[3]),
                };
                c.set(bounds.left, y, l);
                c.set(bounds.right, y, r);
                let text = fragment_label(*kind, label);
                if !text.is_empty() {
                    c.text(bounds.left + 2, y, &text);
                }
            }
            Step::Message(m) => {
                let (Some(f), Some(t)) = (layout.at(&m.from), layout.at(&m.to)) else {
                    y += h;
                    continue;
                };
                let lines = label_lines(&message_text(m));
                let (fx, tx) = (layout.centers[f], layout.centers[t]);
                let arrow_y;
                if f == t {
                    let right = fx + self_loop_width(&lines);
                    let bottom = y + lines.len() as i32 + 1;
                    c.set(fx, y, '├');
                    for x in fx + 1..right {
                        c.set(x, y, '─');
                    }
                    c.set(right, y, '╮');
                    for (n, line) in lines.iter().enumerate() {
                        let ly = y + 1 + n as i32;
                        c.set(fx, ly, '│');
                        c.text(fx + 2, ly, line);
                        c.set(right, ly, '│');
                    }
                    for x in fx + 1..right {
                        c.set(x, bottom, '─');
                    }
                    let head_x = if m.head == Head::Filled { fx } else { fx + 1 };
                    c.set(head_x, bottom, head_char(m.head, false));
                    c.set(right, bottom, '╯');
                    arrow_y = bottom;
                    last_event.insert(m.from.clone(), bottom);
                } else {
                    let rightward = tx > fx;
                    let (left, right) = (fx.min(tx), fx.max(tx));
                    let inline = layout.inline(m, &lines, fx, tx);
                    arrow_y = if inline.is_some() {
                        y
                    } else {
                        y + lines.len() as i32
                    };
                    if inline.is_none() {
                        for (n, line) in lines.iter().enumerate() {
                            c.text(left + 2, y + n as i32, line);
                        }
                    }
                    for x in left + 1..right {
                        c.set(x, arrow_y, '─');
                    }
                    let step = if rightward { 1 } else { -1 };
                    c.set(fx, arrow_y, if rightward { '├' } else { '┤' });
                    for n in 1..FADE {
                        c.set(fx + step * n, arrow_y, '─');
                    }
                    let head_x = if m.head == Head::Filled {
                        tx
                    } else {
                        tx - step
                    };
                    c.set(head_x, arrow_y, head_char(m.head, rightward));
                    if let Some(label) = inline {
                        let lx = (left + right - width_of(&label)).div_euclid(2);
                        c.text(lx, y, &label);
                    }
                    last_event.insert(m.from.clone(), arrow_y);
                    last_event.insert(m.to.clone(), arrow_y);
                }
                if let Some(who) = &m.activate {
                    active.entry(who.clone()).or_default().push(arrow_y);
                }
                if let Some(who) = &m.deactivate {
                    close(&mut active, who, arrow_y, &mut activations);
                }
            }
        }
        y += h;
    }
    let names: Vec<String> = active.keys().cloned().collect();
    for who in names {
        while active.get(&who).is_some_and(|v| !v.is_empty()) {
            close(&mut active, &who, lifeline_end, &mut activations);
        }
    }
    for (x, start, end) in activations {
        for ay in start..=end {
            if c.get(x, ay) == '│' {
                c.set(x, ay, '┃');
            }
        }
    }
    let mut rows = c.grid.rows();
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    rows
}

fn width_of(text: &str) -> i32 {
    width(text) as i32
}

pub fn render(source: &str) -> Result<Vec<String>, String> {
    Ok(draw(&parse(source)?, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full(source: &str) -> String {
        draw(&parse(source).unwrap(), false).join("\n")
    }

    fn compact(source: &str) -> String {
        render(source).unwrap().join("\n")
    }

    /// merman's compact expectation (`sequence/diagram.test.ts`).
    #[test]
    fn compact_layout_like_merman() {
        let rows = render(
            "sequenceDiagram\n  participant Leaf as leaf tool\n  participant Location as LocationMutation\n  participant File as FileMutation\n  Leaf->>Location: resolve(path)\n  Location-->>Leaf: Plan(target, authority anchor)\n  Leaf->>File: commit(plan)\n  File->>Location: revalidate(plan)\n  Location-->>File: same target or reject",
        )
        .unwrap();
        assert_eq!(
            rows,
            [
                "leaf tool                       LocationMutation             FileMutation",
                "    │                                   │                          │",
                "    ├───────── resolve(path) ───────────►                          │",
                "    │                                   │                          │",
                "    ◄─ Plan(target, authority anchor) ──┤                          │",
                "    │                                   │                          │",
                "    ├─────────────────────── commit(plan) ─────────────────────────►",
                "    │                                   │                          │",
                "    │                                   ◄─── revalidate(plan) ─────┤",
                "    │                                   │                          │",
                "    │                                   ├─ same target or reject ──►",
                "    │                                   │                          │",
            ]
        );
    }

    #[test]
    fn full_layout_messages_and_heads_like_merman() {
        assert_eq!(
            full(
                "sequenceDiagram\n  participant B as Browser\n  participant S as Server\n  B->>S: GET /\n  S-->>B: 401 WWW-Auth"
            ),
            "Browser           Server\n───┬───           ───┬──\n   │                 │\n   │ GET /           │\n   ├─────────────────►\n   │                 │\n   │ 401 WWW-Auth    │\n   ◄─────────────────┤\n   │                 │"
        );
        assert_eq!(
            full(
                "sequenceDiagram\n  A->B: open solid\n  B-->A: open dashed\n  A-xB: failed solid\n  B--xA: failed dashed\n  A-)B: async solid\n  B--)A: async dashed"
            ),
            " A                  B\n─┬─                ─┬─\n │                  │\n │ open solid       │\n ├─────────────────>│\n │                  │\n │ open dashed      │\n │<─────────────────┤\n │                  │\n │ failed solid     │\n ├─────────────────✕│\n │                  │\n │ failed dashed    │\n │✕─────────────────┤\n │                  │\n │ async solid      │\n ├─────────────────)│\n │                  │\n │ async dashed     │\n │(─────────────────┤\n │                  │"
        );
    }

    #[test]
    fn self_messages_loop_back() {
        assert_eq!(
            full("sequenceDiagram\n  participant Service\n  Service->>Service: Check Permissions"),
            "Service\n───┬───\n   │\n   ├────────────────────╮\n   │ Check Permissions  │\n   ◄────────────────────╯\n   │"
        );
    }

    #[test]
    fn notes_sit_in_their_rows() {
        let text = full(
            "sequenceDiagram\n  Browser->>Server: one\n  Note over Browser,Server: phase\n  Browser->>Server: two",
        );
        let lines: Vec<&str> = text.lines().collect();
        let note = lines.iter().position(|l| l.contains("phase")).unwrap();
        assert_eq!(lines[note - 1].trim(), "│                 │");
        assert!(lines[note].contains(" phase "));
        assert_eq!(lines[note + 1].trim(), "│                 │");
        assert_eq!(
            lines.iter().position(|l| l.contains("two")).unwrap(),
            note + 2
        );
        let sides = compact(
            "sequenceDiagram\n  participant OpenTUI\n  Note right of OpenTUI: child is not renderable\n  Note left of OpenTUI: remove failed",
        );
        assert!(sides.contains("child is not renderable") && sides.contains("remove failed"));
    }

    #[test]
    fn fragments_frame_their_steps() {
        let text = compact(
            "sequenceDiagram\n  alt accepted\n    Browser->>Server: ok\n  else rejected\n    Server-->>Browser: no\n  end",
        );
        let at = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("{needle}\n{text}"))
        };
        assert!(at("╭─ alt: accepted") < at("ok") && at("ok") < at("├─ else: rejected"));
        assert!(at("├─ else: rejected") < at("no") && at("no") < at("╰"));
        let wide = full(
            "sequenceDiagram\n  A->>B: start\n  alt ok\n    A->>B: yes\n  else validation failed with a substantially longer explanation\n    B-->>A: no\n  end",
        );
        let else_row = wide
            .lines()
            .find(|l| l.contains("validation failed"))
            .unwrap();
        let end_row = wide.lines().rev().find(|l| l.contains('╰')).unwrap();
        assert_eq!(
            else_row.rfind('┤').map(|b| else_row[..b].chars().count()),
            end_row.rfind('╯').map(|b| end_row[..b].chars().count())
        );
        let looped = compact(
            "sequenceDiagram\n  loop retry up to 3x\n    Browser->>API: GET /users/42\n    API-->>Browser: 503\n  end",
        );
        assert!(looped.contains("╭─ ↻ loop: retry up to 3x"));
        assert!(!looped.contains("end loop"));
        let overhang = full(
            "sequenceDiagram\n  participant A\n  participant B\n  alt ok\n    A->>B: yes\n  end",
        );
        let a = overhang.lines().next().unwrap().find('A').unwrap();
        let frame = overhang
            .lines()
            .find(|l| l.contains("alt: ok"))
            .unwrap()
            .find('╭')
            .unwrap();
        assert!(frame < a);
    }

    #[test]
    fn activations_numbers_and_groups() {
        let text = compact(
            "sequenceDiagram\n  Browser->>+Server: request\n  Server-->>-Browser: response",
        );
        assert!(text.contains('┃') && text.contains("request") && text.contains("response"));
        let numbered = compact(
            "sequenceDiagram\n  autonumber 10 5\n  Browser->>API: first\n  API-->>Browser: second",
        );
        assert!(numbered.contains("10. first") && numbered.contains("15. second"));
        let boxed = compact(
            "sequenceDiagram\n  box Aqua Frontend\n    participant UI\n  end\n  participant API\n  UI->>API: call",
        );
        assert!(
            boxed.starts_with("╭─ Frontend")
                || boxed.lines().next().unwrap().contains(" Frontend "),
            "{boxed}"
        );
        assert!(boxed.lines().last().unwrap().contains('╰'));
    }

    #[test]
    fn full_height_groups_like_merman() {
        assert_eq!(
            full(
                "sequenceDiagram\n  participant Browser\n  box Backend\n    participant API\n    participant Cache\n    participant DB\n  end\n  Browser->>API: GET /users/42\n  API->>Cache: get user:42"
            ),
            [
                "                   ╭─ Backend ───────────────────────────────╮",
                "Browser            │ API              Cache              DB  │",
                "───┬───            │ ─┬─              ──┬──              ─┬─ │",
                "   │               │  │                 │                 │  │",
                "   │ GET /users/42 │  │                 │                 │  │",
                "   ├──────────────────►                 │                 │  │",
                "   │               │  │                 │                 │  │",
                "   │               │  │ get user:42     │                 │  │",
                "   │               │  ├─────────────────►                 │  │",
                "   │               │  │                 │                 │  │",
                "                   ╰─────────────────────────────────────────╯",
            ]
            .join("\n")
        );
    }

    #[test]
    fn errors() {
        assert!(render("sequenceDiagram\n  else x").is_err());
        assert!(render("sequenceDiagram\n  alt x\n  A->>B: y").is_err());
        assert!(render("sequenceDiagram\n  A<<->>B: both").is_err());
        assert!(render("sequenceDiagram\n  par x\n  end").is_err());
    }
}
