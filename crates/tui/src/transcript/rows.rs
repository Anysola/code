//! Transcript items laid out as screen rows: user boxes, markdown, thinking, tools, footers.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use serde_json::Value;

use super::diff;
use super::grouping::{self, Groups, Kind};
use super::highlight::filetype;
use super::markdown::{self, Line, wrap_spans};
use super::model::{Assistant, Item, Part, Status, Tool};
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
    pub turn_tokens: crate::state::TurnTokens,
    /// Items before this one are not laid out yet (tail-first mounting).
    pub floor: usize,
    /// Background shells by shell ID, as last polled.
    pub shells: &'a std::collections::HashMap<String, BackgroundView>,
}

/// What the transcript knows of a background shell: whether it still runs, and its output
/// (the last 1 MiB, read while its row is expanded).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BackgroundView {
    pub running: bool,
    pub output: String,
    pub truncated: bool,
    pub file: Option<std::path::PathBuf>,
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

/// The opened tool rows of an item (`shell:<call>`, `call:<call>:<n>`), which change its layout.
fn tool_toggles(item: &Item, toggled: &std::collections::HashSet<String>) -> String {
    let Item::Assistant(a) = item else {
        return String::new();
    };
    let mut keys: Vec<&str> = toggled
        .iter()
        .filter(|key| {
            a.parts.iter().any(|p| match p {
                Part::Tool(t) => {
                    key.strip_prefix("shell:") == Some(t.call_id.as_str())
                        || key
                            .strip_prefix("call:")
                            .and_then(|k| k.strip_prefix(t.call_id.as_str()))
                            .is_some_and(|k| k.starts_with(':'))
                }
                _ => false,
            })
        })
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys.join(",")
}

/// Rendered entries newest first mount on open (`routes/session/index.tsx:147-148`).
pub const TAIL_ROWS: usize = 40;
/// Older entries mount in chunks this big as the reader nears the top.
pub const BACKFILL_CHUNK: usize = 60;

/// The first item to lay out so that `budget` entries from `end` back are shown; an assistant
/// message counts one entry per part.
pub fn floor_before(items: &[Item], end: usize, budget: usize) -> usize {
    let mut used = 0;
    let mut at = end.min(items.len());
    while at > 0 && used < budget {
        at -= 1;
        used += match &items[at] {
            Item::Assistant(a) => a.parts.len().max(1),
            _ => 1,
        };
    }
    at
}

