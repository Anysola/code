//! Clack-style questions on the terminal: yes/no and a pick from a list.

use std::io::Write;

const GRAY: &str = "\x1b[90m";
const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const DIM: &str = "\x1b[2m";
const STRIKE: &str = "\x1b[9m";
const RESET: &str = "\x1b[0m";

fn write(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

fn choice(value: bool) -> String {
    let option = |label: &str, on: bool| {
        if on {
            format!("{GREEN}●{RESET} {label}")
        } else {
            format!("{DIM}○ {label}{RESET}")
        }
    };
    format!(
        "{} {DIM}/{RESET} {}",
        option("Yes", value),
        option("No", !value)
    )
}

enum Key {
    Prev,
    Next,
    Set(bool),
    Submit,
    Cancel,
    Other,
}

/// Asks `message`; `None` when the user cancels (Ctrl+C, Esc).
pub fn confirm(message: &str, initial: bool) -> Option<bool> {
    let mut value = initial;
    write(&format!(
        "\x1b[?25l{GRAY}│{RESET}\n{CYAN}◆{RESET}  {message}\n{CYAN}│{RESET}  {}\n{CYAN}└{RESET}\n",
        choice(value)
    ));
    let answer = loop {
        match read_key() {
            Key::Prev | Key::Next => value = !value,
            Key::Set(chosen) => {
                value = chosen;
                break Some(value);
            }
            Key::Submit => break Some(value),
            Key::Cancel => break None,
            Key::Other => continue,
        }
        write(&format!(
            "\x1b[2A\r\x1b[J{CYAN}│{RESET}  {}\n{CYAN}└{RESET}\n",
            choice(value)
        ));
    };
    let label = if value { "Yes" } else { "No" };
    let ending = match answer {
        Some(_) => format!("{GREEN}◇{RESET}  {message}\n{GRAY}│{RESET}  {DIM}{label}{RESET}\n"),
        None => format!(
            "{RED}■{RESET}  {message}\n{GRAY}│{RESET}  {STRIKE}{DIM}{label}{RESET}\n{GRAY}│{RESET}\n"
        ),
    };
    write(&format!("\x1b[3A\r\x1b[J{ending}\x1b[?25h"));
    answer
}

/// One option of [`select`]: its label and what follows it (dimmed hint, `✓` marker).
pub struct Choice {
    pub label: String,
    pub connected: bool,
    pub hint: Option<String>,
}

fn choices(heading: &str, options: &[Choice], cursor: usize) -> String {
    let mut lines = vec![format!("{CYAN}│{RESET}  \x1b[1m{heading}\x1b[22m")];
    for (index, choice) in options.iter().enumerate() {
        let (radio, label) = if index == cursor {
            (format!("{GREEN}●{RESET}"), choice.label.clone())
        } else {
            (format!("{DIM}○{RESET}"), format!("{DIM}{}{RESET}", choice.label))
        };
        let connected = if choice.connected {
            format!(" {GREEN}✓{RESET}")
        } else {
            String::new()
        };
        let hint = choice
            .hint
            .as_ref()
            .map(|h| format!(" {DIM}({h}){RESET}"))
            .unwrap_or_default();
        lines.push(format!("{CYAN}│{RESET}  {radio} {label}{connected}{hint}"));
    }
    lines.push(format!("{CYAN}└{RESET}"));
    lines.join("\n") + "\n"
}

/// Picks one of `options` under `heading`; `None` when the user cancels (Ctrl+C, Esc).
pub fn select(message: &str, heading: &str, options: &[Choice]) -> Option<usize> {
    let mut cursor = 0;
    let height = options.len() + 2;
    write(&format!(
        "\x1b[?25l{GRAY}│{RESET}\n{CYAN}◆{RESET}  {message}\n{}",
        choices(heading, options, cursor)
    ));
    let answer = loop {
        match read_key() {
            Key::Prev => cursor = (cursor + options.len() - 1) % options.len(),
            Key::Next => cursor = (cursor + 1) % options.len(),
            Key::Submit => break Some(cursor),
            Key::Cancel => break None,
            Key::Set(_) | Key::Other => continue,
        }
        write(&format!(
            "\x1b[{height}A\r\x1b[J{}",
            choices(heading, options, cursor)
        ));
    };
    let label = &options[cursor].label;
    let ending = match answer {
        Some(_) => format!("{GREEN}◇{RESET}  {message}\n{GRAY}│{RESET}  {DIM}{label}{RESET}\n"),
        None => format!("{RED}■{RESET}  {message}\n{GRAY}│{RESET}  {STRIKE}{DIM}{RESET}\n"),
    };
    write(&format!("\x1b[{}A\r\x1b[J{ending}\x1b[?25h", height + 1));
    answer
}

#[cfg(unix)]
fn read_key() -> Key {
    use std::io::Read;
    let Some(_raw) = RawMode::enter() else {
        return Key::Cancel;
    };
    let mut stdin = std::io::stdin().lock();
    let mut byte = [0u8; 1];
    if stdin.read(&mut byte).unwrap_or(0) == 0 {
        return Key::Cancel;
    }
    match byte[0] {
        b'y' | b'Y' => Key::Set(true),
        b'n' | b'N' => Key::Set(false),
        b'\r' | b'\n' => Key::Submit,
        3 => Key::Cancel,
        b'h' | b'k' => Key::Prev,
        b'j' | b'l' | b'\t' => Key::Next,
        0x1b => {
            // A lone Esc cancels; an arrow key arrives as `Esc [ A..D` at once.
            let mut poll = libc::pollfd {
                fd: libc::STDIN_FILENO,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one valid pollfd for the duration of the call.
            if unsafe { libc::poll(&mut poll, 1, 30) } <= 0 {
                return Key::Cancel;
            }
            let mut sequence = [0u8; 2];
            let read = stdin.read(&mut sequence).unwrap_or(0);
            match (read, sequence) {
                (2, [b'[', b'A' | b'D']) => Key::Prev,
                (2, [b'[', b'B' | b'C']) => Key::Next,
                _ => Key::Other,
            }
        }
        _ => Key::Other,
    }
}

#[cfg(not(unix))]
fn read_key() -> Key {
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
        return Key::Cancel;
    }
    match line.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Key::Set(true),
        "n" | "no" => Key::Set(false),
        "" => Key::Submit,
        "k" => Key::Prev,
        "j" => Key::Next,
        _ => Key::Other,
    }
}

/// Raw terminal input until dropped.
#[cfg(unix)]
struct RawMode(libc::termios);

#[cfg(unix)]
impl RawMode {
    fn enter() -> Option<RawMode> {
        // SAFETY: tcgetattr/tcsetattr on stdin with a zeroed termios they fill in.
        unsafe {
            let mut saved: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut saved) != 0 {
                return None;
            }
            let mut raw = saved;
            raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
            raw.c_cc[libc::VMIN] = 1;
            raw.c_cc[libc::VTIME] = 0;
            (libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) == 0)
                .then_some(RawMode(saved))
        }
    }
}

#[cfg(unix)]
impl Drop for RawMode {
    fn drop(&mut self) {
        // SAFETY: restores the attributes read in `enter`.
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.0);
        }
    }
}
