//! Drawing a `State` into a ratatui buffer.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use crate::state::{Route, State, Variant};
use crate::text::{clip, width};
use crate::theme::Theme;

mod home;
mod prompt;
mod prompts;
mod session;

pub use prompt::{max_rows, textarea_width};

/// Width of the session pane's content box.
pub fn session_content_width(state: &State) -> usize {
    let pad = if state.width < 44 { 1 } else { 2 };
    session::pane_width(state).saturating_sub(2 * pad) as usize
}

/// How many rows the session's transcript lays out to at `width`.
pub fn transcript_rows(
    state: &State,
    s: &crate::state::Session,
    theme: &Theme,
    width: usize,
) -> usize {
    session::transcript(state, s, theme, width).len()
}

/// Where the terminal cursor goes after drawing.
pub struct Drawn {
    pub cursor: Option<(u16, u16)>,
}

pub struct Canvas<'a> {
    pub buf: &'a mut Buffer,
    pub theme: &'a Theme,
}

impl Canvas<'_> {
    pub fn style(&self, path: &str) -> Style {
        Style::new().fg(self.theme.fg(path))
    }

    /// Writes `text` at `(x, y)`, clipped to `max` columns and the buffer; returns the end column.
    pub fn put(&mut self, x: u16, y: u16, text: &str, style: Style, max: usize) -> u16 {
        let area = self.buf.area;
        if y >= area.bottom() || x >= area.right() {
            return x;
        }
        let room = max.min((area.right() - x) as usize);
        let text = clip(text, room);
        self.buf.set_stringn(x, y, &text, room, style);
        x + width(&text) as u16
    }

    pub fn spans(&mut self, x: u16, y: u16, spans: &[Span], max: usize) -> u16 {
        let mut at = x;
        let end = x as usize + max;
        for span in spans {
            let room = end.saturating_sub(at as usize);
            if room == 0 {
                break;
            }
            at = self.put(at, y, &span.content, span.style, room);
        }
        at
    }

    pub fn fill(&mut self, rect: Rect, bg: Color) {
        let rect = rect.intersection(self.buf.area);
        for y in rect.top()..rect.bottom() {
            for x in rect.left()..rect.right() {
                let cell = &mut self.buf[(x, y)];
                cell.set_symbol(" ");
                cell.set_bg(bg);
            }
        }
    }

    pub fn bg(&mut self, rect: Rect, bg: Color) {
        let rect = rect.intersection(self.buf.area);
        for y in rect.top()..rect.bottom() {
            for x in rect.left()..rect.right() {
                self.buf[(x, y)].set_bg(bg);
            }
        }
    }
}

/// Rounds a layout coordinate the way the 0.5.1 layout engine does (halves up).
pub fn round(value: f64) -> u16 {
    (value + 0.5).floor().max(0.0) as u16
}

pub fn draw(state: &State, theme: &Theme, buf: &mut Buffer) -> Drawn {
    let area = buf.area;
    let mut c = Canvas { buf, theme };
    let base = Style::new()
        .fg(theme.fg("text.base"))
        .bg(theme.fg("background.base"));
    c.buf.set_style(area, base);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            c.buf[(x, y)].set_symbol(" ");
        }
    }
    if !state.tabs.is_empty() {
        tabs(&mut c, state);
    }
    let cursor = match &state.route {
        Route::Home => home::draw(&mut c, state),
        Route::Session(id) => match state.sessions.get(id) {
            Some(session) => session::draw(&mut c, state, session),
            None => None,
        },
    };
    if let Some(title) = &state.dialog {
        dialog(&mut c, title);
    }
    if let Some(toast) = &state.toast {
        self::toast(&mut c, state, toast);
    }
    Drawn {
        cursor: cursor.filter(|_| state.dialog.is_none()),
    }
}

const TAB_WIDTH: usize = 22;
const TAB_MAX_WIDTH: usize = 32;
const TAB_MIN_WIDTH: usize = 8;
const ADD_TAB_WIDTH: usize = 3;