/// An item that no longer changes with time (no spinner, no running duration).
fn settled(item: &Item) -> bool {
    match item {
        Item::Assistant(a) => {
            a.completed.is_some()
                && a.parts.iter().all(|p| match p {
                    Part::Tool(t) => {
                        !matches!(t.status, Status::Running | Status::Streaming)
                            && super::tools::shell_id(t).is_none()
                    }
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
    let mut usage = TurnUsage::default();
    if ctx.turn_tokens != crate::state::TurnTokens::Off {
        for item in &items[..ctx.floor.min(items.len())] {
            usage.add(item);
        }
    }
    for (index, item) in items.iter().enumerate().skip(ctx.floor) {
        if matches!(item, Item::User { .. } | Item::Assistant(_)) {
            firsts.push((groups.len(), matches!(item, Item::User { .. })));
        }
        layout_item(
            item,
            index,
            &exploration,
            ctx,
            cache.as_deref_mut(),
            &mut groups,
        );
        if ctx.turn_tokens != crate::state::TurnTokens::Off
            && let Some(steps) = usage.add(item)
        {
            groups.push(turn_tokens(&steps, usage.previous.as_ref(), ctx));
            usage.close(&steps);
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

/// One item's groups, reused from the cache while the item and its layout keys are unchanged.
fn layout_item(
    item: &Item,
    index: usize,
    exploration: &Groups,
    ctx: &Ctx,
    cache: Option<&mut Cache>,
    groups: &mut Vec<Vec<Row>>,
) {
    let signature = exploration.signature(index) + &tool_toggles(item, ctx.toggled);
    if let Some(cache) = cache.as_ref()
        && let Some((cached, sig, rows)) = cache.entries.get(&index)
        && cached == item
        && *sig == signature
    {
        groups.extend(rows.iter().cloned());
        return;
    }
    let start = groups.len();
    item_groups(item, index, exploration, ctx, groups);
    if let Some(cache) = cache {
        if settled(item) {
            let rows = groups[start..].to_vec();
            cache.entries.insert(index, (item.clone(), signature, rows));
        } else {
            cache.entries.remove(&index);
        }
    }
}

/// The steps of the open turn and the cache reads its predecessor ended with
/// (`routes/session/rows.ts:351-372`).
#[derive(Default)]
struct TurnUsage {
    steps: Vec<Assistant>,
    previous: Option<(u64, String)>,
}

impl TurnUsage {
    /// Adds an item; returns the turn's steps with tokens once its last step finished.
    fn add(&mut self, item: &Item) -> Option<Vec<Assistant>> {
        let Item::Assistant(a) = item else {
            return None;
        };
        self.steps.push(a.clone());
        if !a.terminal || a.completed.is_none() {
            return None;
        }
        let steps: Vec<Assistant> = std::mem::take(&mut self.steps)
            .into_iter()
            .filter(|s| s.step.as_ref().is_some_and(|u| token_total(&u.tokens) > 0))
            .collect();
        if steps.is_empty() {
            return None;
        }
        Some(steps)
    }

    fn close(&mut self, steps: &[Assistant]) {
        if let Some(usage) = steps.last().and_then(|a| a.step.as_ref()) {
            self.previous = Some((usage.tokens.cache.read, usage.model.clone()));
        }
    }
}

fn token_total(t: &anysola_core::session::message::Tokens) -> u64 {
    t.input + t.output + t.reasoning + t.cache.read + t.cache.write
}

/// Fewer cached tokens than the step before on the same model (`rows.ts:421-433`).
fn cache_drop(previous: Option<&(u64, String)>, read: u64, model: &str) -> Option<u64> {
    let (before, previous_model) = previous?;
    if previous_model != model {
        return None;
    }
    before.checked_sub(read).filter(|drop| *drop > 0)
}

/// `1234567` as `1,234,567`.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The `debug.turn_tokens` summary under a turn (`routes/session/index.tsx:1724-1863`); a click
/// opens the per-step table.
fn turn_tokens(steps: &[Assistant], previous: Option<&(u64, String)>, ctx: &Ctx) -> Vec<Row> {
    let id = steps.last().map(|a| a.id.clone()).unwrap_or_default();
    struct Step<'a> {
        finish: String,
        new: u64,
        cached: u64,
        total: u64,
        drop: Option<u64>,
        tools: Vec<&'a Tool>,
    }
    let verbose = ctx.turn_tokens == crate::state::TurnTokens::Verbose;
    let mut before = previous.cloned();
    let steps: Vec<Step> = steps
        .iter()
        .map(|a| {
            let usage = a.step.as_deref().cloned().unwrap_or_default();
            let t = usage.tokens;
            let total = token_total(&t);
            let drop = cache_drop(before.as_ref(), t.cache.read, &usage.model);
            before = Some((t.cache.read, usage.model.clone()));
            Step {
                finish: match usage.finish.as_str() {
                    "tool-calls" => "tool-call".into(),
                    f => f.to_string(),
                },
                new: total - t.cache.read,
                cached: t.cache.read,
                total,
                drop,
                tools: if verbose {
                    a.parts
                        .iter()
                        .filter_map(|p| match p {
                            Part::Tool(t) => Some(t),
                            _ => None,
                        })
                        .collect()
                } else {
                    Vec::new()
                },
            }
        })
        .collect();
    let hit = format!("tokens:{id}");
    let muted = ctx.style("text.muted");
    let warning = ctx.style("text.feedback.warning.base");
    let latest = steps.last();
    let drops = steps.iter().filter(|s| s.drop.is_some()).count();
    let count = steps.len();
    let mut summary = vec![
        Span::styled(if expanded(&hit, ctx) { "- " } else { "+ " }, muted),
        Span::styled("Tokens", muted.add_modifier(Modifier::BOLD)),
        Span::styled(
            format!(
                ": {count} {} · latest: {} new · {} cached · {} total",
                if count == 1 { "step" } else { "steps" },
                grouped(latest.map_or(0, |s| s.new)),
                grouped(latest.map_or(0, |s| s.cached)),
                grouped(latest.map_or(0, |s| s.total)),
            ),
            muted,
        ),
    ];
    if drops > 0 {
        summary.push(Span::styled(
            format!(
                " · ! {drops} likely cache {}",
                if drops == 1 { "bust" } else { "busts" }
            ),
            warning,
        ));
    }
    let mut rows = vec![Row::text(3, summary)];
    if expanded(&hit, ctx) {
        let w = |f: &dyn Fn(&Step) -> usize, title: &str| {
            steps.iter().map(f).max().unwrap_or(0).max(title.len())
        };
        let step_w = w(&|s| s.finish.len(), "Step");
        let new_w = w(&|s| grouped(s.new).len(), "New");
        let cached_w = w(&|s| grouped(s.cached).len(), "Cached");
        let total_w = w(&|s| grouped(s.total).len(), "Total");
        rows.push(Row::text(
            5,
            vec![Span::styled(
                format!(
                    "{:<sw$}{:>new_w$}  {:>cached_w$}  {:>total_w$}",
                    "Step",
                    "New",
                    "Cached",
                    "Total",
                    sw = step_w + 2
                ),
                muted.add_modifier(Modifier::ITALIC),
            )],
        ));
        for step in &steps {
            let style = if verbose && step.finish == "tool-call" {
                ctx.style("text.base")
            } else {
                muted
            };
            rows.push(Row::text(
                5,
                vec![
                    Span::styled(format!("{:<sw$}", step.finish, sw = step_w + 2), style),
                    Span::styled(
                        format!("{:>new_w$}", grouped(step.new)),
                        style.add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(
                            "  {:>cached_w$}  {:>total_w$}",
                            grouped(step.cached),
                            grouped(step.total)
                        ),
                        style,
                    ),
                ],
            ));
            let name_w = step.tools.iter().map(|t| t.name.len()).max().unwrap_or(0) + 2;
            for tool in &step.tools {
                rows.push(Row::text(
                    7,
                    vec![
                        Span::styled(
                            format!("{:<name_w$}", tool.name),
                            muted.add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(tool_summary(&tool.input), muted.add_modifier(Modifier::DIM)),
                    ],
                ));
            }
            if let Some(drop) = step.drop {
                rows.push(Row::text(
                    5,
                    vec![Span::styled(
                        format!(
                            "! Likely cache bust: {} fewer cached tokens than the previous step",
                            grouped(drop)
                        ),
                        warning,
                    )],
                ));
            }
        }
    }
    for row in &mut rows {
        row.group = Some(hit.clone());
    }
    rows
}

fn expanded(hit: &str, ctx: &Ctx) -> bool {
    ctx.toggled.contains(hit)
}

/// A tool call's input in one line: its main field first, then `key: value` pairs.
fn tool_summary(input: &Value) -> String {
    let Some(fields) = input.as_object() else {
        return input.as_str().unwrap_or_default().to_string();
    };
    let primary_key = [
        "command",
        "id",
        "pattern",
        "url",
        "query",
        "path",
        "description",
        "code",
    ]
    .into_iter()
    .find(|k| fields.contains_key(*k));
    let text = |v: &Value| match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    };
    let mut parts: Vec<String> = primary_key
        .and_then(|k| fields.get(k))
        .and_then(text)
        .into_iter()
        .collect();
    parts.extend(
        fields
            .iter()
            .filter(|(k, _)| Some(k.as_str()) != primary_key)
            .filter_map(|(k, v)| text(v).map(|v| format!("{k}: {v}"))),
    );
    parts.retain(|p| !p.is_empty());
    parts.join("  ")
}

fn item_groups(
    item: &Item,
    index: usize,
    exploration: &Groups,
    ctx: &Ctx,
    groups: &mut Vec<Vec<Row>>,
) {
    match item {
        Item::User { text, agent, .. } => groups.push(user(text, agent, index, ctx)),
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

/// A user message box; a click on any of its rows opens its actions (`user:<item index>`).
fn user(text: &str, agent: &str, index: usize, ctx: &Ctx) -> Vec<Row> {
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
    let hit = format!("user:{index}");
    for row in &mut rows {
        row.group = Some(hit.clone());
    }
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
        let mut row = Row::text(3, vec![Span::styled(label, done)]);
        // A click shows or hides the thinking, like `session.toggle.thinking`.
        row.group = Some("thinking".into());
        rows.push(row);
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
        digits: 0,
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
        let shells = std::collections::HashMap::new();
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
            shells: &shells,
            floor: 0,
            turn_tokens: Default::default(),
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
        // Every row of the user box is a click target for its actions.
        let user_hits: Vec<_> = fresh.iter().filter_map(|r| r.group.as_deref()).collect();
        assert_eq!(user_hits, ["user:0"; 3]);
    }
}
