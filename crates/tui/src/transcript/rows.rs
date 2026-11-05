//! Transcript items laid out as screen rows: user boxes, markdown, thinking, tools, footers.

use ratatui::style::{Color, Style};
use ratatui::text::Span;
use serde_json::Value;

use super::diff;
use super::grouping::{self, Groups, Kind};
use super::highlight::filetype;
use super::markdown::{self, Line, wrap_spans};
use super::model::{Assistant, Item, Part, Status};
use crate::state::Verbosity;
use crate::text::{self, duration, title_case};
use crate::theme::Theme;

pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// A running shell or subagent shows the background hint after this long.
pub const BACKGROUND_HINT_MS: u64 = 3000;

/// One screen line of the transcript, relative to the session pane's content box.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Row {
    /// `┃` drawn in this colour at column 0.
    pub rail: Option<Color>,
    /// Background filled across the row.
    pub bg: Option<Color>,
    pub indent: u16,
    pub spans: Line,
    /// An exploration group's header: a click toggles the group with this ID.
    pub group: Option<String>,
}

impl Row {
    pub fn text(indent: u16, spans: Line) -> Row {
        Row {
            indent,
            spans,
            ..Row::default()
        }
    }
}

pub struct Ctx<'a> {
    pub theme: &'a Theme,
    /// Width of the pane's content box (the pane minus its padding).
    pub width: usize,
    pub now: u64,
    pub animations: bool,
    /// Diffs side by side (content width > 120 under `diffs.view = auto`).
    pub split: bool,
    pub thinking: bool,
    pub source_markdown: bool,
    pub agent_color: &'a dyn Fn(&str) -> Color,
    pub background_key: Option<String>,
    /// Paths under this directory are shown relative to it.
    pub directory: &'a str,
    pub home: &'a str,
    /// `session.grouping` is not `none`.
    pub grouping: bool,
    /// Edit diffs wrap by word (`diffs.wrap`).
    pub diff_wrap: bool,
    pub verbosity: Verbosity,
    /// Groups the user opened (or closed, when they start open).
    pub toggled: &'a std::collections::HashSet<String>,
}

impl Ctx<'_> {
    fn style(&self, path: &str) -> Style {
        Style::new().fg(self.theme.fg(path))
    }

    pub(super) fn spinner(&self) -> &'static str {
        if self.animations {
            spinner_frame(self.now)
        } else {
            "⋯"
        }
    }

    pub fn path(&self, path: &str) -> String {
        if let Some(rest) = path
            .strip_prefix(self.directory)
            .and_then(|r| r.strip_prefix('/'))
        {
            return rest.to_string();
        }
        if !self.home.is_empty()
            && let Some(rest) = path.strip_prefix(self.home)
        {
            return format!("~{rest}");
        }
        path.to_string()
    }

    pub(super) fn block(&self, lines: Vec<Line>, awaiting: bool) -> Vec<Row> {
        let bg = self.theme.fg("background.raised.base");
        let rail = self.theme.fg("background.base");
        let mut rows = vec![Row {
            rail: Some(rail),
            bg: Some(bg),
            ..Row::default()
        }];
        for (i, mut spans) in lines.into_iter().enumerate() {
            if i == 0 && awaiting {
                for span in &mut spans {
                    span.style = span.style.fg(self.theme.fg("text.feedback.warning.base"));
                }
            }
            rows.push(Row {
                rail: Some(rail),
                bg: Some(bg),
                indent: 3,
                spans,
                group: None,
            });
        }
        rows.push(Row {
            rail: Some(rail),
            bg: Some(bg),
            ..Row::default()
        });
        rows
    }
}

/// The braille spinner frame at `now` (80 ms per frame).
pub fn spinner_frame(now: u64) -> &'static str {
    SPINNER[(now / 80) as usize % SPINNER.len()]
}

