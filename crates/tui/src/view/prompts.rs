//! Permission and question prompts, drawn in place of the prompt box.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use serde_json::Value;

use super::Canvas;
use super::session::draw_row;
use crate::state::{Permission, Question, State};
use crate::text::{self, width};
use crate::transcript::markdown::Line;
use crate::transcript::rows::{Row, patch_rows};

const MAX_HEIGHT: u16 = 15;

fn meta<'a>(p: &'a Permission, key: &str) -> Option<&'a str> {
    p.metadata.get(key).and_then(Value::as_str)
}

fn first(p: &Permission) -> String {
    p.resources.first().cloned().unwrap_or_default()
}

/// The title row (`→ Edit <path>`) and the body lines of an ask, per action.
fn presentation(p: &Permission) -> (Option<String>, Vec<String>) {
    let subject = p.subject.clone();
    match p.action.as_str() {
        "shell" => {
            let command = subject.unwrap_or_else(|| p.resources.join(" && "));
            (None, vec![format!("$ {command}")])
        }
        "edit" => (
            Some(format!("→ Edit {}", subject.unwrap_or_else(|| first(p)))),
            Vec::new(),
        ),
        "read" => {
            let path = subject.unwrap_or_else(|| first(p));
            (
                Some(format!("→ Read {path}")),
                vec![format!("Path: {path}")],
            )
        }
        "glob" | "grep" => {
            let pattern = meta(p, "pattern").map_or_else(|| first(p), str::to_string);
            let label = text::title_case(&p.action);
            (
                Some(format!("✱ {label} \"{pattern}\"")),
                vec![format!("Pattern: {pattern}")],
            )
        }
        "webfetch" => {
            let url = meta(p, "url").map_or_else(|| first(p), str::to_string);
            (
                Some(format!("% WebFetch {url}")),
                vec![format!("URL: {url}")],
            )
        }
        "websearch" => {
            let query = meta(p, "query").map_or_else(|| first(p), str::to_string);
            (
                Some(format!("◈ Web Search via Anysola \"{query}\"")),
                vec![format!("Query: {query}")],
            )
        }
        "external_directory" => {
            let directory = subject.unwrap_or_else(|| first(p));
            let mut lines = vec!["Patterns".to_string()];
            lines.extend(p.resources.iter().map(|r| format!("- {r}")));
            (
                Some(format!("← Access external directory {directory}")),
                lines,
            )
        }
        "task" | "subagent" => {
            let agent = text::title_case(meta(p, "subagent_type").unwrap_or("general"));
            let description = meta(p, "description").unwrap_or_default();
            (
                Some(format!("# {agent} Subagent")),
                vec![format!("◉ {description}")],
            )
        }
        action => (
            Some(format!("⚙ Call tool {action}")),
            vec![format!("Tool: {action}")],
        ),
    }
}

fn always_text(p: &Permission) -> Vec<String> {
    if p.save == ["*"] {
        vec![format!(
            "This will always allow {} for this project.",
            p.action
        )]
    } else {
        let mut lines =
            vec!["This will always allow the following patterns for this project.".to_string()];
        lines.extend(p.save.iter().map(|s| format!("- {s}")));
        lines
    }
}

fn narrow(state: &State) -> bool {
    state.width < 80
}

fn body(c: &Canvas, state: &State, p: &Permission, w: u16) -> (Vec<Line>, bool) {
    let muted = c.style("text.base");
    let text_lines = |lines: Vec<String>| {
        lines
            .into_iter()
            .map(|l| vec![Span::styled(l, muted)])
            .collect()
    };
    if p.options().get(p.selected) == Some(&"Always allow") {
        return (text_lines(always_text(p)), false);
    }
    let (_, lines) = presentation(p);
    if p.action == "edit" {
        let map = p.metadata.as_object().cloned().unwrap_or_default();
        let path = p.subject.clone().unwrap_or_else(|| first(p));
        let room = w.saturating_sub(5) as usize;
        return match patch_rows(&map, &path, room, state.width > 120, c.theme) {
            Some(diff) => (diff, true),
            None => (text_lines(vec!["No diff provided".into()]), false),
        };
    }
    (text_lines(lines), false)
}

