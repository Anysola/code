//! Drawing a `State` into a ratatui buffer.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use crate::state::{Route, State, Variant};
use crate::text::{clip, width};
use crate::theme::Theme;

mod dialog;
mod diffview;
mod dock;
mod home;
mod prompt;
mod prompts;
mod session;

pub use diffview::{patch_options, tree_width as diff_tree_width};
pub use home::WORDMARK;
pub use prompt::{max_rows, textarea_width};
pub use session::{
    SIDEBAR_WIDTH, pane_width, sidebar_overlay, sidebar_shown, terminal_size, terminal_width,
};
pub use session::{diff_docked, diff_width, onboarding_shown};

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

/// Where the terminal cursor goes after drawing, and what the drawn controls do when clicked.
pub struct Drawn {
    pub cursor: Option<(u16, u16)>,
    /// Clickable areas in drawing order: the last one containing a point is on top.
    pub hits: Vec<(Rect, Hit)>,
}

/// What a click on a drawn control does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hit {
    /// A horizontal tab, by index into `state.tabs`.
    Tab(usize),
    /// `+ New session` or `+` in the horizontal strip.
    NewTab,
    /// Acts as if the key (`esc`, `enter`, `ctrl+d`) were pressed.
    Key(String),
    /// Runs a command by id.
    Command(String),
    /// The open dialog's panel: clicks on it stay in the dialog, clicks outside close it.
    Panel,
    /// An option of the open list dialog, by index into its options.
    Option(usize),
    /// A button of the open dialog (`Cancel`, `Copy`), by position.
    Button(usize),
    /// A focusable field of the export dialog; `Format(json)` picks the format.
    Field(usize),
    Format(bool),
    /// An option of the pending permission or question, by position.
    Choice(usize),
    /// A tab of the composer dock, and one of its rows, by index.
    DockTab(usize),
    DockRow(usize),
    /// Dismisses the toast.
    CloseToast,
    /// Folds or unfolds the sidebar's todo list.
    ToggleTodos,
}

pub struct Canvas<'a> {
    pub buf: &'a mut Buffer,
    pub theme: &'a Theme,
    pub hits: Vec<(Rect, Hit)>,
}

impl<'a> Canvas<'a> {
    pub fn new(buf: &'a mut Buffer, theme: &'a Theme) -> Canvas<'a> {
        Canvas {
            buf,
            theme,
            hits: Vec::new(),
        }
    }
}

/// The control on top at `at`, among `hits` in drawing order.
pub fn hit_at(hits: &[(Rect, Hit)], at: (u16, u16)) -> Option<&Hit> {
    let at = ratatui::layout::Position::new(at.0, at.1);
    hits.iter()
        .rev()
        .find(|(r, _)| r.contains(at))
        .map(|(_, h)| h)
}

impl Canvas<'_> {
    /// Marks `rect` (clipped to the buffer) as the clickable area of `hit`.
    pub fn hit(&mut self, rect: Rect, hit: Hit) {
        let rect = rect.intersection(self.buf.area);
        if !rect.is_empty() {
            self.hits.push((rect, hit));
        }
    }

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

/// Draws the frame; with a vertical tab column the routes draw beside it, in their own
/// coordinates (`state.width` is what is left of the terminal).
pub fn draw(state: &State, theme: &Theme, buf: &mut Buffer) -> Drawn {
    if state.left == 0 {
        return draw_body(state, theme, buf);
    }
    let mut body = Buffer::empty(Rect::new(0, 0, state.width, state.height));
    let drawn = draw_body(state, theme, &mut body);
    for y in 0..state.height.min(buf.area.height) {
        for x in 0..state.width {
            if state.left + x < buf.area.width {
                buf[(state.left + x, y)] = body[(x, y)].clone();
            }
        }
    }
    vertical_tabs(&mut Canvas::new(buf, theme), state);
    // Hits stay in the routes' own coordinates, like the mouse handler reads them.
    Drawn {
        cursor: drawn.cursor.map(|(x, y)| (x + state.left, y)),
        hits: drawn.hits,
    }
}

