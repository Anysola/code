//! A port of the htmlparser2 8.0.2 tokenizer and parser (HTML mode, default options), limited to
//! the open tag, text and close tag events.
//!
//! Positions are byte offsets into the UTF-8 input. Every decision the tokenizer makes is on
//! ASCII characters, so a non-ASCII character behaves like the single UTF-16 unit it is in
//! JavaScript; only its byte length differs.

use super::entities::{DECODE_TREE, VALUE_LENGTH, determine_branch, replace_code_point};

/// Receives parser events in document order.
pub trait Handler {
    fn open(&mut self, name: &str, attrs: &Attributes);
    fn text(&mut self, text: &str);
    fn close(&mut self, name: &str);
}

/// Attributes of an open tag, in source order; the first occurrence of a name wins.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Attributes(Vec<(String, String)>);

impl Attributes {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    fn insert(&mut self, name: &str, value: String) {
        if !self.has(name) {
            self.0.push((name.to_owned(), value));
        }
    }
}

/// Parses `html` written to htmlparser2 in one piece, then ended.
pub fn parse(html: &str, handler: &mut impl Handler) {
    parse_in_chunks(html, usize::MAX, handler);
}

/// Parses `html` written in slices of `chunk_units` UTF-16 units, then ended. Text events are
/// split at slice boundaries, which the Markdown converter can observe.
pub fn parse_in_chunks(html: &str, chunk_units: usize, handler: &mut impl Handler) {
    let chunks = chunk_ends(html, chunk_units);
    let mut parser = Parser::new(html, &chunks, handler);
    for &end in &chunks {
        parser.write(end);
    }
    parser.finish();
}

/// Byte offsets where each chunk ends. A boundary that would split a surrogate pair is moved
/// before that character, since a Rust string cannot hold half of it.
fn chunk_ends(html: &str, chunk_units: usize) -> Vec<usize> {
    let mut ends = Vec::new();
    let mut units = 0;
    let mut next = chunk_units;
    for (offset, character) in html.char_indices() {
        let after = units + character.len_utf16();
        if after > next {
            ends.push(offset);
            next = next.saturating_add(chunk_units);
        }
        units = after;
        if units == next {
            ends.push(offset + character.len_utf8());
            next = next.saturating_add(chunk_units);
        }
    }
    if ends.last() != Some(&html.len()) && !html.is_empty() {
        ends.push(html.len());
    }
    ends
}

const VOID_ELEMENTS: [&str; 19] = [
    "area", "base", "basefont", "br", "col", "command", "embed", "frame", "hr", "img", "input",
    "isindex", "keygen", "link", "meta", "param", "source", "track", "wbr",
];
const FOREIGN_CONTEXT: [&str; 2] = ["math", "svg"];
const HTML_INTEGRATION: [&str; 9] = [
    "mi",
    "mo",
    "mn",
    "ms",
    "mtext",
    "annotation-xml",
    "foreignobject",
    "desc",
    "title",
];
const FORM_TAGS: &[&str] = &[
    "input", "option", "optgroup", "select", "button", "datalist", "textarea",
];
const P_TAG: &[&str] = &["p"];

fn implied_closes(name: &str) -> &'static [&'static str] {
    match name {
        "tr" => &["tr", "th", "td"],
        "th" => &["th"],
        "td" => &["thead", "th", "td"],
        "body" => &["head", "link", "script"],
        "li" => &["li"],
        "option" => &["option"],
        "optgroup" => &["optgroup", "option"],
        "dd" | "dt" => &["dd", "dt"],
        "rt" | "rp" => &["rt", "rp"],
        "tbody" | "tfoot" => &["thead", "tbody"],
        "select" | "input" | "output" | "button" | "datalist" | "textarea" => FORM_TAGS,
        "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "address" | "article" | "aside"
        | "blockquote" | "details" | "div" | "dl" | "fieldset" | "figcaption" | "figure"
        | "footer" | "form" | "header" | "hr" | "main" | "nav" | "ol" | "pre" | "section"
        | "table" | "ul" => P_TAG,
        _ => &[],
    }
}

fn is_void(name: &str) -> bool {
    VOID_ELEMENTS.contains(&name)
}

