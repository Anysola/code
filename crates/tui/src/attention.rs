//! Desktop notifications when a session needs attention (`attention.notifications`, off by
//! default): sent as OSC 777 and OSC 9 while the terminal does not have focus.

use std::io::Write;

const TITLE_MAX: usize = 80;
const MESSAGE_MAX: usize = 240;

/// ANSI escapes and control characters removed, whitespace runs collapsed, then cut to `max`.
fn clean(text: &str, max: usize) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if c.is_whitespace() {
            out.push(' ');
        } else if !c.is_control() {
            out.push(c);
        }
    }
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(max).collect()
}

/// The escape sequences of one notification.
pub fn sequences(title: &str, message: &str) -> String {
    let title = clean(
        if title.is_empty() {
            "Anysola Code"
        } else {
            title
        },
        TITLE_MAX,
    );
    let message = clean(message, MESSAGE_MAX);
    format!("\x1b]777;notify;{title};{message}\x07\x1b]9;{title}: {message}\x07")
}

pub fn notify(title: &str, message: &str) {
    let mut out = std::io::stdout();
    let _ = out.write_all(sequences(title, message).as_bytes());
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_title_and_message() {
        let text = sequences("", "\x1b[31mfailed\x1b[0m\nbadly\x07");
        assert_eq!(
            text,
            "\x1b]777;notify;Anysola Code;failed badly\x07\x1b]9;Anysola Code: failed badly\x07"
        );
        assert_eq!(clean(&"x".repeat(300), MESSAGE_MAX).len(), 240);
    }
}
