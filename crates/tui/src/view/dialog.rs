//! Dialogs over a dimmed screen: the shared frame, list dialogs, plain dialogs, prompts,
//! confirmations and the export options.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::Canvas;
use crate::dialog::{
    Action, Body, Confirm, Dialog, Export, Line, Prompt, Row, Select, Tone, list_limit,
};
use crate::text::{truncate, width};

/// Where a dialog's panel sits and where its text starts and ends.
struct Frame {
    x: u16,
    w: u16,
    top: u16,
    left: u16,
    right: u16,
}

impl Frame {
    /// `inset` is the text indent from the panel edge; lists sit one column further in.
    fn new(c: &Canvas, size: crate::dialog::Size, inset: u16, top: u16) -> Frame {
        let area = c.buf.area;
        let w = size.width().min(area.width.saturating_sub(2));
        let x = (area.width - w) / 2;
        Frame {
            x,
            w,
            top,
            left: x + inset,
            right: x + w - inset,
        }
    }

    fn room(&self) -> usize {
        self.right.saturating_sub(self.left) as usize
    }
}

fn tone(c: &Canvas, tone: Tone) -> Style {
    let path = match tone {
        Tone::Base | Tone::Bold => "text.base",
        Tone::Muted => "text.muted",
        Tone::Accent => "text.accent.base",
        Tone::Info => "text.feedback.info.base",
        Tone::Success => "text.feedback.success.base",
        Tone::Warning => "text.feedback.warning.base",
        Tone::Error => "text.feedback.error.base",
    };
    let style = Style::new().fg(c.theme.term(c.theme.dialog.get(path)));
    if tone == Tone::Bold {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

fn put_line(c: &mut Canvas, x: u16, y: u16, line: &Line, max: usize) -> u16 {
    let mut at = x;
    for (text, t) in line {
        let room = (x as usize + max).saturating_sub(at as usize);
        let style = tone(c, *t);
        at = c.put(at, y, text, style, room);
    }
    at
}

fn line_width(line: &Line) -> usize {
    line.iter().map(|(t, _)| width(t)).sum()
}

/// Dims the screen and clears the panel rows `top..bottom`.
fn panel(c: &mut Canvas, f: &Frame, bottom: u16) {
    let area = c.buf.area;
    let faint = c.theme.fg("text.faint");
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut c.buf[(x, y)];
            cell.set_fg(faint);
            cell.set_bg(Color::Rgb(0, 0, 0));
        }
    }
    let bg = c.theme.term(c.theme.dialog.get("background.base"));
    c.fill(Rect::new(f.x, f.top, f.w, bottom.saturating_sub(f.top)), bg);
}

fn header(c: &mut Canvas, f: &Frame, title: &Line, close: &str) {
    let y = f.top + 1;
    let room = f.room().saturating_sub(width(close) + 1);
    put_line(c, f.left, y, title, room);
    let style = tone(c, Tone::Muted);
    c.put(f.right - width(close) as u16, y, close, style, width(close));
}

/// `<title> <key>` actions: left ones from the left edge, right ones against the right edge.
fn actions(c: &mut Canvas, f: &Frame, y: u16, actions: &[Action]) {
    let spans = |a: &Action| {
        vec![
            (a.title.clone(), Tone::Base),
            (format!(" {}", a.key), Tone::Muted),
        ]
    };
    let mut x = f.left;
    for a in actions.iter().filter(|a| !a.right) {
        x = put_line(c, x, y, &spans(a), f.room()) + 2;
    }
    let right: Vec<Line> = actions.iter().filter(|a| a.right).map(spans).collect();
    let total: usize =
        right.iter().map(line_width).sum::<usize>() + 2 * right.len().saturating_sub(1);
    let mut x = f.right.saturating_sub(total as u16);
    for line in &right {
        x = put_line(c, x, y, line, f.room()) + 2;
    }
}

pub fn draw(c: &mut Canvas, dialog: &Dialog) -> Option<(u16, u16)> {
    match dialog {
        Dialog::Select(s) => select(c, s),
        Dialog::Body(b) => {
            body(c, b);
            None
        }
        Dialog::Prompt(p) => prompt(c, p),
        Dialog::Confirm(k) => {
            confirm(c, k);
            None
        }
        Dialog::Export(e) => {
            export(c, e);
            None
        }
    }
}

