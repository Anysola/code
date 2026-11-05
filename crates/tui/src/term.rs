//! Terminal setup and teardown (raw mode, alternate screen, mouse, bracketed paste, Kitty key
//! flags), panic restore, the window title, OSC 52 copies, the OSC 11 background and OSC 7501 support queries.

use std::io::{self, Stdout, Write};

use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, SetTitle, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::theme::Mode;

pub type Backend = CrosstermBackend<Stdout>;

/// Restores the terminal; safe to call more than once.
pub fn restore() {
    let mut out = io::stdout();
    let _ = execute!(
        out,
        PopKeyboardEnhancementFlags,
        DisableBracketedPaste,
        DisableFocusChange,
        DisableMouseCapture,
        SetTitle(""),
        LeaveAlternateScreen,
        crossterm::cursor::Show
    );
    let _ = disable_raw_mode();
}

pub fn enter(mouse: bool) -> io::Result<Terminal<Backend>> {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        crate::crash::record(info);
        restore();
        previous(info);
    }));
    resume(mouse)?;
    Terminal::new(CrosstermBackend::new(io::stdout()))
}

/// Puts the terminal (back) into the UI's modes, after start or a suspend.
pub fn resume(mouse: bool) -> io::Result<()> {
    enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(
        out,
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableFocusChange
    )?;
    if mouse {
        execute!(out, EnableMouseCapture)?;
    }
    if matches!(
        crossterm::terminal::supports_keyboard_enhancement(),
        Ok(true)
    ) {
        execute!(
            out,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
            )
        )?;
    }
    Ok(())
}

/// Runs `work` behind a full-screen line of text (the 0.5.x import before the UI starts).
pub fn behind<T>(text: &str, work: impl FnOnce() -> T) -> T {
    let mut out = io::stdout();
    let shown = execute!(out, EnterAlternateScreen, crossterm::cursor::Hide).is_ok();
    if shown && let Ok((w, h)) = crossterm::terminal::size() {
        let x = w.saturating_sub(crate::text::width(text) as u16) / 2;
        let _ = execute!(
            out,
            crossterm::cursor::MoveTo(x, h / 2),
            crossterm::style::Print(text)
        );
    }
    let result = work();
    if shown {
        let _ = execute!(out, LeaveAlternateScreen, crossterm::cursor::Show);
    }
    result
}

/// `Anysola Code` at home and for untitled sessions, else `Anysola | <title>` (40/37-char cut).
pub fn title(session_title: Option<&str>) -> String {
    match session_title.filter(|t| !t.is_empty() && !is_default_title(t)) {
        Some(title) if title.chars().count() > 40 => {
            format!("Anysola | {}…", title.chars().take(37).collect::<String>())
        }
        Some(title) => format!("Anysola | {title}"),
        None => "Anysola Code".to_string(),
    }
}

fn is_default_title(title: &str) -> bool {
    title.starts_with("New session - ") || title.starts_with("Child session - ")
}

pub fn set_title(text: &str) {
    let _ = execute!(io::stdout(), SetTitle(text));
}

