//! Node trees as character boxes, after `@opencode/latex`'s `layout.ts`: rows with TeX-like
//! spacing, fractions on the math axis, hooked roots, scripts (Unicode when every character
//! has a form), stretched delimiters, matrices, braces, accents and the math alphabets.

use unicode_segmentation::UnicodeSegmentation;

use super::parser::{Accent, Node, Role, Variant};

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub width: usize,
    pub height: usize,
    pub baseline: usize,
    cells: Vec<Vec<Option<String>>>,
}

#[derive(Clone, Copy)]
pub struct Context {
    pub display: bool,
    pub compact: bool,
    variant: Option<Variant>,
}

impl Context {
    pub fn new(display: bool, compact: bool) -> Context {
        Context {
            display,
            compact,
            variant: None,
        }
    }
}

const SUPERSCRIPTS: &[(char, char)] = &[
    ('0', '⁰'),
    ('1', '¹'),
    ('2', '²'),
    ('3', '³'),
    ('4', '⁴'),
    ('5', '⁵'),
    ('6', '⁶'),
    ('7', '⁷'),
    ('8', '⁸'),
    ('9', '⁹'),
    ('+', '⁺'),
    ('-', '⁻'),
    ('=', '⁼'),
    ('(', '⁽'),
    (')', '⁾'),
    ('n', 'ⁿ'),
    ('i', 'ⁱ'),
];

const SUBSCRIPTS: &[(char, char)] = &[
    ('0', '₀'),
    ('1', '₁'),
    ('2', '₂'),
    ('3', '₃'),
    ('4', '₄'),
    ('5', '₅'),
    ('6', '₆'),
    ('7', '₇'),
    ('8', '₈'),
    ('9', '₉'),
    ('+', '₊'),
    ('-', '₋'),
    ('=', '₌'),
    ('(', '₍'),
    (')', '₎'),
    ('a', 'ₐ'),
    ('e', 'ₑ'),
    ('h', 'ₕ'),
    ('i', 'ᵢ'),
    ('j', 'ⱼ'),
    ('k', 'ₖ'),
    ('l', 'ₗ'),
    ('m', 'ₘ'),
    ('n', 'ₙ'),
    ('o', 'ₒ'),
    ('p', 'ₚ'),
    ('r', 'ᵣ'),
    ('s', 'ₛ'),
    ('t', 'ₜ'),
    ('u', 'ᵤ'),
    ('v', 'ᵥ'),
    ('x', 'ₓ'),
];

fn cell_width(grapheme: &str) -> usize {
    crate::text::width(grapheme)
}

impl Block {
    pub fn blank(width: usize, height: usize, baseline: usize) -> Block {
        let height = height.max(1);
        Block {
            width,
            height,
            baseline,
            cells: vec![vec![None; width]; height],
        }
    }

    fn text(text: &str) -> Block {
        let graphemes: Vec<&str> = text.graphemes(true).collect();
        let width = graphemes.iter().map(|g| cell_width(g)).sum();
        let mut block = Block::blank(width, 1, 0);
        let mut x = 0;
        for g in graphemes {
            block.set(x, 0, g);
            x += cell_width(g);
        }
        block
    }

    fn set(&mut self, x: usize, y: usize, value: &str) {
        if let Some(cell) = self.cells.get_mut(y).and_then(|r| r.get_mut(x)) {
            *cell = Some(value.to_string());
        }
    }

    fn overlay(&mut self, source: &Block, x: usize, y: usize) {
        for (sy, row) in source.cells.iter().enumerate() {
            for (sx, cell) in row.iter().enumerate() {
                if let Some(cell) = cell {
                    self.set(x + sx, y + sy, cell);
                }
            }
        }
    }

    fn horizontal(&mut self, y: usize, x: usize, width: usize, value: &str) {
        for offset in 0..width {
            self.set(x + offset, y, value);
        }
    }

    pub fn rows(&self) -> Vec<String> {
        self.cells
            .iter()
            .map(|row| {
                let mut out = String::new();
                let mut x = 0;
                while x < self.width {
                    match &row[x] {
                        Some(cell) => {
                            out.push_str(cell);
                            x += cell_width(cell).max(1);
                        }
                        None => {
                            out.push(' ');
                            x += 1;
                        }
                    }
                }
                out.trim_end().to_string()
            })
            .collect()
    }
}

