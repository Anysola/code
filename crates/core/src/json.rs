//! JSON helpers matching JavaScript's `JSON.stringify` output.

use serde_json::Value;

/// `JSON.stringify(value)`.
pub fn compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// `JSON.stringify(value, null, 2)`.
pub fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// The object a tool call's input JSON was becoming when it was cut off (the model was stopped,
/// the stream broke, `max_tokens`): every field written so far, a string cut mid-way kept as far
/// as it got (an escape cut in half dropped), a field with no value yet as `null`. `None` when
/// nothing of an object was written. A whole input parses as itself.
pub fn partial_object(raw: &str) -> Option<serde_json::Map<String, Value>> {
    if let Ok(Value::Object(map)) = serde_json::from_str(raw) {
        return Some(map);
    }
    match serde_json::from_str(&complete(raw)?) {
        Ok(Value::Object(map)) => Some(map),
        _ => None,
    }
}

/// Longest a string field of a cut-off call keeps (characters): enough to name a file or show a
/// command, little enough that a file cut off at `max_tokens` doesn't ride along in every later
/// request (the error already quotes the start of it).
const CUT_OFF_FIELD: usize = 1000;

/// What a call cut off while its input was written is stored with (and sent back with): the
/// fields written so far, long strings shortened. `{}` when nothing was.
pub fn cut_off_input(raw: &str) -> Value {
    let Some(map) = partial_object(raw) else {
        return Value::Object(serde_json::Map::new());
    };
    Value::Object(map.into_iter().map(|(k, v)| (k, clip(v))).collect())
}

fn clip(value: Value) -> Value {
    match value {
        Value::String(s) if s.chars().count() > CUT_OFF_FIELD => {
            let kept: String = s.chars().take(CUT_OFF_FIELD).collect();
            let more = s.chars().count() - CUT_OFF_FIELD;
            Value::String(format!(
                "{kept}\n[cut off: {more} more characters were written]"
            ))
        }
        Value::Array(items) => Value::Array(items.into_iter().map(clip).collect()),
        Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (k, clip(v))).collect()),
        other => other,
    }
}

/// A tool call's whole input. Streamed eagerly, the API no longer checks it is valid JSON; a model
/// that put a raw tab or newline inside a string (the one slip seen in practice) gets it read as
/// the escape it meant instead of the whole call failing.
pub fn tool_input(raw: &str) -> Result<Value, serde_json::Error> {
    match serde_json::from_str(raw) {
        Ok(value) => Ok(value),
        Err(error) => serde_json::from_str(&escape_controls(raw)).map_err(|_| error),
    }
}

/// Control characters inside strings written as escapes; everything else untouched.
fn escape_controls(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 16);
    let mut in_string = false;
    let mut escape = false;
    for c in raw.chars() {
        if in_string {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            } else if (c as u32) < 0x20 {
                match c {
                    '\n' => out.push_str("\\n"),
                    '\t' => out.push_str("\\t"),
                    '\r' => out.push_str("\\r"),
                    other => out.push_str(&format!("\\u{:04x}", other as u32)),
                }
                continue;
            }
        } else if c == '"' {
            in_string = true;
        }
        out.push(c);
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Expect {
    Key,
    Colon,
    Value,
    Comma,
}

/// `raw` closed into a whole document (see [`partial_object`]).
fn complete(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    // Open containers: `true` for an object; with what each expects next.
    let mut stack: Vec<(bool, Expect)> = Vec::new();
    let mut in_string = false;
    let mut key = false;
    let mut escape = false;
    let mut token: Option<usize> = None;
    let mut string_start = 0;
    let mut started = false;
    let done = |stack: &mut Vec<(bool, Expect)>| {
        if let Some(top) = stack.last_mut() {
            top.1 = Expect::Comma;
        }
    };
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_string = false;
                match stack.last_mut() {
                    Some(top) if key => top.1 = Expect::Colon,
                    _ => done(&mut stack),
                }
            }
            continue;
        }
        let structural = matches!(b, b'{' | b'[' | b'}' | b']' | b':' | b',' | b'"');
        if token.is_some() && (structural || b.is_ascii_whitespace()) {
            token = None;
            done(&mut stack);
        }
        match b {
            b'{' => {
                started = true;
                stack.push((true, Expect::Key));
            }
            b'[' => {
                started = true;
                stack.push((false, Expect::Value));
            }
            b'}' | b']' => {
                stack.pop();
                done(&mut stack);
            }
            b':' => {
                if let Some(top) = stack.last_mut() {
                    top.1 = Expect::Value;
                }
            }
            b',' => {
                if let Some(top) = stack.last_mut() {
                    top.1 = if top.0 { Expect::Key } else { Expect::Value };
                }
            }
            b'"' => {
                in_string = true;
                string_start = i;
                key = matches!(stack.last(), Some((true, Expect::Key)));
            }
            _ if b.is_ascii_whitespace() => {}
            _ => {
                if token.is_none() {
                    token = Some(i);
                }
            }
        }
    }
    if !started {
        return None;
    }
    let mut out = raw.to_string();
    if in_string && key {
        // A name not finished names nothing yet.
        out.truncate(string_start);
    } else if in_string {
        if escape {
            out.pop();
        } else if let Some(at) = out.rfind("\\u")
            && out.len() - at < 6
            && !escaped_backslash_before(&out, at)
        {
            out.truncate(at);
        }
        // A high surrogate whose low half wasn't written yet.
        if out.len() >= 6 && out.is_char_boundary(out.len() - 6) {
            let tail = &out[out.len() - 6..];
            if tail.starts_with("\\u")
                && !escaped_backslash_before(&out, out.len() - 6)
                && u16::from_str_radix(&tail[2..], 16).is_ok_and(|u| (0xD800..0xDC00).contains(&u))
            {
                out.truncate(out.len() - 6);
            }
        }
        out.push('"');
        done(&mut stack);
    } else if let Some(at) = token {
        // A number or word the input ends in may go on (`12` of `120`, `tru`): not a value yet.
        out.truncate(at);
    }
    let trimmed = out.trim_end().len();
    out.truncate(trimmed);
    match stack.last() {
        Some((_, Expect::Colon)) => out.push_str(":null"),
        Some((_, Expect::Value)) if out.ends_with(':') => out.push_str("null"),
        Some((_, Expect::Key | Expect::Value)) if out.ends_with(',') => {
            out.pop();
        }
        _ => {}
    }
    for (object, _) in stack.iter().rev() {
        out.push(if *object { '}' } else { ']' });
    }
    Some(out)
}

