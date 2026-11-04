//! A port of `convertHTMLToMarkdown` (html-markdown.ts) and `extractTextFromHTML` (webfetch.ts).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::html::{Attributes, Handler, parse, parse_in_chunks};
use super::js;

pub const MAX_MARKDOWN_BYTES: usize = 5 * 1024 * 1024;
const CONTENT_BYTES: usize = MAX_MARKDOWN_BYTES - 64 * 1024;
/// The converter writes its input to the parser in slices of this many UTF-16 units.
const CHUNK_UNITS: usize = 64 * 1024;
/// Open elements beyond this depth switch the converter to plain text.
const MAX_DEPTH: i64 = 10_000;

const OMITTED: [&str; 9] = [
    "script", "style", "noscript", "iframe", "object", "embed", "meta", "link", "template",
];
const BLOCKS: [&str; 18] = [
    "address",
    "article",
    "aside",
    "details",
    "dialog",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "header",
    "main",
    "nav",
    "p",
    "section",
    "summary",
];
const TEXT_SKIPPED: [&str; 6] = ["script", "style", "noscript", "iframe", "object", "embed"];

/// Extracts the text nodes of `html`, skipping script-like elements, and trims the result.
pub fn to_text(html: &str) -> String {
    struct Extractor {
        text: String,
        skip_depth: usize,
    }
    impl Handler for Extractor {
        fn open(&mut self, name: &str, _attrs: &Attributes) {
            if self.skip_depth > 0 || TEXT_SKIPPED.contains(&name) {
                self.skip_depth += 1;
            }
        }
        fn text(&mut self, text: &str) {
            if self.skip_depth == 0 {
                self.text.push_str(text);
            }
        }
        fn close(&mut self, _name: &str) {
            self.skip_depth = self.skip_depth.saturating_sub(1);
        }
    }
    let mut extractor = Extractor {
        text: String::new(),
        skip_depth: 0,
    };
    parse(html, &mut extractor);
    js::trim(&extractor.text).to_owned()
}

/// Converts `html` to Markdown, capped at [`MAX_MARKDOWN_BYTES`].
pub fn to_markdown(html: &str) -> String {
    let mut converter = Converter::default();
    parse_in_chunks(html, CHUNK_UNITS, &mut converter);
    converter.finish()
}

struct Link {
    href: String,
    title: Option<String>,
}

struct Code {
    inline: bool,
    text: String,
    language: Option<String>,
}

struct Marker {
    // Index of the output chunk holding the opening marker; -1 when nothing was written.
    index: isize,
    block: usize,
    leading_space: Cell<bool>,
    previous: Option<Rc<Marker>>,
}

struct List {
    ordered: bool,
    // A JavaScript number: `start`/`value` may exceed any integer type.
    next: Cell<f64>,
    previous: Option<Rc<List>>,
}

struct Item {
    indent: String,
    previous: Option<Rc<Item>>,
}

#[derive(Default)]
struct Table {
    start: usize,
    rows: Vec<Vec<String>>,
    row: Option<Vec<String>>,
    caption: Option<String>,
    fallback: bool,
    previous: Option<Rc<RefCell<Table>>>,
}

struct Details {
    open: bool,
    summary: Cell<bool>,
    previous: Option<Rc<Details>>,
}

type CodeRef = Rc<RefCell<Code>>;

#[derive(Default)]
struct Frame {
    suppressed: bool,
    link: Option<Rc<Link>>,
    suspended_link: Option<Rc<Link>>,
    marker: Option<Rc<Marker>>,
    code: Option<CodeRef>,
    link_code: Option<CodeRef>,
    resumed_code: Option<CodeRef>,
    list: Option<Rc<List>>,
    item: Option<Rc<Item>>,
    table: Option<Rc<RefCell<Table>>>,
    cell: Option<usize>,
    caption: Option<usize>,
    details: Option<Rc<Details>>,
}

enum Chunk {
    Text(String),
    Raw(String),
}

impl Chunk {
    fn as_str(&self) -> &str {
        match self {
            Self::Text(value) | Self::Raw(value) => value,
        }
    }
}

/// JavaScript object identity for optional references.
fn same<T>(left: &Option<Rc<T>>, right: &Option<Rc<T>>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => Rc::ptr_eq(left, right),
        (None, None) => true,
        _ => false,
    }
}

