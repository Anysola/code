//! The session screen: transcript (sticky to the bottom), the status row, then the prompt or a
//! permission/question prompt; the sidebar docks on the right above 120 columns.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use super::{Canvas, prompt, prompts};
use crate::state::{DiffView, Session, State};
use crate::text::{self, width};
use crate::transcript::rows::{self, Ctx, Row};

pub const SIDEBAR_WIDTH: u16 = 42;

/// Columns of the session pane (the sidebar takes 42 on wide terminals).
/// The diff viewer sits beside the session (0.5.1 docks it from 108 columns: 64 + 44).
pub fn diff_docked(state: &State) -> bool {
    state.diff.as_ref().is_some_and(|d| !d.fullscreen)
        && matches!(state.route, crate::state::Route::Session(_))
        && state.pane.is_none()
        && state.width >= 108
}

/// The width the diff viewer lays out in.
pub fn diff_width(state: &State) -> u16 {
    if diff_docked(state) {
        terminal_width(state)
    } else {
        state.width
    }
}

/// The sidebar sits in the right pane (docked when wide, else as an overlay).
pub fn sidebar_shown(state: &State) -> bool {
    state.pane.is_none()
        && !diff_docked(state)
        && (state.sidebar_open || (state.settings.sidebar && state.width > 120))
}

/// The sidebar is up as an overlay over a narrow session (escape or ctrl+c close it).
pub fn sidebar_overlay(state: &State) -> bool {
    matches!(state.route, crate::state::Route::Session(_))
        && sidebar_shown(state)
        && state.width <= 120
}

pub fn pane_width(state: &State) -> u16 {
    if state.pane.is_some() || diff_docked(state) {
        state.width - terminal_width(state)
    } else if sidebar_shown(state) && state.width > 120 {
        state.width - SIDEBAR_WIDTH
    } else {
        state.width
    }
}

pub fn transcript(
    state: &State,
    session: &Session,
    theme: &crate::theme::Theme,
    width: usize,
) -> Vec<Row> {
    let colors = |agent: &str| theme.term(theme.categorical(state.agent_index(agent)));
    let ctx = Ctx {
        theme,
        width,
        now: state.now,
        animations: state.settings.animations,
        split: match state.settings.diff_view {
            DiffView::Auto => width > 120,
            DiffView::Split => true,
            DiffView::Unified => false,
        },
        thinking: state.settings.thinking,
        source_markdown: state.settings.source_markdown,
        agent_color: &colors,
        background_key: state.hints.background.clone(),
        directory: &session.directory,
        home: &state.home,
        grouping: state.settings.grouping,
        diff_wrap: state.settings.diff_wrap,
        verbosity: state.settings.verbosity,
        toggled: &session.toggled,
        shells: &session.shells,
        floor: session.floor,
    };
    let mut rows = rows::layout(&session.items, &ctx, Some(&session.layout.0));
    rows.extend(rows::tail(
        session.compact_queued,
        session.revert.as_ref(),
        state.hints.redo.as_deref(),
        &ctx,
    ));
    rows
}

