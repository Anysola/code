//! Per-tool transcript rows: inline `icon text` rows and boxed blocks (shell, edit, write, todos,
//! questions).

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use serde_json::Value;

use super::highlight::{filetype, highlight};
use super::markdown::Line;
use super::model::{Status, Tool};
use super::rows::{Ctx, Row, collapsed_output, patch_rows};
use crate::text::{title_case, width};

/// Errors that mean the user declined rather than the tool failing.
fn denied(error: &str) -> bool {
    [
        "QuestionRejectedError",
        "rejected permission",
        "specified a rule",
        "user dismissed",
    ]
    .iter()
    .any(|needle| error.contains(needle))
}

fn canonical(name: &str) -> &str {
    match name {
        "bash" => "shell",
        "task" => "subagent",
        "apply_patch" => "patch",
        other => other,
    }
}

fn inline(icon: &str, text: String, tool: &Tool, ctx: &Ctx) -> Vec<Row> {
    let (icon, text, style) = if tool.status == Status::Streaming {
        (ctx.spinner().to_string(), text, ctx.theme.fg("text.muted"))
    } else if tool.awaiting {
        (
            icon.to_string(),
            text,
            ctx.theme.fg("text.feedback.warning.base"),
        )
    } else if tool.status == Status::Running {
        (ctx.spinner().to_string(), text, ctx.theme.fg("text.muted"))
    } else if tool.status == Status::Failed {
        (
            icon.to_string(),
            text,
            ctx.theme.fg("text.feedback.error.base"),
        )
    } else {
        (icon.to_string(), text, ctx.theme.fg("text.muted"))
    };
    let mut style = Style::new().fg(style);
    if tool.error.as_deref().is_some_and(denied) {
        style = Style::new()
            .fg(ctx.theme.fg("text.muted"))
            .add_modifier(Modifier::CROSSED_OUT);
    }
    let icon = format!("{icon:<2}");
    vec![Row::text(
        3,
        vec![Span::styled(format!("{icon}{text}"), style)],
    )]
}

fn primitive(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.split_whitespace().collect::<Vec<_>>().join(" ")),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn arguments(input: &Value) -> String {
    let pairs: Vec<String> = input
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| primitive(v).map(|v| format!("{k}={v}")))
        .collect();
    if pairs.is_empty() {
        String::new()
    } else {
        format!(" [{}]", pairs.join(", "))
    }
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn search(label: &str, tool: &Tool, count_key: &str, ctx: &Ctx) -> String {
    let pattern = tool.str("pattern").unwrap_or_default();
    let mut text = format!("{label} \"{pattern}\"");
    if let Some(path) = tool.str("path") {
        text.push_str(&format!(" in {}", ctx.path(path)));
    }
    if let Some(count) = tool.metadata.get(count_key).and_then(Value::as_u64) {
        text.push_str(&format!(" ({})", plural(count, "match", "matches")));
    }
    text
}

