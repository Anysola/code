//! Editing one property of a JSONC document in place. Setting works the way `jsonc-parser`'s
//! `modify` + `applyEdits` do (formatting 2 spaces, comments and the rest of the file kept), so
//! `mcp add` writes the same bytes 0.5.1 wrote. Removing cuts the property, its comma and a line
//! it leaves empty, and touches nothing else.

use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    OpenBrace,
    CloseBrace,
    OpenBracket,
    CloseBracket,
    Comma,
    Colon,
    Null,
    True,
    False,
    String,
    Number,
    LineComment,
    BlockComment,
    LineBreak,
    Trivia,
    Unknown,
    Eof,
}

fn is_eol(byte: u8) -> bool {
    byte == b'\n' || byte == b'\r'
}

fn is_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | 0x0b | 0x0c)
}

struct Scanner<'a> {
    text: &'a [u8],
    pos: usize,
    offset: usize,
    error: bool,
    /// Decoded value of the last string token.
    value: String,
}

impl<'a> Scanner<'a> {
    fn new(text: &'a str) -> Self {
        Scanner {
            text: text.as_bytes(),
            pos: 0,
            offset: 0,
            error: false,
            value: String::new(),
        }
    }

    fn at(&self, index: usize) -> u8 {
        self.text.get(index).copied().unwrap_or(0)
    }

    fn length(&self) -> usize {
        self.pos - self.offset
    }

    fn digits(&mut self) -> bool {
        let start = self.pos;
        while self.at(self.pos).is_ascii_digit() {
            self.pos += 1;
        }
        self.pos > start
    }

