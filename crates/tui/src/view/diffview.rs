//! Drawing the `/diff` viewer full screen.

use ratatui::layout::Rect;
use ratatui::style::Modifier;

use super::Canvas;
use crate::diffview::Diff;
use crate::text::width;
use crate::transcript::diff;

/// The file tree's width: none when hidden or narrower than 88 columns.
fn tree_width(view: &Diff, screen: u16) -> u16 {
    if view.tree && screen >= 88 {
        (screen / 4).clamp(30, 40)
    } else {
        0
    }
}

/// How the selected file's patch is laid out on a screen this wide.
pub fn patch_options<'a>(view: &Diff, screen: u16, path: &'a str) -> diff::Options<'a> {
    let tree = tree_width(view, screen);
    let x = tree + u16::from(tree > 0);
    let width = screen.saturating_sub(x + 1) as usize;
    diff::Options {
        width,
        split: width >= 100,
        filetype: crate::transcript::highlight::filetype(path),
    }
}

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
    let help = "esc close  n/p files  ]/[ hunks  m reviewed  d source  b tree";
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
    let tree_w = tree_width(view, area.width);
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
    let Some(file) = files.get(view.selected) else {
        return;
    };
    let options = patch_options(view, area.width, &file.path);
    let pw = options.width;
    c.put(px, body.y, &file.path, bold, pw);
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