/// The horizontal tab strip: open sessions, then `+ New session` at home or `+` in a session.
fn tabs(c: &mut Canvas, state: &State) {
    let active = match &state.route {
        Route::Session(id) => Some(id.as_str()),
        Route::Home => None,
    };
    let home = active.is_none();
    let count = state.tabs.len() + usize::from(home);
    let available = (state.width as usize).saturating_sub(ADD_TAB_WIDTH);
    let visible = if home {
        (available / TAB_MIN_WIDTH).clamp(1, count)
    } else {
        (1 + available.saturating_sub(TAB_WIDTH) / TAB_MIN_WIDTH).clamp(1, count)
    };
    let total = if available >= TAB_WIDTH * visible {
        available.min(TAB_MAX_WIDTH * visible)
    } else {
        available
    };
    let each = total / visible;
    let remainder = total - each * visible;
    let mut x = 0usize;
    let muted = c.style("text.muted");
    let base = c.style("text.base");
    for index in 0..visible {
        let w = each + usize::from(index < remainder);
        match state.tabs.get(index) {
            Some(tab) => {
                let style = if Some(tab.session_id.as_str()) == active {
                    base
                } else {
                    muted
                };
                if tab.attention {
                    c.put(
                        x as u16 + 1,
                        0,
                        "!",
                        c.style("text.feedback.warning.base"),
                        1,
                    );
                }
                c.put(x as u16 + 3, 0, &tab.title, style, w.saturating_sub(3));
            }
            None => {
                c.put(x as u16 + 1, 0, "+ New session", muted, w.saturating_sub(1));
            }
        }
        x += w;
    }
    if !home {
        c.put(x as u16 + 1, 0, "+", muted, 1);
    }
}

fn toast(c: &mut Canvas, state: &State, toast: &crate::state::Toast) {
    let w = 60.min(state.width.saturating_sub(6) as usize);
    let color = match toast.variant {
        Variant::Info => "text.feedback.info.base",
        Variant::Success => "text.feedback.success.base",
        Variant::Warning => "text.feedback.warning.base",
        Variant::Error => "text.feedback.error.base",
    };
    let inner = w.saturating_sub(6);
    let mut lines: Vec<(String, Style)> = Vec::new();
    if let Some(title) = &toast.title {
        lines.push((
            title.clone(),
            c.style("text.base").add_modifier(Modifier::BOLD),
        ));
    }
    for line in crate::text::wrap(&toast.message, inner) {
        lines.push((line, c.style("text.base")));
    }
    let h = lines.len() as u16 + 2;
    let x = state.width.saturating_sub(2 + w as u16);
    let rect = Rect::new(x, 1, w as u16, h);
    c.fill(rect, c.theme.fg("background.raised.high"));
    let border = c.style(color);
    for y in rect.top()..rect.bottom() {
        c.put(rect.left(), y, "┃", border, 1);
        c.put(rect.right() - 1, y, "┃", border, 1);
    }
    for (i, (text, style)) in lines.iter().enumerate() {
        c.put(x + 3, 2 + i as u16, text, *style, inner);
    }
}

/// The frame of a dialog this build does not have yet.
fn dialog(c: &mut Canvas, title: &str) {
    let area = c.buf.area;
    let backdrop = Color::Rgb(0, 0, 0);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut c.buf[(x, y)];
            cell.set_fg(c.theme.fg("text.faint"));
            cell.set_bg(backdrop);
        }
    }
    let w = 60.min(area.width.saturating_sub(4));
    let x = (area.width - w) / 2;
    let y = area.height / 4;
    let rect = Rect::new(x, y, w, 5);
    c.fill(rect, c.theme.term(c.theme.dialog.get("background.base")));
    let bold = c.style("text.base").add_modifier(Modifier::BOLD);
    c.put(x + 4, y + 1, title, bold, w as usize - 12);
    c.put(x + w - 7, y + 1, "esc", c.style("text.muted"), 3);
    c.put(
        x + 4,
        y + 3,
        "Not available in this build yet.",
        c.style("text.muted"),
        w as usize - 8,
    );
}
