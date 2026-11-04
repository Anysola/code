//! JavaScript string and number semantics the converter depends on.

/// The `WhiteSpace` and `LineTerminator` characters of `String.prototype.trim` and `\s`.
pub fn is_whitespace(character: char) -> bool {
    matches!(
        character,
        '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

pub fn trim(value: &str) -> &str {
    value.trim_matches(is_whitespace)
}

/// Replaces every maximal run of characters matching `class` with `replacement`.
pub fn replace_runs(value: &str, class: impl Fn(char) -> bool, replacement: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut in_run = false;
    for character in value.chars() {
        if class(character) {
            if !in_run {
                result.push_str(replacement);
            }
            in_run = true;
        } else {
            result.push(character);
            in_run = false;
        }
    }
    result
}

/// `Number.parseInt(value)`, with `None` for `NaN`.
pub fn parse_int(value: &str) -> Option<f64> {
    let value = value.trim_start_matches(is_whitespace);
    let (negative, value) = match value.as_bytes().first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    let (radix, value) = match value.get(..2) {
        Some("0x" | "0X") => (16, &value[2..]),
        _ => (10, value),
    };
    let digits = value
        .find(|c: char| !c.is_digit(radix))
        .map_or(value, |end| &value[..end]);
    if digits.is_empty() {
        return None;
    }
    let magnitude = if radix == 10 {
        digits.parse::<f64>().ok()?
    } else {
        digits.chars().fold(0.0, |total, digit| {
            total * 16.0 + f64::from(digit.to_digit(16).unwrap_or(0))
        })
    };
    Some(if negative { -magnitude } else { magnitude })
}

/// `String(number)` for the integral values list numbering produces.
pub fn number_to_string(number: f64) -> String {
    if number.is_infinite() {
        return if number > 0.0 {
            "Infinity"
        } else {
            "-Infinity"
        }
        .to_owned();
    }
    if number == 0.0 {
        return "0".to_owned();
    }
    if number.abs() < 1e21 {
        return format!("{number}");
    }
    let exponential = format!("{number:e}");
    exponential.replacen('e', "e+", 1)
}
