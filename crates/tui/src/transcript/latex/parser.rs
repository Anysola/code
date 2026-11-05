//! LaTeX math to a node tree, after `@opencode/latex`'s `parser.ts` in strict mode: an unknown
//! command or environment is an error, so the block stays source.

use super::symbols::{DELIMITERS, LARGE, NAMED, SPACES, SYMBOLS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Ordinary,
    Binary,
    Relation,
    Operator,
    Punctuation,
    Opening,
    Closing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Normal,
    Bold,
    Italic,
    Sans,
    Monospace,
    DoubleStruck,
    Script,
    Fraktur,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accent {
    Hat,
    WideHat,
    Bar,
    Overline,
    Underline,
    Vec,
    Tilde,
    Dot,
    Ddot,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Row(Vec<Node>),
    Symbol(String, Option<Role>),
    Text(String),
    Space(usize),
    Fraction {
        numerator: Box<Node>,
        denominator: Box<Node>,
        bar: bool,
        /// `\cfrac[l]`/`[r]`: `Some(true)` left, `Some(false)` right.
        align: Option<bool>,
    },
    Root(Box<Node>, Option<Box<Node>>),
    Scripts {
        base: Box<Node>,
        sup: Option<Box<Node>>,
        sub: Option<Box<Node>>,
    },
    Delimited(String, Box<Node>, String),
    Matrix {
        rows: Vec<Vec<Node>>,
        environment: String,
        columns: Option<String>,
    },
    Brace(Box<Node>, bool),
    Accent(Accent, Box<Node>),
    Variant(Variant, Box<Node>),
    Operator(String, bool),
    OverUnder {
        base: Box<Node>,
        over: Option<Box<Node>>,
        under: Option<Box<Node>>,
    },
}

const ENVIRONMENTS: [&str; 13] = [
    "matrix",
    "pmatrix",
    "bmatrix",
    "Bmatrix",
    "vmatrix",
    "Vmatrix",
    "cases",
    "aligned",
    "align",
    "gathered",
    "gather",
    "smallmatrix",
    "array",
];

const MAX_DEPTH: usize = 256;

fn accent(name: &str) -> Option<Accent> {
    Some(match name {
        "hat" => Accent::Hat,
        "widehat" => Accent::WideHat,
        "bar" => Accent::Bar,
        "overline" => Accent::Overline,
        "underline" => Accent::Underline,
        "vec" => Accent::Vec,
        "tilde" | "widetilde" => Accent::Tilde,
        "dot" => Accent::Dot,
        "ddot" => Accent::Ddot,
        _ => return None,
    })
}

fn variant(name: &str) -> Option<Variant> {
    Some(match name {
        "mathrm" | "textrm" | "mathnormal" => Variant::Normal,
        "mathbf" | "boldsymbol" | "bm" => Variant::Bold,
        "mathit" => Variant::Italic,
        "mathsf" => Variant::Sans,
        "mathtt" => Variant::Monospace,
        "mathbb" => Variant::DoubleStruck,
        "mathcal" | "mathscr" => Variant::Script,
        "mathfrak" => Variant::Fraktur,
        _ => return None,
    })
}

fn delimiter(token: &str) -> Option<&'static str> {
    DELIMITERS
        .iter()
        .find(|(k, _)| *k == token)
        .map(|(_, v)| *v)
}

fn spacing(name: &str) -> Option<usize> {
    SPACES.iter().find(|(k, _)| *k == name).map(|(_, v)| *v)
}

fn infer_role(c: char) -> Role {
    match c {
        '+' | '-' | '*' | '/' | '×' | '÷' | '±' | '∓' => Role::Binary,
        '=' | '<' | '>' | '≤' | '≥' | '≠' | '≈' | '∈' | '∉' | '⊂' | '⊃' => {
            Role::Relation
        }
        ',' | ';' | ':' => Role::Punctuation,
        '(' | '[' | '{' => Role::Opening,
        ')' | ']' | '}' => Role::Closing,
        _ => Role::Ordinary,
    }
}