/// The rows of one tool call (usually one group).
pub fn render(tool: &Tool, ctx: &Ctx) -> Vec<Vec<Row>> {
    let name = canonical(&tool.name);
    let path = tool
        .str("filePath")
        .or_else(|| tool.str("path"))
        .map(|p| ctx.path(p))
        .unwrap_or_default();
    let pending = tool.status == Status::Streaming;
    let rows = match name {
        "shell" => shell(tool, ctx),
        "read" if pending => inline("→", "Reading file…".into(), tool, ctx),
        "read" => {
            let mut text = format!("Read {path}");
            let offset = tool.input.get("offset").and_then(Value::as_u64);
            let limit = tool.input.get("limit").and_then(Value::as_u64);
            if offset.is_some() || limit.is_some() {
                let start = offset.unwrap_or(1);
                let end = limit.map_or(String::new(), |l| (start + l - 1).to_string());
                text.push_str(&format!(":{start}-{end}"));
            }
            inline("→", text, tool, ctx)
        }
        "glob" if pending => inline("✱", "Finding files…".into(), tool, ctx),
        "glob" => inline("✱", search("Glob", tool, "count", ctx), tool, ctx),
        "grep" if pending => inline("✱", "Searching content…".into(), tool, ctx),
        "grep" => inline("✱", search("Grep", tool, "matches", ctx), tool, ctx),
        "webfetch" if pending => inline("%", "Fetching from the web…".into(), tool, ctx),
        "webfetch" => inline(
            "%",
            format!("WebFetch {}", tool.str("url").unwrap_or_default()),
            tool,
            ctx,
        ),
        "websearch" if pending => inline("◈", "Searching web…".into(), tool, ctx),
        "websearch" => {
            let query = tool.str("query").unwrap_or_default();
            inline(
                "◈",
                format!("Web Search via Anysola \"{query}\""),
                tool,
                ctx,
            )
        }
        "write" if tool.status == Status::Completed => write(tool, &path, ctx),
        "write" if pending => inline("←", "Preparing write…".into(), tool, ctx),
        "write" => inline("←", format!("Write {path}"), tool, ctx),
        "edit" | "patch" => edit(tool, &path, ctx),
        "todowrite" => todos(tool, ctx),
        "question" => questions(tool, ctx),
        "skill" if pending => inline("→", "Loading skill…".into(), tool, ctx),
        "skill" => inline(
            "→",
            format!("Skill \"{}\"", tool.str("name").unwrap_or_default()),
            tool,
            ctx,
        ),
        "subagent" if pending => inline("│", "Delegating…".into(), tool, ctx),
        "subagent" => {
            let agent = title_case(tool.str("subagent_type").unwrap_or("general"));
            let description = tool.str("description").unwrap_or_default();
            let icon = if tool.status == Status::Completed {
                "✓"
            } else {
                "│"
            };
            let mut rows = inline(icon, format!("{agent} Subagent — {description}"), tool, ctx);
            // A click opens the child session.
            if let Some(child) = tool.metadata.get("sessionID").and_then(Value::as_str)
                && let Some(row) = rows.first_mut()
            {
                row.group = Some(format!("child:{child}"));
            }
            let background = tool.status == Status::Completed
                && tool.metadata.get("status").and_then(Value::as_str) == Some("running");
            if background && let Some(row) = rows.first_mut() {
                let bg = ctx
                    .theme
                    .term(ctx.theme.decrease(ctx.theme.get("background.base"), 1));
                row.spans.push(Span::raw(" "));
                row.spans.push(Span::styled(
                    " Background ",
                    Style::new().fg(ctx.theme.fg("text.muted")).bg(bg),
                ));
            }
            rows
        }
        "execute" => {
            let icon = if execute_failed(tool) {
                "✗"
            } else if tool.status == Status::Completed {
                "✓"
            } else {
                "│"
            };
            let mut rows = inline(icon, "execute".into(), tool, ctx);
            for row in &mut rows {
                row.group = Some(format!("execute:{}", tool.call_id));
            }
            rows.extend(execute_calls(tool, ctx));
            rows
        }
        _ => {
            let icon = if tool.status == Status::Failed {
                "✗"
            } else {
                "✓"
            };
            inline(
                icon,
                format!("{}{}", tool.name, arguments(&tool.input)),
                tool,
                ctx,
            )
        }
    };
    let mut groups = vec![rows];
    if tool.status == Status::Failed
        && let Some(error) = shown_error(tool)
        && !denied(error)
        && name != "shell"
        && name != "todowrite"
    {
        let style = Style::new().fg(ctx.theme.fg("text.feedback.error.base"));
        groups[0].push(Row::text(5, vec![Span::styled(error.to_string(), style)]));
    }
    let entries = match name {
        "execute" => own_hooks(tool),
        _ => hook_entries(tool.metadata.get("hooks")),
    };
    add_hook_rows(&mut groups[0], &entries, ctx);
    groups
}