/// Whether the `\` at `at` is itself escaped (`\\u` is a backslash, then `u`).
fn escaped_backslash_before(s: &str, at: usize) -> bool {
    s.as_bytes()[..at]
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count()
        % 2
        == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Every string in `part` is a prefix of the same place in `whole`; every other value equal
    /// or `null`; arrays and objects no longer than the whole's.
    fn prefix_of(part: &Value, whole: &Value) -> bool {
        match (part, whole) {
            (Value::Null, _) => true,
            (Value::String(a), Value::String(b)) => b.starts_with(a.as_str()),
            (Value::Array(a), Value::Array(b)) => {
                a.len() <= b.len() && a.iter().zip(b).all(|(x, y)| prefix_of(x, y))
            }
            (Value::Object(a), Value::Object(b)) => a
                .iter()
                .all(|(k, v)| b.get(k).is_some_and(|w| prefix_of(v, w))),
            (a, b) => a == b,
        }
    }

    #[test]
    fn every_cut_is_a_prefix_of_the_whole() {
        let inputs = [
            json!({"path": "src/a b.html", "content": "<p class=\"x\">\n\tçok güzel 😀 \\ end</p>\n"}),
            json!({"filePath": "C:\\Users\\me\\x.ts", "oldString": "a\u{0001}b", "newString": "q\"\\n", "replaceAll": true}),
            json!({"todos": [{"content": "one", "status": "pending", "priority": 1.5}, {"content": "two", "status": "done", "priority": -2}], "n": null}),
            json!({"command": "echo \"hi\" && ls -la", "timeout": 120000, "description": "List"}),
        ];
        for whole in inputs {
            for raw in [crate::json::compact(&whole), crate::json::pretty(&whole)] {
                let ascii = raw.replace('😀', "\\ud83d\\ude00").replace('ç', "\\u00e7");
                for raw in [raw, ascii] {
                    for cut in 1..=raw.len() {
                        if !raw.is_char_boundary(cut) {
                            continue;
                        }
                        let got = partial_object(&raw[..cut])
                            .unwrap_or_else(|| panic!("no object from {:?}", &raw[..cut]));
                        assert!(
                            prefix_of(&Value::Object(got.clone()), &whole),
                            "{:?} gave {got:?}",
                            &raw[..cut]
                        );
                    }
                    assert_eq!(Value::Object(partial_object(&raw).unwrap()), whole);
                }
            }
        }
    }

    #[test]
    fn raw_control_characters_in_strings_are_read_as_escapes() {
        let raw = "{\"path\": \"a.txt\", \"content\": \"one\n\ttwo\u{1}\"}";
        assert!(serde_json::from_str::<Value>(raw).is_err());
        assert_eq!(
            tool_input(raw).unwrap(),
            json!({"path": "a.txt", "content": "one\n\ttwo\u{1}"})
        );
        // Whitespace between tokens is not inside a string and stays as it is.
        assert_eq!(tool_input("{\n\t\"a\": 1}").unwrap(), json!({"a": 1}));
        assert!(tool_input("{\"a\": ").is_err());
    }

    #[test]
    fn cut_off_input_keeps_names_and_shortens_long_text() {
        let long = "x".repeat(5000);
        let raw = format!(r#"{{"path": "big.html", "content": "{long}"#);
        let got = cut_off_input(&raw);
        assert_eq!(got["path"], "big.html");
        let content = got["content"].as_str().unwrap();
        assert!(content.starts_with(&"x".repeat(1000)));
        assert!(content.ends_with("[cut off: 4000 more characters were written]"));
        assert_eq!(cut_off_input(""), json!({}));
    }

    #[test]
    fn cut_inside_the_path_keeps_what_was_written() {
        let got = partial_object(r#"{"path": "docs/llm-de"#).unwrap();
        assert_eq!(got["path"], "docs/llm-de");
        let got = partial_object(r#"{"path": "a.html", "content": "line\"#).unwrap();
        assert_eq!(got["content"], "line");
        assert_eq!(partial_object("  "), None);
    }
}
