//! The prompt box (rail, textarea, metadata row, half-block edge), the footer row under it and
//! the autocomplete popup above it.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use super::Canvas;
use crate::composer::{AtomKind, Mode};
use crate::state::{HOME_EXAMPLES, SHELL_EXAMPLES, State};
use crate::text::{self, title_case, width};

pub fn max_rows(height: u16) -> usize {
    6.max(height as usize / 3)
}

/// Columns of the textarea in a prompt box `w` wide.
pub fn textarea_width(w: u16, terminal: u16) -> usize {
    let padding = if terminal < 44 { 2 } else { 4 };
    (w as usize).saturating_sub(1 + padding)
}

/// The composer text broken into display rows: `(byte start, byte end)` per row.
fn rows(text: &str, room: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    for line in text.split('\n') {
        let mut row_start = start;
        let mut used = 0;
        for (i, g) in unicode_segmentation::UnicodeSegmentation::grapheme_indices(line, true) {
            let w = width(g);
            if used + w > room.max(1) {
                out.push((row_start, start + i));
                row_start = start + i;
                used = 0;
            }
            used += w;
        }
        out.push((row_start, start + line.len()));
        start += line.len() + 1;
    }
    out
}

pub fn agent_color(c: &Canvas, state: &State, agent: &str) -> Color {
    c.theme.term(c.theme.categorical(state.agent_index(agent)))
}

fn atom_style(c: &Canvas, kind: &AtomKind) -> Style {
    let bold = |path: Color| Style::new().fg(path).add_modifier(Modifier::BOLD);
    match kind {
        AtomKind::File | AtomKind::Attachment(..) => bold(c.theme.fg("text.feedback.warning.base")),
        AtomKind::Agent => bold(c.theme.term(c.theme.categorical(0))),
        AtomKind::Skill => bold(c.theme.term(c.theme.categorical(1))),
        AtomKind::Paste(_) => bold(c.theme.fg("text.action.primary.focused"))
            .bg(c.theme.fg("text.feedback.warning.base")),
    }
}

/// Height of the box (without the footer row) for the current text.
pub fn box_height(state: &State, w: u16) -> u16 {
    let room = textarea_width(w, state.width);
    let lines = rows(&state.composer.text, room)
        .len()
        .clamp(1, max_rows(state.height));
    lines as u16 + 4
}

/// Draws the box at `(x, y)`; returns the cursor position.
pub fn draw_box(
    c: &mut Canvas,
    state: &State,
    x: u16,
    y: u16,
    w: u16,
    home: bool,
) -> Option<(u16, u16)> {
    let shell = state.composer.mode == Mode::Shell;
    let highlight = if shell {
        c.theme.fg("text.action.primary.selected")
    } else {
        agent_color(c, state, &state.agent)
    };
    let bg = c
        .theme
        .term(c.theme.decrease(c.theme.get("background.raised.base"), 1));
    let h = box_height(state, w);
    c.fill(Rect::new(x + 1, y, w.saturating_sub(1), h - 1), bg);
    let rail = Style::new().fg(highlight).bg(bg);
    for row in y..y + h - 1 {
        c.put(x, row, "┃", rail, 1);
    }
    let edge = Style::new().fg(bg).bg(c.theme.fg("background.base"));
    c.put(x, y + h - 1, "╹", Style::new().fg(highlight), 1);
    c.put(
        x + 1,
        y + h - 1,
        &"▀".repeat(w.saturating_sub(1) as usize),
        edge,
        w as usize - 1,
    );
    let pad = if state.width < 44 { 1 } else { 2 };
    let tx = x + 1 + pad;
    let room = textarea_width(w, state.width);
    let composer = &state.composer;
    let mut cursor = None;
    if composer.is_empty() {
        let placeholder = if !home {
            None
        } else if shell {
            Some(format!(
                "Run a command… \"{}\"",
                SHELL_EXAMPLES[state.shell_example % 3]
            ))
        } else {
            Some(format!(
                "Ask anything… \"{}\"",
                HOME_EXAMPLES[state.example % 3]
            ))
        };
        if let Some(placeholder) = placeholder {
            let cut = if state.width < 44 {
                state.width as usize - 5
            } else {
                75.min(state.width as usize - 4) - 5
            };
            let text = text::clip(&placeholder, cut);
            c.put(
                tx,
                y + 1,
                text.trim_end(),
                c.style("text.muted").bg(bg),
                room,
            );
        }
        cursor = Some((tx, y + 1));
    } else {
        let all = rows(&composer.text, room);
        let visible = max_rows(state.height);
        let cursor_row = all
            .iter()
            .rposition(|(s, _)| *s <= composer.cursor)
            .unwrap_or(0);
        let first = cursor_row.saturating_sub(visible - 1);
        let base = c.style("text.base").bg(bg);
        for (i, (start, end)) in all.iter().enumerate().skip(first).take(visible) {
            let row_y = y + 1 + (i - first) as u16;
            let mut at = tx;
            let mut offset = *start;
            while offset < *end {
                let atom = composer
                    .atoms
                    .iter()
                    .find(|a| a.start <= offset && offset < a.end);
                let (stop, style) = match atom {
                    Some(a) => (a.end.min(*end), atom_style(c, &a.kind)),
                    None => {
                        let next = composer
                            .atoms
                            .iter()
                            .filter(|a| a.start > offset)
                            .map(|a| a.start)
                            .min()
                            .unwrap_or(*end)
                            .min(*end);
                        (next, base)
                    }
                };
                at = c.put(at, row_y, &composer.text[offset..stop], style, room);
                offset = stop;
            }
            if i == cursor_row {
                let col = width(&composer.text[*start..composer.cursor.clamp(*start, *end)]);
                cursor = Some((tx + col as u16, row_y));
            }
        }
    }
    metadata(c, state, tx, y + h - 2, room, highlight, bg);
    cursor
}