/// Prefixes of the notes the core adds for the model next to a tool's result; the hooks are
/// shown on their own rows, so the notes are cut from what the row shows.
const HOOK_NOTES: [&str; 3] = [
    "PreToolUse hook context: ",
    "PostToolUse hook feedback: ",
    "PostToolUse hook context: ",
];
/// How the core words a call a PreToolUse hook blocked (`hooks::fire::blocked`).
const HOOK_BLOCKED: &str = "A PreToolUse hook blocked this call";

/// `text` without the hook notes appended after `separator`.
fn without_notes<'a>(text: &'a str, separator: &str) -> &'a str {
    HOOK_NOTES
        .iter()
        .filter_map(|note| text.find(&format!("{separator}{note}")))
        .min()
        .map_or(text, |at| &text[..at])
}

/// The tool's error as its row shows it: none when a hook blocked the call (the hook row says
/// why), and without the hook notes.
fn shown_error(tool: &Tool) -> Option<&str> {
    let error = tool.error.as_deref()?;
    if tool.metadata.contains_key("hooks") {
        if error.starts_with(HOOK_BLOCKED) {
            return None;
        }
        return Some(without_notes(error, "\n\n"));
    }
    Some(error)
}

/// The tool's output without the hook notes the core appended as their own content.
fn shown_output(tool: &Tool) -> &str {
    if tool.metadata.contains_key("hooks") {
        without_notes(&tool.output, "\n")
    } else {
        &tool.output
    }
}

/// What a hook did to a call: `(outcome, message)` of each `metadata.hooks` entry.
pub fn hook_entries(hooks: Option<&Value>) -> Vec<(String, String)> {
    hooks
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|entry| {
            let text = |key: &str| {
                entry
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            (text("outcome"), text("message"))
        })
        .collect()
}

