//! ```` ```latex ````/```` ```math ```` blocks as 2-D Unicode math: fractions over a rule, roots
//! with a bar, raised and lowered scripts, limits above and below big operators. This covers the
//! common subset of 0.5.1's `@opencode/latex`; anything outside it is an error and the block
//! stays source code, as 0.5.1 does for unsupported input.

const MAX_SOURCE: usize = 100_000;
const MAX_DEPTH: usize = 256;

/// A laid-out piece: rows of equal display width and the row the text baseline is on.
#[derive(Debug, Clone, PartialEq)]
struct Block {
    rows: Vec<String>,
    baseline: usize,
}

fn width(text: &str) -> usize {
    crate::text::width(text)
}

impl Block {
    fn text(text: &str) -> Block {
        Block {
            rows: vec![text.to_string()],
            baseline: 0,
        }
    }

    fn width(&self) -> usize {
        self.rows.first().map_or(0, |r| width(r))
    }

    fn pad(row: &str, to: usize) -> String {
        format!("{row}{}", " ".repeat(to.saturating_sub(width(row))))
    }

    fn centered(&self, to: usize) -> Block {
        let left = (to - self.width()) / 2;
        Block {
            rows: self
                .rows
                .iter()
                .map(|r| Block::pad(&format!("{}{r}", " ".repeat(left)), to))
                .collect(),
            baseline: self.baseline,
        }
    }

    /// Side by side, baselines aligned.
    fn row(blocks: Vec<Block>) -> Block {
        let above = blocks.iter().map(|b| b.baseline).max().unwrap_or(0);
        let below = blocks
            .iter()
            .map(|b| b.rows.len() - b.baseline - 1)
            .max()
            .unwrap_or(0);
        let height = above + below + 1;
        let mut rows = vec![String::new(); height];
        for block in &blocks {
            let top = above - block.baseline;
            let w = block.width();
            for (y, row) in rows.iter_mut().enumerate() {
                match y.checked_sub(top).and_then(|i| block.rows.get(i)) {
                    Some(line) => row.push_str(&Block::pad(line, w)),
                    None => row.push_str(&" ".repeat(w)),
                }
            }
        }
        Block {
            rows,
            baseline: above,
        }
    }

    /// `top` over `bottom`, centred; the baseline row comes from `baseline_of`.
    fn stack(parts: Vec<Block>, baseline: usize) -> Block {
        let w = parts.iter().map(Block::width).max().unwrap_or(0);
        let mut rows = Vec::new();
        for part in parts {
            rows.extend(part.centered(w).rows);
        }
        Block { rows, baseline }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Text(String),
    Group(Vec<Node>),
    Frac(Box<Node>, Box<Node>),
    Sqrt(Box<Node>),
    Scripts {
        base: Box<Node>,
        sub: Option<Box<Node>>,
        sup: Option<Box<Node>>,
    },
}

fn symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" | "varepsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" => "θ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" => "π",
        "rho" => "ρ",
        "sigma" => "σ",
        "tau" => "τ",
        "phi" | "varphi" => "φ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        "times" => "×",
        "cdot" => "⋅",
        "div" => "÷",
        "pm" => "±",
        "mp" => "∓",
        "le" | "leq" => "≤",
        "ge" | "geq" => "≥",
        "ne" | "neq" => "≠",
        "approx" => "≈",
        "equiv" => "≡",
        "sim" => "∼",
        "propto" => "∝",
        "infty" => "∞",
        "partial" => "∂",
        "nabla" => "∇",
        "in" => "∈",
        "notin" => "∉",
        "subset" => "⊂",
        "subseteq" => "⊆",
        "cup" => "∪",
        "cap" => "∩",
        "emptyset" => "∅",
        "forall" => "∀",
        "exists" => "∃",
        "neg" | "lnot" => "¬",
        "land" | "wedge" => "∧",
        "lor" | "vee" => "∨",
        "to" | "rightarrow" => "→",
        "leftarrow" => "←",
        "Rightarrow" | "implies" => "⇒",
        "Leftarrow" => "⇐",
        "Leftrightarrow" | "iff" => "⇔",
        "mapsto" => "↦",
        "ldots" | "dots" => "…",
        "cdots" => "⋯",
        "circ" => "∘",
        "degree" => "°",
        "prime" => "′",
        "sum" => "∑",
        "prod" => "∏",
        "int" => "∫",
        "oint" => "∮",
        "langle" => "⟨",
        "rangle" => "⟩",
        "{" => "{",
        "}" => "}",
        "%" => "%",
        "$" => "$",
        "#" => "#",
        "&" => "&",
        "_" => "_",
        "," | ":" | ";" => " ",
        "quad" => "  ",
        "qquad" => "    ",
        "!" => "",
        "sin" | "cos" | "tan" | "log" | "ln" | "exp" | "lim" | "max" | "min" | "det" => {
            return Some(match name {
                "sin" => "sin",
                "cos" => "cos",
                "tan" => "tan",
                "log" => "log",
                "ln" => "ln",
                "exp" => "exp",
                "lim" => "lim",
                "max" => "max",
                "min" => "min",
                _ => "det",
            });
        }
        _ => return None,
    })
}

