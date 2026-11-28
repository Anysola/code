//! A tool call's input while the model writes it. The core sends the input JSON in pieces
//! (`Event::ToolInputDelta`); this reads the string fields known so far out of the unfinished
//! object and sends the window a small snapshot of each call that grew, at most every tick.
//! Display only: nothing waits on it.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

/// A top-level string field of the input, as far as it has arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub key: String,
    /// Decoded; only whole characters, so it is always a prefix of the final value.
    pub value: String,
    /// The closing quote arrived.
    pub done: bool,
    newlines: usize,
}

impl Field {
    /// Lines of text so far (a last line without its newline counts).
    pub fn lines(&self) -> usize {
        self.newlines + usize::from(!self.value.is_empty() && !self.value.ends_with('\n'))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Key,
    Value(usize),
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Escape {
    None,
    /// After `\`.
    Backslash,
    /// Inside `\uXXXX`: the digits so far; `high` is a high surrogate this one would complete.
    Unicode {
        digits: u8,
        code: u32,
        high: Option<u32>,
    },
    /// A high surrogate waits for `\u` and its low half.
    High(u32),
    /// A high surrogate, then `\`.
    HighBackslash(u32),
}

/// Reads an unfinished JSON object fed in pieces of bytes, cut anywhere (inside an escape, a
/// `\uXXXX`, a surrogate pair or a UTF-8 sequence): what it holds back is never shown half.
#[derive(Debug, Default)]
pub struct Scan {
    depth: u32,
    /// Somewhere other than inside the top-level object (the input is not an object, or after it).
    off: bool,
    /// At depth 1, the next string is a key.
    expect_key: bool,
    string: Option<Target>,
    escape: Option<Escape>,
    key: String,
    utf8: Vec<u8>,
    pub fields: Vec<Field>,
}

impl Scan {
    pub fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.byte(b);
        }
    }

