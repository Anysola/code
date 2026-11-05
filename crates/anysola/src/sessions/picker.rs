//! The `session export` picker: clack-style framing on stderr (stdout carries the export), arrow
//! keys to move, enter to pick, Esc or Ctrl+C to cancel.

use std::io::Write;

use anysola_core::storage::SessionSummary;

pub enum Picked {
    Session(String),
    /// Nothing to pick from.
    None,
    Cancelled,
    /// The terminal could not be put into raw mode.
    NoTerminal,
}

const VISIBLE: usize = 10;

fn write(text: &str) {
    let mut err = std::io::stderr().lock();
    let _ = err.write_all(text.as_bytes());
    let _ = err.flush();
}

pub fn pick_session(sessions: &[SessionSummary]) -> Picked {
    write("┌  Export session\n");
    if sessions.is_empty() {
        write("│\n■  No sessions found\n│\n└  Done\n\n");
        return Picked::None;
    }
    let options: Vec<String> = sessions
        .iter()
        .map(|s| {
            let tail = &s.id[s.id.len().saturating_sub(8)..];
            format!(
                "{} ({} - {tail})",
                s.title,
                super::locale_string(s.time_updated)
            )
        })
        .collect();
    let Some(_raw) = raw::Mode::enter() else {
        return Picked::NoTerminal;
    };
    let mut cursor: usize = 0;
    let mut drawn = 0;
    loop {
        let first = cursor.saturating_sub(VISIBLE - 1);
        let mut frame = String::new();
        if drawn > 0 {
            frame.push_str(&format!("\x1b[{drawn}A\r\x1b[J"));
        }
        frame.push_str("│\n◆  Select session to export\n");
        for (index, option) in options.iter().enumerate().skip(first).take(VISIBLE) {
            let mark = if index == cursor { "●" } else { "○" };
            frame.push_str(&format!("│  {mark} {option}\n"));
        }
        frame.push_str("└\n");
        drawn = frame.matches('\n').count();
        write(&frame);
        match raw::key() {
            raw::Key::Up => cursor = cursor.checked_sub(1).unwrap_or(options.len() - 1),
            raw::Key::Down => cursor = (cursor + 1) % options.len(),
            raw::Key::Enter => {
                write(&format!(
                    "\x1b[{drawn}A\r\x1b[J│\n◇  Select session to export\n│  {}\n│\n└  Exporting session...\n\n",
                    sessions[cursor].title
                ));
                return Picked::Session(sessions[cursor].id.clone());
            }
            raw::Key::Cancel => {
                write(&format!(
                    "\x1b[{drawn}A\r\x1b[J│\n■  Select session to export\n│\n└  Cancelled\n\n"
                ));
                return Picked::Cancelled;
            }
            raw::Key::Other => {}
        }
    }
}

mod raw {
    use std::io::Read;

    pub enum Key {
        Up,
        Down,
        Enter,
        Cancel,
        Other,
    }

    /// Raw input on stdin until dropped.
    pub struct Mode {
        saved: libc::termios,
    }

    impl Mode {
        pub fn enter() -> Option<Mode> {
            // SAFETY: tcgetattr/tcsetattr only read and write the termios struct we own.
            unsafe {
                let mut saved: libc::termios = std::mem::zeroed();
                if libc::tcgetattr(0, &mut saved) != 0 {
                    return None;
                }
                let mut raw = saved;
                libc::cfmakeraw(&mut raw);
                raw.c_oflag = saved.c_oflag;
                (libc::tcsetattr(0, libc::TCSANOW, &raw) == 0).then_some(Mode { saved })
            }
        }
    }

    impl Drop for Mode {
        fn drop(&mut self) {
            // SAFETY: restores the settings read in `enter`.
            unsafe {
                libc::tcsetattr(0, libc::TCSANOW, &self.saved);
            }
        }
    }

    pub fn key() -> Key {
        let mut stdin = std::io::stdin().lock();
        let mut byte = [0u8; 1];
        if stdin.read(&mut byte).unwrap_or(0) == 0 {
            return Key::Cancel;
        }
        match byte[0] {
            b'\r' | b'\n' => Key::Enter,
            3 | 4 => Key::Cancel,
            0x1b => {
                let mut rest = [0u8; 2];
                match stdin.read(&mut rest) {
                    Ok(2) if rest == *b"[A" => Key::Up,
                    Ok(2) if rest == *b"[B" => Key::Down,
                    Ok(_) => Key::Other,
                    Err(_) => Key::Cancel,
                }
            }
            _ => Key::Other,
        }
    }
}