fn negate(value: &str) -> String {
    let pairs = [
        ("=", "≠"),
        ("∈", "∉"),
        ("∋", "∌"),
        ("≡", "≢"),
        ("≈", "≉"),
        ("∼", "≁"),
        ("<", "≮"),
        (">", "≯"),
        ("≤", "≰"),
        ("≥", "≱"),
        ("⊂", "⊄"),
        ("⊃", "⊅"),
        ("⊆", "⊈"),
        ("⊇", "⊉"),
        ("∣", "∤"),
        ("∥", "∦"),
    ];
    pairs
        .iter()
        .find(|(a, _)| *a == value)
        .map_or_else(|| format!("{value}\u{338}"), |(_, b)| b.to_string())
}

fn row(mut body: Vec<Node>) -> Node {
    if body.len() == 1 {
        body.remove(0)
    } else {
        Node::Row(body)
    }
}

pub struct Parser<'a> {
    source: &'a str,
    position: usize,
    depth: usize,
}

type Result<T> = std::result::Result<T, String>;

impl<'a> Parser<'a> {
    pub fn new(source: &'a str) -> Parser<'a> {
        Parser {
            source,
            position: 0,
            depth: 0,
        }
    }

    pub fn parse(mut self) -> Result<Node> {
        let body = self.row(&|_| false)?;
        self.skip_space();
        if !self.done() {
            return self.fail(&format!("Unexpected \"{}\"", self.peek_char()));
        }
        Ok(row(body))
    }

    fn rest(&self) -> &'a str {
        &self.source[self.position..]
    }

    fn peek_char(&self) -> char {
        self.rest().chars().next().unwrap_or('\0')
    }

    fn done(&self) -> bool {
        self.position >= self.source.len()
    }

    fn fail<T>(&self, message: &str) -> Result<T> {
        Err(format!("{message} at offset {}", self.position))
    }

    fn bump(&mut self) -> char {
        let c = self.peek_char();
        self.position += c.len_utf8();
        c
    }

    fn expect(&mut self, value: &str) -> Result<()> {
        if !self.rest().starts_with(value) {
            return self.fail(&format!("Expected \"{value}\""));
        }
        self.position += value.len();
        Ok(())
    }

    fn skip_space(&mut self) {
        while !self.done() {
            let c = self.peek_char();
            if c.is_whitespace() {
                self.bump();
            } else if c == '%' {
                while !self.done() && self.peek_char() != '\n' {
                    self.bump();
                }
            } else {
                break;
            }
        }
    }

    fn row(&mut self, stop: &dyn Fn(&Parser) -> bool) -> Result<Vec<Node>> {
        let mut body: Vec<Node> = Vec::new();
        while !self.done() {
            self.skip_space();
            if self.done() || stop(self) {
                break;
            }
            let current = self.peek_char();
            if current == '}' {
                break;
            }
            if current == '^' || current == '_' {
                self.bump();
                let script = Box::new(self.argument()?);
                let previous = body.pop().unwrap_or(Node::Row(Vec::new()));
                let (base, mut sup, mut sub) = match previous {
                    Node::Scripts { base, sup, sub } => (base, sup, sub),
                    other => (Box::new(other), None, None),
                };
                if current == '^' {
                    sup = Some(script);
                } else {
                    sub = Some(script);
                }
                body.push(Node::Scripts { base, sup, sub });
                continue;
            }
            if current == '\\' && self.limits_modifier(&mut body) {
                continue;
            }
            body.push(self.atom()?);
        }
        Ok(body)
    }

