//! The portable shell scanner that decides which commands a shell call runs, ported from 0.5.1's
//! hand-written `shell/scan.ts`. It is not a shell parser: it splits a script into its simple
//! commands (through lists, pipelines, substitutions and compound commands) and gives up with an
//! [`Opaque`] reason whenever a dialect could read the text differently or run code it cannot see.
//! Accept/reject behaviour and the extracted texts must match 0.5.1 exactly, so the code follows the
//! original's structure; positions are character indices.

mod bash;
mod powershell;
#[cfg(test)]
mod tests;

use std::sync::LazyLock;

pub use bash::scan;
pub use powershell::scan_powershell;

/// Why a script cannot be analyzed; a parsing limitation, not a permission decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opaque {
    CommandSubstitution,
    CompoundCommand,
    DynamicCommandName,
    DynamicExecution,
    Heredoc,
    InvalidRedirect,
    InvalidStructure,
    UnterminatedEscape,
    UnterminatedQuote,
}

impl std::fmt::Display for Opaque {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Opaque::CommandSubstitution => "command-substitution",
            Opaque::CompoundCommand => "compound-command",
            Opaque::DynamicCommandName => "dynamic-command-name",
            Opaque::DynamicExecution => "dynamic-execution",
            Opaque::Heredoc => "heredoc",
            Opaque::InvalidRedirect => "invalid-redirect",
            Opaque::InvalidStructure => "invalid-structure",
            Opaque::UnterminatedEscape => "unterminated-escape",
            Opaque::UnterminatedQuote => "unterminated-quote",
        })
    }
}

/// Bash and Zsh name those shells; Posix covers sh, dash and every other shell and stays sound for
/// all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Bash,
    Zsh,
    Posix,
}

/// One simple command.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Command {
    /// Its source text.
    pub resource: String,
    /// Words as the shell reads them (quotes removed), from the command name on.
    pub words: Vec<String>,
    /// The same words as written.
    pub raw_words: Vec<String>,
    /// Exclusive end of each raw word, relative to `resource` (PowerShell).
    pub word_ends: Option<Vec<usize>>,
    /// A PowerShell command at the head of a statement.
    pub statement_head: bool,
    /// A declaration (`export`, `local`, …), not a permission resource of its own.
    pub declaration: bool,
    /// Words before the first redirect, when redirects appear between arguments.
    pub redirect_word_count: Option<usize>,
    /// Exclusive prefix end within `resource` for commandless or group redirects.
    pub redirect_prefix_end: Option<usize>,
}

const MAX_INPUT_LENGTH: usize = 64 * 1024;
const MAX_SUBSTITUTION_DEPTH: usize = 32;

/// Scanned text with character positions and their byte offsets.
struct Src {
    text: String,
    chars: Vec<char>,
    bytes: Vec<usize>,
}

impl Src {
    fn new(text: String) -> Src {
        let mut chars = Vec::with_capacity(text.len());
        let mut bytes = Vec::with_capacity(text.len() + 1);
        for (offset, c) in text.char_indices() {
            chars.push(c);
            bytes.push(offset);
        }
        bytes.push(text.len());
        Src { text, chars, bytes }
    }

    fn len(&self) -> usize {
        self.chars.len()
    }

    fn at(&self, index: usize) -> Option<char> {
        self.chars.get(index).copied()
    }

    /// The character before `index`, if any.
    fn before(&self, index: usize) -> Option<char> {
        index.checked_sub(1).and_then(|i| self.at(i))
    }

    fn is(&self, index: usize, c: char) -> bool {
        self.at(index) == Some(c)
    }

    /// `slice(start, end)` with JavaScript's clamping.
    fn slice(&self, start: usize, end: usize) -> &str {
        let end = end.min(self.len());
        let start = start.min(end);
        &self.text[self.bytes[start]..self.bytes[end]]
    }

    fn rest(&self, start: usize) -> &str {
        self.slice(start, self.len())
    }

    fn starts_with(&self, index: usize, text: &str) -> bool {
        index <= self.len() && self.rest(index).starts_with(text)
    }

    fn index_of(&self, c: char, from: usize) -> Option<usize> {
        (from..self.len()).find(|&i| self.chars[i] == c)
    }

    fn find(&self, text: &str, from: usize) -> Option<usize> {
        let from = from.min(self.len());
        self.text[self.bytes[from]..]
            .find(text)
            .map(|offset| self.char_index(self.bytes[from] + offset))
    }

    fn char_index(&self, byte: usize) -> usize {
        self.bytes.partition_point(|&b| b < byte)
    }

    /// End (exclusive) of a match of the `^`-anchored `regex` at `index` (a sticky regex).
    fn sticky(&self, regex: &fancy_regex::Regex, index: usize) -> Option<usize> {
        if index > self.len() {
            return None;
        }
        let start = self.bytes[index];
        let found = regex.find(&self.text[start..]).ok().flatten()?;
        Some(self.char_index(start + found.end()))
    }
}

/// Compiles a sticky regex: anchored at the position it is applied to.
fn sticky(pattern: &str) -> fancy_regex::Regex {
    fancy_regex::Regex::new(&format!("^(?:{})", js_pattern(pattern)))
        .expect("scanner regex is valid")
}

/// Rewrites JavaScript's ASCII `\d`, `\w` and its own `\s`/`\S` set into explicit classes, since the
/// Rust engines read those escapes as Unicode classes.
fn js_pattern(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    let mut in_class = false;
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let Some(next) = chars.next() else {
                    out.push(c);
                    break;
                };
                let set = match next {
                    'd' => Some(("0-9", false)),
                    'w' => Some(("A-Za-z0-9_", false)),
                    's' => Some((JS_SPACE_SET, false)),
                    'S' => Some((JS_SPACE_SET, true)),
                    _ => None,
                };
                match set {
                    Some((set, negated)) if in_class && !negated => out.push_str(set),
                    Some((set, negated)) => {
                        out.push_str(if negated { "[^" } else { "[" });
                        out.push_str(set);
                        out.push(']');
                    }
                    None => {
                        out.push(c);
                        out.push(next);
                    }
                }
            }
            '[' if !in_class => {
                in_class = true;
                out.push(c);
                // A leading `^` or `]` belongs to the class syntax, not its end.
                if let Some(rest) = chars.as_str().strip_prefix('^') {
                    out.push('^');
                    chars = rest.chars();
                }
            }
            ']' if in_class => {
                in_class = false;
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

fn regex(pattern: &str) -> regex::Regex {
    regex::Regex::new(pattern).expect("scanner regex is valid")
}

/// JavaScript's `\s` set, as the inside of a character class.
pub(super) const JS_SPACE_SET: &str = r" \t\n\r\x0B\x0C\u{A0}\u{1680}\u{2000}-\u{200A}\u{2028}\u{2029}\u{202F}\u{205F}\u{3000}\u{FEFF}";

pub(super) fn js_space(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\n' | '\r' | '\x0B' | '\x0C' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

fn js_trim(text: &str) -> &str {
    text.trim_matches(js_space)
}

fn js_trim_end(text: &str) -> &str {
    text.trim_end_matches(js_space)
}

/// `[\p{L}\p{Nl}\d_]` with JavaScript's ASCII `\d`.
fn ps_word_char(c: char) -> bool {
    static WORD: LazyLock<regex::Regex> = LazyLock::new(|| regex(r"^[\p{L}\p{Nl}0-9_]$"));
    c.is_ascii_alphanumeric()
        || c == '_'
        || (!c.is_ascii() && WORD.is_match(c.encode_utf8(&mut [0; 4])))
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}