fn metadata(
    c: &mut Canvas,
    state: &State,
    x: u16,
    y: u16,
    room: usize,
    highlight: Color,
    bg: Color,
) {
    if state.composer.mode == Mode::Shell {
        c.put(x, y, "Shell", Style::new().fg(highlight).bg(bg), room);
        return;
    }
    let muted = c.style("text.muted").bg(bg);
    let mut spans: Vec<Span> = Vec::new();
    if state.width >= 44 {
        spans.push(Span::styled(
            title_case(&state.agent),
            Style::new().fg(highlight).bg(bg),
        ));
        if state.auto {
            spans.push(Span::styled(" auto", muted));
        }
    }
    let Some(model) = &state.model else {
        c.spans(x, y, &spans, room);
        return;
    };
    let budget = state
        .width
        .saturating_sub(if state.width < 44 { 9 } else { 13 }) as usize;
    let mut suffix: Vec<Span> = Vec::new();
    if let Some(variant) = &state.variant {
        suffix.push(Span::styled(" · ", muted));
        let warning = c
            .style("text.feedback.warning.base")
            .bg(bg)
            .add_modifier(Modifier::BOLD);
        suffix.push(Span::styled(variant.clone(), warning));
    }
    let used = |spans: &[Span]| spans.iter().map(|s| width(&s.content)).sum::<usize>();
    if !spans.is_empty() {
        spans.push(Span::styled(" · ", muted));
    }
    let left = budget.saturating_sub(used(&spans) + used(&suffix));
    let name = text::truncate(&model.name, left.max(9));
    spans.push(Span::styled(name, c.style("text.base").bg(bg)));
    spans.extend(suffix);
    c.spans(x, y, &spans, room);
}