    pub fn field(&self, key: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.key == key)
    }

    fn byte(&mut self, b: u8) {
        if self.off {
            return;
        }
        let Some(target) = self.string else {
            self.structure(b);
            return;
        };
        let escape = self.escape.unwrap_or(Escape::None);
        match escape {
            Escape::None => match b {
                b'"' => self.end_string(target),
                b'\\' => self.escape = Some(Escape::Backslash),
                _ => self.raw(target, b),
            },
            Escape::Backslash => {
                self.escape = Some(Escape::None);
                let c = match b {
                    b'n' => '\n',
                    b't' => '\t',
                    b'r' => '\r',
                    b'b' => '\u{8}',
                    b'f' => '\u{c}',
                    b'u' => {
                        self.escape = Some(Escape::Unicode {
                            digits: 0,
                            code: 0,
                            high: None,
                        });
                        return;
                    }
                    // `\"`, `\\`, `\/`; anything else is invalid JSON and kept as written.
                    other => other as char,
                };
                self.push(target, c);
            }
            Escape::Unicode { digits, code, high } => {
                let Some(digit) = (b as char).to_digit(16) else {
                    // Not JSON: drop what was held and read on.
                    self.escape = Some(Escape::None);
                    if high.is_some() {
                        self.push(target, char::REPLACEMENT_CHARACTER);
                    }
                    self.byte(b);
                    return;
                };
                let code = code * 16 + digit;
                if digits < 3 {
                    self.escape = Some(Escape::Unicode {
                        digits: digits + 1,
                        code,
                        high,
                    });
                    return;
                }
                self.escape = Some(Escape::None);
                match (high, code) {
                    (Some(high), 0xDC00..=0xDFFF) => {
                        let c = 0x10000 + ((high - 0xD800) << 10) + (code - 0xDC00);
                        self.push(target, char::from_u32(c).unwrap_or('\u{FFFD}'));
                    }
                    (high, 0xD800..=0xDBFF) => {
                        if high.is_some() {
                            self.push(target, char::REPLACEMENT_CHARACTER);
                        }
                        self.escape = Some(Escape::High(code));
                    }
                    (high, code) => {
                        if high.is_some() {
                            self.push(target, char::REPLACEMENT_CHARACTER);
                        }
                        self.push(target, char::from_u32(code).unwrap_or('\u{FFFD}'));
                    }
                }
            }
            Escape::High(high) => {
                if b == b'\\' {
                    self.escape = Some(Escape::HighBackslash(high));
                } else {
                    self.escape = Some(Escape::None);
                    self.push(target, char::REPLACEMENT_CHARACTER);
                    self.byte(b);
                }
            }
            Escape::HighBackslash(high) => {
                if b == b'u' {
                    self.escape = Some(Escape::Unicode {
                        digits: 0,
                        code: 0,
                        high: Some(high),
                    });
                } else {
                    // A lone high surrogate, then another escape.
                    self.escape = Some(Escape::Backslash);
                    self.push(target, char::REPLACEMENT_CHARACTER);
                    self.byte(b);
                }
            }
        }
    }

    fn structure(&mut self, b: u8) {
        match b {
            b'{' | b'[' => {
                if self.depth == 0 && b == b'[' {
                    self.off = true;
                    return;
                }
                self.depth += 1;
                if self.depth == 1 {
                    self.expect_key = true;
                }
            }
            b'}' | b']' => {
                self.depth = self.depth.saturating_sub(1);
                if self.depth == 0 {
                    self.off = true;
                }
            }
            b',' if self.depth == 1 => self.expect_key = true,
            b':' if self.depth == 1 => self.expect_key = false,
            b'"' => {
                let target = match (self.depth, self.expect_key) {
                    (1, true) => {
                        self.key.clear();
                        Target::Key
                    }
                    (1, false) => {
                        self.fields.push(Field {
                            key: std::mem::take(&mut self.key),
                            value: String::new(),
                            done: false,
                            newlines: 0,
                        });
                        Target::Value(self.fields.len() - 1)
                    }
                    (0, _) => {
                        self.off = true;
                        return;
                    }
                    _ => Target::Skip,
                };
                self.string = Some(target);
                self.escape = None;
            }
            _ => {}
        }
    }

    fn end_string(&mut self, target: Target) {
        if !self.utf8.is_empty() {
            self.utf8.clear();
            self.push(target, char::REPLACEMENT_CHARACTER);
        }
        if let Target::Value(i) = target {
            self.fields[i].done = true;
        }
        self.string = None;
        self.escape = None;
    }

    /// A byte of the string as written: UTF-8 sequences are held until whole.
    fn raw(&mut self, target: Target, b: u8) {
        if b < 0x80 && self.utf8.is_empty() {
            self.push(target, b as char);
            return;
        }
        self.utf8.push(b);
        match std::str::from_utf8(&self.utf8) {
            Ok(s) => {
                let c = s.chars().next().unwrap_or('\u{FFFD}');
                self.utf8.clear();
                self.push(target, c);
            }
            Err(e) if e.error_len().is_some() => {
                self.utf8.clear();
                self.push(target, char::REPLACEMENT_CHARACTER);
                if b < 0x80 {
                    self.push(target, b as char);
                } else if b >= 0xC0 {
                    self.utf8.push(b);
                }
            }
            Err(_) => {}
        }
    }

    fn push(&mut self, target: Target, c: char) {
        match target {
            Target::Key => self.key.push(c),
            Target::Value(i) => {
                let field = &mut self.fields[i];
                field.value.push(c);
                if c == '\n' {
                    field.newlines += 1;
                }
            }
            Target::Skip => {}
        }
    }

    /// The input as far as `transcript::target` should show it: only the fields it reads (not a
    /// write's body), unfinished paths left out (a file name shows once it is whole), other text
    /// fields as they grow.
    pub fn partial(&self) -> Value {
        let mut out = Map::new();
        for f in &self.fields {
            if !TARGET_KEYS.contains(&f.key.as_str()) {
                continue;
            }
            let path = matches!(f.key.as_str(), "path" | "filePath");
            if f.done || !path {
                out.insert(f.key.clone(), Value::String(f.value.clone()));
            }
        }
        Value::Object(out)
    }
}

/// The fields `transcript::target` reads.
const TARGET_KEYS: [&str; 8] = [
    "path",
    "filePath",
    "pattern",
    "command",
    "url",
    "query",
    "description",
    "prompt",
];

/// The partial input a stored raw holds (`ToolState::Streaming`), for the row read back.
pub fn partial(raw: &str) -> Value {
    let mut scan = Scan::default();
    scan.feed(raw.as_bytes());
    scan.partial()
}

/// The field whose text the row shows growing, and whether its last lines show too.
fn body(name: &str) -> Option<(&'static str, bool)> {
    match name {
        "write" => Some(("content", true)),
        "edit" => Some(("newString", true)),
        // A notebook cell's new source.
        "notebook" => Some(("source", true)),
        // A Code Mode program is counted, not shown.
        "execute" => Some(("code", false)),
        _ => None,
    }
}

pub const TAIL_LINES: usize = 6;
const TAIL_BYTES: usize = 4096;
const LINE_CHARS: usize = 240;
const TARGET_CHARS: usize = 400;