/// Laid-out groups of settled items, so a long transcript is not laid out again every frame.
/// Valid for one layout context (width and display settings).
#[derive(Default)]
pub struct Cache {
    context: String,
    entries: std::collections::HashMap<usize, (Item, String, Vec<Vec<Row>>)>,
    /// Header rows drawn in the last frame: `(screen row, group ID)`.
    pub hits: Vec<(u16, String)>,
    /// Where each message starts in the last layout: `(row, is a user message)`.
    pub messages: Vec<(usize, bool)>,
    /// The last frame's `(row count, visible height)`.
    pub viewport: (usize, usize),
}

/// An item that no longer changes with time (no spinner, no running duration).
fn settled(item: &Item) -> bool {
    match item {
        Item::Assistant(a) => {
            a.completed.is_some()
                && a.parts.iter().all(|p| match p {
                    Part::Tool(t) => !matches!(t.status, Status::Running | Status::Streaming),
                    Part::Reasoning { end, .. } => end.is_some(),
                    Part::Text { done, .. } => *done,
                })
        }
        Item::Compaction { running, .. } => !running,
        _ => true,
    }
}

/// Every item's rows, one blank row between items; with a cache, settled items whose content is
/// unchanged reuse their rows.
pub fn layout(items: &[Item], ctx: &Ctx, cache: Option<&std::sync::Mutex<Cache>>) -> Vec<Row> {
    let context = format!(
        "{}|{}|{}|{}|{}|{}|{:?}|{}|{}",
        ctx.width,
        ctx.split,
        ctx.thinking,
        ctx.source_markdown,
        ctx.animations,
        ctx.theme.name,
        ctx.theme.mode,
        ctx.grouping,
        ctx.verbosity as u8
    );
    let mut cache = cache.map(|c| c.lock().unwrap_or_else(|e| e.into_inner()));
    if let Some(cache) = cache.as_mut()
        && cache.context != context
    {
        cache.context = context;
        cache.entries.clear();
    }
    let exploration = Groups::new(
        items,
        ctx.grouping,
        ctx.verbosity == Verbosity::Low,
        ctx.verbosity == Verbosity::High,
        ctx.toggled,
    );
    let mut groups: Vec<Vec<Row>> = Vec::new();
    let mut firsts: Vec<(usize, bool)> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if matches!(item, Item::User { .. } | Item::Assistant(_)) {
            firsts.push((groups.len(), matches!(item, Item::User { .. })));
        }
        let signature = exploration.signature(index);
        if let Some(cache) = cache.as_ref()
            && let Some((cached, sig, rows)) = cache.entries.get(&index)
            && cached == item
            && *sig == signature
        {
            groups.extend(rows.iter().cloned());
            continue;
        }
        let start = groups.len();
        item_groups(item, index, &exploration, ctx, &mut groups);
        if let Some(cache) = cache.as_mut() {
            if settled(item) {
                let rows = groups[start..].to_vec();
                cache.entries.insert(index, (item.clone(), signature, rows));
            } else {
                cache.entries.remove(&index);
            }
        }
    }
    let mut rows = Vec::new();
    let mut starts = Vec::new();
    for group in groups {
        starts.push(rows.len() + 1);
        rows.push(Row::default());
        rows.extend(group);
    }
    if let Some(cache) = cache.as_mut() {
        cache.messages = firsts
            .into_iter()
            .filter_map(|(group, user)| starts.get(group).map(|row| (*row, user)))
            .collect();
    }
    rows
}