fn is_whitespace(c: u8) -> bool {
    matches!(c, b' ' | b'\n' | b'\t' | b'\x0c' | b'\r')
}

fn is_end_of_tag_section(c: u8) -> bool {
    c == b'/' || c == b'>' || is_whitespace(c)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Text,
    BeforeTagName,
    InTagName,
    InSelfClosingTag,
    BeforeClosingTagName,
    InClosingTagName,
    AfterClosingTagName,
    BeforeAttributeName,
    InAttributeName,
    AfterAttributeName,
    BeforeAttributeValue,
    InAttributeValue(u8),
    InAttributeValueNq,
    BeforeDeclaration,
    InDeclaration,
    InProcessingInstruction,
    BeforeComment,
    CdataSequence,
    InSpecialComment,
    InCommentLike,
    BeforeSpecialS,
    SpecialStartSequence,
    InSpecialTag,
    BeforeEntity,
    BeforeNumericEntity,
    InNamedEntity,
    InNumericEntity,
    InHexEntity,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sequence {
    Cdata,
    CdataEnd,
    CommentEnd,
    ScriptEnd,
    StyleEnd,
    TitleEnd,
}

impl Sequence {
    fn bytes(self) -> &'static [u8] {
        match self {
            Self::Cdata => b"CDATA[",
            Self::CdataEnd => b"]]>",
            Self::CommentEnd => b"-->",
            Self::ScriptEnd => b"</script",
            Self::StyleEnd => b"</style",
            Self::TitleEnd => b"</title",
        }
    }
}

struct Parser<'a, H> {
    html: &'a str,
    input: &'a [u8],
    chunks: &'a [usize],
    handler: &'a mut H,
    // Tokenizer state.
    state: State,
    base_state: State,
    end: usize,
    index: usize,
    // htmlparser2 uses -1 for "no section"; see `trailing_text_start`.
    section_start: isize,
    is_special: bool,
    sequence: Sequence,
    sequence_index: usize,
    trie_index: usize,
    trie_current: u16,
    entity_result: u32,
    entity_excess: usize,
    // Parser state.
    first_chunk: usize,
    tag_name: String,
    attrib_name: String,
    attrib_value: String,
    attribs: Option<Attributes>,
    stack: Vec<String>,
    foreign_context: Vec<bool>,
}

impl<'a, H: Handler> Parser<'a, H> {
    fn new(html: &'a str, chunks: &'a [usize], handler: &'a mut H) -> Self {
        Self {
            html,
            input: html.as_bytes(),
            chunks,
            handler,
            state: State::Text,
            base_state: State::Text,
            end: 0,
            index: 0,
            section_start: 0,
            is_special: false,
            sequence: Sequence::Cdata,
            sequence_index: 0,
            trie_index: 0,
            trie_current: 0,
            entity_result: 0,
            entity_excess: 0,
            first_chunk: 0,
            tag_name: String::new(),
            attrib_name: String::new(),
            attrib_value: String::new(),
            attribs: None,
            stack: Vec::new(),
            foreign_context: Vec::new(),
        }
    }

    fn section(&self) -> usize {
        usize::try_from(self.section_start).unwrap_or(0)
    }

    fn set_section(&mut self, position: usize) {
        self.section_start = position as isize;
    }