pub fn permission_height(c: &Canvas, state: &State, p: &Permission, w: u16) -> u16 {
    let (title, _) = presentation(p);
    let (lines, grows) = body(c, state, p, w);
    let header = 1 + u16::from(title.is_some());
    let footer = if narrow(state) {
        3 + p.options().len() as u16
    } else {
        3
    };
    let fixed = 1 + header + 1 + 1 + footer;
    if grows {
        MAX_HEIGHT
            .min(state.height.saturating_sub(4))
            .max(fixed + 1)
    } else {
        (fixed + lines.len() as u16).min(MAX_HEIGHT)
    }
}

pub fn permission(c: &mut Canvas, state: &State, p: &Permission, x: u16, y: u16, w: u16, h: u16) {
    let bg = c.theme.fg("background.raised.base");
    let footer_bg = c
        .theme
        .term(c.theme.decrease(c.theme.get("background.raised.base"), 1));
    let rail_color = c.theme.fg("background.action.primary.focused");
    c.fill(Rect::new(x, y, w, h), bg);
    let footer_h = if narrow(state) {
        3 + p.options().len() as u16
    } else {
        3
    };
    c.fill(
        Rect::new(x + 1, y + h - footer_h, w - 1, footer_h),
        footer_bg,
    );
    for row in y..y + h {
        let row_bg = if row >= y + h - footer_h {
            footer_bg
        } else {
            bg
        };
        c.put(x, row, "┃", Style::new().fg(rail_color).bg(row_bg), 1);
    }
    let base = c.style("text.base").bg(bg);
    let end = c.put(
        x + 3,
        y + 1,
        "△",
        c.style("text.feedback.warning.base").bg(bg),
        1,
    );
    c.put(end + 1, y + 1, "Permission required", base, w as usize);
    let (title, _) = presentation(p);
    let mut at = y + 2;
    if let Some(title) = title {
        c.put(x + 5, at, &title, base, w.saturating_sub(8) as usize);
        at += 1;
    }
    at += 1;
    let (lines, diff) = body(c, state, p, w);
    let room = (y + h - footer_h - 1).saturating_sub(at) as usize;
    let indent = if diff { 1 } else { 2 };
    for line in lines.into_iter().take(room) {
        let row = Row {
            rail: None,
            bg: Some(bg),
            indent,
            spans: line,
        };
        draw_row(c, &row, x + 1, at, w - 1);
        at += 1;
    }
    let options = p.options();
    let mut bx = x + 3;
    let mut by = y + h - footer_h + 1;
    for (i, option) in options.iter().enumerate() {
        let selected = i == p.selected;
        let (fg, button_bg): (Color, Color) = if selected {
            (
                c.theme.fg("text.action.primary.focused"),
                c.theme.fg("background.action.primary.focused"),
            )
        } else {
            (c.theme.fg("text.action.primary.base"), footer_bg)
        };
        let label = format!(" {option} ");
        c.put(
            bx,
            by,
            &label,
            Style::new().fg(fg).bg(button_bg),
            label.len(),
        );
        if narrow(state) {
            by += 1;
        } else {
            bx += width(&label) as u16 + 1;
        }
    }
    let base = c.style("text.base").bg(footer_bg);
    let muted = c.style("text.muted").bg(footer_bg);
    let mut hints: Vec<Span> = Vec::new();
    if let Some(key) = &state.hints.fullscreen {
        hints.push(Span::styled(format!("{key} "), base));
        hints.push(Span::styled(
            if p.fullscreen {
                "minimize"
            } else {
                "fullscreen"
            },
            muted,
        ));
        hints.push(Span::styled("  ", muted));
    }
    if options.len() > 1 {
        hints.push(Span::styled("⇆ ", base));
        hints.push(Span::styled("select", muted));
        hints.push(Span::styled("  ", muted));
    }
    hints.push(Span::styled("enter ", base));
    hints.push(Span::styled("confirm", muted));
    let total: usize = hints.iter().map(|s| width(&s.content)).sum();
    let hx = if narrow(state) {
        x + 3
    } else {
        (x + w).saturating_sub(3 + total as u16)
    };
    let hy = if narrow(state) { by } else { y + h - 2 };
    c.spans(hx, hy, &hints, total);
}