/// The vertical tab column's width on a terminal this wide, when it fits (`ui/layout.ts`).
pub fn vertical_tabs_width(total: u16, preferred: u16) -> Option<u16> {
    let width = preferred.min(72).min(total.saturating_sub(44)).max(5);
    (total >= width + 64).then_some(width)
}

/// Which vertical tab a row of the column shows: `Some(index)`, or `None` for `+ New session`.
pub fn vertical_tab_at(state: &State, row: u16) -> Option<Option<usize>> {
    let compact = state.left < 12;
    let stride = if compact { 2 } else { 3 };
    let top = if compact { 2 } else { 1 };
    let offset = row.checked_sub(top)? as usize;
    let index = offset / stride;
    if index < state.tabs.len() {
        return (offset % stride < stride - 1).then_some(Some(index));
    }
    (offset == state.tabs.len() * stride).then_some(None)
}

/// The vertical tab column (`component/session-tabs.tsx:542-1273`): per tab a status or number
/// column and the title, then the project (and worktree branch); `+ New session` below.
fn vertical_tabs(c: &mut Canvas, state: &State) {
    let w = state.left;
    let bg = c.theme.fg("background.raised.base");
    c.fill(Rect::new(0, 0, w, state.height), bg);
    let selected_bg = c.theme.fg("background.action.primary.selected");
    let active = match &state.route {
        Route::Session(id) => Some(id.as_str()),
        Route::Home => None,
    };
    let compact = w < 12;
    let number_w = state.tabs.len().to_string().len().max(2);
    let mut y = if compact { 2 } else { 1 };
    for (index, tab) in state.tabs.iter().enumerate() {
        if y >= state.height {
            return;
        }
        let selected = active == Some(tab.session_id.as_str());
        let busy = state.sessions.get(&tab.session_id).is_some_and(|s| s.busy);
        let fg = if selected { "text.base" } else { "text.muted" };
        let mut style = c.style(fg);
        if selected {
            style = style.add_modifier(Modifier::BOLD);
        }
        let marker = if state.settings.tab_numbers {
            ((index + 1).to_string(), fg)
        } else {
            match (tab.attention, tab.question) {
                (true, true) => ("?".into(), "text.feedback.warning.base"),
                (true, false) => ("!".into(), "text.feedback.warning.base"),
                _ if busy && state.settings.animations => (
                    crate::transcript::rows::spinner_frame(state.now).into(),
                    "text.muted",
                ),
                _ if busy => ("⋯".into(), "text.muted"),
                _ if tab.unread => ("•".into(), "text.action.primary.base"),
                _ if compact => (
                    tab.title
                        .trim_start()
                        .chars()
                        .next()
                        .unwrap_or('U')
                        .to_string(),
                    fg,
                ),
                _ => (String::new(), fg),
            }
        };
        let row_bg = match (selected, compact) {
            (true, false) => selected_bg,
            (true, true) => c.theme.fg("background.raised.high"),
            _ => bg,
        };
        if compact {
            c.fill(Rect::new(0, y, w, 1), row_bg);
            let x = (w.saturating_sub(width(&marker.0) as u16)) / 2;
            c.put(x, y, &marker.0, c.style(marker.1).bg(row_bg), w as usize);
            y += 2;
            continue;
        }
        c.fill(Rect::new(0, y, w, 2.min(state.height - y)), row_bg);
        let mark_x = (number_w - width(&marker.0).min(number_w)) as u16;
        c.put(mark_x, y, &marker.0, c.style(marker.1).bg(row_bg), number_w);
        let title_x = number_w as u16 + 1;
        let room = (w as usize).saturating_sub(title_x as usize + 1);
        let title = if tab.title.is_empty() {
            "Untitled session"
        } else {
            &tab.title
        };
        c.put(title_x, y, title, style.bg(row_bg), room);
        if y + 1 < state.height {
            let directory = state
                .sessions
                .get(&tab.session_id)
                .map_or(state.directory.as_str(), |s| s.directory.as_str());
            let project = std::path::Path::new(directory)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let detail_fg = prompt::blend(c.theme.fg("text.muted"), row_bg, 0.35);
            c.put(
                title_x,
                y + 1,
                &project,
                Style::new().fg(detail_fg).bg(row_bg),
                room.saturating_sub(1),
            );
        }
        y += 3;
    }
    if y < state.height {
        let home = active.is_none();
        let row_bg = if home && !compact { selected_bg } else { bg };
        c.fill(Rect::new(0, y, w, 1), row_bg);
        let fg = if home { "text.base" } else { "text.muted" };
        let style = c.style(fg).bg(row_bg);
        if compact {
            c.put(w / 2, y, "+", style, 1);
        } else {
            c.put(1, y, "+", style, 1);
            c.put(3, y, "New session", style, (w as usize).saturating_sub(4));
        }
    }
}