    fn atom(&mut self) -> Result<Node> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return self.fail("LaTeX nesting exceeds the 256-level limit");
        }
        let node = self.atom_inner();
        self.depth -= 1;
        node
    }

    fn atom_inner(&mut self) -> Result<Node> {
        match self.peek_char() {
            '{' => self.group(),
            '\\' => self.command(),
            '~' => {
                self.bump();
                Ok(Node::Space(1))
            }
            _ => {
                let c = self.bump();
                Ok(Node::Symbol(c.to_string(), Some(infer_role(c))))
            }
        }
    }

    fn read_command(&mut self) -> Result<String> {
        self.expect("\\")?;
        if self.done() {
            return Ok("\\".into());
        }
        let next = self.peek_char();
        if !(next.is_ascii_alphabetic() || next == '@') {
            self.bump();
            return Ok(next.to_string());
        }
        let start = self.position;
        while !self.done() && (self.peek_char().is_ascii_alphabetic() || self.peek_char() == '@') {
            self.bump();
        }
        let name = self.source[start..self.position].to_string();
        if self.peek_char() == ' ' {
            self.bump();
        }
        Ok(name)
    }

    fn command(&mut self) -> Result<Node> {
        let command = self.read_command()?;
        let name = command.as_str();
        match name {
            "\\" => return Ok(Node::Row(Vec::new())),
            "begin" => return self.environment(),
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                self.skip_space();
                let mut align = None;
                if name == "cfrac" && self.peek_char() == '[' {
                    let rest = self.rest();
                    align = match () {
                        _ if rest.starts_with("[l]") => Some(Some(true)),
                        _ if rest.starts_with("[r]") => Some(Some(false)),
                        _ if rest.starts_with("[]") => Some(None),
                        _ => {
                            return self
                                .fail("Unsupported \\cfrac alignment; expected [l], [r], or []");
                        }
                    };
                    self.position += if rest.starts_with("[]") { 2 } else { 3 };
                }
                return Ok(Node::Fraction {
                    numerator: Box::new(self.argument()?),
                    denominator: Box::new(self.argument()?),
                    bar: true,
                    align: align.flatten(),
                });
            }
            "binom" | "dbinom" | "tbinom" => {
                let fraction = Node::Fraction {
                    numerator: Box::new(self.argument()?),
                    denominator: Box::new(self.argument()?),
                    bar: false,
                    align: None,
                };
                return Ok(Node::Delimited("(".into(), Box::new(fraction), ")".into()));
            }
            "sqrt" => {
                let index = self.optional_argument()?;
                return Ok(Node::Root(Box::new(self.argument()?), index.map(Box::new)));
            }
            "left" => return self.left_right(),
            "middle" => return Ok(Node::Symbol(self.read_delimiter()?, None)),
            "right" => return self.fail("Unexpected \\right"),
            "text" | "mbox" => return Ok(Node::Text(self.text_group()?)),
            "operatorname" => {
                let limits = self.peek_char() == '*';
                if limits {
                    self.bump();
                }
                return Ok(Node::Operator(self.text_group()?, limits));
            }
            "overset" | "stackrel" | "underset" => {
                let mark = Some(Box::new(self.argument()?));
                let base = Box::new(self.argument()?);
                let (over, under) = if name == "underset" {
                    (None, mark)
                } else {
                    (mark, None)
                };
                return Ok(Node::OverUnder { base, over, under });
            }
            "overbrace" | "underbrace" => {
                return Ok(Node::Brace(Box::new(self.argument()?), name == "overbrace"));
            }
            "textcolor" => {
                self.raw_group()?;
                return self.argument();
            }
            "color" => {
                self.raw_group()?;
                return Ok(row(self.row(&|_| false)?));
            }
            "not" => {
                return Ok(match self.atom()? {
                    Node::Symbol(value, role) => Node::Symbol(negate(&value), role),
                    target => Node::Row(vec![Node::Symbol("¬".into(), None), target]),
                });
            }
            "pmod" => {
                let argument = self.argument()?;
                return Ok(Node::Row(vec![
                    Node::Space(1),
                    Node::Text("(mod ".into()),
                    argument,
                    Node::Text(")".into()),
                ]));
            }
            "mod" | "bmod" => return Ok(Node::Operator("mod".into(), false)),
            "displaylines" => {
                self.skip_space();
                self.expect("{")?;
                return self.matrix("gathered", "}", None);
            }
            "limits" | "nolimits" | "displaystyle" | "textstyle" | "scriptstyle"
            | "scriptscriptstyle" => return Ok(Node::Row(Vec::new())),
            "backslash" => return Ok(Node::Symbol("\\".into(), None)),
            "{" | "}" | "%" | "#" | "$" | "&" | "_" => return Ok(Node::Symbol(command, None)),
            _ => {}
        }
        if let Some(kind) = accent(name) {
            return Ok(Node::Accent(kind, Box::new(self.argument()?)));
        }
        if let Some(v) = variant(name) {
            let body = if name == "textrm" {
                Node::Text(self.text_group()?)
            } else {
                self.argument()?
            };
            return Ok(Node::Variant(v, Box::new(body)));
        }
        let sized = ["big", "Big", "bigg", "Bigg"].iter().any(|b| {
            name.strip_prefix(b)
                .is_some_and(|r| matches!(r, "" | "l" | "r" | "m"))
        });
        if sized {
            return Ok(Node::Symbol(self.read_delimiter()?, None));
        }
        if let Some(width) = spacing(name) {
            return Ok(Node::Space(width));
        }
        if let Some((_, value, role)) = SYMBOLS.iter().find(|(k, _, _)| *k == name) {
            return Ok(Node::Symbol(value.to_string(), Some(*role)));
        }
        if let Some((_, value)) = LARGE.iter().find(|(k, _)| *k == name) {
            return Ok(Node::Operator(value.to_string(), !name.contains("int")));
        }
        if NAMED.contains(&name) {
            let limits = name.starts_with("lim") || name == "min" || name == "max";
            return Ok(Node::Operator(command.clone(), limits));
        }
        if let Some(d) = delimiter(&format!("\\{name}")).or_else(|| delimiter(name)) {
            return Ok(Node::Symbol(d.to_string(), None));
        }
        self.fail(&format!("Unsupported command \\{name}"))
    }

    fn environment(&mut self) -> Result<Node> {
        let raw = self.raw_group()?;
        let name = raw.strip_suffix('*').unwrap_or(&raw);
        if !ENVIRONMENTS.contains(&name) {
            return self.fail(&format!("Unsupported environment {name}"));
        }
        let columns = if name == "array" {
            let columns: String = self
                .raw_group()?
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            let valid = columns.chars().all(|c| "lcr|".contains(c))
                && columns.chars().any(|c| "lcr".contains(c));
            if !valid {
                return self.fail("Unsupported array columns; expected l, c, r, and |");
            }
            Some(columns)
        } else {
            None
        };
        self.matrix(name, &format!("\\end{{{raw}}}"), columns)
    }

    fn matrix(&mut self, environment: &str, end: &str, columns: Option<String>) -> Result<Node> {
        let mut rows: Vec<Vec<Node>> = Vec::new();
        let mut cells: Vec<Node> = Vec::new();
        let end_owned = end.to_string();
        while !self.done() {
            self.skip_space();
            if self.rest().starts_with(end) && cells.is_empty() {
                break;
            }
            let start = self.position;
            let stop = |p: &Parser| {
                p.peek_char() == '&'
                    || p.rest().starts_with("\\\\")
                    || p.rest().starts_with(&end_owned)
            };
            cells.push(row(self.row(&stop)?));
            self.skip_space();
            if self.peek_char() == '&' {
                self.bump();
                continue;
            }
            if self.rest().starts_with("\\\\") {
                self.position += 2;
                self.optional_bracket();
                rows.push(std::mem::take(&mut cells));
                continue;
            }
            if self.rest().starts_with(end) {
                break;
            }
            if self.position == start {
                return self.fail(&format!(
                    "Unexpected \"{}\" in {environment}",
                    self.peek_char()
                ));
            }
        }
        if !self.rest().starts_with(end) {
            return self.fail(&format!("Missing {end}"));
        }
        self.expect(end)?;
        if !cells.is_empty() || rows.is_empty() {
            rows.push(cells);
        }
        Ok(Node::Matrix {
            rows,
            environment: environment.to_string(),
            columns,
        })
    }

    fn left_right(&mut self) -> Result<Node> {
        let left = self.read_delimiter()?;
        let at_right = |p: &Parser| {
            p.rest().starts_with("\\right")
                && !p.rest()[6..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '@')
        };
        let body = row(self.row(&at_right)?);
        if !at_right(self) {
            return self.fail("Missing \\right");
        }
        self.read_command()?;
        let right = self.read_delimiter()?;
        Ok(Node::Delimited(left, Box::new(body), right))
    }

    fn argument(&mut self) -> Result<Node> {
        self.skip_space();
        if self.peek_char() == '{' {
            return self.group();
        }
        if self.done() {
            return self.fail("Expected an argument");
        }
        self.atom()
    }

    fn group(&mut self) -> Result<Node> {
        self.expect("{")?;
        let body = row(self.row(&|_| false)?);
        self.expect("}")?;
        Ok(body)
    }

    fn optional_argument(&mut self) -> Result<Option<Node>> {
        self.skip_space();
        if self.peek_char() != '[' {
            return Ok(None);
        }
        self.bump();
        let body = row(self.row(&|p| p.peek_char() == ']')?);
        self.expect("]")?;
        Ok(Some(body))
    }

    fn optional_bracket(&mut self) {
        self.skip_space();
        if self.peek_char() != '[' {
            return;
        }
        let mut depth = 0;
        while !self.done() {
            match self.bump() {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    fn read_delimiter(&mut self) -> Result<String> {
        self.skip_space();
        if self.done() {
            return self.fail("Expected a delimiter");
        }
        if self.peek_char() == '\\' {
            let command = self.read_command()?;
            return match delimiter(&format!("\\{command}")).or_else(|| delimiter(&command)) {
                Some(d) => Ok(d.to_string()),
                None => self.fail(&format!("Unsupported delimiter \\{command}")),
            };
        }
        let token = self.bump().to_string();
        Ok(delimiter(&token).map_or(token, str::to_string))
    }

    fn raw_group(&mut self) -> Result<String> {
        self.skip_space();
        self.expect("{")?;
        let start = self.position;
        let mut depth = 1;
        while !self.done() {
            let at = self.position;
            let c = self.bump();
            let escaped = matches!(c, '{' | '}')
                && self.source[..at]
                    .chars()
                    .rev()
                    .take_while(|c| *c == '\\')
                    .count()
                    % 2
                    == 1;
            if c == '{' && !escaped {
                depth += 1;
            }
            if c == '}' && !escaped {
                depth -= 1;
                if depth == 0 {
                    return Ok(self.source[start..at].to_string());
                }
            }
        }
        self.position = start;
        self.fail("Unterminated group")
    }

    fn limits_modifier(&mut self, body: &mut [Node]) -> bool {
        let rest = self.rest();
        let Some(name) = ["\\limits", "\\nolimits"].into_iter().find(|m| {
            rest.starts_with(m)
                && !rest[m.len()..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '@')
        }) else {
            return false;
        };
        self.position += name.len();
        let on = name == "\\limits";
        match body.last_mut() {
            Some(Node::Operator(_, limits)) => *limits = on,
            Some(Node::Scripts { base, .. }) => {
                if let Node::Operator(_, limits) = base.as_mut() {
                    *limits = on;
                }
            }
            _ => {}
        }
        true
    }

    /// A `\text{…}` argument with its escapes and spacing commands resolved.
    fn text_group(&mut self) -> Result<String> {
        let raw = self.raw_group()?;
        let mut out = String::new();
        let mut chars = raw.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '~' => out.push(' '),
                '\\' => {
                    let mut name = String::new();
                    while let Some(&n) = chars.peek() {
                        if n.is_ascii_alphabetic() || n == '@' {
                            name.push(n);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    if name.is_empty()
                        && let Some(n) = chars.next()
                    {
                        name.push(n);
                    }
                    match name.as_str() {
                        n if n.len() == 1 && "{}%#$&_ ".contains(n) => out.push_str(n),
                        "textbackslash" => out.push('\\'),
                        "!" => {}
                        n => match spacing(n) {
                            Some(w) => out.push_str(&" ".repeat(w.max(1))),
                            None => {
                                out.push('\\');
                                out.push_str(n);
                            }
                        },
                    }
                }
                c => out.push(c),
            }
        }
        Ok(out)
    }
}