/// Spaced like `=`.
const RELATIONS: &[&str] = &[
    "≤", "≥", "≠", "≈", "≡", "∼", "∝", "∈", "∉", "⊂", "⊆", "→", "←", "⇒", "⇐", "⇔", "↦", "×", "⋅",
    "±", "÷",
];

/// Operators whose scripts go above and below in display math.
fn takes_limits(text: &str) -> bool {
    matches!(text, "∑" | "∏" | "lim" | "max" | "min")
}

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    depth: usize,
}

impl Parser<'_> {
    fn command(&mut self) -> Result<String, String> {
        let mut name = String::new();
        while let Some(&c) = self.chars.peek() {
            if c.is_ascii_alphabetic() {
                name.push(c);
                self.chars.next();
            } else {
                break;
            }
        }
        if name.is_empty() {
            name = self.chars.next().ok_or("dangling \\")?.to_string();
        }
        Ok(name)
    }

    /// One argument: a braced group or a single token.
    fn argument(&mut self) -> Result<Node, String> {
        while self.chars.peek() == Some(&' ') {
            self.chars.next();
        }
        match self.chars.peek() {
            Some('{') => {
                self.chars.next();
                Ok(Node::Group(self.sequence(Some('}'))?))
            }
            Some(_) => self.atom()?.ok_or_else(|| "missing argument".into()),
            None => Err("missing argument".into()),
        }
    }

    /// Text up to the closing brace of a `\text{…}` argument.
    fn raw_text(&mut self) -> Result<String, String> {
        if self.chars.next() != Some('{') {
            return Err("\\text needs braces".into());
        }
        let mut text = String::new();
        for c in self.chars.by_ref() {
            if c == '}' {
                return Ok(text);
            }
            text.push(c);
        }
        Err("unclosed \\text".into())
    }

    fn atom(&mut self) -> Result<Option<Node>, String> {
        let Some(c) = self.chars.next() else {
            return Ok(None);
        };
        Ok(Some(match c {
            '\\' => {
                let name = self.command()?;
                match name.as_str() {
                    "frac" | "dfrac" | "tfrac" => {
                        let num = self.argument()?;
                        let den = self.argument()?;
                        Node::Frac(Box::new(num), Box::new(den))
                    }
                    "sqrt" => Node::Sqrt(Box::new(self.argument()?)),
                    "text" | "mathrm" | "textrm" | "operatorname" => Node::Text(self.raw_text()?),
                    "mathbf" | "mathit" | "boldsymbol" | "mathbb" | "mathcal" => self.argument()?,
                    "left" | "right" => {
                        let delimiter = self.chars.next().ok_or("missing delimiter")?;
                        let text = match delimiter {
                            '.' => String::new(),
                            '\\' => symbol(&self.command()?)
                                .ok_or("unknown delimiter")?
                                .to_string(),
                            other => other.to_string(),
                        };
                        Node::Text(text)
                    }
                    name => {
                        let text =
                            symbol(name).ok_or_else(|| format!("unsupported command \\{name}"))?;
                        if RELATIONS.contains(&text) {
                            Node::Text(format!(" {text} "))
                        } else {
                            Node::Text(text.to_string())
                        }
                    }
                }
            }
            '{' => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return Err("too deep".into());
                }
                let group = self.sequence(Some('}'))?;
                self.depth -= 1;
                Node::Group(group)
            }
            '}' => return Err("unbalanced }".into()),
            '&' | '$' | '#' | '%' => return Err(format!("unsupported {c}")),
            ' ' | '\n' | '\t' => Node::Text(String::new()),
            '-' => Node::Text("−".into()),
            '=' | '<' | '>' | '+' => Node::Text(format!(" {c} ")),
            c => Node::Text(c.to_string()),
        }))
    }

    fn sequence(&mut self, end: Option<char>) -> Result<Vec<Node>, String> {
        let mut nodes: Vec<Node> = Vec::new();
        loop {
            match self.chars.peek().copied() {
                None if end.is_none() => return Ok(nodes),
                None => return Err("unclosed {".into()),
                Some(c) if Some(c) == end => {
                    self.chars.next();
                    return Ok(nodes);
                }
                Some(c @ ('^' | '_')) => {
                    self.chars.next();
                    let script = Box::new(self.argument()?);
                    let base = nodes.pop().unwrap_or(Node::Text(String::new()));
                    let node = match base {
                        Node::Scripts { base, sub, sup } => match (c, &sub, &sup) {
                            ('^', _, None) => Node::Scripts {
                                base,
                                sub,
                                sup: Some(script),
                            },
                            ('_', None, _) => Node::Scripts {
                                base,
                                sub: Some(script),
                                sup,
                            },
                            _ => return Err("double script".into()),
                        },
                        base => Node::Scripts {
                            base: Box::new(base),
                            sub: (c == '_').then(|| script.clone()),
                            sup: (c == '^').then_some(script),
                        },
                    };
                    nodes.push(node);
                }
                Some(_) => {
                    if let Some(node) = self.atom()? {
                        nodes.push(node);
                    }
                }
            }
        }
    }
}