pub fn draw(c: &mut Canvas, state: &State, session: &Session) -> Option<(u16, u16)> {
    let top = u16::from(!state.tabs.is_empty());
    let pane = pane_width(state);
    let pad = if state.width < 44 { 1 } else { 2 };
    let x = pad;
    let w = pane.saturating_sub(2 * pad);
    let bottom = state.height.saturating_sub(1);
    let (block_h, cursor) = if let Some(permission) = &session.permission {
        let h = prompts::permission_height(c, state, permission, w);
        prompts::permission(c, state, permission, x, bottom.saturating_sub(h), w, h);
        (h, None)
    } else if let Some(question) = &session.question {
        let h = prompts::question_height(question, w);
        prompts::question(c, question, x, bottom.saturating_sub(h), w);
        (h, None)
    } else if session.location_missing() {
        let h = prompts::LOCATION_HEIGHT;
        prompts::location_missing(c, state, &session.directory, x, bottom.saturating_sub(h), w);
        (h, None)
    } else if let Some(dock) = &state.dock {
        let h = super::dock::height(dock);
        let y = bottom.saturating_sub(h + 1);
        super::dock::draw(c, dock, x, y, w);
        prompt::footer(c, state, x, y + h, w);
        (h + 1, None)
    } else {
        let box_h = prompt::box_height(state, w);
        let y = bottom.saturating_sub(box_h + 1);
        let cursor = prompt::draw_box(c, state, x, y, w, false);
        prompt::footer(c, state, x, y + box_h, w);
        let banners = banners(c, state, x, y, w);
        prompt::popup(c, state, x, w, y, top);
        (box_h + 1 + banners, cursor)
    };
    let status_y = bottom.saturating_sub(block_h + 1);
    let height = status_y.saturating_sub(top) as usize;
    let rows = transcript(state, session, c.theme, w as usize);
    let scroll = session.scroll.min(rows.len().saturating_sub(height));
    let end = rows.len() - scroll;
    let start = end.saturating_sub(height);
    let mut hits = Vec::new();
    for (i, row) in rows[start..end].iter().enumerate() {
        draw_row(c, row, x, top + i as u16, w);
        if let Some(id) = &row.group {
            hits.push((top + i as u16, id.clone()));
        }
    }
    let mut cache = session.layout.0.lock().unwrap_or_else(|e| e.into_inner());
    cache.hits = hits;
    cache.viewport = (rows.len(), height);
    drop(cache);
    if session.history_loading {
        let label = "Loading session history…";
        let lx = x + w - width(label) as u16;
        c.put(
            lx,
            status_y,
            label,
            c.style("text.feedback.info.base"),
            label.len(),
        );
    } else if session.scroll > 0 {
        let label = "Jump to latest ↓";
        let lx = x + w - width(label) as u16;
        c.put(
            lx,
            status_y,
            label,
            c.style("text.action.secondary.base"),
            label.len(),
        );
    }
    match &state.pane {
        Some(terminal) => self::terminal(c, terminal, pane, top, state.height),
        None if pane < state.width => sidebar(c, state, session, pane, top),
        None => {}
    }
    cursor
}

/// Dims everything drawn so far by black at 70/255 and draws the sidebar right-aligned on top.
pub fn overlay(c: &mut Canvas, state: &State, session: &Session) {
    let alpha = 70.0 / 255.0;
    let black = ratatui::style::Color::Rgb(0, 0, 0);
    let area = c.buf.area;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut c.buf[(x, y)];
            cell.fg = super::prompt::blend(cell.fg, black, alpha);
            cell.bg = super::prompt::blend(cell.bg, black, alpha);
        }
    }
    let x = state.width.saturating_sub(SIDEBAR_WIDTH);
    sidebar(c, state, session, x, u16::from(!state.tabs.is_empty()));
}

/// The account banners (event, low balance) stacked right above the composer; their rows.
fn banners(c: &mut Canvas, state: &State, x: u16, composer: u16, w: u16) -> u16 {
    let banners =
        crate::menus::anysola::banners(state.account.me.as_ref(), state.low_balance(), state.zone);
    let count = banners.len() as u16;
    for (i, (banner, tone)) in banners.into_iter().enumerate() {
        let path = match tone {
            crate::dialog::Tone::Warning => "text.feedback.warning.base",
            _ => "text.feedback.info.base",
        };
        let y = composer.saturating_sub(count - i as u16);
        c.put(x, y, &banner, c.style(path), w as usize);
    }
    count
}

/// The right pane's width: half the screen until dragged, at least 24 columns and leaving the
/// session at least 40.
pub fn terminal_width(state: &State) -> u16 {
    let wanted = state.pane_width.unwrap_or(state.width / 2);
    let most = state.width.saturating_sub(40).max(24);
    wanted.clamp(24, most).min(state.width)
}

/// The terminal grid inside the right pane: `(rows, columns)`.
pub fn terminal_size(state: &State) -> (u16, u16) {
    let top = u16::from(!state.tabs.is_empty());
    (
        state.height.saturating_sub(top).max(1),
        terminal_width(state).saturating_sub(2).max(1),
    )
}

fn vt_color(color: vt100::Color) -> Option<ratatui::style::Color> {
    use ratatui::style::Color;
    match color {
        vt100::Color::Default => None,
        vt100::Color::Idx(i) => Some(Color::Indexed(i)),
        vt100::Color::Rgb(r, g, b) => Some(Color::Rgb(r, g, b)),
    }
}