fn item_groups(
    item: &Item,
    index: usize,
    exploration: &Groups,
    ctx: &Ctx,
    groups: &mut Vec<Vec<Row>>,
) {
    match item {
        Item::User { text, agent, .. } => groups.push(user(text, agent, ctx)),
        Item::Assistant(a) => assistant(a, index, exploration, ctx, groups),
        Item::Shell {
            command,
            output,
            exit,
            timed_out,
        } => {
            let mut lines = vec![vec![Span::styled(
                format!("$ {command}"),
                ctx.style("text.base"),
            )]];
            lines.extend(collapsed_output(output, ctx));
            let error = if *timed_out {
                Some("Command timed out".to_string())
            } else {
                exit.filter(|c| *c != 0)
                    .map(|c| format!("Command exited with code {c}"))
            };
            if let Some(error) = error {
                lines.push(Vec::new());
                lines.push(vec![Span::styled(
                    error,
                    ctx.style("text.feedback.error.base"),
                )]);
            }
            groups.push(ctx.block(lines, false));
        }
        Item::Notice(text) => groups.push(vec![Row::text(
            3,
            vec![Span::styled(text.clone(), ctx.style("text.muted"))],
        )]),
        Item::Compaction {
            running,
            summary,
            error,
        } => {
            let label = if *running {
                format!(" {} Compaction ", ctx.spinner())
            } else if error.is_some() {
                " ✗ Compaction ".to_string()
            } else {
                " Compaction ".to_string()
            };
            let side = ctx.width.saturating_sub(3 + text::width(&label)) / 2;
            let rule = format!("{}{label}{}", "─".repeat(side), "─".repeat(side));
            let mut rows = vec![Row::text(
                3,
                vec![Span::styled(rule, ctx.style("border.base"))],
            )];
            if let Some(summary) = summary {
                rows.push(Row::default());
                rows.extend(
                    markdown::render(
                        summary,
                        ctx.width.saturating_sub(3),
                        ctx.theme,
                        ctx.source_markdown,
                    )
                    .into_iter()
                    .map(|l| Row::text(3, l)),
                );
            }
            groups.push(rows);
        }
    }
}

fn user(text: &str, agent: &str, ctx: &Ctx) -> Vec<Row> {
    let rail = Some((ctx.agent_color)(agent));
    let bg = Some(ctx.theme.fg("background.raised.base"));
    let style = ctx.style("text.base");
    let mut rows = vec![Row {
        rail,
        bg,
        ..Row::default()
    }];
    for line in wrap_spans(
        &[Span::styled(text.to_string(), style)],
        ctx.width.saturating_sub(4),
    ) {
        rows.push(Row {
            rail,
            bg,
            indent: 3,
            spans: line,
            group: None,
        });
    }
    rows.push(Row {
        rail,
        bg,
        ..Row::default()
    });
    rows
}