fn byte_len(value: &str) -> i64 {
    value.len() as i64
}

fn slice_bytes(value: &str, bytes: i64) -> &str {
    let bytes = usize::try_from(bytes).unwrap_or(0);
    if value.len() <= bytes {
        return value;
    }
    let mut end = bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn is_heading(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 2 && bytes[0] == b'h' && (b'1'..=b'6').contains(&bytes[1])
}

fn inline_marker(name: &str) -> Option<&'static str> {
    match name {
        "strong" | "b" => Some("**"),
        "em" | "i" => Some("*"),
        "s" | "strike" | "del" => Some("~~"),
        _ => None,
    }
}

fn escape(part: &str) -> String {
    let mut escaped = String::with_capacity(part.len());
    for character in part.chars() {
        if matches!(
            character,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '|' | '~'
        ) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    if escaped.starts_with(['#', '+', '-']) {
        escaped.insert(0, '\\');
    } else {
        let digits = escaped.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 && escaped[digits..].starts_with('.') {
            escaped.insert(digits, '\\');
        }
    }
    escaped
}

fn destination(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '\\' | '(' | ')') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    js::replace_runs(&escaped, |c| matches!(c, '\t' | '\n' | '\r' | ' '), "%20")
}

fn title(value: Option<&str>) -> String {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return String::new();
    };
    let collapsed = js::replace_runs(value, |c| matches!(c, '\t' | '\n' | '\r' | ' '), " ");
    let mut escaped = String::from(" \"");
    for character in js::trim(&collapsed).chars() {
        if matches!(character, '\\' | '"') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped.push('"');
    escaped
}

fn link_close(link: &Link) -> String {
    format!(
        "]({}{})",
        destination(&link.href),
        title(link.title.as_deref())
    )
}

fn collapse_cell(value: &str) -> String {
    let collapsed = js::replace_runs(value, |c| matches!(c, '\t' | '\r' | '\n' | ' '), " ");
    js::trim(&collapsed).to_owned()
}

fn code_language(class: &str) -> Option<String> {
    let mut from = 0;
    while from < class.len() {
        let rest = &class[from..];
        let found = [rest.find("language-"), rest.find("lang-")];
        let start = found.into_iter().flatten().min()?;
        for prefix in ["language-", "lang-"] {
            if let Some(tail) = rest[start..].strip_prefix(prefix) {
                let end = tail.find(js::is_whitespace).unwrap_or(tail.len());
                if end > 0 {
                    return Some(tail[..end].to_owned());
                }
            }
        }
        from += start + rest[start..].chars().next().map_or(1, char::len_utf8);
    }
    None
}

/// Prefixes every line, the way `value.replace(/^/gm, prefix)` does.
fn prefix_lines(value: &str, prefix: &str) -> String {
    let mut result = String::with_capacity(value.len());
    result.push_str(prefix);
    for character in value.chars() {
        result.push(character);
        if matches!(character, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
            result.push_str(prefix);
        }
    }
    result
}

fn quote_run(line: &str) -> usize {
    let mut length = 0;
    while line[length..].starts_with("> ") {
        length += 2;
    }
    length
}

fn normalize_text(text: &str) -> String {
    let mut spaced = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find([' ', '\t']) {
        spaced.push_str(&rest[..start]);
        let run = rest[start..]
            .find(|c| c != ' ' && c != '\t')
            .map_or(rest.len(), |length| start + length);
        if rest[run..].starts_with('\n') {
            let space = &rest[start..run];
            spaced.push_str(if space.starts_with("  ") {
                "  \n"
            } else {
                "\n"
            });
            rest = &rest[run + 1..];
        } else {
            spaced.push_str(&rest[start..run]);
            rest = &rest[run..];
        }
    }
    spaced.push_str(rest);

    let mut squeezed = String::with_capacity(spaced.len());
    let mut newlines = 0;
    for character in spaced.chars() {
        if character == '\n' {
            newlines += 1;
            continue;
        }
        squeezed.push_str(if newlines >= 3 {
            "\n\n"
        } else {
            &"\n\n"[..newlines]
        });
        newlines = 0;
        squeezed.push(character);
    }
    squeezed.push_str(if newlines >= 3 {
        "\n\n"
    } else {
        &"\n\n"[..newlines]
    });

    let lines: Vec<&str> = squeezed.split('\n').collect();
    let mut result = String::with_capacity(squeezed.len());
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            result.push('\n');
        }
        if !line.is_empty() {
            result.push_str(line);
            continue;
        }
        let before = index
            .checked_sub(1)
            .map_or(0, |previous| quote_run(lines[previous]));
        let after = lines.get(index + 1).map_or(0, |next| quote_run(next));
        if before > 0 && after > 0 && before != after {
            result.push_str("> ".repeat(before.min(after) / 2).trim_end());
        }
    }
    result
}