/// The row under the box: location (or the running spinner) left, status and hints right.
pub fn footer(c: &mut Canvas, state: &State, x: u16, y: u16, w: u16) {
    let session = state.session();
    let base = c.style("text.base");
    let muted = c.style("text.muted");
    let shell = state.composer.mode == Mode::Shell;
    let mut right: Vec<Vec<Span>> = Vec::new();
    if state.asides > 0 {
        let frame = if state.settings.animations {
            crate::transcript::rows::spinner_frame(state.now)
        } else {
            "⋯"
        };
        right.push(vec![Span::styled(format!("{frame} /btw"), muted)]);
    }
    if let Some(status) = state.status_text() {
        let style = if state.status_warning() {
            c.style("text.feedback.warning.base")
        } else {
            muted
        };
        right.push(vec![Span::styled(status, style)]);
    }
    let hint = |key: &Option<String>, label: &str| -> Option<Vec<Span<'static>>> {
        key.as_ref().map(|k| {
            vec![
                Span::styled(format!("{k} "), base),
                Span::styled(label.to_string(), muted),
            ]
        })
    };
    if shell {
        let label = if state.width < 44 {
            "shell"
        } else {
            "exit shell mode"
        };
        right.push(vec![Span::styled("esc ", base), Span::styled(label, muted)]);
    } else {
        // The live counts and the goal read as one run joined by ` · `.
        let mut item = Vec::new();
        if let Some(live) = &state.live {
            if let Some(key) = &state.hints.child {
                item.push(Span::styled(format!("{key} "), base));
            }
            item.push(Span::styled(live.clone(), muted));
        }
        if let Some(goal) = session.and_then(|s| s.goal.as_ref()) {
            if !item.is_empty() {
                item.push(Span::styled(" · ", muted));
            }
            let (text, color) = crate::app::goal_status(goal);
            item.push(Span::styled(text, c.style(color)));
        }
        if !item.is_empty() {
            right.push(item);
        }
        let usage = session.and_then(|s| s.context_tokens()).map(|tokens| {
            let pct = state
                .model
                .as_ref()
                .and_then(|m| m.context)
                .map_or(String::new(), |ctx| {
                    format!(" ({}%)", (tokens as f64 / ctx as f64 * 100.0).round())
                });
            format!("{}{pct}", text::compact(tokens))
        });
        let commands = hint(&state.hints.commands, "commands");
        match usage {
            Some(usage) => {
                let room = (state.width as usize).saturating_sub(8);
                let available = room.saturating_sub(28.min(room / 2));
                let both = format!(
                    "{usage} · {} commands",
                    state.hints.commands.clone().unwrap_or_default()
                );
                let interrupting = state.interrupt_armed.is_some() && state.width < 80;
                if !interrupting && width(&usage) <= available {
                    right.push(vec![Span::styled(usage, muted)]);
                    if width(&both) <= available
                        && let Some(commands) = commands
                    {
                        right.push(commands);
                    }
                }
            }
            None => {
                if state.width >= 44
                    && let Some(agents) = hint(&state.hints.agents, "agents")
                {
                    right.push(agents);
                }
                if state.width >= 44
                    && let Some(commands) = commands
                {
                    right.push(commands);
                }
            }
        }
    }
    let right_width: usize = right
        .iter()
        .map(|item| item.iter().map(|s| width(&s.content)).sum::<usize>())
        .sum::<usize>()
        + 2 * right.len();
    let mut at = (x + w).saturating_sub(right_width as u16 - 2.min(right_width as u16));
    for item in &right {
        at = c.spans(at, y, item, (x + w).saturating_sub(at) as usize) + 2;
    }
    let mut left_room = (w as usize).saturating_sub(right_width);
    if let Some((label, pending)) = &state.editor_label {
        let most = 12.max(48.min(state.width as usize / 3));
        let label = text::truncate_middle(label, most);
        let lw = width(&label);
        if lw + 4 < left_room {
            let style = if *pending {
                Style::new().fg(c.theme.term(c.theme.hue("accent", 500)))
            } else {
                muted
            };
            let lx = x + (left_room - lw - 2) as u16;
            c.put(lx, y, &label, style, lw);
            left_room -= lw + 2;
        }
    }
    if left_room == 0 {
        return;
    }
    if session.is_some_and(|s| s.busy) {
        let interrupt = match state.interrupt_armed {
            Some(_) => vec![
                Span::styled("esc ", c.style("text.feedback.warning.base")),
                Span::styled("again to interrupt", c.style("text.feedback.warning.base")),
            ],
            None => vec![Span::styled("esc ", base), Span::styled("interrupt", muted)],
        };
        let spinner = if state.settings.animations {
            let frame = (state.now / 40) as usize % 14;
            let position = if frame < 8 { frame } else { 14 - frame };
            (0..8)
                .map(|i| if i == position { '■' } else { '⬝' })
                .collect::<String>()
        } else {
            "[⋯]".to_string()
        };
        let color = agent_color(c, state, &state.agent);
        let end = c.put(x + 1, y, &spinner, Style::new().fg(color), left_room);
        c.spans(
            end + 1,
            y,
            &interrupt,
            left_room.saturating_sub((end + 1 - x) as usize),
        );
    } else if let Some(progress) = &state.worktree_progress {
        let accent = Style::new().fg(c.theme.term(c.theme.hue("accent", 500)));
        let spinner = if state.settings.animations {
            crate::transcript::rows::spinner_frame(state.now)
        } else {
            "⋯"
        };
        let end = c.put(
            x + 3,
            y,
            &format!("{spinner} {progress}"),
            accent,
            left_room,
        );
        c.put(
            end,
            y,
            "...",
            muted,
            left_room.saturating_sub((end - x) as usize),
        );
    } else if state.new_worktree.is_some() {
        let accent = Style::new().fg(c.theme.term(c.theme.hue("accent", 500)));
        c.put(
            x + 3,
            y,
            "(new worktree)",
            accent,
            left_room.saturating_sub(3),
        );
    } else {
        let label = text::path(&state.location, left_room);
        c.put(x, y, &label, muted, left_room);
    }
}