fn select(c: &mut Canvas, s: &Select) -> Option<(u16, u16)> {
    let height = c.buf.area.height;
    let f = Frame::new(c, s.size, 4, height / 4);
    let rows = s.rows();
    let limit = list_limit(height);
    let shown = if s.status.is_some() || rows.is_empty() {
        1
    } else {
        rows.len().min(limit)
    };
    let list = f.top + 5;
    let mut bottom = list + shown as u16;
    if !s.actions.is_empty() {
        bottom += 2;
    }
    let panel_bottom = if s.tall {
        (list + limit as u16 + 3).max(bottom + 1)
    } else {
        bottom + 1
    };
    panel(c, &f, panel_bottom);
    let mut title = vec![(s.title.clone(), Tone::Bold)];
    if !s.suffix.is_empty() {
        title.push((s.suffix.clone(), Tone::Muted));
    }
    header(c, &f, &title, "esc");
    let search_y = f.top + 3;
    if s.query.is_empty() {
        let style = tone(c, Tone::Muted);
        c.put(f.left, search_y, &s.placeholder, style, f.room());
    } else {
        let style = tone(c, Tone::Base);
        c.put(f.left, search_y, &s.query, style, f.room());
    }
    let cursor = (f.left + width(&s.query).min(f.room()) as u16, search_y);
    if let Some((text, t)) = &s.status {
        let style = tone(c, *t);
        c.put(f.left, list, text, style, f.room());
    } else if rows.is_empty() {
        let text = if s.query.trim().is_empty() {
            s.empty
        } else {
            "No results found"
        };
        let style = tone(c, Tone::Muted);
        c.put(f.left, list, text, style, f.room());
    } else {
        let active = s.matches().get(s.selected).copied();
        for (i, row) in rows.iter().skip(s.offset).take(limit).enumerate() {
            let y = list + i as u16;
            match *row {
                Row::Header(index) => {
                    let style = Style::new()
                        .fg(c.theme.term(c.theme.hue("accent", 200)))
                        .add_modifier(Modifier::BOLD);
                    c.put(f.left, y, &s.options[index].category, style, f.room());
                }
                Row::Blank => {}
                Row::Option(index) => option(c, &f, y, s, index, active == Some(index)),
            }
        }
    }
    if !s.actions.is_empty() {
        actions(c, &f, bottom - 1, &s.actions);
    }
    Some(cursor)
}

fn option(c: &mut Canvas, f: &Frame, y: u16, s: &Select, index: usize, active: bool) {
    let o = &s.options[index];
    let confirming = active && s.confirm.as_deref() == Some(o.value.as_str());
    let (base, secondary) = if active {
        let fg = c
            .theme
            .term(c.theme.dialog.get("text.action.primary.focused"));
        let bg_path = if confirming {
            "background.action.destructive.focused"
        } else {
            "background.action.primary.focused"
        };
        let bg = c.theme.term(c.theme.dialog.get(bg_path));
        c.bg(Rect::new(f.left - 3, y, f.right + 2 - (f.left - 3), 1), bg);
        let style = Style::new().fg(fg).bg(bg);
        (style.add_modifier(Modifier::BOLD), style)
    } else if o.current {
        let fg = c.theme.term(c.theme.dialog.get("text.formfield.selected"));
        (Style::new().fg(fg), tone(c, Tone::Muted))
    } else {
        (tone(c, Tone::Base), tone(c, Tone::Muted))
    };
    if let Some((mark, t)) = &o.gutter {
        let style = tone(c, *t).add_modifier(Modifier::BOLD);
        c.put(f.left - 2, y, mark, style, 1);
    } else if o.current {
        c.put(f.left - 2, y, "●", base.remove_modifier(Modifier::BOLD), 1);
    }
    if confirming {
        c.put(f.left, y, "Press ctrl+d again to confirm", base, f.room());
        return;
    }
    let footer_w = width(&o.footer);
    let text_room = if footer_w > 0 {
        f.room().saturating_sub(footer_w + 1)
    } else {
        f.room()
    };
    let title = truncate(&o.title, 61);
    let x = c.put(f.left, y, &title, base, text_room);
    if !o.description.is_empty() {
        let room = (f.left as usize + text_room).saturating_sub(x as usize);
        c.put(x, y, &format!(" {}", o.description), secondary, room);
    }
    if footer_w > 0 {
        let style = match (active, o.footer_tone) {
            (false, Some(t)) => tone(c, t),
            _ => secondary,
        };
        c.put(f.right - footer_w as u16, y, &o.footer, style, footer_w);
    }
}