#[derive(Default)]
struct Converter {
    output: Vec<Chunk>,
    stack: Vec<Frame>,
    pending_space: bool,
    pending_indent: String,
    last: Option<char>,
    quote_depth: i64,
    needs_quote_prefix: bool,
    block_count: usize,
    depth: i64,
    stopped: bool,
    output_bytes: i64,
    active_code: Option<CodeRef>,
    active_link: Option<Rc<Link>>,
    link_open: bool,
    active_marker: Option<Rc<Marker>>,
    active_list: Option<Rc<List>>,
    active_item: Option<Rc<Item>>,
    active_table: Option<Rc<RefCell<Table>>>,
    in_cell: bool,
    table_depth: i64,
    fallback_suppressed_depth: i64,
    fallback_omitted_depth: i64,
    active_details: Option<Rc<Details>>,
}

impl Converter {
    fn append(&mut self, value: &str) {
        self.append_limited(value, MAX_MARKDOWN_BYTES);
    }

    fn append_limited(&mut self, value: &str, limit: usize) {
        let limit = limit as i64;
        if value.is_empty() || self.output_bytes >= limit {
            return;
        }
        let next = slice_bytes(value, limit - self.output_bytes);
        self.output_bytes += byte_len(next);
        if let Some(character) = next.chars().next_back() {
            self.last = Some(character);
        }
        self.output.push(Chunk::Text(next.to_owned()));
    }

    fn append_raw(&mut self, value: &str) {
        let before = self.output.len();
        self.append(value);
        if self.output.len() > before
            && let Some(Chunk::Text(text)) = self.output.pop()
        {
            self.output.push(Chunk::Raw(text));
        }
    }

    fn take(&mut self, start: usize) -> String {
        let start = start.min(self.output.len());
        let value: String = self
            .output
            .drain(start..)
            .map(|chunk| match chunk {
                Chunk::Text(value) | Chunk::Raw(value) => value,
            })
            .collect();
        self.output_bytes -= byte_len(&value);
        value
    }

    fn quote_prefix(&self) -> String {
        "> ".repeat(self.quote_depth.clamp(0, 8) as usize)
    }

    fn prefix_quote(&mut self) {
        if !self.needs_quote_prefix || self.quote_depth == 0 || self.in_cell {
            return;
        }
        let prefix = self.quote_prefix();
        self.append(&prefix);
        self.needs_quote_prefix = false;
    }

    fn flush_space(&mut self) {
        if !self.pending_space {
            return;
        }
        if let Some(marker) = self.active_marker.clone()
            && self.output.len() as isize == marker.index + 1
            && self.last != Some(' ')
            && self.last != Some('\n')
        {
            if let Some(Chunk::Text(value)) = self.output.get_mut(marker.index as usize) {
                value.insert(0, ' ');
            }
            self.output_bytes += 1;
            marker.leading_space.set(true);
            self.pending_space = false;
            return;
        }
        if self.last.is_some_and(|last| last != '\n' && last != ' ') {
            self.append(" ");
        }
        self.pending_space = false;
    }

    fn start_line(&mut self) {
        self.prefix_quote();
        if !self.pending_indent.is_empty() {
            let indent = std::mem::take(&mut self.pending_indent);
            self.append(&indent);
        }
        if self.active_link.is_some() && !self.link_open {
            self.append("[");
            self.link_open = true;
        }
    }

    fn inline(&mut self, value: &str, open: bool) {
        if open {
            self.flush_space();
        }
        self.start_line();
        self.append(value);
    }

    fn close_active_link(&mut self) {
        if let Some(link) = self.active_link.clone() {
            self.append(&link_close(&link));
        }
    }

