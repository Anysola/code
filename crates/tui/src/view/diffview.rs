//! Drawing the `/diff` viewer full screen.

use ratatui::layout::Rect;
use ratatui::style::Modifier;

use super::Canvas;
use crate::diffview::Diff;
use crate::text::width;
use crate::transcript::diff;

pub fn draw(c: &mut Canvas, view: &Diff) {
    let area = c.buf.area;
    c.fill(area, c.theme.fg("background.base"));
    let files = view.list();
    let reviewed = files.iter().filter(|f| f.reviewed).count();
    let bold = c.style("text.base").add_modifier(Modifier::BOLD);
    let muted = c.style("text.muted");
    let x = c.put(2, 0, view.source.title(), bold, area.width as usize);
    if !files.is_empty() {
        c.put(
            x + 2,
            0,
            &format!("{reviewed}/{} reviewed", files.len()),
            muted,
            40,
        );
    }
    let help = "esc close  n/p files  m reviewed  d source  b tree";
    if area.width as usize > width(help) + 30 {
        c.put(
            area.width - 2 - width(help) as u16,
            0,
            help,
            muted,
            width(help),
        );
    }
    let body = Rect::new(0, 2, area.width, area.height.saturating_sub(2));
    let message = match &view.files {
        None => Some(("Loading diff…".to_string(), muted)),
        Some(Err(error)) => Some((
            format!("Could not load diff. {error}"),
            c.style("text.feedback.error.base"),
        )),
        Some(Ok(files)) if files.is_empty() => Some(("No changes to show".to_string(), muted)),
        _ => None,
    };
    if let Some((text, style)) = message {
        c.put(2, body.y, &text, style, area.width as usize - 4);
        return;
    }
    let compact = area.width < 88;
    let tree_w = if view.tree && !compact {
        (area.width / 4).clamp(30, 40)
    } else {
        0
    };
    for (i, file) in files.iter().enumerate().take(body.height as usize) {
        if tree_w == 0 {
            break;
        }
        let (del, add) = diff::counts(&file.patch);
        let mark = if file.reviewed { "✓ " } else { "  " };
        let style = if i == view.selected {
            bold
        } else {
            c.style("text.base")
        };
        let y = body.y + i as u16;
        let end = c.put(1, y, mark, c.style("text.feedback.success.base"), 2);
        let counts = format!("+{add} -{del}");
        let room = (tree_w as usize).saturating_sub(width(&counts) + 5);
        c.put(
            end,
            y,
            &crate::text::truncate_middle(&file.path, room),
            style,
            room,
        );
        c.put(
            tree_w - 1 - width(&counts) as u16,
            y,
            &counts,
            muted,
            width(&counts),
        );
    }
    let px = tree_w + if tree_w > 0 { 1 } else { 0 };
    let pw = area.width.saturating_sub(px + 1) as usize;
    let Some(file) = files.get(view.selected) else {
        return;
    };
    c.put(px, body.y, &file.path, bold, pw);
    let options = diff::Options {
        width: pw,
        split: pw >= 100,
        filetype: crate::transcript::highlight::filetype(&file.path),
    };
    let lines = diff::render(&file.patch, &options, c.theme);
    for (i, line) in lines
        .iter()
        .skip(view.scroll)
        .take(body.height.saturating_sub(2) as usize)
        .enumerate()
    {
        c.spans(px, body.y + 2 + i as u16, line, pw);
    }
}