    fn write(&mut self, end: usize) {
        self.end = end;
        while self.index < self.end {
            let c = self.input[self.index];
            match self.state {
                State::Text => self.state_text(c),
                State::SpecialStartSequence => self.state_special_start_sequence(c),
                State::InSpecialTag => self.state_in_special_tag(c),
                State::CdataSequence => self.state_cdata_sequence(c),
                State::InAttributeValue(quote) => self.state_in_attribute_value(c, quote),
                State::InAttributeName => self.state_in_attribute_name(c),
                State::InCommentLike => self.state_in_comment_like(c),
                State::InSpecialComment => self.state_in_special_comment(c),
                State::BeforeAttributeName => self.state_before_attribute_name(c),
                State::InTagName => self.state_in_tag_name(c),
                State::InClosingTagName => self.state_in_closing_tag_name(c),
                State::BeforeTagName => self.state_before_tag_name(c),
                State::AfterAttributeName => self.state_after_attribute_name(c),
                State::BeforeAttributeValue => self.state_before_attribute_value(c),
                State::BeforeClosingTagName => self.state_before_closing_tag_name(c),
                State::AfterClosingTagName => self.state_after_closing_tag_name(c),
                State::BeforeSpecialS => self.state_before_special_s(c),
                State::InAttributeValueNq => self.state_in_attribute_value_nq(c),
                State::InSelfClosingTag => self.state_in_self_closing_tag(c),
                State::InDeclaration => self.state_in_declaration(c),
                State::BeforeDeclaration => self.state_before_declaration(c),
                State::BeforeComment => self.state_before_comment(c),
                State::InProcessingInstruction => self.state_in_processing_instruction(c),
                State::InNamedEntity => self.state_in_named_entity(c),
                State::BeforeEntity => self.state_before_entity(c),
                State::InHexEntity => self.state_in_numeric_entity(c, 16),
                State::InNumericEntity => self.state_in_numeric_entity(c, 10),
                State::BeforeNumericEntity => self.state_before_numeric_entity(c),
            }
            self.index += 1;
        }
        self.cleanup();
    }

    fn cleanup(&mut self) {
        if self.section_start == self.index as isize {
            return;
        }
        if self.state == State::Text
            || (self.state == State::InSpecialTag && self.sequence_index == 0)
        {
            self.emit_text(self.section(), self.index);
            self.set_section(self.index);
        } else if matches!(
            self.state,
            State::InAttributeValue(_) | State::InAttributeValueNq
        ) {
            self.attrib_data(self.section(), self.index);
            self.set_section(self.index);
        }
    }

    fn finish(&mut self) {
        if self.state == State::InNamedEntity {
            self.emit_named_entity();
        }
        if self.section_start < self.index as isize {
            self.handle_trailing_data();
        }
        for index in (0..self.stack.len()).rev() {
            let name = std::mem::take(&mut self.stack[index]);
            self.handler.close(&name);
        }
    }

    fn handle_trailing_data(&mut self) {
        let end = self.input.len();
        match self.state {
            State::InNumericEntity | State::InHexEntity if self.allow_legacy_entity() => {
                self.emit_numeric_entity(false);
            }
            State::InCommentLike => {
                if self.sequence == Sequence::CdataEnd {
                    self.track_slice(self.section(), end);
                }
            }
            State::InTagName
            | State::BeforeAttributeName
            | State::BeforeAttributeValue
            | State::AfterAttributeName
            | State::InAttributeName
            | State::InAttributeValue(_)
            | State::InAttributeValueNq
            | State::InClosingTagName => {}
            _ if self.section_start < 0 => {
                let start = self.trailing_text_start();
                self.emit_text(start, end);
            }
            _ => self.emit_text(self.section(), end),
        }
    }

    /// htmlparser2 slices its first retained buffer from -1 here (JS `slice` counts a negative
    /// start from the buffer's end): the last character of the first chunk, or the start of a
    /// later retained chunk.
    fn trailing_text_start(&self) -> usize {
        if self.first_chunk > 0 {
            return self.chunks[self.first_chunk - 1];
        }
        let first_end = self.chunks[0];
        self.html[..first_end]
            .char_indices()
            .next_back()
            .map_or(0, |(offset, _)| offset)
    }

    // Mirrors `Parser.getSlice` dropping buffers that end before the requested slice.
    fn track_slice(&mut self, start: usize, end: usize) {
        while self.first_chunk + 1 < self.chunks.len() && start >= self.chunks[self.first_chunk] {
            self.first_chunk += 1;
        }
        while self.first_chunk + 1 < self.chunks.len() && end > self.chunks[self.first_chunk] {
            self.first_chunk += 1;
        }
    }

