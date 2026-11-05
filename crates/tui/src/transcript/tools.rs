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
        "shell" => return vec![shell(tool, ctx)],
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
            inline(icon, format!("{agent} Subagent — {description}"), tool, ctx)
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
        && let Some(error) = &tool.error
        && !denied(error)
        && name != "shell"
        && name != "todowrite"
    {
        let style = Style::new().fg(ctx.theme.fg("text.feedback.error.base"));
        groups[0].push(Row::text(5, vec![Span::styled(error.clone(), style)]));
    }
    groups
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
        None => tool.output.clone(),
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
        && let Some(error) = &tool.error
    {
        lines.push(Vec::new());
        let style = Style::new().fg(ctx.theme.fg("text.feedback.error.base"));
        lines.push(vec![Span::styled(error.clone(), style)]);
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
    for call in calls.into_iter().flatten() {
        let (Some(name), Some(status)) = (
            call.get("tool").and_then(Value::as_str),
            call.get("status").and_then(Value::as_str),
        ) else {
            continue;
        };
        let failed = status == "error";
        let input = call.get("input").cloned().unwrap_or(Value::Null);
        let color = if failed {
            ctx.theme.fg("text.feedback.error.base")
        } else {
            ctx.theme.fg("text.muted")
        };
        let summary = format!("{name}{}", arguments(&input));
        let text = crate::text::clip(&summary, ctx.width.saturating_sub(5));
        rows.push(Row::text(
            3,
            vec![Span::styled(
                format!("{:<2}{text}", if failed { "✗" } else { "›" }),
                Style::new().fg(color),
            )],
        ));
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