fn assistant(
    a: &Assistant,
    index: usize,
    exploration: &Groups,
    ctx: &Ctx,
    groups: &mut Vec<Vec<Row>>,
) {
    let mut thoughts: Vec<&Part> = Vec::new();
    let flush = |thoughts: &mut Vec<&Part>, groups: &mut Vec<Vec<Row>>| {
        if !thoughts.is_empty() {
            groups.push(thinking(thoughts, ctx));
            thoughts.clear();
        }
    };
    for (at, part) in a.parts.iter().enumerate() {
        if let Some(headers) = exploration.headers.get(&(index, at)) {
            flush(&mut thoughts, groups);
            groups.extend(headers.iter().map(|h| vec![group_header(h, ctx)]));
        }
        if exploration.hidden.contains(&(index, at)) {
            continue;
        }
        match part {
            Part::Reasoning { text, end, .. } => {
                let redacted_only = text.replace("[REDACTED]", "").trim().is_empty();
                if !(redacted_only && end.is_some()) {
                    thoughts.push(part);
                }
            }
            Part::Text { text, .. } => {
                flush(&mut thoughts, groups);
                if text.trim().is_empty() {
                    continue;
                }
                let lines = markdown::render(
                    text.trim(),
                    ctx.width.saturating_sub(3),
                    ctx.theme,
                    ctx.source_markdown,
                );
                groups.push(lines.into_iter().map(|l| Row::text(3, l)).collect());
            }
            Part::Tool(tool) => {
                flush(&mut thoughts, groups);
                groups.extend(super::tools::render(tool, ctx));
                if tool.status == Status::Running
                    && matches!(tool.name.as_str(), "shell" | "bash" | "task" | "subagent")
                    && ctx.now.saturating_sub(tool.started) >= BACKGROUND_HINT_MS
                    && let Some(key) = &ctx.background_key
                {
                    groups.push(vec![Row::text(
                        3,
                        vec![
                            Span::styled("Press ", ctx.style("text.muted")),
                            Span::styled(key.clone(), ctx.style("text.base")),
                            Span::styled(
                                " to move running work to the background",
                                ctx.style("text.muted"),
                            ),
                        ],
                    )]);
                }
            }
        }
    }
    flush(&mut thoughts, groups);
    if let Some(retry) = &a.retry {
        let seconds = retry.at.saturating_sub(ctx.now).div_ceil(1000);
        let head = if seconds > 0 {
            format!("Retrying in {seconds}s")
        } else {
            "Retry due".to_string()
        };
        groups.push(vec![Row::text(
            3,
            vec![Span::styled(
                format!("⚠ {head} · attempt {} · {}", retry.attempt, retry.message),
                ctx.style("text.feedback.warning.base"),
            )],
        )]);
    }
    if !a.terminal {
        return;
    }
    let mut rows = Vec::new();
    if let Some(error) = &a.error {
        rows.push(Row::text(
            3,
            vec![Span::styled(
                format!("Error: {error}"),
                ctx.style("text.feedback.error.base"),
            )],
        ));
        rows.push(Row::default());
    }
    let agent_style = if a.error.is_some() {
        ctx.style("text.muted")
    } else {
        Style::new().fg((ctx.agent_color)(&a.agent))
    };
    let mut spans = vec![Span::styled(title_case(&a.agent), agent_style)];
    let muted = ctx.style("text.muted");
    let screen = ctx.width + 4;
    if screen >= 28 {
        spans.push(Span::styled(format!(" · {}", a.model), muted));
    }
    if let Some(completed) = a.completed
        && !(28..36).contains(&screen)
    {
        let elapsed = completed.saturating_sub(a.turn_start.min(a.created));
        spans.push(Span::styled(format!(" · {}", duration(elapsed)), muted));
        let streamed = completed.saturating_sub(a.created);
        if a.output_tokens > 0 && streamed > 0 {
            let rate = a.output_tokens as f64 / (streamed as f64 / 1000.0);
            spans.push(Span::styled(format!(" · {rate:.1} tok/s"), muted));
        }
    }
    if a.interrupted {
        spans.push(Span::styled(" · interrupted", muted));
    }
    rows.push(Row::text(3, spans));
    groups.push(rows);
}

/// Strips a leading `**title**` (summary style) and returns it with the rest.
/// `→ Explored: 2 reads, 1 search`, or the spinner and `Exploring:` while a call runs.
/// Low verbosity: `+ 3 commands, 1 edit` (`−` when open, the spinner while closed and busy).
fn group_header(header: &grouping::Header, ctx: &Ctx) -> Row {
    let icon = match header.kind {
        Kind::Exploration { done: true } => "→",
        Kind::Exploration { done: false } => ctx.spinner(),
        Kind::Activity { .. } if header.open => "−",
        Kind::Activity { active: true } => ctx.spinner(),
        Kind::Activity { active: false } => "+",
    };
    Row {
        group: Some(header.id.clone()),
        ..Row::text(
            3,
            vec![Span::styled(
                format!("{icon:<2}{}", header.label),
                ctx.style("text.muted"),
            )],
        )
    }
}

fn thought_title(text: &str) -> (Option<String>, String) {
    let trimmed = text.trim_start();
    if let Some(rest) = trimmed.strip_prefix("**")
        && let Some(end) = rest.find("**")
    {
        return (
            Some(rest[..end].to_string()),
            rest[end + 2..].trim().to_string(),
        );
    }
    (None, text.to_string())
}