fn draw_body(state: &State, theme: &Theme, buf: &mut Buffer) -> Drawn {
    let area = buf.area;
    let mut c = Canvas::new(buf, theme);
    let base = Style::new()
        .fg(theme.fg("text.base"))
        .bg(theme.fg("background.base"));
    c.buf.set_style(area, base);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            c.buf[(x, y)].set_symbol(" ");
        }
    }
    if state.strip() {
        tabs(&mut c, state);
    }
    let cursor = match &state.route {
        Route::Home => home::draw(&mut c, state),
        Route::Session(id) => match state.sessions.get(id) {
            Some(session) => {
                let cursor = session::draw(&mut c, state, session);
                session::terminal_cursor(state).or(cursor)
            }
            None => None,
        },
    };
    let cursor = match &state.diff {
        Some(view) if session::diff_docked(state) => {
            let x = session::pane_width(state);
            let top = u16::from(state.strip());
            let area = Rect::new(0, 0, state.width - x, state.height - top);
            let mut panel = Buffer::empty(area);
            let mut docked = Canvas::new(&mut panel, c.theme);
            diffview::draw(&mut docked, view);
            let hits = std::mem::take(&mut docked.hits);
            for (r, hit) in hits {
                c.hit(Rect::new(r.x + x, r.y + top, r.width, r.height), hit);
            }
            for (px, py) in (0..area.width).flat_map(|px| (0..area.height).map(move |py| (px, py)))
            {
                c.buf[(x + px, top + py)] = panel[(px, py)].clone();
            }
            cursor
        }
        Some(view) => {
            c.hits.clear();
            diffview::draw(&mut c, view);
            None
        }
        None => cursor,
    };
    let cursor = match state.session() {
        Some(session) if session::sidebar_overlay(state) => {
            session::overlay(&mut c, state, session);
            None
        }
        _ => cursor,
    };
    let cursor = match state.dialogs.last() {
        Some(top) => dialog::draw(&mut c, top),
        None => cursor,
    };
    if let Some(toast) = &state.toast {
        self::toast(&mut c, state, toast);
    }
    Drawn {
        cursor,
        hits: c.hits,
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
                let busy = state.sessions.get(&tab.session_id).is_some_and(|s| s.busy);
                let marker = match (tab.attention, tab.question) {
                    (true, true) => Some(("?", "text.feedback.warning.base")),
                    (true, false) => Some(("!", "text.feedback.warning.base")),
                    _ if busy && state.settings.animations => Some((
                        crate::transcript::rows::spinner_frame(state.now),
                        "text.muted",
                    )),
                    _ if busy => Some(("⋯", "text.muted")),
                    _ if tab.unread => Some(("•", "text.action.primary.base")),
                    _ => None,
                };
                if let Some((mark, color)) = marker {
                    c.put(x as u16 + 1, 0, mark, c.style(color), 1);
                }
                c.put(x as u16 + 3, 0, &tab.title, style, w.saturating_sub(3));
                c.hit(Rect::new(x as u16, 0, w as u16, 1), Hit::Tab(index));
            }
            None => {
                c.put(x as u16 + 1, 0, "+ New session", muted, w.saturating_sub(1));
                c.hit(Rect::new(x as u16, 0, w as u16, 1), Hit::NewTab);
            }
        }
        x += w;
    }
    if !home {
        c.put(x as u16 + 1, 0, "+", muted, 1);
        c.hit(Rect::new(x as u16, 0, ADD_TAB_WIDTH as u16, 1), Hit::NewTab);
    }
}

