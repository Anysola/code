//! JSON with comments and trailing commas, as `jsonc-parser` with `allowTrailingComma` accepts it.

use serde_json::Value;

/// Removes `//` and `/* */` comments and trailing commas outside strings, then parses.
pub fn parse(text: &str) -> Result<Value, serde_json::Error> {
    serde_json::from_str(&strip(text))
}

fn strip(text: &str) -> String {
    let chars: Vec<char> = text.trim_start_matches('\u{feff}').chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            i += 1;
            out.extend(&chars[start..i.min(chars.len())]);
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
            out.push(' ');
            continue;
        }
        if c == ',' {
            let next = chars[i + 1..]
                .iter()
                .position(|c| !c.is_whitespace())
                .map(|p| chars[i + 1 + p]);
            if matches!(next, Some('}') | Some(']')) {
                i += 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn comments_and_trailing_commas() {
        let text =
            "{\n // comment\n \"a\": \"x // not a comment\", /* block */\n \"b\": [1, 2,],\n}";
        assert_eq!(
            parse(text).unwrap(),
            json!({"a": "x // not a comment", "b": [1, 2]})
        );
    }
}