const SUPERSCRIPT: &[(char, char)] = &[
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
    ('−', '⁻'),
    ('n', 'ⁿ'),
    ('i', 'ⁱ'),
];
const SUBSCRIPT: &[(char, char)] = &[
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
    ('−', '₋'),
    ('i', 'ᵢ'),
    ('j', 'ⱼ'),
    ('n', 'ₙ'),
    ('k', 'ₖ'),
];

/// A script as single-row Unicode, when every character has a script form.
fn small(node: &Node, table: &[(char, char)]) -> Option<String> {
    let text = match node {
        Node::Text(t) => t.trim().to_string(),
        Node::Group(nodes) => nodes
            .iter()
            .map(|n| match n {
                Node::Text(t) => Some(t.trim().to_string()),
                _ => None,
            })
            .collect::<Option<String>>()?,
        _ => return None,
    };
    text.chars()
        .map(|c| table.iter().find(|(a, _)| *a == c).map(|(_, b)| *b))
        .collect()
}

fn layout(node: &Node) -> Block {
    match node {
        Node::Text(text) => Block::text(text),
        Node::Group(nodes) => Block::row(nodes.iter().map(layout).collect()),
        Node::Frac(num, den) => {
            let (num, den) = (layout(num), layout(den));
            let w = num.width().max(den.width()) + 2;
            let rule = Block::text(&"─".repeat(w));
            let baseline = num.rows.len();
            Block::stack(vec![num, rule, den], baseline)
        }
        Node::Sqrt(body) => {
            let body = layout(body);
            let bar = Block::text(&format!(" {}", "─".repeat(body.width())));
            let mut root = Block::row(vec![Block::text("√"), body]);
            root.rows.insert(0, bar.rows[0].clone());
            root.baseline += 1;
            root
        }
        Node::Scripts { base, sub, sup } => {
            let operator = match base.as_ref() {
                Node::Text(t) if takes_limits(t.trim()) => Some(t.trim().to_string()),
                _ => None,
            };
            if let Some(operator) = operator {
                let mut parts = Vec::new();
                if let Some(sup) = sup {
                    parts.push(layout(sup));
                }
                let baseline = parts.iter().map(|p| p.rows.len()).sum();
                parts.push(Block::text(&operator));
                if let Some(sub) = sub {
                    parts.push(layout(sub));
                }
                let stacked = Block::stack(parts, baseline);
                return Block::row(vec![stacked, Block::text(" ")]);
            }
            let base = layout(base);
            let up = sup.as_deref().map(|s| small(s, SUPERSCRIPT));
            let down = sub.as_deref().map(|s| small(s, SUBSCRIPT));
            if up.as_ref().is_none_or(Option::is_some) && down.as_ref().is_none_or(Option::is_some)
            {
                let inline = format!(
                    "{}{}",
                    down.flatten().unwrap_or_default(),
                    up.flatten().unwrap_or_default()
                );
                return Block::row(vec![base, Block::text(&inline)]);
            }
            let mut scripts = Vec::new();
            let mut baseline = 0;
            if let Some(sup) = sup {
                let block = layout(sup);
                baseline = block.rows.len();
                scripts.push(block);
            }
            scripts.push(Block::text(""));
            if let Some(sub) = sub {
                scripts.push(layout(sub));
            }
            let w = scripts.iter().map(Block::width).max().unwrap_or(0);
            let rows = scripts
                .into_iter()
                .flat_map(|b| b.rows.into_iter().map(|r| Block::pad(&r, w)))
                .collect();
            Block::row(vec![base, Block { rows, baseline }])
        }
    }
}

/// The formula's rows, or why it cannot be shown (the block then stays source).
pub fn render(source: &str) -> Result<Vec<String>, String> {
    if source.len() > MAX_SOURCE {
        return Err("source too long".into());
    }
    let source = source.trim();
    if source.is_empty() {
        return Err("empty".into());
    }
    let mut parser = Parser {
        chars: source.chars().peekable(),
        depth: 0,
    };
    let nodes = parser.sequence(None)?;
    let block = layout(&Node::Group(nodes));
    Ok(block
        .rows
        .into_iter()
        .map(|r| r.trim_end().to_string())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_scripts_and_symbols() {
        assert_eq!(render("E = mc^2").unwrap(), vec!["E = mc²"]);
        assert_eq!(render("x_i \\leq \\alpha").unwrap(), vec!["xᵢ ≤ α"]);
    }

    #[test]
    fn fractions_and_roots() {
        assert_eq!(
            render("\\frac{a+b}{2}").unwrap(),
            vec![" a + b", "───────", "   2"]
        );
        assert_eq!(render("\\sqrt{x}").unwrap(), vec![" ─", "√x"]);
    }

    #[test]
    fn limits_over_sums() {
        assert_eq!(
            render("\\sum_{i=1}^{n} i").unwrap(),
            vec!["  n", "  ∑   i", "i = 1"]
        );
    }

    #[test]
    fn unsupported_input_falls_back() {
        assert!(render("\\begin{matrix} a \\end{matrix}").is_err());
        assert!(render("\\frac{a}{").is_err());
    }
}
