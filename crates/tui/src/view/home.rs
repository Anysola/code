//! Home: the ANYSOLA wordmark (or the A symbol on narrow terminals), the prompt, recent sessions,
//! and the plan line with the version at the bottom.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::{Canvas, Hit, prompt, round};
use crate::state::State;
use crate::text::{self, width};

pub const WORDMARK: [&str; 3] = [
    "   ▄█▜▖  ▐█▙▖  ▐█ ▝▜▙▖▗▟▛▘ ▄▛▀▀▀▀▘ ▗▟▀▀▀▀▜▄ ▐█         ▄█▜▖ ",
    " ▗▟▛▘▐█▖ ▐█▝█▄ ▐█   ▝██▘    ▀▀▀▀▜▙ ▐█    ▐█ ▐█       ▗▟▛▘▐█▖",
    "▗█▀   ▜█ ▐█  ▀▙▟█    ██    ▗▄▄▄▄▟▀ ▝▜▄▄▄▄▟▀ ▐█▄▄▄▄▖ ▗█▀   ▜█",
];
const SYMBOL: [&str; 3] = ["   ▄█▜▖ ", " ▗▟▛▘▐█▖", "▗█▀   ▜█"];
const WORDMARK_WIDTH: u16 = 60;

pub fn draw(c: &mut Canvas, state: &State) -> Option<(u16, u16)> {
    let (w, h) = (state.width, state.height);
    let top = u16::from(state.strip());
    let footer = if h >= 14 && w >= 44 {
        if h < 16 { 1 } else { 3 }
    } else {
        0
    };
    let column = h.saturating_sub(top + footer);
    let pad = if w < 44 { 1 } else { 2 };
    let inner = w.saturating_sub(2 * pad);
    let logo = h >= 12;
    let narrow = w < WORDMARK_WIDTH + 4;
    let prompt_w = 75.min(inner);
    let box_h = prompt::box_height(state, prompt_w);
    let recent = h >= 24 && !state.recent.is_empty();
    let fixed = if logo { 4 } else { 0 }
        + 1
        + u16::from(state.update.is_some())
        + 1
        + box_h
        + 1
        + if recent {
            2 + state.recent.len() as u16
        } else {
            0
        };
    let spacer = 3.min(column.saturating_sub(fixed));
    let free = column.saturating_sub(fixed + spacer);
    let start = f64::from(top) + f64::from(free) / 2.0 + f64::from(spacer);
    let mut y = round(start);
    let center = |item: u16| round(f64::from(pad) + f64::from(inner.saturating_sub(item)) / 2.0);
    if logo {
        let bold = c.style("text.base").add_modifier(Modifier::BOLD);
        let muted = c.style("text.muted");
        if narrow {
            let x = center(12);
            for (i, line) in SYMBOL.iter().enumerate() {
                c.put(x + 2, y + i as u16, line, bold, 8);
            }
            c.put(x, y + 3, "Anysola Code", muted, 12);
        } else {
            let x = center(WORDMARK_WIDTH);
            for (i, line) in WORDMARK.iter().enumerate() {
                c.put(x, y + i as u16, line, bold, WORDMARK_WIDTH as usize);
            }
            c.put(x + WORDMARK_WIDTH - 4, y + 3, "Code", muted, 4);
        }
        y += 4;
    }
    y += 1;
    if let Some(version) = &state.update {
        let notice = format!("/update to install v{version}");
        let x = center(width(&notice) as u16);
        let end = c.put(x, y, "/update", c.style("text.action.primary.selected"), 7);
        let end = c.put(end, y, &notice[7..], c.style("text.muted"), notice.len());
        c.hit(
            Rect::new(x, y, end - x, 1),
            Hit::Command("opencode.update".into()),
        );
        y += 1;
    }
    let x = center(prompt_w);
    let box_y = y + 1;
    let cursor = prompt::draw_box(c, state, x, box_y, prompt_w, true);
    prompt::footer(c, state, x, box_y + box_h, prompt_w);
    if recent {
        recent_sessions(c, state, x, box_y + box_h + 2, prompt_w);
    }
    if footer > 0 {
        home_footer(c, state, if h < 16 { h - 1 } else { h - 2 });
    }
    prompt::popup(c, state, x, prompt_w, box_y, top);
    cursor
}

fn recent_sessions(c: &mut Canvas, state: &State, x: u16, y: u16, w: u16) {
    let faint = c.style("text.faint");
    let muted = c.style("text.muted");
    c.put(x, y, "Recent", faint, w as usize);
    if let Some(key) = &state.hints.sessions {
        let hint = format!("{key} all sessions");
        let hx = x + w - width(&hint) as u16;
        let end = c.put(hx, y, key, muted, w as usize);
        let end = c.put(end, y, " all sessions", faint, w as usize);
        c.hit(
            Rect::new(hx, y, end - hx, 1),
            Hit::Command("session.list".into()),
        );
    }
    for (i, session) in state.recent.iter().enumerate() {
        let row = y + 1 + i as u16;
        let when = text::ago(session.updated, state.now);
        let room = 8.max(w as usize - width(&when) - 2);
        c.put(x, row, &text::truncate(&session.title, room), muted, room);
        c.put(x + w - width(&when) as u16, row, &when, faint, width(&when));
    }
}

/// `Team · renews 23 Nov`, the account banners, and the version right.
fn home_footer(c: &mut Canvas, state: &State, y: u16) {
    let muted: Style = c.style("text.muted");
    let max = state.width as usize - 4;
    let mut x = 2;
    let (configured, connected, failed) = state.mcp;
    if configured > 0 {
        let (dot, label) = if failed > 0 {
            ("text.feedback.error.base", format!("{failed} MCP failed"))
        } else if connected > 0 {
            ("text.feedback.success.base", format!("{connected} MCP"))
        } else {
            ("text.muted", format!("{connected} MCP"))
        };
        x = c.put(x, y, "⊙ ", c.style(dot), max);
        x = c.put(x, y, &label, c.style("text.base"), max);
        if state.width >= 64 {
            x = c.put(x + 1, y, "/mcps", muted, max);
        }
        x += 2;
    }
    if let Some(plan) = &state.account.plan {
        let mut line = text::title_case(plan);
        if let Some(renews) = state.account.renews {
            line.push_str(&format!(
                " · renews {}",
                text::short_date(renews, state.now)
            ));
        }
        x = c.put(x, y, &line, c.style("text.faint"), max) + 2;
    }
    let banners =
        crate::menus::anysola::banners(state.account.me.as_ref(), state.low_balance(), state.zone);
    for (banner, tone) in banners {
        let path = match tone {
            crate::dialog::Tone::Warning => "text.feedback.warning.base",
            _ => "text.feedback.info.base",
        };
        let room = (state.width as usize).saturating_sub(x as usize);
        x = c.put(x, y, &banner, c.style(path), room) + 2;
    }
    if state.width >= 64 {
        let right = state.width - 2 - width(&state.version) as u16;
        let at = right.max(x + 2);
        let room = (state.width as usize).saturating_sub(at as usize);
        c.put(at, y, &state.version, muted, room);
    }
}