/// `execute`'s own entries: its `hooks` array lists the nested calls' entries first.
fn own_hooks(tool: &Tool) -> Vec<(String, String)> {
    let nested: usize = tool
        .metadata
        .get("toolCalls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|call| hook_entries(call.get("hooks")).len())
        .sum();
    let mut all = hook_entries(tool.metadata.get("hooks"));
    all.drain(..nested.min(all.len()));
    all
}

/// Whether an entry keeps its tool in view inside a closed group: the user should see it.
pub fn loud(outcome: &str) -> bool {
    outcome != "context"
}

/// One line for a hook entry: what happened, the first line of what the hook said, and the
/// theme colour (warning when the call was stopped or the hook failed).
pub fn hook_line(outcome: &str, message: &str) -> (String, &'static str) {
    let first = crate::text::visible(
        &crate::text::strip_ansi(message)
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    );
    let (label, path) = match outcome {
        "blocked" => ("Blocked by hook", "text.feedback.warning.base"),
        "error" => ("Hook failed", "text.feedback.warning.base"),
        "feedback" => ("Hook feedback", "text.muted"),
        "context" => ("Hook added context", "text.muted"),
        _ => ("Hook", "text.muted"),
    };
    let text = if first.is_empty() {
        label.to_string()
    } else {
        format!("{label}: {first}")
    };
    (text, path)
}

/// One `↳` row per entry under the tool: inside its block when it has one, else under its text.
fn add_hook_rows(rows: &mut Vec<Row>, entries: &[(String, String)], ctx: &Ctx) {
    if entries.is_empty() {
        return;
    }
    let block = rows.last().filter(|r| r.bg.is_some()).cloned();
    let indent = if block.is_some() { 3 } else { 5 };
    let room = ctx.width.saturating_sub(indent as usize + 3).max(10);
    let lines: Vec<Row> = entries
        .iter()
        .map(|(outcome, message)| {
            let (text, path) = hook_line(outcome, message);
            let style = Style::new().fg(ctx.theme.fg(path));
            let text = crate::text::truncate(&text, room);
            Row::text(indent, vec![Span::styled(format!("↳ {text}"), style)])
        })
        .collect();
    match block {
        // Before the block's closing padding row, in its colours, after a blank line.
        Some(padding) => {
            let at = rows.len() - 1;
            let fill = |row: Row| Row {
                rail: padding.rail,
                bg: padding.bg,
                group: padding.group.clone(),
                ..row
            };
            let mut inside = vec![fill(Row::default())];
            inside.extend(lines.into_iter().map(fill));
            rows.splice(at..at, inside);
        }
        None => rows.extend(lines),
    }
}

/// The shell ID of a command moved to the background (`metadata.status = running`).
pub fn shell_id(tool: &Tool) -> Option<&str> {
    (tool.name == "shell"
        && tool.status == Status::Completed
        && tool.metadata.get("status").and_then(Value::as_str) == Some("running"))
    .then(|| tool.metadata.get("shellID").and_then(Value::as_str))
    .flatten()
}

fn shell(tool: &Tool, ctx: &Ctx) -> Vec<Row> {
    let base = Style::new().fg(ctx.theme.fg("text.base"));
    let muted = Style::new().fg(ctx.theme.fg("text.muted"));
    let Some(command) = tool.str("command") else {
        let text = format!("{} Writing command…", ctx.spinner());
        return ctx.block(vec![vec![Span::styled(text, base)]], tool.awaiting);
    };
    let command = match tool.str("workdir").filter(|w| *w != ".") {
        Some(dir) => format!("cd {dir} && {command}"),
        None => command.to_string(),
    };
    let background = shell_id(tool).map(|id| ctx.shells.get(id).cloned().unwrap_or_default());
    let hit = format!("shell:{}", tool.call_id);
    let expanded = ctx.toggled.contains(&hit);
    let running = tool.status == Status::Running || background.as_ref().is_some_and(|b| b.running);
    let mut lines = if running {
        vec![vec![Span::styled(
            format!("{} {command}", ctx.spinner()),
            base,
        )]]
    } else {
        vec![vec![Span::styled(format!("$ {command}"), base)]]
    };
    let output = match &background {
        Some(_) if !expanded => String::new(),
        Some(view) if view.truncated => {
            format!(
                "[earlier output omitted]\n{}",
                crate::text::strip_ansi(view.output.trim())
            )
        }
        Some(view) => crate::text::strip_ansi(view.output.trim()),
        None if tool.status == Status::Running => String::new(),
        None => shown_output(tool).to_string(),
    };
    let collapsed = collapsed_output(&output, ctx);
    let overflow = collapsed.get(1).is_some_and(|l| {
        l.first().is_some_and(|s| {
            s.content.ends_with("earlier lines)") || s.content.ends_with("earlier line)")
        })
    });
    if expanded && !output.trim_end().is_empty() {
        let room = ctx.width.saturating_sub(7).max(20);
        lines.push(Vec::new());
        lines.extend(super::markdown::wrap_spans(
            &[Span::styled(
                output.trim_end_matches('\n').to_string(),
                muted,
            )],
            room,
        ));
    } else {
        lines.extend(collapsed);
    }
    if tool.status == Status::Failed
        && let Some(error) = shown_error(tool)
    {
        lines.push(Vec::new());
        let style = Style::new().fg(ctx.theme.fg("text.feedback.error.base"));
        lines.push(vec![Span::styled(error.to_string(), style)]);
    }
    if background.is_some() {
        let raised = ctx.theme.get("background.raised.base");
        let badge = muted.bg(ctx.theme.term(ctx.theme.decrease(raised, 1)));
        lines.push(Vec::new());
        lines.push(vec![Span::styled(" Background ", badge)]);
    }
    let mut rows = ctx.block(lines, tool.awaiting);
    if background.is_some() || overflow || expanded {
        for row in &mut rows {
            row.group = Some(hit.clone());
        }
    }
    rows
}

fn header(label: &str, value: &str, ctx: &Ctx) -> Line {
    let base = Style::new().fg(ctx.theme.fg("text.base"));
    let room = ctx.width.saturating_sub(4 + width(label) + 1);
    vec![Span::styled(
        format!("{label} {}", crate::text::path(value, room)),
        base,
    )]
}

/// The program failed (a runtime error, not a failing nested call).
fn execute_failed(tool: &Tool) -> bool {
    tool.metadata.get("error") == Some(&Value::Bool(true)) || tool.status == Status::Failed
}

/// The nested calls an `execute` program made (`› tool [k=v]`) and its runtime error preview.
fn execute_calls(tool: &Tool, ctx: &Ctx) -> Vec<Row> {
    let mut rows = Vec::new();
    let calls = tool.metadata.get("toolCalls").and_then(Value::as_array);
    for (index, call) in calls.into_iter().flatten().enumerate() {
        let (Some(name), Some(status)) = (
            call.get("tool").and_then(Value::as_str),
            call.get("status").and_then(Value::as_str),
        ) else {
            continue;
        };
        let failed = status == "error";
        let input = call.get("input").cloned().unwrap_or(Value::Null);
        let fields = input.as_object().filter(|o| !o.is_empty());
        let hit = format!("call:{}:{index}", tool.call_id);
        let expanded = fields.is_some() && ctx.toggled.contains(&hit);
        let expanded_color = ctx
            .theme
            .term(ctx.theme.decrease(ctx.theme.get("text.muted"), 1));
        let color = if failed {
            ctx.theme.fg("text.feedback.error.base")
        } else if expanded {
            expanded_color
        } else {
            ctx.theme.fg("text.muted")
        };
        let summary = if expanded {
            name.to_string()
        } else {
            format!("{name}{}", arguments(&input))
        };
        let text = crate::text::clip(&summary, ctx.width.saturating_sub(5));
        let mut row = Row::text(
            3,
            vec![Span::styled(
                format!("{:<2}{text}", if failed { "✗" } else { "›" }),
                Style::new().fg(color),
            )],
        );
        if fields.is_some() {
            row.group = Some(hit.clone());
        }
        rows.push(row);
        add_hook_rows(&mut rows, &hook_entries(call.get("hooks")), ctx);
        let Some(fields) = fields.filter(|_| expanded) else {
            continue;
        };
        let muted = Style::new().fg(ctx.theme.fg("text.muted"));
        let base = Style::new().fg(ctx.theme.fg("text.base"));
        let border = Span::styled("│ ", Style::new().fg(expanded_color));
        for (key, value) in fields {
            let label = format!("{key}: ");
            let value = match value {
                Value::String(text) => text.clone(),
                other => serde_json::to_string_pretty(other).unwrap_or_default(),
            };
            let room = ctx.width.saturating_sub(5 + width(&label)).max(1);
            for (i, line) in super::markdown::wrap_spans(&[Span::styled(value, base)], room)
                .into_iter()
                .enumerate()
            {
                let lead = if i == 0 {
                    Span::styled(label.clone(), muted)
                } else {
                    Span::raw(" ".repeat(width(&label)))
                };
                let mut spans = vec![border.clone(), lead];
                spans.extend(line);
                let mut row = Row::text(3, spans);
                row.group = Some(hit.clone());
                rows.push(row);
            }
        }
    }
    let output = crate::text::strip_ansi(tool.output.trim());
    if execute_failed(tool) && !output.is_empty() {
        let style = Style::new().fg(ctx.theme.fg("text.feedback.error.base"));
        for (i, line) in output.lines().take(4).enumerate() {
            let text = crate::text::clip(line, ctx.width.saturating_sub(9));
            let lead = if i == 0 { "↳ " } else { "  " };
            rows.push(Row::text(
                6,
                vec![Span::styled(format!("{lead}{text}"), style)],
            ));
        }
    }
    rows
}

fn edit(tool: &Tool, path: &str, ctx: &Ctx) -> Vec<Row> {
    let inner = ctx.width.saturating_sub(4);
    let diff_width = inner.saturating_sub(4);
    match patch_rows(
        &tool.metadata,
        path,
        diff_width,
        ctx.split,
        ctx.diff_wrap,
        ctx.theme,
    ) {
        Some(diff) => {
            let mut lines = vec![header("← Edit", path, ctx), Vec::new()];
            lines.extend(diff.into_iter().map(|mut l| {
                l.insert(0, Span::raw(" "));
                l
            }));
            ctx.block(lines, tool.awaiting)
        }
        None if path.is_empty() => {
            let text = format!("{} Preparing edit…", ctx.spinner());
            ctx.block(
                vec![vec![Span::styled(
                    text,
                    Style::new().fg(ctx.theme.fg("text.base")),
                )]],
                tool.awaiting,
            )
        }
        None => ctx.block(vec![header("← Edit", path, ctx)], tool.awaiting),
    }
}

fn write(tool: &Tool, path: &str, ctx: &Ctx) -> Vec<Row> {
    let content = tool.str("content").unwrap_or_default();
    let numbers = content.lines().count().max(1).to_string().len().max(3);
    let number_style = Style::new().fg(ctx.theme.fg("text.muted"));
    let mut lines = vec![header("# Wrote", path, ctx), Vec::new()];
    for (i, line) in highlight(content, filetype(path), ctx.theme)
        .into_iter()
        .enumerate()
    {
        let mut row = vec![Span::styled(format!("{:>numbers$} ", i + 1), number_style)];
        row.extend(line);
        lines.push(row);
    }
    ctx.block(lines, tool.awaiting)
}

fn todos(tool: &Tool, ctx: &Ctx) -> Vec<Row> {
    let items = tool
        .input
        .get("todos")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    match tool.status {
        Status::Streaming => inline("⚙", "Updating todos…".into(), tool, ctx),
        Status::Completed if !items.is_empty() => {
            let mut lines = vec![header("#", "Todos", ctx), Vec::new()];
            for item in items {
                let content = item
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let status = item
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let (marker, path, modifier) = match status {
                    "completed" => ("[✓]", "text.base", Modifier::empty()),
                    "in_progress" => ("[•]", "text.feedback.warning.base", Modifier::empty()),
                    "cancelled" => ("[-]", "text.faint", Modifier::CROSSED_OUT),
                    _ => ("[ ]", "text.base", Modifier::empty()),
                };
                let style = Style::new().fg(ctx.theme.fg(path)).add_modifier(modifier);
                lines.push(vec![Span::styled(format!("{marker} {content}"), style)]);
            }
            ctx.block(lines, tool.awaiting)
        }
        Status::Completed => inline("⚙", "Cleared todos".into(), tool, ctx),
        _ => inline("⚙", "Todo update failed".into(), tool, ctx),
    }
}

fn questions(tool: &Tool, ctx: &Ctx) -> Vec<Row> {
    let asked = tool
        .input
        .get("questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if tool.status == Status::Streaming {
        return inline("→", "Asking questions…".into(), tool, ctx);
    }
    let answers = tool.metadata.get("answers").and_then(Value::as_array);
    match answers {
        Some(answers) if tool.status == Status::Completed => {
            let muted = Style::new().fg(ctx.theme.fg("text.muted"));
            let base = Style::new().fg(ctx.theme.fg("text.base"));
            let mut lines = vec![header("#", "Questions", ctx), Vec::new()];
            for (i, question) in asked.iter().enumerate() {
                let text = question
                    .get("question")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let answer: Vec<&str> = answers
                    .get(i)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                let answer = if answer.is_empty() {
                    "(no answer)".to_string()
                } else {
                    answer.join(", ")
                };
                lines.push(vec![Span::styled(text.to_string(), muted)]);
                lines.push(vec![Span::styled(answer, base)]);
            }
            ctx.block(lines, false)
        }
        _ => {
            let n = asked.len() as u64;
            inline(
                "→",
                format!("Asked {}", plural(n, "question", "questions")),
                tool,
                ctx,
            )
        }
    }
}