fn hpack(boxes: Vec<Block>) -> Block {
    if boxes.is_empty() {
        return Block::blank(0, 1, 0);
    }
    let ascent = boxes.iter().map(|b| b.baseline).max().unwrap_or(0);
    let descent = boxes
        .iter()
        .map(|b| b.height - b.baseline - 1)
        .max()
        .unwrap_or(0);
    let width = boxes.iter().map(|b| b.width).sum();
    let mut out = Block::blank(width, ascent + descent + 1, ascent);
    let mut x = 0;
    for b in &boxes {
        out.overlay(b, x, ascent - b.baseline);
        x += b.width;
    }
    out
}

pub fn layout(node: &Node, cx: Context) -> Block {
    match node {
        Node::Row(nodes) => row(nodes, cx),
        Node::Symbol(value, _) | Node::Text(value) | Node::Operator(value, _) => {
            Block::text(&apply_variant(value, cx.variant))
        }
        Node::Space(width) => Block::blank(*width, 1, 0),
        Node::Fraction {
            numerator,
            denominator,
            bar,
            align,
        } => fraction(numerator, denominator, *bar, *align, cx),
        Node::Root(body, index) => root(body, index.as_deref(), cx),
        Node::Scripts { base, sup, sub } => scripts(base, sup.as_deref(), sub.as_deref(), cx),
        Node::Delimited(left, body, right) => {
            let body = layout(body, cx);
            let l = delimiter(left, body.height, body.baseline, true);
            let r = delimiter(right, body.height, body.baseline, false);
            hpack(vec![l, body, r])
        }
        Node::Matrix {
            rows,
            environment,
            columns,
        } => matrix(rows, environment, columns.as_deref(), cx),
        Node::Brace(body, over) => brace(body, *over, cx),
        Node::Accent(kind, body) => accent(*kind, body, cx),
        Node::Variant(variant, body) => layout(
            body,
            Context {
                variant: Some(*variant),
                ..cx
            },
        ),
        Node::OverUnder { base, over, under } => {
            over_under(base, over.as_deref(), under.as_deref(), cx)
        }
    }
}

fn role(node: &Node) -> Option<Role> {
    match node {
        Node::Symbol(_, role) => *role,
        Node::Operator(..) | Node::Fraction { .. } | Node::Root(..) | Node::Matrix { .. } => {
            Some(Role::Operator)
        }
        Node::Scripts { base, .. } => role(base),
        _ => None,
    }
}

fn spaced(previous: Option<Role>, current: Option<Role>, count: usize) -> bool {
    use Role::*;
    if count == 0
        || matches!(previous, Some(Punctuation | Opening))
        || matches!(current, Some(Punctuation | Closing))
    {
        return false;
    }
    matches!(previous, Some(Binary | Relation | Operator))
        || matches!(current, Some(Binary | Relation | Operator))
}

/// A binary operator without operands on both sides reads as ordinary (a sign).
fn binary(role: Option<Role>, previous: Option<Role>, next: Option<Role>) -> Option<Role> {
    use Role::*;
    if role != Some(Binary) {
        return role;
    }
    let lone = previous
        .is_none_or(|p| matches!(p, Binary | Relation | Operator | Punctuation | Opening))
        || next.is_none_or(|n| matches!(n, Binary | Relation | Punctuation | Closing));
    if lone { Some(Ordinary) } else { role }
}

fn row(nodes: &[Node], cx: Context) -> Block {
    if nodes.is_empty() {
        return Block::blank(0, 1, 0);
    }
    let mut boxes = Vec::new();
    let mut previous: Option<Role> = None;
    for (index, node) in nodes.iter().enumerate() {
        let next = nodes[index + 1..]
            .iter()
            .find(|n| !matches!(n, Node::Space(_)))
            .map(|n| role(n).unwrap_or(Role::Ordinary));
        let current = binary(role(node), previous, next);
        if spaced(previous, current, boxes.len()) {
            boxes.push(Block::blank(1, 1, 0));
        }
        boxes.push(layout(node, cx));
        if !matches!(node, Node::Space(_)) {
            previous = Some(current.unwrap_or(Role::Ordinary));
        }
    }
    hpack(boxes)
}

fn fraction(
    numerator: &Node,
    denominator: &Node,
    bar: bool,
    align: Option<bool>,
    cx: Context,
) -> Block {
    let (num, den) = (layout(numerator, cx), layout(denominator, cx));
    let width = num.width.max(den.width) + 2;
    // A binomial keeps the empty axis row so neighbours still align on it.
    let mut out = Block::blank(width, num.height + den.height + 1, num.height);
    let nx = match align {
        Some(true) => 1,
        Some(false) => width - num.width - 1,
        None => (width - num.width) / 2,
    };
    out.overlay(&num, nx, 0);
    if bar {
        out.horizontal(num.height, 0, width, "─");
    }
    out.overlay(&den, (width - den.width) / 2, num.height + 1);
    out
}

