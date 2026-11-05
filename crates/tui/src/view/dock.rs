//! The composer dock drawn in place of the prompt: tab header with `esc`, up to five rows,
//! then the key hints.

use ratatui::layout::Rect;
use ratatui::style::Modifier;

use super::Canvas;
use crate::state::Dock;
use crate::text::width;

const ROWS: usize = 5;

/// Rows the dock takes: padding, header, list (at least the empty line), hints.
pub fn height(dock: &Dock) -> u16 {
    dock.rows.len().clamp(1, ROWS) as u16 + 5
}

pub fn draw(c: &mut Canvas, dock: &Dock, x: u16, y: u16, w: u16) {
    let h = height(dock);
    c.fill(Rect::new(x, y, w, h), c.theme.fg("background.raised.base"));
    let (left, right) = (x + 2, x + w.saturating_sub(2));
    let room = right.saturating_sub(left) as usize;
    let muted = c.style("text.muted");
    let base = c.style("text.base");
    let mut at = left;
    for (i, tab) in dock.tabs.iter().enumerate() {
        let style = if i == dock.tab {
            base.add_modifier(Modifier::BOLD)
        } else {
            muted
        };
        at = c.put(at, y + 1, tab.label(), style, room) + 2;
    }
    c.put(right.saturating_sub(3), y + 1, "esc", muted, 3);
    let first = dock.selected.saturating_sub(ROWS - 1);
    if dock.rows.is_empty() {
        c.put(left, y + 2, dock.empty(), muted, room);
    }
    for (i, row) in dock.rows.iter().skip(first).take(ROWS).enumerate() {
        let ry = y + 2 + i as u16;
        let active = first + i == dock.selected;
        let style = if active {
            c.style("text.accent.base").add_modifier(Modifier::BOLD)
        } else {
            base
        };
        let right_w = width(&row.right);
        c.put(
            left,
            ry,
            &row.label,
            style,
            room.saturating_sub(right_w + 1),
        );
        if right_w > 0 {
            c.put(right - right_w as u16, ry, &row.right, muted, right_w);
        }
    }
    let mut hx = left;
    for (label, key) in dock.hints() {
        hx = c.put(hx, y + h - 2, label, muted, room) + 1;
        hx = c.put(hx, y + h - 2, key, base, room) + 2;
    }
}