    fn number(&mut self) {
        if self.at(self.pos) == b'0' {
            self.pos += 1;
        } else {
            self.pos += 1;
            self.digits();
        }
        if self.at(self.pos) == b'.' {
            self.pos += 1;
            if !self.digits() {
                self.error = true;
                return;
            }
        }
        if matches!(self.at(self.pos), b'e' | b'E') {
            self.pos += 1;
            if matches!(self.at(self.pos), b'+' | b'-') {
                self.pos += 1;
            }
            if !self.digits() {
                self.error = true;
            }
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let digits = self.text.get(self.pos..self.pos + 4)?;
        let value = u32::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()?;
        self.pos += 4;
        Some(value)
    }

    fn string(&mut self) {
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&byte) = self.text.get(self.pos) else {
                self.error = true;
                break;
            };
            match byte {
                b'"' => {
                    self.pos += 1;
                    break;
                }
                b'\\' => {
                    self.pos += 1;
                    let Some(&escape) = self.text.get(self.pos) else {
                        self.error = true;
                        break;
                    };
                    self.pos += 1;
                    match escape {
                        b'"' | b'\\' | b'/' => out.push(escape),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => match self.hex4() {
                            Some(code) => {
                                let ch = char::from_u32(code).unwrap_or('\u{fffd}');
                                out.extend_from_slice(ch.to_string().as_bytes());
                            }
                            None => self.error = true,
                        },
                        _ => self.error = true,
                    }
                }
                b'\n' | b'\r' => {
                    self.error = true;
                    break;
                }
                control if control < 0x20 => {
                    self.error = true;
                    self.pos += 1;
                    out.push(control);
                }
                other => {
                    self.pos += 1;
                    out.push(other);
                }
            }
        }
        self.value = String::from_utf8_lossy(&out).into_owned();
    }

    fn scan(&mut self) -> Kind {
        self.error = false;
        self.offset = self.pos;
        let Some(&byte) = self.text.get(self.pos) else {
            return Kind::Eof;
        };
        if is_whitespace(byte) {
            while is_whitespace(self.at(self.pos)) && self.pos < self.text.len() {
                self.pos += 1;
            }
            return Kind::Trivia;
        }
        if is_eol(byte) {
            self.pos += 1;
            if byte == b'\r' && self.at(self.pos) == b'\n' {
                self.pos += 1;
            }
            return Kind::LineBreak;
        }
        self.pos += 1;
        match byte {
            b'{' => Kind::OpenBrace,
            b'}' => Kind::CloseBrace,
            b'[' => Kind::OpenBracket,
            b']' => Kind::CloseBracket,
            b':' => Kind::Colon,
            b',' => Kind::Comma,
            b'"' => {
                self.string();
                Kind::String
            }
            b'/' if self.at(self.pos) == b'/' => {
                while self.pos < self.text.len() && !is_eol(self.at(self.pos)) {
                    self.pos += 1;
                }
                Kind::LineComment
            }
            b'/' if self.at(self.pos) == b'*' => {
                self.pos += 1;
                loop {
                    if self.pos + 1 >= self.text.len() {
                        self.pos = (self.pos + 1).min(self.text.len());
                        self.error = true;
                        break;
                    }
                    if self.at(self.pos) == b'*' && self.at(self.pos + 1) == b'/' {
                        self.pos += 2;
                        break;
                    }
                    self.pos += 1;
                }
                Kind::BlockComment
            }
            b'-' if !self.at(self.pos).is_ascii_digit() => Kind::Unknown,
            b'-' => {
                self.number();
                Kind::Number
            }
            b'0'..=b'9' => {
                self.pos -= 1;
                self.number();
                Kind::Number
            }
            _ => {
                let content = |b: u8| {
                    !(is_whitespace(b)
                        || is_eol(b)
                        || matches!(b, b'}' | b']' | b'{' | b'[' | b'"' | b':' | b',' | b'/'))
                };
                if content(byte) {
                    while self.pos < self.text.len() && content(self.at(self.pos)) {
                        self.pos += 1;
                    }
                    match &self.text[self.offset..self.pos] {
                        b"true" => Kind::True,
                        b"false" => Kind::False,
                        b"null" => Kind::Null,
                        _ => Kind::Unknown,
                    }
                } else {
                    Kind::Unknown
                }
            }
        }
    }

    fn scan_significant(&mut self) -> Kind {
        loop {
            let kind = self.scan();
            if !matches!(
                kind,
                Kind::LineComment | Kind::BlockComment | Kind::LineBreak | Kind::Trivia
            ) {
                return kind;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeType {
    Object,
    Array,
    Property,
    String,
    Number,
    Boolean,
    Null,
}

impl NodeType {
    fn name(self) -> &'static str {
        match self {
            NodeType::Object => "object",
            NodeType::Array => "array",
            NodeType::Property => "property",
            NodeType::String => "string",
            NodeType::Number => "number",
            NodeType::Boolean => "boolean",
            NodeType::Null => "null",
        }
    }
}

#[derive(Debug)]
struct Node {
    kind: NodeType,
    offset: usize,
    length: usize,
    children: Vec<usize>,
    value: String,
}

/// `parseTree`: nodes in an arena, the root first.
struct Tree {
    nodes: Vec<Node>,
}

impl Tree {
    fn parse(text: &str) -> Tree {
        let mut tree = Tree { nodes: Vec::new() };
        let mut scanner = Scanner::new(text);
        let token = scanner.scan_significant();
        tree.value(&mut scanner, token);
        tree
    }

    fn push(&mut self, kind: NodeType, offset: usize, length: usize) -> usize {
        self.nodes.push(Node {
            kind,
            offset,
            length,
            children: Vec::new(),
            value: String::new(),
        });
        self.nodes.len() - 1
    }

    /// Parses the value starting at `token`; the scanner is left on its last token.
    fn value(&mut self, scanner: &mut Scanner, token: Kind) -> Option<usize> {
        let leaf = match token {
            Kind::String => NodeType::String,
            Kind::Number => NodeType::Number,
            Kind::True | Kind::False => NodeType::Boolean,
            Kind::Null => NodeType::Null,
            Kind::OpenBrace => return Some(self.container(scanner, NodeType::Object)),
            Kind::OpenBracket => return Some(self.container(scanner, NodeType::Array)),
            _ => return None,
        };
        let node = self.push(leaf, scanner.offset, scanner.length());
        if leaf == NodeType::String {
            self.nodes[node].value = scanner.value.clone();
        }
        Some(node)
    }

    fn container(&mut self, scanner: &mut Scanner, kind: NodeType) -> usize {
        let node = self.push(kind, scanner.offset, 0);
        let close = if kind == NodeType::Object {
            Kind::CloseBrace
        } else {
            Kind::CloseBracket
        };
        let mut token = scanner.scan_significant();
        loop {
            if token == close || token == Kind::Eof {
                break;
            }
            let child = if kind == NodeType::Object {
                if token != Kind::String {
                    break;
                }
                let property = self.push(NodeType::Property, scanner.offset, 0);
                let key = self.push(NodeType::String, scanner.offset, scanner.length());
                self.nodes[key].value = scanner.value.clone();
                self.nodes[property].children.push(key);
                if scanner.scan_significant() != Kind::Colon {
                    break;
                }
                let next = scanner.scan_significant();
                let Some(value) = self.value(scanner, next) else {
                    break;
                };
                self.nodes[property].children.push(value);
                let end = self.nodes[value].offset + self.nodes[value].length;
                self.nodes[property].length = end - self.nodes[property].offset;
                property
            } else {
                match self.value(scanner, token) {
                    Some(value) => value,
                    None => break,
                }
            };
            self.nodes[node].children.push(child);
            token = scanner.scan_significant();
            if token == Kind::Comma {
                token = scanner.scan_significant();
            } else if token != close {
                break;
            }
        }
        let end = if token == close {
            scanner.pos
        } else {
            scanner.text.len()
        };
        self.nodes[node].length = end - self.nodes[node].offset;
        node
    }

    /// `findNodeAtLocation`, except that of duplicate keys the last one is found: it is the one
    /// a parser keeps.
    fn find(&self, root: usize, path: &[&str]) -> Option<usize> {
        let mut node = root;
        for segment in path {
            let current = &self.nodes[node];
            if current.kind != NodeType::Object {
                return None;
            }
            node = current.children.iter().rev().find_map(|&property| {
                let children = &self.nodes[property].children;
                (children.len() == 2 && self.nodes[children[0]].value == *segment)
                    .then(|| children[1])
            })?;
        }
        Some(node)
    }
}

struct Edit {
    offset: usize,
    length: usize,
    content: String,
}

fn apply(text: &str, edit: &Edit) -> String {
    format!(
        "{}{}{}",
        &text[..edit.offset],
        edit.content,
        &text[edit.offset + edit.length..]
    )
}

const INDENT: &str = "  ";
const BOM: char = '\u{feff}';

fn indent_level(text: &str) -> usize {
    let columns: usize = text
        .bytes()
        .take_while(|b| *b == b' ' || *b == b'\t')
        .map(|b| if b == b'\t' { INDENT.len() } else { 1 })
        .sum();
    columns / INDENT.len()
}

fn eol_of(text: &str) -> &'static str {
    let bytes = text.as_bytes();
    match bytes.iter().position(|b| is_eol(*b)) {
        Some(index) if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') => "\r\n",
        Some(index) if bytes[index] == b'\r' => "\r",
        _ => "\n",
    }
}

/// `format(document, range, {tabSize: 2, insertSpaces: true, keepLines: false})`.
fn format(document: &str, range_start: usize, range_end: usize) -> Vec<Edit> {
    let bytes = document.as_bytes();
    let mut start = range_start;
    while start > 0 && !is_eol(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = range_end;
    while end < bytes.len() && !is_eol(bytes[end]) {
        end += 1;
    }
    let text = &document[start..end];
    let initial = indent_level(text);
    let eol = eol_of(document);
    // Unbalanced input can take the level below zero, as in jsonc-parser.
    let mut level: isize = 0;
    let mut scanner = Scanner::new(text);
    let mut line_breaks = false;
    let mut error = false;
    let mut edits = Vec::new();

    let breaks = |level: isize| {
        let count = (initial as isize + level).max(0) as usize;
        format!("{eol}{}", INDENT.repeat(count))
    };
    let next = |scanner: &mut Scanner, line_breaks: &mut bool, error: &mut bool| {
        let mut token = scanner.scan();
        *line_breaks = false;
        while matches!(token, Kind::Trivia | Kind::LineBreak) {
            if token == Kind::LineBreak {
                *line_breaks = true;
            }
            token = scanner.scan();
        }
        *error = token == Kind::Unknown || scanner.error;
        token
    };
    let add = |edits: &mut Vec<Edit>, error: bool, content: String, from: usize, to: usize| {
        if !error && from < range_end && to > range_start && document[from..to] != content {
            edits.push(Edit {
                offset: from,
                length: to - from,
                content,
            });
        }
    };

    let mut first = next(&mut scanner, &mut line_breaks, &mut error);
    if first != Kind::Eof {
        let token_start = scanner.offset + start;
        add(
            &mut edits,
            error,
            INDENT.repeat(initial),
            start,
            token_start,
        );
    }
    while first != Kind::Eof {
        let mut first_end = scanner.offset + scanner.length() + start;
        let mut second = next(&mut scanner, &mut line_breaks, &mut error);
        let mut replace = String::new();
        let mut needs_break = false;
        while !line_breaks && matches!(second, Kind::LineComment | Kind::BlockComment) {
            let comment_start = scanner.offset + start;
            add(&mut edits, error, " ".into(), first_end, comment_start);
            first_end = scanner.offset + scanner.length() + start;
            needs_break = second == Kind::LineComment;
            replace = if needs_break {
                breaks(level)
            } else {
                String::new()
            };
            second = next(&mut scanner, &mut line_breaks, &mut error);
        }
        if second == Kind::CloseBrace || second == Kind::CloseBracket {
            let open = if second == Kind::CloseBrace {
                Kind::OpenBrace
            } else {
                Kind::OpenBracket
            };
            if first != open {
                level -= 1;
                replace = breaks(level);
            }
        } else {
            match first {
                Kind::OpenBracket | Kind::OpenBrace => {
                    level += 1;
                    replace = breaks(level);
                }
                Kind::Comma | Kind::LineComment => replace = breaks(level),
                Kind::BlockComment => {
                    if line_breaks {
                        replace = breaks(level);
                    } else if !needs_break {
                        replace = " ".into();
                    }
                }
                Kind::Colon => {
                    if !needs_break {
                        replace = " ".into();
                    }
                }
                Kind::String => {
                    if second == Kind::Colon && !needs_break {
                        replace = String::new();
                    }
                }
                Kind::Null
                | Kind::True
                | Kind::False
                | Kind::Number
                | Kind::CloseBrace
                | Kind::CloseBracket => {
                    if matches!(second, Kind::LineComment | Kind::BlockComment) && !needs_break {
                        replace = " ".into();
                    } else if second != Kind::Comma && second != Kind::Eof {
                        error = true;
                    }
                }
                Kind::Unknown => error = true,
                _ => {}
            }
            if line_breaks && matches!(second, Kind::LineComment | Kind::BlockComment) {
                replace = breaks(level);
            }
        }
        if second == Kind::Eof {
            replace = String::new();
        }
        let second_start = scanner.offset + start;
        add(&mut edits, error, replace, first_end, second_start);
        first = second;
    }
    edits
}

/// `withFormatting`: applies `edit`, then formats the touched lines.
fn with_formatting(text: &str, edit: Edit) -> String {
    let inserting = edit.length == 0 || edit.content.is_empty();
    let mut begin = edit.offset;
    let mut end = edit.offset + edit.content.len();
    let mut out = apply(text, &edit);
    let bytes = out.as_bytes();
    if inserting {
        while begin > 0 && !is_eol(bytes[begin - 1]) {
            begin -= 1;
        }
        while end < bytes.len() && !is_eol(bytes[end]) {
            end += 1;
        }
    }
    for edit in format(&out, begin, end).iter().rev() {
        out = apply(&out, edit);
    }
    out
}

/// Sets `path` (object keys) to `value` in `text`, as `applyEdits(text, modify(text, path, value,
/// {formattingOptions: {tabSize: 2, insertSpaces: true}}))`.
pub fn set(text: &str, path: &[&str], value: Value) -> Result<String, String> {
    // A byte order mark is kept in front and not edited (the scanner would take it for a token).
    if let Some(rest) = text.strip_prefix(BOM) {
        return set(rest, path, value).map(|out| format!("{BOM}{out}"));
    }
    let tree = Tree::parse(text);
    let root = (!tree.nodes.is_empty()).then_some(0);
    let mut value = value;
    let mut depth = path.len();
    let mut found = None;
    while depth > 0 {
        depth -= 1;
        found = root.and_then(|root| tree.find(root, &path[..depth]));
        if found.is_some() {
            break;
        }
        value = json!({ path[depth]: value });
    }
    let content = |v: &Value| crate::json::compact(v);
    let Some(parent) = found else {
        let (offset, length) =
            root.map_or((0, 0), |r| (tree.nodes[r].offset, tree.nodes[r].length));
        return Ok(with_formatting(
            text,
            Edit {
                offset,
                length,
                content: content(&value),
            },
        ));
    };
    let key = path[depth];
    let node = &tree.nodes[parent];
    if node.kind != NodeType::Object {
        return Err(format!(
            "Can not add index to parent of type {}",
            node.kind.name()
        ));
    }
    if let Some(existing) = tree.find(parent, &[key]) {
        let existing = &tree.nodes[existing];
        return Ok(with_formatting(
            text,
            Edit {
                offset: existing.offset,
                length: existing.length,
                content: content(&value),
            },
        ));
    }
    let property = format!("{}: {}", content(&json!(key)), content(&value));
    let edit = match node.children.last() {
        Some(&previous) => Edit {
            offset: tree.nodes[previous].offset + tree.nodes[previous].length,
            length: 0,
            content: format!(",{property}"),
        },
        None => Edit {
            offset: node.offset + 1,
            length: 0,
            content: property,
        },
    };
    Ok(with_formatting(text, edit))
}

/// The first significant token at or after `from`, with its offset.
fn next_token(text: &str, from: usize) -> (Kind, usize) {
    let mut scanner = Scanner::new(text);
    scanner.pos = from;
    let kind = scanner.scan_significant();
    (kind, scanner.offset)
}

/// Removes `path` (object keys) from `text`: the property, the comma that separated it from its
/// neighbour, and its line when nothing else was on it. An object the removal leaves empty goes
/// too, up to (not including) the root. A missing key leaves the text as it was.
pub fn remove(text: &str, path: &[&str]) -> String {
    if let Some(rest) = text.strip_prefix(BOM) {
        return format!("{BOM}{}", remove(rest, path));
    }
    let mut out = text.to_string();
    for depth in (1..=path.len()).rev() {
        let Some(next) = remove_one(&out, &path[..depth]) else {
            break;
        };
        out = next;
        if depth == 1 {
            break;
        }
        // The parent goes too when it is now `{}` with nothing inside, comments included.
        let tree = Tree::parse(&out);
        let Some(parent) = (!tree.nodes.is_empty())
            .then(|| tree.find(0, &path[..depth - 1]))
            .flatten()
        else {
            break;
        };
        let node = &tree.nodes[parent];
        let empty = node.kind == NodeType::Object
            && node.children.is_empty()
            && node.length >= 2
            && out[node.offset + 1..node.offset + node.length - 1]
                .trim()
                .is_empty();
        if !empty {
            break;
        }
    }
    out
}

fn remove_one(text: &str, path: &[&str]) -> Option<String> {
    let tree = Tree::parse(text);
    if tree.nodes.is_empty() {
        return None;
    }
    let (key, parents) = path.split_last()?;
    let parent = &tree.nodes[tree.find(0, parents)?];
    if parent.kind != NodeType::Object {
        return None;
    }
    // The last of duplicate keys, the one a parser keeps.
    let index = parent.children.iter().rposition(|&property| {
        let children = &tree.nodes[property].children;
        children.len() == 2 && tree.nodes[children[0]].value == *key
    })?;
    let property = &tree.nodes[parent.children[index]];
    let mut start = property.offset;
    let mut end = property.offset + property.length;
    // The comma after the property, else (the last one) the comma before it.
    let mut comma_before = None;
    match next_token(text, end) {
        (Kind::Comma, at) => end = at + 1,
        _ if index > 0 => {
            let previous = &tree.nodes[parent.children[index - 1]];
            if let (Kind::Comma, at) = next_token(text, previous.offset + previous.length) {
                comma_before = Some(at);
            }
        }
        _ => {}
    }
    let line_start = text[..start].rfind(['\n', '\r']).map_or(0, |i| i + 1);
    let line_end = text[end..].find(['\n', '\r']).map(|i| end + i);
    // A comment after it on its line is about it, and goes with it.
    let alone = text[line_start..start].trim().is_empty()
        && line_end.is_some_and(|line_end| {
            let rest = text[end..line_end].trim();
            rest.is_empty() || rest.starts_with("//")
        });
    match line_end {
        Some(line_end) if alone => {
            start = line_start;
            end = line_end
                + if text[line_end..].starts_with("\r\n") {
                    2
                } else {
                    1
                };
        }
        _ => {
            // On a shared line: no gap left behind, and a comma just before goes with it.
            end += text[end..]
                .bytes()
                .take_while(|b| *b == b' ' || *b == b'\t')
                .count();
            if let Some(at) = comma_before
                && text[at + 1..start].trim().is_empty()
            {
                start = at;
                comma_before = None;
            }
        }
    }
    let mut out = format!("{}{}", &text[..start], &text[end..]);
    if let Some(at) = comma_before {
        out.remove(at);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Value {
        super::super::parse(text).unwrap_or_else(|e| panic!("{e}: {text}"))
    }

    #[test]
    fn set_adds_and_replaces_keeping_comments() {
        let text = "{\n  // servers\n  \"mcp\": {}\n}\n";
        let added = set(text, &["mcp", "servers", "x"], json!({"type": "remote"})).unwrap();
        assert!(added.contains("// servers"));
        assert_eq!(
            parsed(&added),
            json!({"mcp": {"servers": {"x": {"type": "remote"}}}})
        );
        let replaced = set(&added, &["mcp", "servers", "x"], json!(1)).unwrap();
        assert_eq!(parsed(&replaced), json!({"mcp": {"servers": {"x": 1}}}));
        assert_eq!(
            set("", &["model"], json!("anysola/m")).unwrap(),
            "{\n  \"model\": \"anysola/m\"\n}"
        );
    }

    #[test]
    fn remove_a_line_of_its_own() {
        let text =
            "{\n  // the model\n  \"model\": \"anysola/m\",\n  \"shell\": \"zsh\" // mine\n}\n";
        assert_eq!(
            remove(text, &["model"]),
            "{\n  // the model\n  \"shell\": \"zsh\" // mine\n}\n"
        );
        assert_eq!(
            remove(text, &["shell"]),
            "{\n  // the model\n  \"model\": \"anysola/m\"\n}\n"
        );
    }

    #[test]
    fn remove_with_trailing_commas() {
        assert_eq!(remove("{\"a\": 1,}", &["a"]), "{}");
        assert_eq!(remove("{\n  \"a\": 1,\n}\n", &["a"]), "{\n}\n");
        let last = "{\n  \"a\": 1,\n  \"b\": 2,\n}\n";
        let out = remove(last, &["b"]);
        assert_eq!(out, "{\n  \"a\": 1,\n}\n");
        assert_eq!(parsed(&out), json!({"a": 1}));
    }

    #[test]
    fn remove_the_last_key_after_a_commented_line() {
        let text = "{\n  \"a\": 1, // note\n  \"b\": 2,\n}\n";
        let out = remove(text, &["b"]);
        assert_eq!(out, "{\n  \"a\": 1, // note\n}\n");
        assert_eq!(parsed(&out), json!({"a": 1}));
        let text = "{\n  \"a\": 1, // note\n  \"b\": 2\n}\n";
        assert_eq!(parsed(&remove(text, &["b"])), json!({"a": 1}));
    }

    #[test]
    fn duplicate_keys_edit_the_one_that_counts() {
        let text = "{\n  \"a\": 1,\n  \"a\": 2\n}\n";
        let set_out = set(text, &["a"], json!(3)).unwrap();
        assert_eq!(set_out, "{\n  \"a\": 1,\n  \"a\": 3\n}\n");
        assert_eq!(parsed(&set_out), json!({"a": 3}));
        let removed = remove(text, &["a"]);
        assert_eq!(removed, "{\n  \"a\": 1\n}\n");
        let nested = "{\"m\": {\"x\": 1}, \"m\": {\"y\": 2}}";
        assert_eq!(
            parsed(&set(nested, &["m", "z"], json!(3)).unwrap()),
            json!({"m": {"y": 2, "z": 3}})
        );
    }

    #[test]
    fn remove_on_one_line() {
        assert_eq!(remove("{\"a\": 1, \"b\": 2}", &["a"]), "{\"b\": 2}");
        assert_eq!(remove("{\"a\": 1, \"b\": 2}", &["b"]), "{\"a\": 1}");
        assert_eq!(
            remove("{\"a\": 1, \"b\": 2, \"c\": 3}", &["b"]),
            "{\"a\": 1, \"c\": 3}"
        );
    }

    #[test]
    fn remove_nested_drops_a_parent_left_empty() {
        let text = "{\n  \"experimental\": {\n    \"todo\": true\n  },\n  \"x\": 1\n}\n";
        let out = remove(text, &["experimental", "todo"]);
        assert_eq!(out, "{\n  \"x\": 1\n}\n");
        let kept = "{\n  \"experimental\": {\n    \"todo\": true,\n    \"other\": 1\n  }\n}\n";
        assert_eq!(
            parsed(&remove(kept, &["experimental", "todo"])),
            json!({"experimental": {"other": 1}})
        );
        // A comment inside keeps the parent.
        let commented = "{\n  \"experimental\": {\n    // why\n    \"todo\": true\n  }\n}\n";
        assert_eq!(
            remove(commented, &["experimental", "todo"]),
            "{\n  \"experimental\": {\n    // why\n  }\n}\n"
        );
    }

    #[test]
    fn remove_missing_keys_changes_nothing() {
        let text = "{\n  \"a\": 1 // c\n}";
        assert_eq!(remove(text, &["b"]), text);
        assert_eq!(remove(text, &["a", "deeper"]), text);
        assert_eq!(remove("", &["a"]), "");
    }

    #[test]
    fn a_byte_order_mark_stays_in_front() {
        let text = "\u{feff}{\n  \"a\": 1\n}\n";
        let added = set(text, &["b"], json!(2)).unwrap();
        assert!(added.starts_with('\u{feff}') && added.matches('{').count() == 1);
        assert_eq!(parsed(&added), json!({"a": 1, "b": 2}));
        let removed = remove(&added, &["a"]);
        assert!(removed.starts_with('\u{feff}'));
        assert_eq!(parsed(&removed), json!({"b": 2}));
    }

    #[test]
    fn remove_keeps_crlf() {
        let text = "{\r\n  \"a\": 1,\r\n  \"b\": 2\r\n}\r\n";
        assert_eq!(remove(text, &["a"]), "{\r\n  \"b\": 2\r\n}\r\n");
        assert_eq!(remove(text, &["b"]), "{\r\n  \"a\": 1\r\n}\r\n");
    }
}