fn root(body: &Node, index: Option<&Node>, cx: Context) -> Block {
    let body = layout(body, cx);
    let index = index.map(|i| layout(i, cx));
    let index_width = index.as_ref().map_or(0, |i| i.width.saturating_sub(1));
    // The index ends beside the overbar, never inside the hook or the radicand.
    let top = index.as_ref().map_or(0, |i| i.height - 1);
    let body_x = index_width + 2;
    let height = top + body.height + 1;
    let mut out = Block::blank(body_x + body.width, height, top + body.baseline + 1);
    out.set(body_x - 1, top, "╭");
    out.horizontal(top, body_x, body.width, "─");
    for y in top + 1..height - 1 {
        out.set(body_x - 1, y, "│");
    }
    out.set(body_x - 2, height - 1, "╰");
    out.set(body_x - 1, height - 1, "╯");
    out.overlay(&body, body_x, top + 1);
    if let Some(index) = index {
        out.overlay(&index, 0, 0);
    }
    out
}

/// The plain text of a node made only of symbols and text.
fn simple_text(node: &Node) -> Option<String> {
    match node {
        Node::Symbol(v, _) | Node::Text(v) | Node::Operator(v, _) => Some(v.clone()),
        Node::Row(nodes) => nodes.iter().map(simple_text).collect(),
        _ => None,
    }
}

fn map_script(value: &str, table: &[(char, char)]) -> Option<String> {
    value
        .chars()
        .map(|c| table.iter().find(|(a, _)| *a == c).map(|(_, b)| *b))
        .collect()
}

/// A script that is not empty (`x^{}` has none).
fn present(node: Option<&Node>) -> Option<&Node> {
    node.filter(|n| simple_text(n).as_deref() != Some(""))
}

fn scripts(base: &Node, sup: Option<&Node>, sub: Option<&Node>, cx: Context) -> Block {
    let (sup, sub) = (present(sup), present(sub));
    let limits = matches!(base, Node::Operator(_, true)) && cx.display;
    if matches!(base, Node::Brace(..)) || limits {
        return over_under(base, sup, sub, cx);
    }
    let base = layout(base, cx);
    if cx.compact && base.height == 1 {
        let text = |n: Option<&Node>, table| match n {
            None => Some(String::new()),
            Some(n) => simple_text(n).and_then(|t| map_script(&t, table)),
        };
        if let (Some(up), Some(down)) = (text(sup, SUPERSCRIPTS), text(sub, SUBSCRIPTS)) {
            return hpack(vec![base, Block::text(&format!("{up}{down}"))]);
        }
    }
    let up = sup.map(|n| layout(n, cx));
    let down = sub.map(|n| layout(n, cx));
    let script_width = up
        .as_ref()
        .map_or(0, |b| b.width)
        .max(down.as_ref().map_or(0, |b| b.width));
    let top = up.as_ref().map_or(0, |b| b.height);
    let bottom = down.as_ref().map_or(0, |b| b.height);
    let mut out = Block::blank(
        base.width + script_width,
        top + base.height + bottom,
        top + base.baseline,
    );
    out.overlay(&base, 0, top);
    if let Some(up) = &up {
        out.overlay(up, base.width, 0);
    }
    if let Some(down) = &down {
        out.overlay(down, base.width, top + base.height);
    }
    out
}

fn over_under(base: &Node, over: Option<&Node>, under: Option<&Node>, cx: Context) -> Block {
    let base = layout(base, cx);
    let over = over.map(|n| layout(n, cx));
    let under = under.map(|n| layout(n, cx));
    let width = base
        .width
        .max(over.as_ref().map_or(0, |b| b.width))
        .max(under.as_ref().map_or(0, |b| b.width));
    let over_height = over.as_ref().map_or(0, |b| b.height);
    let height = over_height + base.height + under.as_ref().map_or(0, |b| b.height);
    let mut out = Block::blank(width, height, over_height + base.baseline);
    if let Some(over) = &over {
        out.overlay(over, (width - over.width) / 2, 0);
    }
    out.overlay(&base, (width - base.width) / 2, over_height);
    if let Some(under) = &under {
        out.overlay(under, (width - under.width) / 2, over_height + base.height);
    }
    out
}