fn terminal(c: &mut Canvas, pane: &crate::state::Pane, x: u16, top: u16, height: u16) {
    let border = if pane.focused {
        c.style("border.focused")
    } else {
        c.style("border.base")
    };
    for y in top..height {
        c.put(x, y, "│", border, 1);
    }
    let parser = pane.screen.0.lock().unwrap_or_else(|e| e.into_inner());
    let screen = parser.screen();
    let (rows, cols) = screen.size();
    let base = c.style("text.base");
    for row in 0..rows.min(height.saturating_sub(top)) {
        for col in 0..cols {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            let mut style = base;
            if let Some(fg) = vt_color(cell.fgcolor()) {
                style = style.fg(fg);
            }
            if let Some(bg) = vt_color(cell.bgcolor()) {
                style = style.bg(bg);
            }
            if cell.bold() {
                style = style.add_modifier(ratatui::style::Modifier::BOLD);
            }
            if cell.inverse() {
                style = style.add_modifier(ratatui::style::Modifier::REVERSED);
            }
            let text = if cell.has_contents() {
                cell.contents()
            } else {
                " "
            };
            c.put(x + 2 + col, top + row, text, style, 2);
        }
    }
}

/// Where the terminal's cursor is on screen, when the pane has the keys.
pub fn terminal_cursor(state: &State) -> Option<(u16, u16)> {
    let pane = state.pane.as_ref().filter(|p| p.focused)?;
    let parser = pane.screen.0.lock().unwrap_or_else(|e| e.into_inner());
    if parser.screen().hide_cursor() {
        return None;
    }
    let (row, col) = parser.screen().cursor_position();
    let top = u16::from(!state.tabs.is_empty());
    Some((pane_width(state) + 2 + col, top + row))
}

pub fn draw_row(c: &mut Canvas, row: &Row, x: u16, y: u16, w: u16) {
    if let Some(bg) = row.bg {
        c.fill(Rect::new(x, y, w, 1), bg);
    }
    if let Some(rail) = row.rail {
        let mut style = Style::new().fg(rail);
        if let Some(bg) = row.bg {
            style = style.bg(bg);
        }
        c.put(x, y, "┃", style, 1);
    }
    let room = w.saturating_sub(row.indent) as usize;
    let spans: Vec<Span> = row
        .spans
        .iter()
        .map(|s| match (row.bg, s.style.bg) {
            (Some(bg), None) => Span::styled(s.content.clone(), s.style.bg(bg)),
            _ => s.clone(),
        })
        .collect();
    c.spans(x + row.indent, y, &spans, room);
}

