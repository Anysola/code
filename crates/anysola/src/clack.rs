//! The framed prompt output of 0.5.1's interactive commands (`@clack/prompts`): `┌` intro, `│` log
//! lines with a symbol, `└` outro. Everything goes to stdout; colours only on a terminal.

use std::io::{IsTerminal, Write};

const GRAY: &str = "\x1b[90m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const MAGENTA: &str = "\x1b[35m";
const RESET: &str = "\x1b[39m";

fn colored() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

fn paint(color: &str, text: &str) -> String {
    if colored() {
        format!("{color}{text}{RESET}")
    } else {
        text.to_string()
    }
}

fn write(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

pub fn intro(title: &str) {
    write(&format!("{}  {title}\n", paint(GRAY, "┌")));
}

pub fn outro(message: &str) {
    write(&format!(
        "{}\n{}  {message}\n\n",
        paint(GRAY, "│"),
        paint(GRAY, "└")
    ));
}

pub fn cancel(message: &str) {
    write(&format!(
        "{}  {}\n\n",
        paint(GRAY, "└"),
        paint(RED, message)
    ));
}

/// A `│` line, then the message after `symbol` (continuation lines after `│`).
fn message_with(symbol: &str, message: &str) {
    let bar = paint(GRAY, "│");
    let mut parts = vec![bar.clone()];
    for (index, line) in message.split('\n').enumerate() {
        let lead = if index == 0 { symbol } else { bar.as_str() };
        parts.push(if line.is_empty() {
            lead.to_string()
        } else {
            format!("{lead}  {line}")
        });
    }
    write(&format!("{}\n", parts.join("\n")));
}

pub fn message(text: &str) {
    message_with(&paint(GRAY, "│"), text);
}

pub fn info(text: &str) {
    message_with(&paint(BLUE, "●"), text);
}

pub fn success(text: &str) {
    message_with(&paint(GREEN, "◆"), text);
}

pub fn step(text: &str) {
    message_with(&paint(GREEN, "◇"), text);
}

pub fn warn(text: &str) {
    message_with(&paint(YELLOW, "▲"), text);
}

pub fn error(text: &str) {
    message_with(&paint(RED, "■"), text);
}

/// The failure ending of a framed command: `■ <message>` then `└ Failed`.
pub fn failed(text: &str) {
    error(text);
    outro("Failed");
}

/// A one-shot spinner: shows `message` while work runs (on a terminal), then the result line.
pub struct Spinner {
    live: bool,
}

impl Spinner {
    pub fn start(message: &str) -> Spinner {
        write(&format!("{}\n", paint(GRAY, "│")));
        let live = std::io::stdout().is_terminal();
        if live {
            let message = message.trim_end_matches('.');
            write(&format!("\x1b[?25l{}  {message}...", paint(MAGENTA, "◒")));
        }
        Spinner { live }
    }

    /// Ends with `◇ message` (`ok`) or `■ message`.
    pub fn stop(self, message: &str, ok: bool) {
        let clear = if self.live { "\r\x1b[J" } else { "" };
        let symbol = if ok {
            paint(GREEN, "◇")
        } else {
            paint(RED, "■")
        };
        let show = if self.live { "\x1b[?25h" } else { "" };
        write(&format!("{clear}{symbol}  {message}\n{show}"));
    }
}