fn delimiter(value: &str, height: usize, baseline: usize, left: bool) -> Block {
    if value.is_empty() {
        return Block::blank(0, height, baseline);
    }
    if height <= 1 {
        return Block::text(value);
    }
    let glyphs: [&str; 3] = match value {
        "(" => ["⎛", "⎜", "⎝"],
        ")" => ["⎞", "⎟", "⎠"],
        "[" => ["⎡", "⎢", "⎣"],
        "]" => ["⎤", "⎥", "⎦"],
        "{" => ["⎧", "⎪", "⎩"],
        "}" => ["⎫", "⎪", "⎭"],
        "⌊" => ["│", "│", "⌊"],
        "⌋" => ["│", "│", "⌋"],
        "⌈" => ["⌈", "│", "│"],
        "⌉" => ["⌉", "│", "│"],
        "⟨" => ["/", "│", "\\"],
        "⟩" => ["\\", "│", "/"],
        other => [other, other, other],
    };
    let width = glyphs.iter().map(|g| cell_width(g)).max().unwrap_or(1);
    let mut out = Block::blank(width, height, baseline);
    for y in 0..height {
        let glyph = match y {
            0 => glyphs[0],
            y if y == height - 1 => glyphs[2],
            _ => glyphs[1],
        };
        out.set(0, y, glyph);
    }
    if matches!(value, "{" | "}") && height >= 3 {
        out.set(0, height / 2, if left { "⎨" } else { "⎬" });
    }
    out
}

fn matrix(rows: &[Vec<Node>], environment: &str, columns: Option<&str>, cx: Context) -> Block {
    let cells: Vec<Vec<Block>> = rows
        .iter()
        .map(|r| r.iter().map(|c| layout(c, cx)).collect())
        .collect();
    let aligns: Option<Vec<char>> =
        columns.map(|c| c.chars().filter(|c| "lcr".contains(*c)).collect());
    let rules: Vec<usize> = columns.map_or_else(Vec::new, |c| {
        c.split(['l', 'c', 'r']).map(str::len).collect()
    });
    let count = aligns
        .as_ref()
        .map_or(0, Vec::len)
        .max(cells.iter().map(Vec::len).max().unwrap_or(0));
    let widths: Vec<usize> = (0..count)
        .map(|c| {
            cells
                .iter()
                .filter_map(|r| r.get(c))
                .map(|b| b.width)
                .max()
                .unwrap_or(0)
        })
        .collect();
    let ascents: Vec<usize> = cells
        .iter()
        .map(|r| r.iter().map(|b| b.baseline).max().unwrap_or(0))
        .collect();
    let descents: Vec<usize> = cells
        .iter()
        .map(|r| {
            r.iter()
                .map(|b| b.height - b.baseline - 1)
                .max()
                .unwrap_or(0)
        })
        .collect();
    let heights: Vec<usize> = ascents
        .iter()
        .zip(&descents)
        .map(|(a, d)| a + 1 + d)
        .collect();
    let aligned = matches!(environment, "aligned" | "align");
    let column_gap = if environment == "cases" || aligned {
        2
    } else {
        1
    };
    let gaps: Vec<usize> = (0..=count)
        .map(|boundary| {
            let edge = boundary == 0 || boundary == count;
            match rules.get(boundary).copied().unwrap_or(0) {
                0 if edge => 0,
                0 => column_gap,
                rule => rule + if edge { 1 } else { 2 },
            }
        })
        .collect();
    let width = widths.iter().sum::<usize>() + gaps.iter().sum::<usize>();
    let height = (heights.iter().sum::<usize>() + rows.len().saturating_sub(1)).max(1);
    let mut out = Block::blank(width, height, height / 2);
    let mut y = 0;
    for (r, row) in cells.iter().enumerate() {
        let mut x = gaps[0];
        for (c, column_width) in widths.iter().enumerate() {
            if let Some(cell) = row.get(c) {
                let align = aligns.as_ref().and_then(|a| a.get(c).copied()).unwrap_or(
                    match (environment, aligned) {
                        ("cases", _) => 'l',
                        (_, true) if c % 2 == 0 => 'r',
                        (_, true) => 'l',
                        _ => 'c',
                    },
                );
                let cx = x + match align {
                    'l' => 0,
                    'r' => column_width - cell.width,
                    _ => (column_width - cell.width) / 2,
                };
                out.overlay(cell, cx, y + ascents[r] - cell.baseline);
            }
            x += column_width + gaps[c + 1];
        }
        y += heights[r] + 1;
    }
    let mut boundary_x = 0;
    for (boundary, gap) in gaps.iter().enumerate() {
        for rule in 0..rules.get(boundary).copied().unwrap_or(0) {
            for ry in 0..height {
                out.set(boundary_x + usize::from(boundary != 0) + rule, ry, "│");
            }
        }
        boundary_x += gap + widths.get(boundary).copied().unwrap_or(0);
    }
    let delimiters = match environment {
        "pmatrix" => Some(("(", ")")),
        "bmatrix" => Some(("[", "]")),
        "Bmatrix" => Some(("{", "}")),
        "vmatrix" => Some(("│", "│")),
        "Vmatrix" => Some(("║", "║")),
        "cases" => Some(("{", "")),
        _ => None,
    };
    match delimiters {
        Some((l, r)) => {
            let baseline = out.baseline;
            hpack(vec![
                delimiter(l, height, baseline, true),
                out,
                delimiter(r, height, baseline, false),
            ])
        }
        None => out,
    }
}