    fn block(&mut self) {
        if self.active_link.is_some() && self.link_open {
            self.close_active_link();
            self.link_open = false;
        }
        self.pending_space = false;
        self.append("\n\n");
        self.block_count += 1;
        self.needs_quote_prefix = self.quote_depth > 0;
        self.pending_indent = self
            .active_item
            .as_ref()
            .map(|item| item.indent.clone())
            .unwrap_or_default();
    }

    fn suspend_link(&mut self, frame: &mut Frame) {
        if self.active_link.is_none() {
            return;
        }
        frame.suspended_link = self.active_link.clone();
        self.block();
        self.active_link = None;
        self.link_open = false;
    }

    fn text(&mut self, value: &str) {
        if let Some(code) = &self.active_code {
            code.borrow_mut().text.push_str(value);
            return;
        }
        let is_space = |c: char| matches!(c, '\t' | '\n' | '\x0c' | '\r' | ' ');
        let mut rest = value;
        while let Some(first) = rest.chars().next() {
            let space = is_space(first);
            let end = rest.find(|c| is_space(c) != space).unwrap_or(rest.len());
            let part = &rest[..end];
            rest = &rest[end..];
            if space {
                self.pending_space = true;
                continue;
            }
            self.flush_space();
            self.start_line();
            self.append_limited(&escape(part), CONTENT_BYTES);
        }
    }

    fn finish_code(&mut self, code: &CodeRef) {
        let code = code.borrow();
        if code.inline && code.text.is_empty() {
            return;
        }
        let (mut backticks, mut tildes, mut current_backticks, mut current_tildes) = (0, 0, 0, 0);
        for character in code.text.chars() {
            current_backticks = if character == '`' {
                current_backticks + 1
            } else {
                0
            };
            current_tildes = if character == '~' {
                current_tildes + 1
            } else {
                0
            };
            backticks = backticks.max(current_backticks);
            tildes = tildes.max(current_tildes);
        }
        if code.inline {
            let fence = "`".repeat(backticks + 1);
            self.flush_space();
            self.prefix_quote();
            let available =
                (CONTENT_BYTES as i64 - self.output_bytes - 2 * byte_len(&fence)).max(0);
            let mut payload = slice_bytes(&code.text, available);
            while !payload.is_empty() {
                let padded = (payload.starts_with([' ', '`']) || payload.ends_with([' ', '`']))
                    && !payload.bytes().all(|byte| byte == b' ');
                let padding = if padded { " " } else { "" };
                let bytes = byte_len(payload);
                if bytes + 2 * byte_len(padding) <= available {
                    self.append_raw(&format!("{fence}{padding}{payload}{padding}{fence}"));
                    return;
                }
                payload = slice_bytes(payload, bytes - 1);
            }
            return;
        }
        if self.in_cell {
            let text = code.text.clone();
            drop(code);
            self.text(&text);
            return;
        }
        let (marker, count) = if backticks <= tildes {
            ("`", backticks)
        } else {
            ("~", tildes)
        };
        let fence = marker.repeat((count + 1).max(3));
        self.block();
        let prefix = format!("{fence}{}\n", code.language.as_deref().unwrap_or(""));
        let quote = if self.quote_depth > 0 {
            self.quote_prefix()
        } else {
            String::new()
        };
        let mut payload = code.text.as_str();
        loop {
            let newline = if payload.ends_with('\n') { "" } else { "\n" };
            let candidate = format!("{prefix}{payload}{newline}{fence}");
            let value = if quote.is_empty() {
                candidate
            } else {
                prefix_lines(&candidate, &quote)
            };
            let value_bytes = byte_len(&value);
            if self.output_bytes + value_bytes <= CONTENT_BYTES as i64 {
                self.append_raw(&value);
                self.block();
                return;
            }
            // No amount of trimming can make the empty fenced block fit.
            if payload.is_empty() {
                return;
            }
            let excess = value_bytes - (CONTENT_BYTES as i64 - self.output_bytes).max(0);
            payload = slice_bytes(payload, (byte_len(payload) - excess).max(0));
        }
    }