fn affordance(toast: &crate::state::Toast) -> String {
    toast
        .action
        .as_ref()
        .map_or("x".to_string(), |(label, _)| format!("› {label}"))
}

/// Where the toast sits: top right, as wide as its longest line allows.
pub fn toast_rect(state: &State) -> Option<Rect> {
    let toast = state.toast.as_ref()?;
    let max = 60.min(state.width as usize);
    let room = max.saturating_sub(9);
    let rows = toast.title.as_ref().map_or(0, |_| 2)
        + crate::text::wrap(&toast.message, room).len()
        + if state.toast_queue.is_empty() { 0 } else { 2 };
    let longest = toast
        .message
        .lines()
        .chain(toast.title.as_deref())
        .map(width)
        .max()
        .unwrap_or(0);
    // A toast whose text wraps takes the full width.
    let extra = width(&affordance(toast)).saturating_sub(1);
    let w = (longest + 9 + extra).min(max) as u16;
    let x = state.width.saturating_sub(w + 2);
    Some(Rect::new(x, 1, w, rows as u16 + 2))
}

/// The toast at the top right: coloured side borders, text, and the `x` close mark.
fn toast(c: &mut Canvas, state: &State, toast: &crate::state::Toast) {
    let color = match toast.variant {
        Variant::Info => "text.feedback.info.base",
        Variant::Success => "text.feedback.success.base",
        Variant::Warning => "text.feedback.warning.base",
        Variant::Error => "text.feedback.error.base",
    };
    let max = 60.min(state.width as usize);
    let room = max.saturating_sub(9);
    let mut lines: Vec<(String, Style)> = Vec::new();
    if let Some(title) = &toast.title {
        lines.push((
            title.clone(),
            c.style("text.base").add_modifier(Modifier::BOLD),
        ));
    }
    if !lines.is_empty() {
        lines.push((String::new(), c.style("text.base")));
    }
    for line in crate::text::wrap(&toast.message, room) {
        lines.push((line, c.style("text.base")));
    }
    if !state.toast_queue.is_empty() {
        lines.push((String::new(), c.style("text.muted")));
        let more = format!("+{} more", state.toast_queue.len());
        lines.push((more, c.style("text.muted")));
    }
    let affordance = affordance(toast);
    let Some(rect) = toast_rect(state) else {
        return;
    };
    let x = rect.x;
    c.fill(rect, c.theme.fg("background.raised.high"));
    let border = c.style(color);
    for y in rect.top()..rect.bottom() {
        c.put(rect.left(), y, "┃", border, 1);
        c.put(rect.right() - 1, y, "┃", border, 1);
    }
    let ax = rect.right() - 3 - width(&affordance) as u16;
    c.put(
        ax,
        2,
        &affordance,
        c.style("text.muted"),
        width(&affordance),
    );
    for (i, (text, style)) in lines.iter().enumerate() {
        c.put(x + 3, 2 + i as u16, text, *style, room);
    }
    // A toast with an action runs it from `› label`; one without closes on any click.
    if toast.action.is_none() {
        c.hit(rect, Hit::CloseToast);
    }
}