/// The `/` or `@` popup sitting on top of the box at `box_y`.
pub fn popup(c: &mut Canvas, state: &State, x: u16, w: u16, box_y: u16, top: u16) {
    let Some(popup) = &state.popup else { return };
    let rows = popup.rows.len().max(1);
    let h = 10.min(rows).min(box_y.saturating_sub(top) as usize) as u16;
    if h == 0 {
        return;
    }
    let y = box_y - h;
    let bg = c.theme.fg("background.raised.high");
    c.fill(Rect::new(x, y, w, h), bg);
    let border = c.style("border.base").bg(bg);
    for row in y..y + h {
        c.put(x, row, "┃", border, 1);
        c.put(x + w - 1, row, "┃", border, 1);
    }
    let inner = w.saturating_sub(4) as usize;
    if popup.rows.is_empty() {
        let text = if popup.trigger == '/' {
            "No matching commands"
        } else {
            "No matching files, agents, or references"
        };
        c.put(x + 2, y, text, c.style("text.muted").bg(bg), inner);
        return;
    }
    for (i, row) in popup
        .rows
        .iter()
        .enumerate()
        .skip(popup.offset)
        .take(h as usize)
    {
        let row_y = y + (i - popup.offset) as u16;
        let selected = i == popup.selected;
        if selected && state.popup_confirm {
            let fg = c.theme.fg("text.action.destructive.focused");
            let row_bg = c.theme.fg("background.action.destructive.focused");
            c.fill(Rect::new(x + 1, row_y, w - 2, 1), row_bg);
            let style = Style::new().fg(fg).bg(row_bg);
            c.put(x + 2, row_y, "Press ctrl+d to confirm", style, inner);
            continue;
        }
        let (fg, desc_fg, row_bg) = if selected {
            let focused = c.theme.fg("text.action.primary.focused");
            (
                focused,
                focused,
                c.theme.fg("background.action.primary.focused"),
            )
        } else {
            (c.theme.fg("text.base"), c.theme.fg("text.muted"), bg)
        };
        if selected {
            c.fill(Rect::new(x + 1, row_y, w - 2, 1), row_bg);
        }
        let display = text::pad_right(&row.display, popup.pad);
        let label_room = if row.label.is_empty() {
            0
        } else {
            width(&row.label) + 2
        };
        let at = c.put(
            x + 2,
            row_y,
            &display,
            Style::new().fg(fg).bg(row_bg),
            inner,
        );
        let desc_room = inner.saturating_sub(width(&display) + 1 + label_room);
        let description = row
            .description
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        c.put(
            at + 1,
            row_y,
            text::clip(&description, desc_room).trim_end(),
            Style::new().fg(desc_fg).bg(row_bg),
            desc_room,
        );
        if !row.label.is_empty() {
            let lx = x + 2 + (inner - width(&row.label)) as u16;
            c.put(
                lx,
                row_y,
                &row.label,
                Style::new().fg(desc_fg).bg(row_bg),
                width(&row.label),
            );
        }
    }
}