    fn open_tag(&mut self, name: &str, attributes: &Attributes) {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            if self.stack.last().is_some_and(|frame| frame.suppressed) {
                let visible_parent = self
                    .stack
                    .iter()
                    .rposition(|frame| !frame.suppressed)
                    .map_or(-1, |index| index as i64);
                self.fallback_suppressed_depth = visible_parent + 2;
            }
            self.active_code = None;
            self.stopped = true;
        }
        if self.stopped {
            if OMITTED.contains(&name) {
                self.fallback_omitted_depth += 1;
            } else {
                self.pending_space = true;
            }
            return;
        }
        let parent_suppressed = self.stack.last().is_some_and(|frame| frame.suppressed);
        let mut frame = Frame {
            suppressed: parent_suppressed || OMITTED.contains(&name),
            ..Frame::default()
        };
        let hidden = attributes.has("hidden")
            || attributes
                .get("aria-hidden")
                .is_some_and(|value| value.to_lowercase() == "true")
            || name == "head";
        let details = self.active_details.clone();
        if hidden
            || details
                .as_ref()
                .is_some_and(|details| !details.open && !details.summary.get() && name != "summary")
        {
            frame.suppressed = true;
        }
        if !frame.suppressed {
            self.open_visible(name, attributes, &mut frame, details);
        }
        self.stack.push(frame);
    }

    fn open_visible(
        &mut self,
        name: &str,
        attributes: &Attributes,
        frame: &mut Frame,
        details: Option<Rc<Details>>,
    ) {
        if let Some(code) = &self.active_code
            && !code.borrow().inline
        {
            let mut code = code.borrow_mut();
            if name == "br" {
                code.text.push('\n');
            }
            if name == "code"
                && let Some(class) = attributes.get("class").filter(|class| !class.is_empty())
            {
                code.language = code_language(class);
            }
            return;
        }
        if name == "details" {
            let details = Rc::new(Details {
                open: attributes.has("open"),
                summary: Cell::new(false),
                previous: self.active_details.take(),
            });
            self.active_details = Some(details.clone());
            frame.details = Some(details);
            self.block();
            return;
        }
        if name == "summary" {
            if let Some(details) = details {
                details.summary.set(true);
            }
            self.block();
            return;
        }
        if name == "pre" {
            self.suspend_link(frame);
            let code = Rc::new(RefCell::new(Code {
                inline: false,
                text: String::new(),
                language: None,
            }));
            self.active_code = Some(code.clone());
            frame.code = Some(code);
            return;
        }
        if name == "code" {
            if self
                .active_code
                .as_ref()
                .is_some_and(|code| code.borrow().inline)
            {
                return;
            }
            let code = Rc::new(RefCell::new(Code {
                inline: true,
                text: String::new(),
                language: None,
            }));
            self.active_code = Some(code.clone());
            frame.code = Some(code);
            return;
        }
        if is_heading(name) {
            self.block();
            let level = usize::from(name.as_bytes()[1] - b'0');
            self.inline(&format!("{} ", "#".repeat(level)), false);
            return;
        }
        if BLOCKS.contains(&name) {
            self.suspend_link(frame);
            if name == "p" && self.last == Some(' ') {
                return;
            }
            self.block();
            return;
        }
        match name {
            "br" => {
                self.pending_space = false;
                self.inline("  \n", false);
                self.needs_quote_prefix = self.quote_depth > 0;
            }
            "hr" => {
                self.block();
                self.inline("---", false);
                self.block();
            }
            "a" => self.open_link(attributes, frame),
            "img" => {
                let alt = attributes.get("alt").unwrap_or("");
                let mut escaped = String::with_capacity(alt.len());
                for character in alt.chars() {
                    if matches!(character, '\\' | ']') {
                        escaped.push('\\');
                    }
                    escaped.push(character);
                }
                let close = format!(
                    "]({}{})",
                    destination(attributes.get("src").unwrap_or("")),
                    title(attributes.get("title"))
                );
                let open = "![";
                let available =
                    CONTENT_BYTES as i64 - self.output_bytes - byte_len(open) - byte_len(&close);
                let alt = slice_bytes(&escaped, available.max(0));
                self.inline(&format!("{open}{alt}{close}"), true);
            }
            "blockquote" => {
                self.suspend_link(frame);
                self.block();
                self.quote_depth += 1;
                self.needs_quote_prefix = true;
            }
            "ul" | "ol" => {
                self.suspend_link(frame);
                let start = js::parse_int(attributes.get("start").unwrap_or("1")).unwrap_or(1.0);
                let list = Rc::new(List {
                    ordered: name == "ol",
                    next: Cell::new(start),
                    previous: self.active_list.take(),
                });
                self.active_list = Some(list.clone());
                frame.list = Some(list);
                self.block();
            }
            "li" => self.open_item(attributes, frame),
            "table" => {
                self.suspend_link(frame);
                self.table_depth += 1;
                if self.table_depth == 1 {
                    self.block();
                    let table = Rc::new(RefCell::new(Table {
                        start: self.output.len(),
                        previous: self.active_table.take(),
                        ..Table::default()
                    }));
                    self.active_table = Some(table.clone());
                    frame.table = Some(table);
                } else {
                    self.pending_space = true;
                }
            }
            "tr" => {
                if self.table_depth != 1 {
                    self.pending_space = true;
                } else if let Some(table) = &self.active_table {
                    table.borrow_mut().row = Some(Vec::new());
                }
            }
            "th" | "td" => {
                if self.table_depth != 1 {
                    self.pending_space = true;
                    return;
                }
                let spans = [attributes.get("colspan"), attributes.get("rowspan")];
                if spans.iter().flatten().any(|span| !span.is_empty())
                    && let Some(table) = &self.active_table
                {
                    table.borrow_mut().fallback = true;
                }
                frame.cell = Some(self.output.len());
                self.in_cell = true;
            }
            "caption" => frame.caption = Some(self.output.len()),
            "dt" => {
                self.block();
                self.inline("**", false);
            }
            "dd" => self.inline("\n: ", false),
            _ => {
                if let Some(marker) = inline_marker(name) {
                    self.inline(marker, true);
                    let marker = Rc::new(Marker {
                        index: self.output.len() as isize - 1,
                        block: self.block_count,
                        leading_space: Cell::new(false),
                        previous: self.active_marker.take(),
                    });
                    self.active_marker = Some(marker.clone());
                    frame.marker = Some(marker);
                }
            }
        }
    }

    fn new_link(attributes: &Attributes) -> Rc<Link> {
        Rc::new(Link {
            href: attributes.get("href").unwrap_or("").to_owned(),
            title: attributes.get("title").map(str::to_owned),
        })
    }

    fn active_inline_code(&self) -> Option<CodeRef> {
        self.active_code.clone().filter(|code| code.borrow().inline)
    }

    fn open_link(&mut self, attributes: &Attributes, frame: &mut Frame) {
        if self.active_link.is_some()
            && let Some(code) = self.active_inline_code()
        {
            let active_code = Some(code.clone());
            let parent = self.stack.iter().rposition(|candidate| {
                same(&candidate.link, &self.active_link) && same(&candidate.link_code, &active_code)
            });
            if let Some(parent) = parent {
                self.finish_code(&code);
                if self.link_open {
                    self.close_active_link();
                }
                let parent = &mut self.stack[parent];
                self.active_code = parent.resumed_code.clone();
                parent.link = None;
                parent.link_code = None;
                self.active_link = None;
                self.link_open = false;
            }
        }
        if let Some(code) = self.active_inline_code() {
            frame.resumed_code = Some(code.clone());
            if !code.borrow().text.is_empty() {
                self.finish_code(&code);
            }
            code.borrow_mut().text.clear();
            self.active_code = None;
            let link = Self::new_link(attributes);
            self.active_link = Some(link.clone());
            frame.link = Some(link);
            self.link_open = true;
            self.inline("[", true);
            let link_code = Rc::new(RefCell::new(Code {
                inline: true,
                text: String::new(),
                language: None,
            }));
            self.active_code = Some(link_code.clone());
            frame.link_code = Some(link_code);
            return;
        }
        if self.active_link.is_some() {
            if self.link_open {
                self.close_active_link();
            }
            if let Some(parent) = self
                .stack
                .iter_mut()
                .rev()
                .find(|candidate| same(&candidate.link, &self.active_link))
            {
                parent.link = None;
            }
            self.active_link = None;
            self.link_open = false;
        }
        let link = Self::new_link(attributes);
        self.active_link = Some(link.clone());
        frame.link = Some(link);
        self.link_open = true;
        self.inline("[", true);
    }

    fn open_item(&mut self, attributes: &Attributes, frame: &mut Frame) {
        self.block();
        let value = js::parse_int(attributes.get("value").unwrap_or(""));
        let marker = match &self.active_list {
            Some(list) if list.ordered => {
                if let Some(value) = value {
                    list.next.set(value);
                }
                let next = list.next.get();
                list.next.set(next + 1.0);
                format!("{}.", js::number_to_string(next))
            }
            _ => "-".to_owned(),
        };
        let indent = self
            .active_item
            .as_ref()
            .map_or("", |item| &item.indent[..item.indent.len().min(24)]);
        let prefix = format!("{indent}{marker} ");
        let item = Rc::new(Item {
            indent: " ".repeat(prefix.len()),
            previous: self.active_item.take(),
        });
        self.active_item = Some(item.clone());
        frame.item = Some(item);
        self.pending_indent.clear();
        self.inline(&prefix, false);
    }

    fn on_text(&mut self, value: &str) {
        if self.stopped {
            if self.fallback_suppressed_depth == 0 && self.fallback_omitted_depth == 0 {
                self.text(value);
            }
            return;
        }
        if self.stack.last().is_some_and(|frame| frame.suppressed) {
            return;
        }
        self.text(value);
    }

    fn close_tag(&mut self, name: &str) {
        self.depth -= 1;
        if self.stopped {
            if self.fallback_omitted_depth > 0 && OMITTED.contains(&name) {
                self.fallback_omitted_depth -= 1;
            }
            if self.fallback_suppressed_depth > 0 && self.depth < self.fallback_suppressed_depth {
                self.fallback_suppressed_depth = 0;
            }
            return;
        }
        let Some(frame) = self.stack.pop() else {
            return;
        };
        if frame.suppressed {
            return;
        }
        if let Some(link_code) = &frame.link_code {
            let resumed = frame.resumed_code.clone();
            if !same(&self.active_link, &frame.link) || !self.link_open {
                if let Some(resumed) = &resumed {
                    let text = link_code.borrow().text.clone();
                    resumed.borrow_mut().text.push_str(&text);
                }
                self.active_code = resumed;
                if same(&self.active_link, &frame.link) {
                    self.active_link = None;
                }
                self.link_open = false;
                return;
            }
            self.active_code = None;
            self.finish_code(link_code);
            if let Some(link) = &frame.link
                && self.link_open
            {
                self.append(&link_close(link));
            }
            self.active_link = None;
            self.link_open = false;
            self.active_code = resumed;
            return;
        }
        if self
            .active_code
            .as_ref()
            .is_some_and(|code| !code.borrow().inline)
            && frame.code.is_none()
        {
            return;
        }
        if let Some(code) = &frame.code {
            self.active_code = None;
            self.finish_code(code);
            if frame.suspended_link.is_some() {
                self.active_link = frame.suspended_link;
            }
            return;
        }
        match name {
            "summary" => {
                if let Some(details) = &self.active_details {
                    details.summary.set(false);
                }
                self.block();
            }
            "details" => {
                self.active_details = frame.details.and_then(|details| details.previous.clone());
                self.block();
            }
            "dt" => self.inline("**", false),
            "dd" => self.block(),
            "a" => {
                if frame.link.is_some()
                    && (same(&self.active_link, &frame.link) || self.active_link.is_none())
                {
                    if let Some(link) = &frame.link
                        && (self.link_open || self.last.is_some_and(|last| last != '\n'))
                    {
                        self.append(&link_close(link));
                    }
                    self.link_open = false;
                    self.active_link = None;
                }
            }
            "blockquote" => {
                self.quote_depth -= 1;
                self.block();
                self.resume_link(frame);
            }
            "li" => {
                self.active_item = frame.item.and_then(|item| item.previous.clone());
                self.block();
            }
            "ul" | "ol" => {
                self.active_list = frame.list.as_ref().and_then(|list| list.previous.clone());
                self.block();
                self.resume_link(frame);
            }
            "th" | "td" if self.table_depth == 1 => {
                self.in_cell = false;
                if let Some(start) = frame.cell {
                    let value = escape_cell_pipes(&collapse_cell(&self.take(start)));
                    if let Some(table) = &self.active_table
                        && let Some(row) = &mut table.borrow_mut().row
                    {
                        row.push(value);
                    }
                }
            }
            "tr" => {
                if self.table_depth != 1 {
                    return;
                }
                if let Some(table) = &self.active_table {
                    let mut table = table.borrow_mut();
                    if let Some(row) = table.row.take() {
                        table.rows.push(row);
                    }
                }
                self.pending_space = true;
            }
            "caption" if frame.caption.is_some() && self.active_table.is_some() => {
                let caption = collapse_cell(&self.take(frame.caption.unwrap_or_default()));
                if let Some(table) = &self.active_table {
                    table.borrow_mut().caption = Some(caption);
                }
            }
            "table" => self.close_table(frame),
            _ => {
                if let Some(marker) = inline_marker(name) {
                    self.close_marker(marker, frame.marker.as_ref());
                } else if is_heading(name) || BLOCKS.contains(&name) {
                    self.block();
                    self.resume_link(frame);
                }
            }
        }
    }

    fn resume_link(&mut self, frame: Frame) {
        if frame.suspended_link.is_some() {
            self.active_link = frame.suspended_link;
        }
    }

    fn close_marker(&mut self, marker: &str, frame_marker: Option<&Rc<Marker>>) {
        let trailing_space = self.pending_space;
        self.pending_space = false;
        if let Some(frame_marker) = frame_marker {
            self.active_marker = frame_marker.previous.clone();
            if frame_marker.block != self.block_count
                || self.output.len() as isize == frame_marker.index + 1
            {
                if let Some(chunk) = usize::try_from(frame_marker.index)
                    .ok()
                    .and_then(|index| self.output.get_mut(index))
                {
                    *chunk = Chunk::Text(String::new());
                }
                self.pending_space = trailing_space || frame_marker.leading_space.get();
                return;
            }
        }
        self.inline(marker, false);
        self.pending_space =
            trailing_space || frame_marker.is_some_and(|marker| marker.leading_space.get());
    }

    fn close_table(&mut self, frame: Frame) {
        self.table_depth -= 1;
        if self.table_depth != 0 {
            self.pending_space = true;
            return;
        }
        let table = frame.table.clone();
        self.active_table = table
            .as_ref()
            .and_then(|table| table.borrow().previous.clone());
        if let Some(table) = table {
            let table = table.borrow();
            let loose = collapse_cell(&self.take(table.start));
            let width = table.rows.first().map_or(0, Vec::len);
            let rectangular = width > 0 && table.rows.iter().all(|row| row.len() == width);
            if !loose.is_empty() {
                self.append(&loose);
                self.block();
            }
            if let Some(caption) = table
                .caption
                .as_deref()
                .filter(|caption| !caption.is_empty())
            {
                self.append(caption);
                self.block();
            }
            if !table.fallback && rectangular {
                let quote = if self.quote_depth > 0 {
                    self.quote_prefix()
                } else {
                    String::new()
                };
                let prefix = quote + &std::mem::take(&mut self.pending_indent);
                self.append(&format!(
                    "{prefix}| {} |\n{prefix}|{}",
                    table.rows[0].join(" | "),
                    " --- |".repeat(width)
                ));
                for row in &table.rows[1..] {
                    self.append(&format!("\n{prefix}| {} |", row.join(" | ")));
                }
            } else {
                for (index, row) in table.rows.iter().enumerate() {
                    if index > 0 {
                        self.block();
                    }
                    self.append(&row.join(" | "));
                }
            }
        }
        self.block();
        self.resume_link(frame);
    }

    fn finish(self) -> String {
        let mut normalized = String::new();
        let mut pending = String::new();
        for chunk in &self.output {
            match chunk {
                Chunk::Raw(raw) => {
                    normalized.push_str(&normalize_text(&pending));
                    pending.clear();
                    normalized.push_str(raw);
                }
                Chunk::Text(_) => pending.push_str(chunk.as_str()),
            }
        }
        normalized.push_str(&normalize_text(&pending));
        slice_bytes(js::trim(&normalized), MAX_MARKDOWN_BYTES as i64).to_owned()
    }
}

/// `value.replace(/(?<!\\)\|/g, "\\|")`.
fn escape_cell_pipes(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    let mut previous = None;
    for character in value.chars() {
        if character == '|' && previous != Some('\\') {
            escaped.push('\\');
        }
        escaped.push(character);
        previous = Some(character);
    }
    escaped
}

impl Handler for Converter {
    fn open(&mut self, name: &str, attrs: &Attributes) {
        self.open_tag(name, attrs);
    }

    fn text(&mut self, text: &str) {
        self.on_text(text);
    }

    fn close(&mut self, name: &str) {
        self.close_tag(name);
    }
}