pub fn question_height(q: &Question, w: u16) -> u16 {
    let Some(current) = q.questions.get(q.tab) else {
        return 6 + q.questions.len() as u16;
    };
    let text_rows = text::wrap(&current.question, w.saturating_sub(8) as usize).len() as u16;
    (6 + text_rows + current.options.len() as u16 + 1 + u16::from(q.questions.len() > 1))
        .min(MAX_HEIGHT * 2)
}

pub fn question(c: &mut Canvas, q: &Question, x: u16, y: u16, w: u16) {
    let h = question_height(q, w);
    let bg = c.theme.fg("background.raised.base");
    c.fill(Rect::new(x, y, w, h), bg);
    let rail = Style::new()
        .fg(c.theme.fg("background.action.primary.focused"))
        .bg(bg);
    for row in y..y + h {
        c.put(x, row, "┃", rail, 1);
    }
    let base = c.style("text.base").bg(bg);
    let muted = c.style("text.muted").bg(bg);
    let room = w.saturating_sub(8) as usize;
    let mut at = y + 1;
    if q.questions.len() > 1 {
        let done = q.answers.iter().filter(|a| !a.is_empty()).count();
        let line = format!(
            "Field {} of {} · {done}/{} completed",
            q.tab + 1,
            q.questions.len(),
            q.questions.len()
        );
        c.put(x + 3, at, &line, muted, room);
        at += 1;
    }
    let Some(current) = q.questions.get(q.tab) else {
        for (i, question) in q.questions.iter().enumerate() {
            let answer = q
                .answers
                .get(i)
                .filter(|a| !a.is_empty())
                .map_or("(not answered)".to_string(), |a| a.join(", "));
            c.put(
                x + 3,
                at,
                &format!("{}: {answer}", question.header),
                base,
                room,
            );
            at += 1;
        }
        hints(c, x, y + h - 2, w, "submit", bg);
        return;
    };
    c.put(x + 3, at, &current.header, muted, room);
    at += 1;
    for line in text::wrap(&current.question, room) {
        c.put(x + 3, at, &line, base, room);
        at += 1;
    }
    at += 1;
    let multiple = current.multiple == Some(true);
    let chosen = q.answers.get(q.tab).cloned().unwrap_or_default();
    let mut labels: Vec<(String, String)> = current
        .options
        .iter()
        .map(|o| (o.label.clone(), o.description.clone()))
        .collect();
    labels.push(("Type your own answer".to_string(), String::new()));
    for (i, (label, description)) in labels.iter().enumerate() {
        let selected = i == q.selected;
        let picked = chosen.contains(label);
        let mark = if multiple && i + 1 < labels.len() {
            if picked { "[✓] " } else { "[ ] " }
        } else {
            ""
        };
        let shown = if i + 1 == labels.len() {
            q.custom.clone().unwrap_or_else(|| label.clone())
        } else {
            label.clone()
        };
        let check = if !multiple && picked { " ✓" } else { "" };
        let style = if selected {
            c.style("text.action.primary.selected").bg(bg)
        } else {
            base
        };
        let end = c.put(
            x + 3,
            at,
            &format!("{}. {mark}{shown}{check}", i + 1),
            style,
            room,
        );
        if !description.is_empty() {
            c.put(
                end + 1,
                at,
                description,
                muted,
                room.saturating_sub((end - x) as usize),
            );
        }
        at += 1;
    }
    let action = if multiple {
        "toggle"
    } else if q.questions.len() == 1 {
        "submit"
    } else {
        "confirm"
    };
    hints(c, x, y + h - 2, w, action, bg);
}

fn hints(c: &mut Canvas, x: u16, y: u16, w: u16, action: &str, bg: Color) {
    let base = c.style("text.base").bg(bg);
    let muted = c.style("text.muted").bg(bg);
    let spans = vec![
        Span::styled("⇆ ", base),
        Span::styled("tab  ", muted),
        Span::styled("↑↓ ", base),
        Span::styled("select  ", muted),
        Span::styled("enter ", base),
        Span::styled(format!("{action}  "), muted),
        Span::styled("esc ", base),
        Span::styled("dismiss", muted),
    ];
    c.spans(x + 3, y, &spans, w.saturating_sub(6) as usize);
}