fn thinking(parts: &[&Part], ctx: &Ctx) -> Vec<Row> {
    let running = parts
        .iter()
        .any(|p| matches!(p, Part::Reasoning { end: None, .. }));
    let (start, end) = parts.iter().fold((u64::MAX, 0), |(s, e), p| match p {
        Part::Reasoning { start, end, .. } => (s.min(*start), e.max(end.unwrap_or(ctx.now))),
        _ => (s, e),
    });
    let last_text = parts.iter().rev().find_map(|p| match p {
        Part::Reasoning { text, .. } => Some(text.replace("[REDACTED]", "")),
        _ => None,
    });
    let title = last_text.as_deref().and_then(|t| thought_title(t).0);
    let warning = ctx.theme.get("text.feedback.warning.base");
    let done = Style::new().fg(ctx.theme.term(warning.with_alpha(0.6)));
    let mut rows = Vec::new();
    if running {
        let label = match &title {
            Some(title) => format!("{} Thinking: {title}", ctx.spinner()),
            None => format!("{} Thinking", ctx.spinner()),
        };
        rows.push(Row::text(
            3,
            vec![Span::styled(label, ctx.style("text.base"))],
        ));
    } else {
        let icon = if ctx.thinking { "-" } else { "+" };
        let mut label = format!("{icon} Thought");
        if !ctx.thinking
            && let Some(title) = &title
        {
            label.push_str(&format!(": {title}"));
        }
        if parts.len() > 1 {
            label.push_str(&format!(" · {} steps", parts.len()));
        }
        label.push_str(&format!(" · {}", duration(end.saturating_sub(start))));
        rows.push(Row::text(3, vec![Span::styled(label, done)]));
    }
    if ctx.thinking {
        let rail = Some(ctx.theme.fg("border.base"));
        for part in parts {
            if let Part::Reasoning { text, .. } = part {
                let body = thought_title(&text.replace("[REDACTED]", "")).1;
                for line in markdown::render(
                    &body,
                    ctx.width.saturating_sub(6),
                    ctx.theme,
                    ctx.source_markdown,
                ) {
                    let muted: Line = line
                        .into_iter()
                        .map(|s| Span::styled(s.content, s.style.fg(ctx.theme.fg("text.muted"))))
                        .collect();
                    rows.push(Row {
                        rail,
                        bg: None,
                        indent: 3,
                        spans: muted,
                        group: None,
                    });
                }
            }
        }
    }
    rows
}

/// Shell output under its command: the last 10 lines with an `(N earlier lines)` note.
/// What closes the transcript: `◇ Compaction queued` and the revert banner (a click on it
/// redoes, through the `revert` group id).
pub fn tail(
    queued: bool,
    revert: Option<&crate::state::Revert>,
    redo_key: Option<&str>,
    ctx: &Ctx,
) -> Vec<Row> {
    let mut rows = Vec::new();
    if queued {
        let label = " ◇ Compaction queued ";
        let side = ctx.width.saturating_sub(3 + text::width(label)) / 2;
        let rule = format!("{}{label}{}", "─".repeat(side), "─".repeat(side));
        rows.push(Row::text(
            3,
            vec![Span::styled(rule, ctx.style("border.base"))],
        ));
    }
    let Some(revert) = revert else { return rows };
    let bg = ctx.theme.fg("background.raised.base");
    let muted = ctx.style("text.muted");
    let base = ctx.style("text.base");
    let banner = |spans: Line| Row {
        rail: Some(bg),
        bg: Some(bg),
        indent: 3,
        spans,
        group: Some("revert".into()),
    };
    rows.push(Row::default());
    rows.push(banner(Vec::new()));
    let plural = if revert.count == 1 { "" } else { "s" };
    rows.push(banner(vec![Span::styled(
        format!("{} message{plural} reverted", revert.count),
        muted,
    )]));
    if !revert.files.is_empty() {
        rows.push(banner(Vec::new()));
        for (mark, path, added, removed) in &revert.files {
            let counts = (if *added > 0 {
                format!(" +{added}")
            } else {
                String::new()
            }) + &(if *removed > 0 {
                format!(" -{removed}")
            } else {
                String::new()
            });
            let room = ctx.width.saturating_sub(5 + text::width(&counts));
            let mut spans = vec![
                Span::styled(format!("{mark} "), muted),
                Span::styled(text::truncate_file_path(&ctx.path(path), room.max(2)), base),
            ];
            if *added > 0 {
                spans.push(Span::styled(
                    format!(" +{added}"),
                    Style::new().fg(ctx.theme.fg("diff.text.added")),
                ));
            }
            if *removed > 0 {
                spans.push(Span::styled(
                    format!(" -{removed}"),
                    Style::new().fg(ctx.theme.fg("diff.text.removed")),
                ));
            }
            rows.push(banner(spans));
        }
        rows.push(banner(Vec::new()));
    }
    let mut spans = Vec::new();
    if let Some(key) = redo_key {
        spans.push(Span::styled(key.to_string(), base));
        spans.push(Span::styled(" or /redo to restore", muted));
    } else {
        spans.push(Span::styled("/redo to restore", muted));
    }
    rows.push(banner(spans));
    rows.push(banner(Vec::new()));
    rows
}