    fn slice(&mut self, start: usize, end: usize) -> &'a str {
        self.track_slice(start, end);
        &self.html[start..end]
    }

    // Tokenizer states.

    fn state_text(&mut self, c: u8) {
        if c == b'<' {
            if self.index as isize > self.section_start {
                self.emit_text(self.section(), self.index);
            }
            self.state = State::BeforeTagName;
            self.set_section(self.index);
        } else if c == b'&' {
            self.state = State::BeforeEntity;
        }
    }

    fn state_special_start_sequence(&mut self, c: u8) {
        let sequence = self.sequence.bytes();
        let is_end = self.sequence_index == sequence.len();
        let is_match = if is_end {
            is_end_of_tag_section(c)
        } else {
            c | 0x20 == sequence[self.sequence_index]
        };
        if !is_match {
            self.is_special = false;
        } else if !is_end {
            self.sequence_index += 1;
            return;
        }
        self.sequence_index = 0;
        self.state = State::InTagName;
        self.state_in_tag_name(c);
    }

    fn state_in_special_tag(&mut self, c: u8) {
        let sequence = self.sequence.bytes();
        if self.sequence_index == sequence.len() {
            if c == b'>' || is_whitespace(c) {
                let end_of_text = self.index - sequence.len();
                if self.section_start < end_of_text as isize {
                    self.emit_text(self.section(), end_of_text);
                }
                self.is_special = false;
                self.set_section(end_of_text + 2);
                self.state_in_closing_tag_name(c);
                return;
            }
            self.sequence_index = 0;
        }
        if c | 0x20 == sequence[self.sequence_index] {
            self.sequence_index += 1;
        } else if self.sequence_index == 0 {
            if self.sequence == Sequence::TitleEnd {
                if c == b'&' {
                    self.state = State::BeforeEntity;
                }
            } else if self.fast_forward_to(b'<') {
                self.sequence_index = 1;
            }
        } else {
            self.sequence_index = usize::from(c == b'<');
        }
    }

    fn state_cdata_sequence(&mut self, c: u8) {
        let cdata = Sequence::Cdata.bytes();
        if c == cdata[self.sequence_index] {
            self.sequence_index += 1;
            if self.sequence_index == cdata.len() {
                self.state = State::InCommentLike;
                self.sequence = Sequence::CdataEnd;
                self.sequence_index = 0;
                self.set_section(self.index + 1);
            }
        } else {
            self.sequence_index = 0;
            self.state = State::InDeclaration;
            self.state_in_declaration(c);
        }
    }

    fn fast_forward_to(&mut self, target: u8) -> bool {
        self.index += 1;
        while self.index < self.end {
            if self.input[self.index] == target {
                return true;
            }
            self.index += 1;
        }
        self.index = self.end - 1;
        false
    }

    fn state_in_comment_like(&mut self, c: u8) {
        let sequence = self.sequence.bytes();
        if c == sequence[self.sequence_index] {
            self.sequence_index += 1;
            if self.sequence_index == sequence.len() {
                if self.sequence == Sequence::CdataEnd {
                    self.track_slice(self.section(), self.index - 2);
                }
                self.sequence_index = 0;
                self.set_section(self.index + 1);
                self.state = State::Text;
            }
        } else if self.sequence_index == 0 {
            if self.fast_forward_to(sequence[0]) {
                self.sequence_index = 1;
            }
        } else if c != sequence[self.sequence_index - 1] {
            self.sequence_index = 0;
        }
    }

    fn start_special(&mut self, sequence: Sequence, offset: usize) {
        self.is_special = true;
        self.sequence = sequence;
        self.sequence_index = offset;
        self.state = State::SpecialStartSequence;
    }

    fn state_before_tag_name(&mut self, c: u8) {
        if c == b'!' {
            self.state = State::BeforeDeclaration;
            self.set_section(self.index + 1);
        } else if c == b'?' {
            self.state = State::InProcessingInstruction;
            self.set_section(self.index + 1);
        } else if c.is_ascii_alphabetic() {
            let lower = c | 0x20;
            self.set_section(self.index);
            if lower == b't' {
                self.start_special(Sequence::TitleEnd, 3);
            } else if lower == b's' {
                self.state = State::BeforeSpecialS;
            } else {
                self.state = State::InTagName;
            }
        } else if c == b'/' {
            self.state = State::BeforeClosingTagName;
        } else {
            self.state = State::Text;
            self.state_text(c);
        }
    }

    fn state_in_tag_name(&mut self, c: u8) {
        if is_end_of_tag_section(c) {
            self.on_open_tag_name(self.section(), self.index);
            self.section_start = -1;
            self.state = State::BeforeAttributeName;
            self.state_before_attribute_name(c);
        }
    }

    fn state_before_closing_tag_name(&mut self, c: u8) {
        if c == b'>' {
            self.state = State::Text;
        } else if !is_whitespace(c) {
            self.state = if c.is_ascii_alphabetic() {
                State::InClosingTagName
            } else {
                State::InSpecialComment
            };
            self.set_section(self.index);
        }
    }

    fn state_in_closing_tag_name(&mut self, c: u8) {
        if c == b'>' || is_whitespace(c) {
            self.on_close_tag(self.section(), self.index);
            self.section_start = -1;
            self.state = State::AfterClosingTagName;
            self.state_after_closing_tag_name(c);
        }
    }

    fn state_after_closing_tag_name(&mut self, c: u8) {
        if c == b'>' || self.fast_forward_to(b'>') {
            self.state = State::Text;
            self.base_state = State::Text;
            self.set_section(self.index + 1);
        }
    }

    fn state_before_attribute_name(&mut self, c: u8) {
        if c == b'>' {
            self.end_open_tag();
            self.state = if self.is_special {
                self.sequence_index = 0;
                State::InSpecialTag
            } else {
                State::Text
            };
            self.base_state = self.state;
            self.set_section(self.index + 1);
        } else if c == b'/' {
            self.state = State::InSelfClosingTag;
        } else if !is_whitespace(c) {
            self.state = State::InAttributeName;
            self.set_section(self.index);
        }
    }

    fn state_in_self_closing_tag(&mut self, c: u8) {
        if c == b'>' {
            self.on_self_closing_tag();
            self.state = State::Text;
            self.base_state = State::Text;
            self.set_section(self.index + 1);
            self.is_special = false;
        } else if !is_whitespace(c) {
            self.state = State::BeforeAttributeName;
            self.state_before_attribute_name(c);
        }
    }

    fn state_in_attribute_name(&mut self, c: u8) {
        if c == b'=' || is_end_of_tag_section(c) {
            let name = self.slice(self.section(), self.index);
            self.attrib_name = name.to_lowercase();
            self.section_start = -1;
            self.state = State::AfterAttributeName;
            self.state_after_attribute_name(c);
        }
    }

    fn state_after_attribute_name(&mut self, c: u8) {
        if c == b'=' {
            self.state = State::BeforeAttributeValue;
        } else if c == b'/' || c == b'>' {
            self.attrib_end();
            self.state = State::BeforeAttributeName;
            self.state_before_attribute_name(c);
        } else if !is_whitespace(c) {
            self.attrib_end();
            self.state = State::InAttributeName;
            self.set_section(self.index);
        }
    }

    fn state_before_attribute_value(&mut self, c: u8) {
        if c == b'"' || c == b'\'' {
            self.state = State::InAttributeValue(c);
            self.set_section(self.index + 1);
        } else if !is_whitespace(c) {
            self.set_section(self.index);
            self.state = State::InAttributeValueNq;
            self.state_in_attribute_value_nq(c);
        }
    }

    fn state_in_attribute_value(&mut self, c: u8, quote: u8) {
        if c == quote {
            self.attrib_data(self.section(), self.index);
            self.section_start = -1;
            self.attrib_end();
            self.state = State::BeforeAttributeName;
        } else if c == b'&' {
            self.base_state = self.state;
            self.state = State::BeforeEntity;
        }
    }

    fn state_in_attribute_value_nq(&mut self, c: u8) {
        if is_whitespace(c) || c == b'>' {
            self.attrib_data(self.section(), self.index);
            self.section_start = -1;
            self.attrib_end();
            self.state = State::BeforeAttributeName;
            self.state_before_attribute_name(c);
        } else if c == b'&' {
            self.base_state = self.state;
            self.state = State::BeforeEntity;
        }
    }

    fn state_before_declaration(&mut self, c: u8) {
        if c == b'[' {
            self.state = State::CdataSequence;
            self.sequence_index = 0;
        } else {
            self.state = if c == b'-' {
                State::BeforeComment
            } else {
                State::InDeclaration
            };
        }
    }

    fn state_in_declaration(&mut self, c: u8) {
        if c == b'>' || self.fast_forward_to(b'>') {
            self.state = State::Text;
            self.set_section(self.index + 1);
        }
    }

    fn state_in_processing_instruction(&mut self, c: u8) {
        self.state_in_declaration(c);
    }

    fn state_before_comment(&mut self, c: u8) {
        if c == b'-' {
            self.state = State::InCommentLike;
            self.sequence = Sequence::CommentEnd;
            // Allows short comments such as `<!-->`.
            self.sequence_index = 2;
            self.set_section(self.index + 1);
        } else {
            self.state = State::InDeclaration;
        }
    }

    fn state_in_special_comment(&mut self, c: u8) {
        self.state_in_declaration(c);
    }

    fn state_before_special_s(&mut self, c: u8) {
        let lower = c | 0x20;
        if lower == b'c' {
            self.start_special(Sequence::ScriptEnd, 4);
        } else if lower == b't' {
            self.start_special(Sequence::StyleEnd, 4);
        } else {
            self.state = State::InTagName;
            self.state_in_tag_name(c);
        }
    }

    fn state_before_entity(&mut self, c: u8) {
        self.entity_excess = 1;
        self.entity_result = 0;
        if c == b'#' {
            self.state = State::BeforeNumericEntity;
        } else if c != b'&' {
            self.trie_index = 0;
            self.trie_current = DECODE_TREE[0];
            self.state = State::InNamedEntity;
            self.state_in_named_entity(c);
        }
    }

    fn state_in_named_entity(&mut self, c: u8) {
        self.entity_excess += 1;
        let Some(trie_index) = determine_branch(self.trie_current, self.trie_index + 1, c) else {
            self.emit_named_entity();
            self.index -= 1;
            return;
        };
        self.trie_index = trie_index;
        self.trie_current = DECODE_TREE[trie_index];
        let masked = self.trie_current & VALUE_LENGTH;
        if masked != 0 {
            let value_length = usize::from(masked >> 14) - 1;
            if !self.allow_legacy_entity() && c != b';' {
                self.trie_index += value_length;
            } else {
                let entity_start = self.index + 1 - self.entity_excess;
                if entity_start as isize > self.section_start {
                    self.emit_partial(self.section(), entity_start);
                }
                self.entity_result = trie_index as u32;
                self.trie_index += value_length;
                self.entity_excess = 0;
                self.set_section(self.index + 1);
                if value_length == 0 {
                    self.emit_named_entity();
                }
            }
        }
    }

    fn emit_named_entity(&mut self) {
        self.state = self.base_state;
        if self.entity_result == 0 {
            return;
        }
        let result = self.entity_result as usize;
        let node = DECODE_TREE[result];
        match (node & VALUE_LENGTH) >> 14 {
            1 => self.emit_code_point(u32::from(node & !VALUE_LENGTH)),
            2 => self.emit_code_point(u32::from(DECODE_TREE[result + 1])),
            3 => {
                let units = [DECODE_TREE[result + 1], DECODE_TREE[result + 2]];
                // A surrogate pair arrives as two events in JavaScript; Rust keeps it whole.
                match char::decode_utf16(units).collect::<Result<Vec<_>, _>>() {
                    Ok(chars) if chars.len() == 1 => self.emit_char(chars[0]),
                    _ => {
                        for unit in units {
                            self.emit_code_point(u32::from(unit));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn state_before_numeric_entity(&mut self, c: u8) {
        if c | 0x20 == b'x' {
            self.entity_excess += 1;
            self.state = State::InHexEntity;
        } else {
            self.state = State::InNumericEntity;
            self.state_in_numeric_entity(c, 10);
        }
    }

    fn emit_numeric_entity(&mut self, strict: bool) {
        let entity_start = self.index - self.entity_excess - 1;
        let number_start = entity_start + 2 + usize::from(self.state == State::InHexEntity);
        if number_start != self.index {
            if entity_start as isize > self.section_start {
                self.emit_partial(self.section(), entity_start);
            }
            self.set_section(self.index + usize::from(strict));
            self.emit_char(replace_code_point(self.entity_result));
        }
        self.state = self.base_state;
    }

    fn state_in_numeric_entity(&mut self, c: u8, radix: u32) {
        if c == b';' {
            self.emit_numeric_entity(true);
        } else if let Some(digit) = char::from(c).to_digit(radix) {
            self.entity_result = self
                .entity_result
                .saturating_mul(radix)
                .saturating_add(digit);
            self.entity_excess += 1;
        } else {
            if self.allow_legacy_entity() {
                self.emit_numeric_entity(false);
            } else {
                self.state = self.base_state;
            }
            self.index -= 1;
        }
    }

    fn allow_legacy_entity(&self) -> bool {
        matches!(self.base_state, State::Text | State::InSpecialTag)
    }

    fn emit_partial(&mut self, start: usize, end: usize) {
        if self.allow_legacy_entity() {
            self.emit_text(start, end);
        } else {
            self.attrib_data(start, end);
        }
    }

    fn emit_code_point(&mut self, code_point: u32) {
        self.emit_char(char::from_u32(code_point).unwrap_or('\u{fffd}'));
    }

    fn emit_char(&mut self, character: char) {
        if self.allow_legacy_entity() {
            self.handler.text(character.encode_utf8(&mut [0; 4]));
        } else {
            self.attrib_value.push(character);
        }
    }

    // Parser callbacks.

    fn emit_text(&mut self, start: usize, end: usize) {
        let text = self.slice(start, end);
        self.handler.text(text);
    }

    fn attrib_data(&mut self, start: usize, end: usize) {
        let data = self.slice(start, end);
        self.attrib_value.push_str(data);
    }

    fn attrib_end(&mut self) {
        let value = std::mem::take(&mut self.attrib_value);
        if let Some(attribs) = &mut self.attribs {
            attribs.insert(&self.attrib_name, value);
        }
    }

    fn on_open_tag_name(&mut self, start: usize, end: usize) {
        let name = self.slice(start, end).to_lowercase();
        self.emit_open_tag(name);
    }

    fn emit_open_tag(&mut self, name: String) {
        let implied = implied_closes(&name);
        while let Some(top) = self.stack.last() {
            if !implied.contains(&top.as_str()) {
                break;
            }
            let element = self.stack.pop().unwrap_or_default();
            self.handler.close(&element);
        }
        if !is_void(&name) {
            self.stack.push(name.clone());
            if FOREIGN_CONTEXT.contains(&name.as_str()) {
                self.foreign_context.push(true);
            } else if HTML_INTEGRATION.contains(&name.as_str()) {
                self.foreign_context.push(false);
            }
        }
        self.tag_name = name;
        self.attribs = Some(Attributes::default());
    }

    fn end_open_tag(&mut self) {
        if let Some(attribs) = self.attribs.take() {
            self.handler.open(&self.tag_name, &attribs);
        }
        if is_void(&self.tag_name) {
            self.handler.close(&self.tag_name);
        }
        self.tag_name.clear();
    }

    fn on_close_tag(&mut self, start: usize, end: usize) {
        let name = self.slice(start, end).to_lowercase();
        if FOREIGN_CONTEXT.contains(&name.as_str()) || HTML_INTEGRATION.contains(&name.as_str()) {
            self.foreign_context.pop();
        }
        if !is_void(&name) {
            if let Some(position) = self.stack.iter().rposition(|open| *open == name) {
                for element in self.stack.split_off(position).into_iter().rev() {
                    self.handler.close(&element);
                }
            } else if name == "p" {
                self.emit_open_tag(name);
                self.close_current_tag();
            }
        } else if name == "br" {
            self.handler.open("br", &Attributes::default());
            self.handler.close("br");
        }
    }

    fn on_self_closing_tag(&mut self) {
        if self.foreign_context.last() == Some(&true) {
            self.close_current_tag();
        } else {
            self.end_open_tag();
        }
    }

    fn close_current_tag(&mut self) {
        let name = self.tag_name.clone();
        self.end_open_tag();
        if self.stack.last() == Some(&name) {
            self.handler.close(&name);
            self.stack.pop();
        }
    }
}
