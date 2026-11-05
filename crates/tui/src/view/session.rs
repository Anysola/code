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
pub fn pane_width(state: &State) -> u16 {
    if state.settings.sidebar && state.width > 120 {
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
    };
    rows::layout(&session.items, &ctx, Some(&session.layout.0))
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
    } else {
        let box_h = prompt::box_height(state, w);
        let y = bottom.saturating_sub(box_h + 1);
        let cursor = prompt::draw_box(c, state, x, y, w, false);
        prompt::footer(c, state, x, y + box_h, w);
        prompt::popup(c, state, x, w, y, top);
        (box_h + 1, cursor)
    };
    let status_y = bottom.saturating_sub(block_h + 1);
    let height = status_y.saturating_sub(top) as usize;
    let rows = transcript(state, session, c.theme, w as usize);
    let scroll = session.scroll.min(rows.len().saturating_sub(height));
    let end = rows.len() - scroll;
    let start = end.saturating_sub(height);
    for (i, row) in rows[start..end].iter().enumerate() {
        draw_row(c, row, x, top + i as u16, w);
    }
    if session.scroll > 0 {
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
    if pane < state.width {
        sidebar(c, state, session, pane, top);
    }
    cursor
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
    for line in text::wrap(title, room) {
        c.put(tx, y, &line, bold, room);
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