/// The last `TAIL_LINES` lines of a text, each clipped, from at most its last `TAIL_BYTES`.
pub fn tail(text: &str) -> String {
    let mut start = text.len().saturating_sub(TAIL_BYTES);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let window = &text[start..];
    let lines: Vec<&str> = window.split('\n').collect();
    // A text that ends with a newline has an empty last line: the cursor sits on it.
    let from = lines.len().saturating_sub(TAIL_LINES);
    lines[from..]
        .iter()
        .map(|l| clip(l, LINE_CHARS))
        .collect::<Vec<_>>()
        .join("\n")
}

fn clip(text: &str, chars: usize) -> &str {
    match text.char_indices().nth(chars) {
        Some((at, _)) => &text[..at],
        None => text,
    }
}

struct Call {
    session_id: String,
    message_id: String,
    name: String,
    scan: Scan,
    /// Bytes received; a piece that does not start here means pieces were lost.
    len: usize,
    dirty: bool,
    lost: bool,
}

/// The calls whose input is streaming, in every session of one folder's core.
#[derive(Default)]
pub struct Live {
    calls: HashMap<String, Call>,
}

impl Live {
    pub fn piece(
        &mut self,
        session_id: &str,
        message_id: &str,
        call_id: &str,
        name: &str,
        at: usize,
        text: &str,
    ) {
        let call = self
            .calls
            .entry(call_id.to_string())
            .or_insert_with(|| Call {
                session_id: session_id.to_string(),
                message_id: message_id.to_string(),
                name: name.to_string(),
                scan: Scan::default(),
                len: 0,
                dirty: false,
                lost: false,
            });
        if call.lost {
            return;
        }
        if call.len != at {
            call.lost = true;
            call.dirty = true;
            return;
        }
        call.scan.feed(text.as_bytes());
        call.len += text.len();
        call.dirty = true;
    }

    /// The call is ready (or gone): the transcript shows it from here.
    pub fn end_call(&mut self, call_id: &str) {
        self.calls.remove(call_id);
    }

    pub fn end_session(&mut self, session_id: &str) {
        self.calls.retain(|_, c| c.session_id != session_id);
    }

    /// Events were dropped: no call can trust what it has.
    pub fn lagged(&mut self) {
        for call in self.calls.values_mut() {
            if !call.lost {
                call.lost = true;
                call.dirty = true;
            }
        }
    }

    pub fn pending(&self) -> bool {
        self.calls.values().any(|c| c.dirty)
    }

    /// A snapshot of every call that changed since the last flush. A call that lost pieces
    /// sends one empty snapshot: its row goes back to its name and a spinner.
    pub fn flush(&mut self, directory: &str) -> Vec<Value> {
        let mut out = Vec::new();
        for (call_id, call) in &mut self.calls {
            if !call.dirty {
                continue;
            }
            call.dirty = false;
            let (target, lines, tail) = if call.lost {
                (String::new(), None, None)
            } else {
                snapshot(&call.name, &call.scan, directory)
            };
            out.push(json!({
                "type": "toolInput",
                "sessionId": call.session_id,
                "messageId": call.message_id,
                "callId": call_id,
                "name": call.name,
                "target": target,
                "lines": lines,
                "tail": tail,
            }));
        }
        out
    }
}