pub fn collapsed_output(output: &str, ctx: &Ctx) -> Vec<Line> {
    let output = output.trim_end_matches('\n');
    if output.is_empty() {
        return Vec::new();
    }
    let muted = ctx.style("text.muted");
    let room = ctx.width.saturating_sub(7).max(20);
    let mut lines: Vec<Line> = wrap_spans(&[Span::styled(output.to_string(), muted)], room);
    let mut out = vec![Vec::new()];
    if lines.len() > 10 {
        let earlier = lines.len() - 9;
        lines.drain(..earlier);
        let s = if earlier == 1 { "" } else { "s" };
        out.push(vec![Span::styled(
            format!("({earlier} earlier line{s})"),
            muted,
        )]);
    }
    out.extend(lines);
    out
}

/// A patch from tool metadata (`files[0].patch`), as diff rows for a width.
pub fn patch_rows(
    metadata: &serde_json::Map<String, Value>,
    path: &str,
    width: usize,
    split: bool,
    wrap: bool,
    theme: &Theme,
) -> Option<Vec<Line>> {
    let patch = metadata
        .get("files")
        .and_then(|f| f.get(0))
        .and_then(|f| f.get("patch"))
        .or_else(|| metadata.get("diff"))
        .and_then(Value::as_str)?;
    let options = diff::Options {
        width,
        split,
        filetype: filetype(path),
        wrap,
    };
    Some(diff::render(patch, &options, theme))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_reuses_settled_items_and_drops_on_context_change() {
        let theme = crate::theme::Catalog::bundled()
            .load("anysola", crate::theme::Mode::Dark)
            .unwrap();
        let color = |_: &str| Color::Reset;
        let toggled = std::collections::HashSet::new();
        let ctx = |width| Ctx {
            theme: &theme,
            width,
            now: 0,
            animations: true,
            split: false,
            thinking: false,
            source_markdown: false,
            agent_color: &color,
            background_key: None,
            directory: "/w",
            home: "/h",
            grouping: true,
            diff_wrap: true,
            verbosity: Verbosity::Medium,
            toggled: &toggled,
        };
        let items = vec![
            Item::User {
                id: "m".into(),
                text: "hello".into(),
                agent: "build".into(),
                files: Vec::new(),
                created: 0,
            },
            Item::Compaction {
                running: true,
                summary: None,
                error: None,
            },
        ];
        let cache = std::sync::Mutex::new(Cache::default());
        let fresh = layout(&items, &ctx(80), None);
        assert_eq!(layout(&items, &ctx(80), Some(&cache)), fresh);
        assert_eq!(cache.lock().unwrap().entries.len(), 1);
        assert_eq!(layout(&items, &ctx(80), Some(&cache)), fresh);
        layout(&items, &ctx(60), Some(&cache));
        assert_eq!(cache.lock().unwrap().context.split('|').next(), Some("60"));
        assert_eq!(cache.lock().unwrap().messages, [(1, true)]);
    }
}