fn body(c: &mut Canvas, b: &Body) {
    let height = c.buf.area.height;
    let rows = b.lines.len().min(height.saturating_sub(10) as usize);
    let footer = !b.footer.is_empty() || !b.actions.is_empty() || !b.buttons.is_empty();
    let panel_h = 3 + rows as u16 + if footer { 2 } else { 0 } + 1;
    let top = if b.centered {
        super::round(f64::from(height.saturating_sub(panel_h)) / 2.0)
    } else {
        height / 4
    };
    let f = Frame::new(c, b.size, 3, top);
    panel(c, &f, top + panel_h);
    header(c, &f, &b.title, &b.close);
    for (i, line) in b.lines.iter().skip(b.scroll).take(rows).enumerate() {
        put_line(c, f.left, top + 3 + i as u16, line, f.room());
    }
    if !footer {
        return;
    }
    let y = top + 3 + rows as u16 + 1;
    put_line(c, f.left, y, &b.footer, f.room());
    actions(c, &f, y, &b.actions);
    let total: usize =
        b.buttons.iter().map(|t| width(t) + 2).sum::<usize>() + b.buttons.len().saturating_sub(1);
    let mut x = f.right.saturating_sub(b.button_margin + total as u16);
    for (i, title) in b.buttons.iter().enumerate() {
        let w = width(title) as u16 + 2;
        if i == b.active {
            let bg = c
                .theme
                .term(c.theme.dialog.get("background.action.primary.focused"));
            c.bg(Rect::new(x, y, w, 1), bg);
            let fg = c
                .theme
                .term(c.theme.dialog.get("text.action.primary.focused"));
            c.put(x + 1, y, title, Style::new().fg(fg).bg(bg), w as usize);
        } else {
            let style = tone(c, Tone::Muted);
            c.put(x + 1, y, title, style, w as usize);
        }
        x += w + 1;
    }
}

fn prompt(c: &mut Canvas, p: &Prompt) -> Option<(u16, u16)> {
    let height = c.buf.area.height;
    let top = height / 4;
    let f = Frame::new(c, crate::dialog::Size::Medium, 3, top);
    panel(c, &f, top + 8);
    header(c, &f, &vec![(p.title.clone(), Tone::Bold)], "esc");
    let y = top + 3;
    if p.text.is_empty() {
        let style = tone(c, Tone::Muted);
        c.put(f.left, y, &p.placeholder, style, f.room());
    } else {
        let style = tone(c, Tone::Base);
        c.put(f.left, y, &p.text, style, f.room());
    }
    let footer = match &p.busy {
        Some(busy) => vec![(busy.clone(), Tone::Muted)],
        None => vec![
            ("submit".to_string(), Tone::Base),
            (" enter".to_string(), Tone::Muted),
        ],
    };
    put_line(c, f.left, top + 5, &footer, f.room());
    p.busy
        .is_none()
        .then_some((f.left + width(&p.text).min(f.room()) as u16, y))
}

fn confirm(c: &mut Canvas, k: &Confirm) {
    let mut b = Body::new(
        crate::dialog::BodyKind::Status,
        &k.title,
        crate::text::wrap(&k.message, 52)
            .into_iter()
            .map(|l| vec![(l, Tone::Muted)])
            .collect(),
    );
    b.buttons = vec!["Cancel".into(), k.button.clone()];
    b.active = usize::from(k.yes);
    body(c, &b);
}

fn export(c: &mut Canvas, e: &Export) {
    let radio = |on: bool| if on { "◉" } else { "○" };
    let check = |on: bool| if on { "[x]" } else { "[ ]" };
    let focus = |i: usize| if e.focus == i { Tone::Bold } else { Tone::Base };
    let mut lines: Vec<Line> = vec![
        vec![("Export as:".into(), Tone::Muted)],
        vec![
            (format!("{} Markdown", radio(!e.json)), focus(0)),
            ("  ".into(), Tone::Base),
            (format!("{} JSON", radio(e.json)), focus(0)),
        ],
        Vec::new(),
    ];
    if e.json {
        lines.push(vec![(
            format!("{} Sanitize sensitive data", check(e.sanitize)),
            focus(1),
        )]);
    } else {
        lines.push(vec![(
            format!("{} Include thinking", check(e.thinking)),
            focus(1),
        )]);
        lines.push(vec![(
            format!("{} Include tools", check(e.tools)),
            focus(2),
        )]);
    }
    let mut b = Body::new(crate::dialog::BodyKind::Status, "Export session", lines);
    b.buttons = vec!["Copy".into(), "Export".into()];
    b.active = e.focus.checked_sub(e.fields()).unwrap_or(usize::MAX);
    body(c, &b);
}