/// What the window shows of a call so far: its one-line target, its body's line count and the
/// body's last lines.
pub fn snapshot(
    name: &str,
    scan: &Scan,
    directory: &str,
) -> (String, Option<usize>, Option<String>) {
    let target = crate::transcript::target(name, &scan.partial(), directory);
    let target = clip(&target, TARGET_CHARS).to_string();
    let Some((key, shown)) = body(name) else {
        return (target, None, None);
    };
    match scan.field(key) {
        Some(f) => (target, Some(f.lines()), shown.then(|| tail(&f.value))),
        None => (target, None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inputs as models write them: escapes of every kind, a Windows path, non-ASCII text and
    /// emoji written both raw and as surrogate pairs, nested values to skip.
    fn samples() -> Vec<String> {
        let mut out = vec![
            json!({"path": "C:\\Users\\dev\\src\\app.tsx", "content": "line one\n\tindented \"quoted\" \\ back\r\nşğİ 🎉 é\u{1}\u{8}\n/ slash\n"}).to_string(),
            json!({"path": "src/lib.rs", "oldString": "fn a() {}", "newString": "fn a() {\n    println!(\"hi\");\n}\n", "replaceAll": false}).to_string(),
            json!({"command": "cd 'my dir' && echo \"ok\" | grep -c \\\\", "timeout": 120, "background": false}).to_string(),
            json!({"questions": [{"question": "Which?", "options": [{"label": "a\"b"}]}], "description": "after nested"}).to_string(),
            json!({"code": "const r = await tools.tracker.ask({\"a\": [1, {\"b\": \"}\"}]})\nreturn r", "nested": {"path": "x"}}).to_string(),
        ];
        // As Python's json.dumps writes it: non-ASCII as \u escapes, emoji as surrogate pairs.
        out.push(r#"{"path": "notes/\u00e7al\u0131\u015fma.md", "content": "\ud83c\udf89 party \u00e9\n\u4e2d\u6587\n"}"#.to_string());
        // Pretty-printed, with spaces and newlines between tokens.
        out.push(
            "{\n  \"description\" : \"Look around\" ,\n  \"prompt\":\"Find \\u0041ll\"\n}"
                .to_string(),
        );
        out
    }

    fn finals(raw: &str) -> Vec<(String, String)> {
        let value: Value = serde_json::from_str(raw).unwrap();
        value
            .as_object()
            .unwrap()
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect()
    }

    fn whole(raw: &[u8]) -> Scan {
        let mut scan = Scan::default();
        scan.feed(raw);
        scan
    }

    #[test]
    fn the_whole_input_reads_as_json_does() {
        for raw in samples() {
            let scan = whole(raw.as_bytes());
            let got: Vec<(String, String)> = scan
                .fields
                .iter()
                .map(|f| (f.key.clone(), f.value.clone()))
                .collect();
            assert_eq!(got, finals(&raw), "{raw}");
            assert!(scan.fields.iter().all(|f| f.done));
        }
    }

    #[test]
    fn every_cut_is_a_prefix_of_the_final_values() {
        for raw in samples() {
            let bytes = raw.as_bytes();
            let end = finals(&raw);
            for cut in 0..=bytes.len() {
                let scan = whole(&bytes[..cut]);
                for f in &scan.fields {
                    let (_, value) = end
                        .iter()
                        .find(|(k, _)| *k == f.key)
                        .unwrap_or_else(|| panic!("{raw} cut at {cut}: unknown key {:?}", f.key));
                    assert!(
                        value.starts_with(&f.value),
                        "{raw} cut at {cut}: {:?} is not a prefix of {value:?}",
                        f.value
                    );
                    if f.done {
                        assert_eq!(&f.value, value, "{raw} cut at {cut}");
                    }
                    assert!(!f.value.contains('\u{FFFD}'), "{raw} cut at {cut}");
                }
                // Fed in two pieces at the cut, the end is the same as fed whole.
                let mut two = whole(&bytes[..cut]);
                two.feed(&bytes[cut..]);
                assert_eq!(two.fields, whole(bytes).fields, "{raw} split at {cut}");
            }
            // One byte at a time.
            let mut bytewise = Scan::default();
            for b in bytes {
                bytewise.feed(std::slice::from_ref(b));
            }
            assert_eq!(bytewise.fields, whole(bytes).fields, "{raw} byte by byte");
        }
    }

    #[test]
    fn surrogate_pairs_and_unicode_escapes_are_held_until_whole() {
        let raw = br#"{"content": "a\ud83c\udf89b"}"#;
        let cut = |n: usize| whole(&raw[..n]).fields[0].value.clone();
        let at = raw.windows(2).position(|w| w == b"\\u").unwrap();
        // From the first backslash until the low half's last digit, only "a" shows.
        for n in at..at + 12 {
            assert_eq!(cut(n), "a", "cut at {n}");
        }
        assert_eq!(cut(at + 12), "a🎉");
        // A lone high surrogate becomes a replacement character once it is clear it is lone.
        assert_eq!(whole(br#"{"c": "\ud83cx"}"#).fields[0].value, "\u{FFFD}x");
        assert_eq!(whole(br#"{"c": "\ud83c\n"}"#).fields[0].value, "\u{FFFD}\n");
    }

    #[test]
    fn utf8_split_inside_a_character_is_held() {
        let raw = "{\"c\": \"ş🎉\"}".as_bytes();
        let start = 7;
        assert_eq!(whole(&raw[..start + 1]).fields[0].value, "");
        assert_eq!(whole(&raw[..start + 2]).fields[0].value, "ş");
        for n in start + 3..start + 6 {
            assert_eq!(whole(&raw[..n]).fields[0].value, "ş");
        }
        assert_eq!(whole(&raw[..start + 6]).fields[0].value, "ş🎉");
    }

    #[test]
    fn paths_show_once_whole_and_other_fields_as_they_grow() {
        let raw = br#"{"path": "/work/proj/src/app.tsx", "content": "x"}"#;
        let at = |n: usize| {
            crate::transcript::target("write", &whole(&raw[..n]).partial(), "/work/proj")
        };
        assert_eq!(at(20), "");
        assert_eq!(at(36), "src/app.tsx");
        let raw = br#"{"command": "npm test -- --watch"}"#;
        let shell =
            |n: usize| crate::transcript::target("shell", &whole(&raw[..n]).partial(), "/w");
        assert_eq!(shell(17), "npm ");
        assert_eq!(shell(raw.len()), "npm test -- --watch");
        // A Windows path, relative to a Windows folder.
        let raw = json!({"path": "C:\\proj\\src\\main.rs"}).to_string();
        assert_eq!(
            crate::transcript::target("edit", &partial(&raw), "C:\\proj"),
            "src\\main.rs"
        );
    }

    #[test]
    fn not_an_object_reads_as_nothing() {
        assert!(whole(br#"["a", "b"]"#).fields.is_empty());
        assert!(whole(br#""text""#).fields.is_empty());
        assert!(whole(br#"{"a": "b"} {"c": "d"}"#).fields.len() == 1);
    }

    #[test]
    fn lines_count_and_the_tail_is_bounded() {
        let scan = whole(br#"{"content": "a\nb\nc"}"#);
        assert_eq!(scan.fields[0].lines(), 3);
        let scan = whole(br#"{"content": "a\nb\n"}"#);
        assert_eq!(scan.fields[0].lines(), 2);
        let scan = whole(br#"{"content": ""}"#);
        assert_eq!(scan.fields[0].lines(), 0);
        let text: String = (1..=500).map(|i| format!("line {i}\n")).collect();
        assert_eq!(
            tail(&text),
            (496..=500)
                .map(|i| format!("line {i}\n"))
                .collect::<String>()
        );
        let long = format!("start\n{}", "x".repeat(10_000));
        let t = tail(&long);
        assert_eq!(t.chars().count(), LINE_CHARS);
        let wide = "é".repeat(5000);
        assert!(tail(&wide).len() <= TAIL_BYTES);
    }

    #[test]
    fn snapshots_are_sent_once_per_change_and_lost_pieces_reset_the_row() {
        let mut live = Live::default();
        let raw = r#"{"path": "/p/a.txt", "content": "one\ntwo"}"#;
        live.piece("s", "m", "c1", "write", 0, &raw[..25]);
        live.piece("s", "m", "c1", "write", 25, &raw[25..]);
        let sent = live.flush("/p");
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["target"], "a.txt");
        assert_eq!(sent[0]["lines"], 2);
        assert_eq!(sent[0]["tail"], "one\ntwo");
        assert!(live.flush("/p").is_empty(), "nothing changed");
        // A piece that doesn't follow on: the row falls back once, then stays quiet.
        live.piece("s", "m", "c1", "write", 999, "x");
        let sent = live.flush("/p");
        assert_eq!(
            (sent[0]["target"].as_str(), sent[0]["lines"].is_null()),
            (Some(""), true)
        );
        live.piece("s", "m", "c1", "write", raw.len(), "y");
        assert!(live.flush("/p").is_empty());
        // A call first seen mid-input (its start was dropped) never shows.
        live.piece("s", "m", "c2", "shell", 10, "abc");
        assert_eq!(live.flush("/p")[0]["target"], "");
        live.end_session("s");
        assert!(!live.pending() && live.calls.is_empty());
        live.piece("s", "m", "c3", "shell", 0, r#"{"command": "ls"#);
        live.lagged();
        assert_eq!(live.flush("/p")[0]["target"], "");
        live.end_call("c3");
        assert!(live.calls.is_empty());
    }

    #[test]
    fn a_large_input_sends_small_snapshots() {
        let content: String = (0..40_000)
            .map(|i| format!("line {i} with some text\n"))
            .collect();
        let raw = json!({"path": "big.txt", "content": content}).to_string();
        assert!(raw.len() > 1_000_000);
        let mut live = Live::default();
        let mut at = 0;
        let mut sent = 0;
        for chunk in raw.as_bytes().chunks(37) {
            let piece = std::str::from_utf8(chunk).unwrap();
            live.piece("s", "m", "c", "write", at, piece);
            at += piece.len();
            if at % (37 * 200) == 0 {
                sent += live.flush("/").len();
            }
        }
        let last = live.flush("/");
        assert_eq!(last[0]["lines"], 40_000);
        assert!(sent > 100);
        // Each snapshot carries the body's last lines, not the body.
        assert!(last[0]["tail"].as_str().unwrap().len() <= TAIL_BYTES);
    }
}
