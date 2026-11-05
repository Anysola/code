//! Terminal setup and teardown (raw mode, alternate screen, mouse, bracketed paste, Kitty key
//! flags), panic restore, the window title, OSC 52 copies and the OSC 11 background query.

use std::io::{self, Stdout, Write};

use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
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
    execute!(out, EnterAlternateScreen, EnableBracketedPaste)?;
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

/// Copies through the terminal (OSC 52), which also works over SSH.
pub fn copy(text: &str) {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    let mut out = io::stdout();
    let _ = write!(out, "\x1b]52;c;{encoded}\x07");
    let _ = out.flush();
}

/// Asks the terminal for its background (OSC 11) and picks light or dark by brightness; dark
/// when it does not answer in time. Runs before the input reader starts so the reply is not
/// read as keys.
#[cfg(unix)]
pub fn detect_mode(wait_ms: i32) -> Mode {
    use std::io::Read;
    use std::os::fd::AsRawFd;
    if enable_raw_mode().is_err() {
        return Mode::Dark;
    }
    let mut out = io::stdout();
    let _ = write!(out, "\x1b]11;?\x07");
    let _ = out.flush();
    let stdin = io::stdin();
    let fd = stdin.as_raw_fd();
    let mut reply = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(wait_ms as u64);
    loop {
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
        if reply.contains(&0x07) || reply.windows(2).any(|w| w == b"\x1b\\") {
            break;
        }
    }
    let _ = disable_raw_mode();
    parse_background(&String::from_utf8_lossy(&reply)).unwrap_or(Mode::Dark)
}

#[cfg(not(unix))]
pub fn detect_mode(_wait_ms: i32) -> Mode {
    Mode::Dark
}

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