/// Opens a URL in the browser; false when no opener could start.
pub fn open_url(url: &str) -> bool {
    let mut command = if cfg!(windows) {
        let mut command = std::process::Command::new("cmd");
        command.args(["/c", "start", ""]);
        command
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else {
        std::process::Command::new("xdg-open")
    };
    command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

/// Escape sequences straight to the terminal, outside the frame.
pub fn write_raw(text: &str) {
    let mut out = io::stdout();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

/// Copies through the terminal (OSC 52), which also works over SSH.
pub fn copy(text: &str) {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    let mut out = io::stdout();
    let _ = write!(out, "\x1b]52;c;{encoded}\x07");
    let _ = out.flush();
}

/// What the terminal answered at start.
pub struct Probe {
    /// From the OSC 11 background reply, when asked for and answered.
    pub mode: Option<Mode>,
    /// The terminal answered the OSC 7501 program-status query.
    pub program_status: bool,
}

/// Asks for program-status support (OSC 7501) and either the background (OSC 11, light or
/// dark by brightness) or, with the mode locked, device attributes, whose reply ends the wait
/// early. Runs before the input reader starts so the replies are not read as keys.
#[cfg(unix)]
pub fn probe(wait_ms: i32, background: bool) -> Probe {
    use std::io::Read;
    use std::os::fd::AsRawFd;
    let none = Probe {
        mode: None,
        program_status: false,
    };
    if enable_raw_mode().is_err() {
        return none;
    }
    let mut out = io::stdout();
    let last = if background {
        "\x1b]11;?\x07"
    } else {
        "\x1b[c"
    };
    let _ = write!(out, "\x1b]7501;?\x1b\\{last}");
    let _ = out.flush();
    let stdin = io::stdin();
    let fd = stdin.as_raw_fd();
    let mut reply = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(wait_ms as u64);
    while !answered(&reply, background) {
        let left = deadline
            .saturating_duration_since(std::time::Instant::now())
            .as_millis() as i32;
        if left <= 0 {
            break;
        }
        let mut poll = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd for the duration of the call.
        if unsafe { libc::poll(&mut poll, 1, left) } <= 0 {
            break;
        }
        let mut byte = [0u8; 64];
        let Ok(n) = stdin.lock().read(&mut byte) else {
            break;
        };
        reply.extend_from_slice(&byte[..n]);
    }
    let _ = disable_raw_mode();
    let text = String::from_utf8_lossy(&reply);
    Probe {
        mode: background.then(|| parse_background(&text)).flatten(),
        program_status: text.contains("\x1b]7501;"),
    }
}

/// The reply to the last query is complete.
#[cfg(any(unix, test))]
fn answered(reply: &[u8], background: bool) -> bool {
    let text = String::from_utf8_lossy(reply);
    if background {
        text.split("\x1b]11;")
            .nth(1)
            .is_some_and(|rest| rest.contains('\x07') || rest.contains("\x1b\\"))
    } else {
        text.split("\x1b[?")
            .nth(1)
            .is_some_and(|rest| rest.contains('c'))
    }
}

#[cfg(not(unix))]
pub fn probe(_wait_ms: i32, _background: bool) -> Probe {
    Probe {
        mode: None,
        program_status: false,
    }
}

#[cfg(any(unix, test))]
fn parse_background(reply: &str) -> Option<Mode> {
    let rgb = reply.split("rgb:").nth(1)?;
    let channel = |s: &str| -> Option<f64> {
        let hex: String = s.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        let max = 16f64.powi(hex.len() as i32) - 1.0;
        Some(f64::from(u32::from_str_radix(&hex, 16).ok()?) / max * 255.0)
    };
    let mut parts = rgb.split('/');
    let (r, g, b) = (
        channel(parts.next()?)?,
        channel(parts.next()?)?,
        channel(parts.next()?)?,
    );
    let brightness = (299.0 * r + 587.0 * g + 114.0 * b) / 1000.0;
    Some(if brightness > 128.0 {
        Mode::Light
    } else {
        Mode::Dark
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_waits_for_the_last_reply() {
        assert!(!answered(b"\x1b]7501;?:states=idle\x1b\\", true));
        assert!(answered(b"\x1b]7501;?\x1b\\\x1b]11;rgb:0/0/0\x07", true));
        assert!(answered(b"\x1b[?62;22c", false));
        assert!(!answered(b"\x1b[?62", false));
    }

    #[test]
    fn titles() {
        assert_eq!(title(None), "Anysola Code");
        assert_eq!(
            title(Some("New session - 2026-11-05T10:00:00.000Z")),
            "Anysola Code"
        );
        assert_eq!(title(Some("Fix typo")), "Anysola | Fix typo");
        let long = "a".repeat(41);
        assert_eq!(title(Some(&long)), format!("Anysola | {}…", "a".repeat(37)));
    }

    #[test]
    fn background_replies() {
        assert_eq!(
            parse_background("\x1b]11;rgb:ffff/ffff/ffff\x07"),
            Some(Mode::Light)
        );
        assert_eq!(
            parse_background("\x1b]11;rgb:1111/0f0f/0d0d\x1b\\"),
            Some(Mode::Dark)
        );
        assert_eq!(parse_background(""), None);
    }
}