fn brace(body: &Node, over: bool, cx: Context) -> Block {
    let body = layout(body, cx);
    let width = body.width.max(3);
    let mut out = Block::blank(width, body.height + 1, body.baseline + usize::from(over));
    let y = if over { 0 } else { body.height };
    out.overlay(&body, (width - body.width) / 2, usize::from(over));
    out.horizontal(y, 0, width, "─");
    out.set(0, y, if over { "╭" } else { "╰" });
    out.set(width - 1, y, if over { "╮" } else { "╯" });
    out.set((width - 1) / 2, y, if over { "┴" } else { "┬" });
    out
}

fn accent(kind: Accent, body: &Node, cx: Context) -> Block {
    let body = layout(body, cx);
    if kind == Accent::Underline {
        let mut out = Block::blank(body.width, body.height + 1, body.baseline);
        out.overlay(&body, 0, 0);
        out.horizontal(body.height, 0, body.width, "─");
        return out;
    }
    let mut out = Block::blank(body.width, body.height + 1, body.baseline + 1);
    out.overlay(&body, 0, 1);
    let mark = match kind {
        Accent::Hat | Accent::WideHat if body.width == 1 => "^",
        Accent::Hat | Accent::WideHat => "⌢",
        Accent::Bar | Accent::Overline => "─",
        Accent::Vec => "→",
        Accent::Tilde => "~",
        Accent::Dot => "·",
        _ => "¨",
    };
    if matches!(kind, Accent::Bar | Accent::Overline) {
        out.horizontal(0, 0, body.width, mark);
    } else {
        out.set(body.width.saturating_sub(cell_width(mark)) / 2, 0, mark);
    }
    out
}

fn apply_variant(value: &str, variant: Option<Variant>) -> String {
    let (exceptions, upper, lower, digits): (&[(char, char)], u32, u32, Option<u32>) = match variant
    {
        Some(Variant::DoubleStruck) => (
            &[
                ('C', 'ℂ'),
                ('H', 'ℍ'),
                ('N', 'ℕ'),
                ('P', 'ℙ'),
                ('Q', 'ℚ'),
                ('R', 'ℝ'),
                ('Z', 'ℤ'),
            ],
            0x1d538,
            0x1d552,
            Some(0x1d7d8),
        ),
        Some(Variant::Script) => (
            &[
                ('B', 'ℬ'),
                ('E', 'ℰ'),
                ('F', 'ℱ'),
                ('H', 'ℋ'),
                ('I', 'ℐ'),
                ('L', 'ℒ'),
                ('M', 'ℳ'),
                ('R', 'ℛ'),
                ('e', 'ℯ'),
                ('g', 'ℊ'),
                ('o', 'ℴ'),
            ],
            0x1d49c,
            0x1d4b6,
            None,
        ),
        Some(Variant::Fraktur) => (
            &[('C', 'ℭ'), ('H', 'ℌ'), ('I', 'ℑ'), ('R', 'ℜ'), ('Z', 'ℨ')],
            0x1d504,
            0x1d51e,
            None,
        ),
        Some(Variant::Sans) => (&[], 0x1d5a0, 0x1d5ba, Some(0x1d7e2)),
        Some(Variant::Monospace) => (&[], 0x1d670, 0x1d68a, Some(0x1d7f6)),
        _ => return value.to_string(),
    };
    value
        .chars()
        .map(|c| {
            if let Some((_, e)) = exceptions.iter().find(|(a, _)| *a == c) {
                return *e;
            }
            let code = c as u32;
            let mapped = match c {
                'A'..='Z' => Some(upper + code - 65),
                'a'..='z' => Some(lower + code - 97),
                '0'..='9' => digits.map(|d| d + code - 48),
                _ => None,
            };
            mapped.and_then(char::from_u32).unwrap_or(c)
        })
        .collect()
}
