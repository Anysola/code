//! The crash screen (`component/error-component.tsx`): the error, a scrollable stack trace and
//! Copy report / Restart / Quit, in a fixed palette since the theme may be what failed.

use std::sync::Mutex;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

const ISSUES: &str = "https://github.com/Anysola/code/issues/new?template=bug-report.yml";
const MAX_URL: usize = 6000;

/// The last panic's message and backtrace, kept by the panic hook for the crash screen.
static LAST: Mutex<Option<(String, String)>> = Mutex::new(None);

pub fn record(info: &std::panic::PanicHookInfo<'_>) {
    let message = match info.payload().downcast_ref::<&str>() {
        Some(text) => text.to_string(),
        None => info
            .payload()
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_else(|| "An unknown error occurred.".into()),
    };
    let stack = format!(
        "{}\n{}",
        info.location().map(|l| l.to_string()).unwrap_or_default(),
        std::backtrace::Backtrace::force_capture()
    );
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some((message, stack));
}

pub fn take() -> (String, String) {
    LAST.lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .unwrap_or_else(|| {
            (
                "An unknown error occurred.".into(),
                "No stack trace available.".into(),
            )
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Copy {
    Idle,
    Copied,
    Failed,
}

pub struct Screen {
    pub message: String,
    pub stack: String,
    pub selected: usize,
    pub copy: Copy,
    pub scroll: usize,
    pub light: bool,
}

struct Palette {
    bg: Color,
    element: Color,
    border: Color,
    text: Color,
    muted: Color,
    primary: Color,
    on_primary: Color,
    error: Color,
    success: Color,
}

fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn palette(light: bool) -> Palette {
    if light {
        Palette {
            bg: rgb(0xffffff),
            element: rgb(0xf5f5f5),
            border: rgb(0xd4d4d4),
            text: rgb(0x1a1a1a),
            muted: rgb(0x8a8a8a),
            primary: rgb(0x3b7dd8),
            on_primary: rgb(0xffffff),
            error: rgb(0xd1383d),
            success: rgb(0x3d9a57),
        }
    } else {
        Palette {
            bg: rgb(0x0a0a0a),
            element: rgb(0x1e1e1e),
            border: rgb(0x3c3c3c),
            text: rgb(0xeeeeee),
            muted: rgb(0x808080),
            primary: rgb(0xfab283),
            on_primary: rgb(0x0a0a0a),
            error: rgb(0xe06c75),
            success: rgb(0x7fd88f),
        }
    }
}

/// `Anysola Code <ver>`.
pub fn version_line() -> String {
    anysola_core::version::version_line()
}

/// The pre-filled GitHub issue for the report, its stack cut to keep the link under 6000.
pub fn issue_url(message: &str, stack: &str) -> String {
    let head = format!(
        "The Anysola Code TUI crashed with an unexpected error.\n\n**Error:** {message}\n\n**Stack trace:**\n"
    );
    let build = |body: &str| {
        let mut url = url::Url::parse(ISSUES).expect("valid issue URL");
        url.query_pairs_mut()
            .append_pair("title", &format!("TUI crash: {message}"))
            .append_pair("opencode-version", &version_line())
            .append_pair("os", std::env::consts::OS)
            .append_pair(
                "terminal",
                &std::env::var("TERM_PROGRAM")
                    .or_else(|_| std::env::var("TERM"))
                    .unwrap_or_else(|_| "unknown".into()),
            )
            .append_pair(
                "reproduce",
                "Reported automatically from the Anysola Code crash screen. If you can, describe what you were doing when it crashed.",
            )
            .append_pair("description", &format!("{head}```\n{body}\n```"));
        url.to_string()
    };
    let full = build(stack);
    if full.len() <= MAX_URL {
        return full;
    }
    let marker = "\n… (truncated)";
    let chars: Vec<char> = stack.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let body: String = chars[..mid].iter().collect::<String>() + marker;
        if build(&body).len() <= MAX_URL {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    build(&(chars[..lo].iter().collect::<String>() + marker))
}

fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, style: Style, room: u16) {
    if y < buf.area.height && x < buf.area.width {
        buf.set_stringn(x, y, text, room as usize, style);
    }
}

fn frame(buf: &mut Buffer, rect: Rect, color: Color, title: &str, bg: Color) {
    let style = Style::new().fg(color).bg(bg);
    let (x2, y2) = (rect.right() - 1, rect.bottom() - 1);
    for x in rect.left()..=x2 {
        put(buf, x, rect.top(), "─", style, 1);
        put(buf, x, y2, "─", style, 1);
    }
    for y in rect.top()..=y2 {
        put(buf, rect.left(), y, "│", style, 1);
        put(buf, x2, y, "│", style, 1);
    }
    put(buf, rect.left(), rect.top(), "╭", style, 1);
    put(buf, x2, rect.top(), "╮", style, 1);
    put(buf, rect.left(), y2, "╰", style, 1);
    put(buf, x2, y2, "╯", style, 1);
    put(
        buf,
        rect.left() + 1,
        rect.top(),
        title,
        style,
        rect.width - 2,
    );
}

impl Screen {
    pub const ACTIONS: [(&'static str, &'static str); 3] =
        [("c", "Copy report"), ("r", "Restart"), ("q", "Quit")];

    fn label(&self, index: usize) -> &'static str {
        match (index, self.copy) {
            (0, Copy::Copied) => "✓ Copied",
            (0, Copy::Failed) => "Copy failed",
            (i, _) => Self::ACTIONS[i].1,
        }
    }

    pub fn draw(&self, buf: &mut Buffer) {
        let p = palette(self.light);
        let area = buf.area;
        let base = Style::new().bg(p.bg);
        for y in 0..area.height {
            put(
                buf,
                0,
                y,
                &" ".repeat(area.width as usize),
                base,
                area.width,
            );
        }
        let width = 84.min(area.width.saturating_sub(4).max(24)).min(area.width);
        let left = (area.width - width) / 2;
        let center =
            |text: &str| left + (width.saturating_sub(crate::text::width(text) as u16)) / 2;
        let mut y = 1;
        let title = "Anysola Code crashed";
        put(
            buf,
            center(title),
            y,
            title,
            base.fg(p.text).add_modifier(Modifier::BOLD),
            width,
        );
        y += 1;
        if area.height >= 18 {
            let sub = "An unexpected error stopped the session.";
            put(buf, center(sub), y, sub, base.fg(p.muted), width);
            y += 1;
        }
        y += 1;
        let lines = crate::text::wrap(&self.message, width.saturating_sub(6) as usize);
        let box_h = lines.len() as u16 + 2;
        frame(
            buf,
            Rect::new(left, y, width, box_h),
            p.error,
            " Error ",
            p.bg,
        );
        for (i, line) in lines.iter().enumerate() {
            put(
                buf,
                left + 3,
                y + 1 + i as u16,
                line,
                base.fg(p.text),
                width - 6,
            );
        }
        y += box_h + 1;
        let labels: Vec<String> = (0..3)
            .map(|i| format!("  {:^11}  ", self.label(i)))
            .collect();
        let total: u16 = labels
            .iter()
            .map(|l| crate::text::width(l) as u16)
            .sum::<u16>()
            + 4;
        let mut x = left + width.saturating_sub(total) / 2;
        for (i, label) in labels.iter().enumerate() {
            let copy_color = match (i, self.copy) {
                (0, Copy::Copied) => Some(p.success),
                (0, Copy::Failed) => Some(p.error),
                _ => None,
            };
            let selected = i == self.selected;
            let bg = copy_color.unwrap_or(if selected { p.primary } else { p.element });
            let fg = if copy_color.is_some() || selected {
                p.on_primary
            } else {
                p.text
            };
            let w = crate::text::width(label) as u16;
            put(
                buf,
                x,
                y,
                label,
                Style::new().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
                w,
            );
            let key = Self::ACTIONS[i].0;
            let key_color = if selected { p.primary } else { p.muted };
            put(buf, x + w / 2, y + 1, key, base.fg(key_color), 1);
            x += w + 2;
        }
        y += 3;
        let footer = if area.height >= 20 { 2 } else { 0 };
        let stack_h = area.height.saturating_sub(y + footer + 1).max(3);
        frame(
            buf,
            Rect::new(left, y, width, stack_h),
            p.border,
            " Stack trace ",
            p.bg,
        );
        let scroll_hint = " ↑↓ scroll ";
        put(
            buf,
            left + width - 1 - crate::text::width(scroll_hint) as u16,
            y + stack_h - 1,
            scroll_hint,
            base.fg(p.muted),
            width,
        );
        let stack: Vec<String> = self
            .stack
            .lines()
            .flat_map(|l| crate::text::wrap(l, width.saturating_sub(4) as usize))
            .collect();
        for (i, line) in stack
            .iter()
            .skip(self.scroll)
            .take(stack_h.saturating_sub(2) as usize)
            .enumerate()
        {
            put(
                buf,
                left + 2,
                y + 1 + i as u16,
                line,
                base.fg(p.muted),
                width - 4,
            );
        }
        if footer > 0 {
            let note = match self.copy {
                Copy::Copied => "Report copied — paste it into a new GitHub issue.",
                Copy::Failed => "Clipboard write failed. Try again or report the crash manually.",
                Copy::Idle => "Copy the report and open a GitHub issue to help us fix this.",
            };
            let fy = area.height - 2;
            put(buf, center(note), fy - 1, note, base.fg(p.muted), width);
            let version = version_line();
            put(buf, center(&version), fy, &version, base.fg(p.muted), width);
        }
    }
}

/// What the person chose on the crash screen.
pub enum Choice {
    Restart,
    Quit,
}

/// Shows the crash screen until Restart or Quit.
pub fn show(
    terminal: &mut ratatui::Terminal<crate::term::Backend>,
    message: String,
    stack: String,
    light: bool,
) -> Choice {
    use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
    let mut screen = Screen {
        message,
        stack,
        selected: 0,
        copy: Copy::Idle,
        scroll: 0,
        light,
    };
    loop {
        let _ = terminal.draw(|frame| screen.draw(frame.buffer_mut()));
        let Ok(Event::Key(key)) = crossterm::event::read() else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        let action = match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => 2,
            KeyCode::Enter => screen.selected,
            KeyCode::Char('c') => 0,
            KeyCode::Char('r') => 1,
            KeyCode::Char('q') => 2,
            KeyCode::Left | KeyCode::BackTab => {
                screen.selected = (screen.selected + 2) % 3;
                continue;
            }
            KeyCode::Right | KeyCode::Tab => {
                screen.selected = (screen.selected + 1) % 3;
                continue;
            }
            KeyCode::Up => {
                screen.scroll = screen.scroll.saturating_sub(1);
                continue;
            }
            KeyCode::Down => {
                screen.scroll += 1;
                continue;
            }
            KeyCode::Home => {
                screen.scroll = 0;
                continue;
            }
            _ => continue,
        };
        match action {
            0 => {
                crate::term::copy(&issue_url(&screen.message, &screen.stack));
                screen.copy = Copy::Copied;
            }
            1 => return Choice::Restart,
            _ => return Choice::Quit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_link_is_prefilled_and_bounded() {
        let url = issue_url("boom", "at main");
        assert!(url.starts_with(ISSUES));
        assert!(url.contains("title=TUI+crash%3A+boom"));
        let long = issue_url("boom", &"frame\n".repeat(5000));
        assert!(long.len() <= MAX_URL);
        assert!(long.contains("truncated"));
    }

    #[test]
    fn screen_texts() {
        let screen = Screen {
            message: "index out of bounds".into(),
            stack: "src/app.rs:1:1".into(),
            selected: 0,
            copy: Copy::Idle,
            scroll: 0,
            light: false,
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
        screen.draw(&mut buf);
        let text: String = (0..30)
            .map(|y| {
                (0..100)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
                    + "\n"
            })
            .collect();
        for needle in [
            "Anysola Code crashed",
            "An unexpected error stopped the session.",
            "╭ Error ",
            "index out of bounds",
            "Copy report",
            "Restart",
            "Quit",
            "╭ Stack trace ",
            "↑↓ scroll",
            "Copy the report and open a GitHub issue to help us fix this.",
            "Anysola Code ",
        ] {
            assert!(text.contains(needle), "{needle}\n{text}");
        }
    }
}