fn sidebar(c: &mut Canvas, state: &State, session: &Session, x: u16, top: u16) {
    let bg = c.theme.fg("background.raised.base");
    let h = state.height.saturating_sub(top);
    c.fill(Rect::new(x, top, SIDEBAR_WIDTH, h), bg);
    let tx = x + 2;
    let room = (SIDEBAR_WIDTH - 4) as usize;
    let base = c.style("text.base").bg(bg);
    let muted = c.style("text.muted").bg(bg);
    let bold = base.add_modifier(Modifier::BOLD);
    let mut y = top + 1;
    let title = if session.title.is_empty() {
        "New session"
    } else {
        &session.title
    };
    // The core reports no pending auto-title, so a busy session still without one stands for it.
    let pending = session.title.is_empty() && session.busy;
    for line in text::wrap(title, room) {
        if pending && state.settings.animations {
            shimmer(c, tx, y, &line, state.now, base, muted);
        } else if pending {
            c.put(tx, y, &line, muted.add_modifier(Modifier::DIM), room);
        } else {
            c.put(tx, y, &line, bold, room);
        }
        y += 1;
    }
    if let Some(tokens) = session.context_tokens() {
        y += 1;
        c.put(tx, y, "Context", bold, room);
        c.put(
            tx,
            y + 1,
            &format!("{} tokens", text::credits(tokens as f64)),
            muted,
            room,
        );
        y += 2;
        if let Some(context) = state.model.as_ref().and_then(|m| m.context) {
            let pct = (tokens as f64 / context as f64 * 100.0).round();
            c.put(tx, y, &format!("{pct}% used"), muted, room);
            y += 1;
        }
    }
    if !state.mcp_list.is_empty() {
        y += 1;
        let (on, bad) = (state.mcp.1, state.mcp.2);
        let many = state.mcp_list.len() > 2;
        let mut x = tx;
        if many {
            x = c.put(tx, y, if state.mcp_closed { "▶" } else { "▼" }, base, 1) + 1;
        }
        let end = c.put(x, y, "MCP", bold, room);
        if many && state.mcp_closed {
            let errors = match bad {
                0 => String::new(),
                1 => ", 1 error".into(),
                n => format!(", {n} errors"),
            };
            c.put(end + 1, y, &format!("({on} active{errors})"), muted, room);
        } else {
            for (name, dot, word) in &state.mcp_list {
                y += 1;
                c.put(tx, y, "•", c.style(dot).bg(bg), 1);
                let word_style = if *word == "Error" {
                    c.style("text.feedback.error.base").bg(bg)
                } else {
                    muted
                };
                let word_x = tx + room as u16 - width(word) as u16;
                c.put(tx + 2, y, name, bold, room.saturating_sub(width(word) + 3));
                c.put(word_x, y, word, word_style, width(word));
            }
        }
        y += 1;
    }
    let todos = latest_todos(session);
    if !todos.is_empty() {
        y += 1;
        c.put(tx, y, "Todo", bold, room);
        for (status, content) in todos {
            y += 1;
            let (marker, style) = match status.as_str() {
                "completed" => ("[✓]", base),
                "in_progress" => ("[•]", c.style("text.feedback.warning.base").bg(bg)),
                "cancelled" => (
                    "[-]",
                    c.style("text.faint")
                        .bg(bg)
                        .add_modifier(Modifier::CROSSED_OUT),
                ),
                _ => ("[ ]", base),
            };
            c.put(tx, y, &format!("{marker} {content}"), style, room);
        }
    }
    let location = text::path(&state.location, 38);
    c.put(tx, state.height.saturating_sub(2), &location, muted, room);
    if onboarding_shown(state) {
        let card_h = 8;
        let card_y = state.height.saturating_sub(3 + card_h);
        let raised = c.theme.fg("background.raised.high");
        c.fill(Rect::new(tx, card_y, room as u16, card_h), raised);
        let base = base.bg(raised);
        let muted = muted.bg(raised);
        c.put(tx + 2, card_y + 1, "⬖", base, 1);
        c.put(
            tx + 4,
            card_y + 1,
            "Getting started",
            base.add_modifier(Modifier::BOLD),
            15,
        );
        c.put(tx + room as u16 - 3, card_y + 1, "✕", muted, 1);
        let inner = room.saturating_sub(6);
        for (i, line) in text::wrap("Sign in with your Anysola account to start.", inner)
            .iter()
            .take(2)
            .enumerate()
        {
            c.put(tx + 4, card_y + 3 + i as u16, line, muted, inner);
        }
        c.put(tx + 4, card_y + 6, "Sign in", base, 7);
        c.put(tx + room as u16 - 2 - 8, card_y + 6, "/connect", muted, 8);
    }
}

/// The title while an auto-title is pending: a bright band sweeps across over 1200 ms.
fn shimmer(c: &mut Canvas, x: u16, y: u16, line: &str, now: u64, bright: Style, dim: Style) {
    let len = line.chars().count() as u64;
    let band = (now % 1200) * (len + 6) / 1200;
    let mut at = x;
    for (i, ch) in line.chars().enumerate() {
        let near = (i as u64 + 3).abs_diff(band) <= 2;
        let style = if near {
            bright.add_modifier(Modifier::BOLD)
        } else {
            dim
        };
        let mut buf = [0u8; 4];
        at = c.put(at, y, ch.encode_utf8(&mut buf), style, 1);
    }
}

/// The `Getting started` card: signed out, tall enough, not dismissed.
pub fn onboarding_shown(state: &State) -> bool {
    state.height >= 22 && state.account.signed_out && !state.onboarding_dismissed
}

fn latest_todos(session: &Session) -> Vec<(String, String)> {
    use crate::transcript::model::{Item, Part, Status};
    session
        .items
        .iter()
        .rev()
        .find_map(|item| match item {
            Item::Assistant(a) => a.parts.iter().rev().find_map(|p| match p {
                Part::Tool(t) if t.name == "todowrite" && t.status == Status::Completed => {
                    t.input.get("todos").and_then(|v| v.as_array()).cloned()
                }
                _ => None,
            }),
            _ => None,
        })
        .unwrap_or_default()
        .iter()
        .map(|todo| {
            let field = |k: &str| {
                todo.get(k)
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string()
            };
            (field("status"), field("content"))
        })
        .collect()
}
