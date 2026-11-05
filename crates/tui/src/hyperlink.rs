//! OSC 8 hyperlinks. A link rides on its cells as their underline colour, so ratatui's diff and
//! every buffer copy keep it; [`Backend`] wraps each linked cell in OSC 8 and clears that colour
//! before the terminal sees it (nothing else in the TUI sets underline colours).

use std::io::{self, Write};
use std::sync::{Mutex, OnceLock};

use ratatui::backend::{ClearType, CrosstermBackend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::style::{Color, Style};

static URLS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// URLs longer than this are not sent as OSC 8 (`MAX_LINK_URL_BYTES` in OpenTUI).
pub const MAX_URL_BYTES: usize = 512;

fn urls() -> std::sync::MutexGuard<'static, Vec<String>> {
    URLS.lock().unwrap_or_else(|e| e.into_inner())
}

/// `style` with its cells linked to `url`.
pub fn mark(style: Style, url: &str) -> Style {
    let mut list = urls();
    let index = match list.iter().position(|u| u == url) {
        Some(index) => index,
        None => {
            list.push(url.to_string());
            list.len() - 1
        }
    };
    let id = index as u32 + 1;
    style.underline_color(Color::Rgb((id >> 16) as u8, (id >> 8) as u8, id as u8))
}

/// The URL a drawn cell links to.
pub fn url(cell: &Cell) -> Option<String> {
    let Color::Rgb(a, b, c) = cell.underline_color else {
        return None;
    };
    let id = (u32::from(a) << 16) | (u32::from(b) << 8) | u32::from(c);
    urls().get(id.checked_sub(1)? as usize).cloned()
}

/// Whether the terminal shows OSC 8 hyperlinks (otherwise links print ` (url)` after the label).
/// Apple Terminal and multiplexers swallow or garble them, so they count as unsupported.
pub fn supported() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        let var = |name: &str| std::env::var(name).unwrap_or_default();
        detect(
            &var("TERM_PROGRAM"),
            &var("TERM"),
            &var("VTE_VERSION"),
            !var("WT_SESSION").is_empty(),
            !var("TMUX").is_empty(),
        )
    })
}

fn detect(program: &str, term: &str, vte: &str, windows_terminal: bool, tmux: bool) -> bool {
    if tmux || term.starts_with("screen") || program == "Apple_Terminal" {
        return false;
    }
    let program = program.to_ascii_lowercase();
    let known = [
        "iterm.app",
        "wezterm",
        "ghostty",
        "vscode",
        "warpterminal",
        "rio",
        "contour",
        "zed",
    ];
    windows_terminal
        || known.contains(&program.as_str())
        || [
            "xterm-kitty",
            "xterm-ghostty",
            "foot",
            "alacritty",
            "contour",
            "wezterm",
        ]
        .iter()
        .any(|t| term.starts_with(t))
        || vte.parse::<u32>().is_ok_and(|v| v >= 5000)
}

/// The crossterm backend with OSC 8 around linked cells.
pub struct Backend<W: Write> {
    inner: CrosstermBackend<W>,
    links: bool,
}

impl<W: Write> Backend<W> {
    pub fn new(writer: W, links: bool) -> Self {
        Backend {
            inner: CrosstermBackend::new(writer),
            links,
        }
    }
}

impl<W: Write> ratatui::backend::Backend for Backend<W> {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let mut run: Vec<(u16, u16, Cell)> = Vec::new();
        let mut open: Option<String> = None;
        for (x, y, cell) in content {
            let link = if self.links { url(cell) } else { None };
            let mut plain = cell.clone();
            plain.underline_color = Color::Reset;
            if link != open {
                self.inner.draw(run.iter().map(|(x, y, c)| (*x, *y, c)))?;
                run.clear();
                let target = link.as_deref().filter(|u| u.len() <= MAX_URL_BYTES);
                write!(self.inner, "\x1b]8;;{}\x1b\\", target.unwrap_or(""))?;
                open = link;
            }
            run.push((x, y, plain));
        }
        self.inner.draw(run.iter().map(|(x, y, c)| (*x, *y, c)))?;
        if open.is_some() {
            write!(self.inner, "\x1b]8;;\x1b\\")?;
        }
        Ok(())
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> io::Result<()> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> io::Result<Size> {
        self.inner.size()
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> io::Result<()> {
        ratatui::backend::Backend::flush(&mut self.inner)
    }
}

#[cfg(test)]
mod tests {
    use ratatui::backend::Backend as _;

    use super::*;

    #[test]
    fn cells_carry_their_link_to_the_terminal() {
        let style = mark(Style::new(), "https://example.com/a");
        let mut cell = Cell::new("x");
        cell.set_style(style);
        assert_eq!(url(&cell).as_deref(), Some("https://example.com/a"));
        let mut bytes = Vec::new();
        let mut backend = Backend::new(&mut bytes, true);
        let plain = Cell::new("y");
        backend
            .draw([(0, 0, &cell), (1, 0, &cell), (2, 0, &plain)].into_iter())
            .unwrap();
        ratatui::backend::Backend::flush(&mut backend).unwrap();
        drop(backend);
        let out = String::from_utf8(bytes).unwrap();
        let open = out.find("\x1b]8;;https://example.com/a\x1b\\").unwrap();
        let close = out.rfind("\x1b]8;;\x1b\\").unwrap();
        assert!(open < out.find('x').unwrap() && close > out.rfind('x').unwrap());
        assert!(close < out.find('y').unwrap());
        assert!(
            !out.contains("\x1b[58"),
            "the carrier colour never reaches the terminal"
        );
    }

    #[test]
    fn detection() {
        assert!(detect("ghostty", "xterm-ghostty", "", false, false));
        assert!(detect("", "xterm-256color", "7600", false, false));
        assert!(detect("", "", "", true, false));
        assert!(!detect(
            "Apple_Terminal",
            "xterm-256color",
            "",
            false,
            false
        ));
        assert!(!detect("iTerm.app", "screen-256color", "", false, true));
        assert!(!detect("", "xterm-256color", "", false, false));
    }
}
