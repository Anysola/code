//! Compaction: when the conversation nears the model's window (or the provider rejects it as too
//! long, or the user asks), the history is summarized into a checkpoint that later requests start
//! from. A port of 0.5.1's `session/compaction.ts` for summary compaction.

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::Selection;
use super::lower::{Entry, Target, checkpoint, lower};
use super::message::{CompactionData, Message, StoredModel, Tokens};
use super::recent::message_to_text;
use super::request::{Request, build, output_limit};
use super::stream::Assembly;
use crate::api::Shared;
use crate::config::Config;
use crate::gateway::StreamItem;
use crate::gateway::error::{ApiError, Reason, SessionError};
use crate::gateway::retry::Retry;
use crate::model::{Model, ModelRef};
use crate::storage::SessionRow;
use crate::time::now_ms;
use crate::tool::ToolDefinition;

const TEMPLATE: &str = include_str!("../prompt/compaction-template.txt");
const RULES: &str = include_str!("../prompt/compaction-rules.txt");
const NUDGE: &str = "The previous response did not fill in the required summary template. Do not call tools. Return the summary as text using the exact section headings from the template.";
/// Summaries written with the previous template carry this catch-all heading.
const LEGACY_HEADING: &str = "## Additional Context";
/// After each "too long" rejection, the next attempt aims at this share of the first rejected size.
const SHRINK_STEPS: [f64; 4] = [0.85, 0.7, 0.5, 0.35];
const RESERVE_MIN: u64 = 16_000;
const RESERVE_SHARE: f64 = 0.15;
/// Assumed for the compaction request when the model's window is unknown.
const UNKNOWN_WINDOW: u64 = 200_000;
const SUMMARY_OUTPUT_FLOOR: u64 = 32_000;
const OUTPUTS_CLIPPED: usize = 1;
const OUTPUTS_REMOVED: usize = 2;
const USER_ONLY: usize = 3;
const OMITTED: usize = 4;
const LEVELS: usize = 5;
const TRANSCRIPT_CLIP: usize = 2_000;
const IMAGE_TOKENS: u64 = 1_500;
const PDF_TOKENS: u64 = 2_000;

/// About four characters per token (UTF-16 units, as 0.5.1 counted).
pub fn tokens(text: &str) -> u64 {
    (text.encode_utf16().count() as f64 / 4.0).round() as u64
}

/// A prompt's size: what the provider reported for the latest response plus an estimate of what
/// came after it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Estimate {
    pub measured: u64,
    pub estimated: u64,
}

impl Estimate {
    pub fn total(&self) -> u64 {
        self.measured + self.estimated
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub auto: bool,
    /// Tokens of recent conversation kept verbatim beside the summary.
    pub keep: u64,
    /// Room kept free below the window: `"N%"` or tokens.
    pub buffer: Option<Value>,
}

impl Settings {
    pub fn from_config(config: &Config) -> Settings {
        let compaction = config.get("compaction");
        let field = |key: &str| compaction.and_then(|c| c.get(key));
        Settings {
            auto: field("auto").and_then(Value::as_bool).unwrap_or(true),
            keep: field("keep")
                .and_then(|k| k.get("tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(15_000),
            buffer: field("buffer").cloned(),
        }
    }
}

/// The largest request the model takes while leaving room for its reply; `None` when the window
/// is unknown.
pub fn ceiling(window: Option<u64>, buffer: Option<&Value>) -> Option<u64> {
    let window = window.filter(|w| *w > 0)?;
    let tokens = match buffer {
        Some(Value::String(share)) => share
            .trim_end_matches('%')
            .trim()
            .parse::<f64>()
            .ok()
            .map(|p| (window as f64 * p / 100.0).round() as u64),
        Some(Value::Number(n)) => n.as_u64(),
        _ => None,
    };
    if let Some(tokens) = tokens.filter(|t| *t < window) {
        return Some(window - tokens);
    }
    let floor = if window >= 2 * RESERVE_MIN {
        RESERVE_MIN
    } else {
        0
    };
    Some(window - ((window as f64 * RESERVE_SHARE).floor() as u64).max(floor))
}

fn summary_room(window: Option<u64>, output: u64) -> u64 {
    window.unwrap_or(0).saturating_sub(RESERVE_MIN.min(output))
}

/// The history a request is built from: the latest completed compaction and everything after
/// it, with that compaction's kept text rebuilt from the messages it references.
pub struct History {
    pub ids: Vec<String>,
    pub messages: Vec<Message>,
}

pub(crate) fn history(shared: &Shared, session_id: &str) -> History {
    let rows = shared.db.messages(session_id).unwrap_or_default();
    let decoded: Vec<Message> = rows
        .iter()
        .map(|r| Message::decode(&r.kind, &r.data))
        .collect();
    let start = decoded
        .iter()
        .rposition(|m| matches!(m, Message::Compaction(c) if c.completed()))
        .unwrap_or(0);
    let mut messages = decoded[start..].to_vec();
    if let Some(Message::Compaction(compaction)) = messages.first_mut()
        && let Some(keep) = compaction.keep_id()
        && let Some(from) = rows.iter().position(|r| r.id == keep)
    {
        let recent = decoded[from..start]
            .iter()
            .map(message_to_text)
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        let metadata = compaction.metadata.get_or_insert_with(|| json!({}));
        metadata["recent"] = Value::String(recent);
    }
    History {
        ids: rows[start..].iter().map(|r| r.id.clone()).collect(),
        messages,
    }
}

fn block_tokens(block: &Value) -> u64 {
    let text = |key: &str| tokens(block[key].as_str().unwrap_or_default());
    match block["type"].as_str().unwrap_or_default() {
        "text" => text("text"),
        "thinking" => text("thinking"),
        "image" => IMAGE_TOKENS,
        "document" => PDF_TOKENS,
        "tool_use" => tokens(&format!(
            "{}{}",
            block["name"].as_str().unwrap_or_default(),
            crate::json::compact(&block["input"])
        )),
        "tool_result" => match &block["content"] {
            Value::String(s) => tokens(s),
            Value::Array(items) => items.iter().map(block_tokens).sum(),
            _ => 0,
        },
        _ => 0,
    }
}

pub fn entry_tokens(entry: &Entry) -> u64 {
    match entry {
        Entry::Turn(message) => message["content"]
            .as_array()
            .map_or(0, |blocks| blocks.iter().map(block_tokens).sum()),
        Entry::Update { text, .. } => tokens(text),
        Entry::Effort { .. } => 0,
    }
}

fn request_tokens(system: &[String], tools: &[ToolDefinition], entries: &[Entry]) -> u64 {
    system.iter().map(|s| tokens(s)).sum::<u64>()
        + tools
            .iter()
            .map(|t| {
                tokens(&format!(
                    "{}{}{}",
                    t.name,
                    t.description,
                    crate::json::compact(&t.input_schema)
                ))
            })
            .sum::<u64>()
        + entries.iter().map(entry_tokens).sum::<u64>()
}

/// The latest response whose token count describes this provider's prompt.
fn anchor(messages: &[Message], provider: &str) -> Option<usize> {
    messages.iter().rposition(|m| match m {
        Message::Assistant(a) => {
            a.model.provider_id == provider
                && a.error.is_none()
                && a.tokens
                    .is_some_and(|t| t.input + t.cache.read + t.cache.write > 0)
        }
        _ => false,
    })
}

/// The size of the prompt `messages` make: the anchor's reported usage plus an estimate of what
/// came after it (its own tool results included, which the provider has not seen yet).
pub fn estimate_prompt(
    messages: &[Message],
    target: &Target,
    system: &[String],
    tools: &[ToolDefinition],
) -> Estimate {
    let Some(index) = anchor(messages, target.provider) else {
        return Estimate {
            measured: 0,
            estimated: request_tokens(system, tools, &lower(messages, target)),
        };
    };
    let Message::Assistant(data) = &messages[index] else {
        unreachable!("the anchor is an assistant message");
    };
    let tokens = data.tokens.unwrap_or_default();
    let mut lowered = lower(&messages[index..], target);
    if let Some(Entry::Turn(first)) = lowered.first()
        && first["role"] == "assistant"
    {
        lowered.remove(0);
    }
    Estimate {
        measured: tokens.input
            + tokens.cache.read
            + tokens.cache.write
            + tokens.output
            + tokens.reasoning,
        estimated: lowered.iter().map(entry_tokens).sum(),
    }
}

pub fn build_prompt(update: bool, legacy: bool) -> String {
    let shared = [
        "Summarize only what the user and the assistant said and did. Leave out instructions and setup the assistant was given rather than told by the user: repository conventions, instruction files such as AGENTS.md, and environment details like the session ID. The next agent receives current versions of all of these separately.",
        TEMPLATE,
        RULES,
        "Do not continue the task or call tools.",
        "Return only the structured summary in the requested format. Do not include a preamble, explanation, or other commentary.",
    ];
    let mut parts: Vec<&str> = Vec::new();
    if update {
        parts.push("Update the existing checkpoint in the conversation above into one consolidated summary.");
        if legacy {
            parts.push("The existing checkpoint was written with an earlier format that recorded far more detail than this one asks for. Rewrite it at the level of detail described below rather than carrying its detail forward. Keep its requirements, decisions, and open questions; they came from earlier conversation with the user.");
        }
        parts.push("Newer history always takes precedence over the existing checkpoint. Preserve previous information unless newer history clearly contradicts, supersedes, resolves, or makes it stale. If something is no longer relevant to continuing the work, you may remove it.");
        parts.push("Incorporate newer requirements, decisions, progress, and context. Reconcile Work State and Next Move: move completed work out of Active, remove resolved blockers and answered questions, and preserve unresolved or pending work.");
        parts.push("Return only the updated Markdown sections. Do not reproduce the `<conversation-checkpoint>`, `<summary>`, or `<recent-context>` wrapper tags from the previous checkpoint.");
    } else {
        parts.push("You MUST summarize the conversation above into a structured summary that will be given to another agent to resume the work.");
    }
    parts.extend(shared);
    parts.join("\n\n")
}

/// Whether the reply uses at least one of the template's `##` headings.
fn filled(text: &str) -> bool {
    let headings: Vec<&str> = TEMPLATE.lines().filter(|l| l.starts_with("##")).collect();
    text.lines().any(|l| headings.contains(&l.trim()))
}

fn previous(messages: &[Message]) -> Option<&CompactionData> {
    match messages.first() {
        Some(Message::Compaction(c)) if c.completed() => Some(c),
        _ => None,
    }
}

fn kept_text(compaction: &CompactionData) -> &str {
    compaction
        .metadata
        .as_ref()
        .and_then(|m| m.get("recent"))
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// The first message kept verbatim beside the summary: the newest messages within `keep` tokens,
/// starting at a user message. `None` when there is nothing to compact; `Some(None)` keeps nothing.
fn split(messages: &[Message], keep: u64) -> Option<Option<usize>> {
    let entries: Vec<(usize, String)> = messages
        .iter()
        .enumerate()
        .map(|(i, m)| (i, message_to_text(m)))
        .filter(|(_, t)| !t.is_empty())
        .collect();
    if entries.is_empty() {
        return None;
    }
    let mut total = 0;
    let mut start = entries.len();
    while start > 0 {
        let next = total + tokens(&entries[start - 1].1);
        if next > keep {
            break;
        }
        total = next;
        start -= 1;
    }
    let dropped = start.min(entries.len() - 1);
    let is_user = |i: usize| matches!(messages[entries[i].0], Message::User(_));
    let boundary = (0..=dropped).rev().find(|&i| is_user(i));
    let first = match boundary {
        Some(b) if b > 0 => Some(b),
        _ => match (0..entries.len()).rev().find(|&i| is_user(i)) {
            Some(latest) if latest > 0 => Some(latest),
            _ if previous(messages).is_some_and(|p| !kept_text(p).is_empty()) => Some(0),
            _ => None,
        },
    };
    Some(first.map(|i| entries[i].0))
}

fn clip(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= TRANSCRIPT_CLIP {
        return value.to_string();
    }
    let half = TRANSCRIPT_CLIP / 2;
    let head: String = chars[..half].iter().collect();
    let tail: String = chars[chars.len() - half..].iter().collect();
    format!(
        "{head}\n[… {} characters omitted …]\n{tail}",
        chars.len() - TRANSCRIPT_CLIP
    )
}

fn tool_output(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(|item| match item["type"].as_str() {
                Some("text") => item["text"].as_str().unwrap_or_default().to_string(),
                _ => format!(
                    "[Attached {}]",
                    item["source"]["media_type"].as_str().unwrap_or_default()
                ),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn is_result_turn(message: &Value) -> bool {
    message["content"]
        .as_array()
        .is_some_and(|c| !c.is_empty() && c.iter().all(|b| b["type"] == "tool_result"))
}

/// One message as transcript lines at a shortening level; empty for what that level leaves out.
fn flatten(entry: &Entry, level: usize) -> String {
    let Entry::Turn(message) = entry else {
        return String::new();
    };
    let user = message["role"] == "user" && !is_result_turn(message);
    if level == OMITTED || (level >= USER_ONLY && !user) {
        return String::new();
    }
    let label = if user { "User" } else { "Assistant" };
    let blocks = message["content"].as_array().cloned().unwrap_or_default();
    blocks
        .iter()
        .filter_map(|block| {
            let kind = block["type"].as_str().unwrap_or_default();
            if kind == "text" {
                let text = block["text"].as_str().unwrap_or_default();
                if text.is_empty() {
                    return None;
                }
                let text = if level >= USER_ONLY {
                    clip(text)
                } else {
                    text.to_string()
                };
                return Some(format!("[{label}]: {text}"));
            }
            if level >= USER_ONLY {
                return None;
            }
            match kind {
                "image" | "document" => Some(format!(
                    "[{} omitted]",
                    block["source"]["media_type"].as_str().unwrap_or_default()
                )),
                "tool_use" => {
                    let input = crate::json::compact(&block["input"]);
                    let input = if level >= OUTPUTS_REMOVED {
                        clip(&input)
                    } else {
                        input
                    };
                    Some(format!(
                        "[Assistant tool call]: {}({input})",
                        block["name"].as_str().unwrap_or_default()
                    ))
                }
                "tool_result" => {
                    let name = if block["is_error"] == true {
                        "Tool error"
                    } else {
                        "Tool result"
                    };
                    if level >= OUTPUTS_REMOVED {
                        return Some(format!("[{name} omitted]"));
                    }
                    let output = tool_output(&block["content"]);
                    let output = if level >= OUTPUTS_CLIPPED {
                        clip(&output)
                    } else {
                        output
                    };
                    Some(format!("[{name}]: {output}"))
                }
                _ => None,
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn user(text: String) -> Entry {
    Entry::Turn(json!({"role": "user", "content": [{"type": "text", "text": text}]}))
}

/// What a summary request carries.
#[derive(Clone)]
struct Draft {
    system: Vec<String>,
    tools: Vec<ToolDefinition>,
    entries: Vec<Entry>,
    max_tokens: u64,
}

impl Draft {
    fn tokens(&self) -> u64 {
        request_tokens(&self.system, &self.tools, &self.entries)
    }
}

/// The history sent as one text transcript (after any previous summary), exchanges shortened
/// oldest first one level at a time until it fits `target`. The newest exchange is never left
/// out; `None` when it cannot fit even so.
fn flatten_and_drop_oldest(
    history: &Draft,
    previous: Option<&CompactionData>,
    target: f64,
) -> Option<Draft> {
    let (lead, recent, rest) = match previous {
        Some(summary) => (
            vec![user(checkpoint(
                summary.summary.as_deref().unwrap_or_default(),
                "",
            ))],
            kept_text(summary),
            history.entries.get(1..).unwrap_or_default(),
        ),
        None => (Vec::new(), "", &history.entries[..]),
    };
    let mut groups: Vec<Vec<&Entry>> = Vec::new();
    for entry in rest {
        let starts = matches!(entry, Entry::Turn(m) if m["role"] == "user" && !is_result_turn(m));
        match groups.last_mut() {
            Some(group) if !starts => group.push(entry),
            _ => groups.push(vec![entry]),
        }
    }
    let mut exchanges: Vec<Vec<String>> = Vec::new();
    if !recent.is_empty() {
        exchanges.push(
            (0..LEVELS)
                .map(|l| {
                    if l == OMITTED {
                        String::new()
                    } else {
                        recent.to_string()
                    }
                })
                .collect(),
        );
    }
    for group in groups {
        let renders: Vec<String> = (0..LEVELS)
            .map(|level| {
                group
                    .iter()
                    .map(|e| flatten(e, level))
                    .filter(|t| !t.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect();
        if !renders[0].is_empty() {
            exchanges.push(renders);
        }
    }
    if exchanges.is_empty() {
        return None;
    }
    let note = |omitted: usize| match omitted {
        0 => String::new(),
        1 => "[1 older exchange omitted]\n\n".to_string(),
        n => format!("[{n} older exchanges omitted]\n\n"),
    };
    let size = |text: &str| if text.is_empty() { 0 } else { tokens(text) + 1 };
    let room = target
        - request_tokens(&history.system, &[], &lead) as f64
        - tokens(&note(exchanges.len())) as f64;
    let mut levels = vec![0; exchanges.len()];
    let mut total: f64 = exchanges.iter().map(|r| size(&r[0]) as f64).sum();
    let newest = exchanges.len() - 1;
    for level in 1..LEVELS {
        for index in 0..exchanges.len() {
            if total <= room {
                break;
            }
            if index == newest && exchanges[index][level].is_empty() {
                continue;
            }
            total += size(&exchanges[index][level]) as f64
                - size(&exchanges[index][levels[index]]) as f64;
            levels[index] = level;
        }
    }
    if total > room {
        return None;
    }
    let omitted = levels
        .iter()
        .enumerate()
        .filter(|(i, l)| exchanges[*i][**l].is_empty())
        .count();
    let text = exchanges
        .iter()
        .zip(&levels)
        .map(|(renders, level)| renders[*level].as_str())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut entries = lead;
    entries.push(user(format!("{}{text}", note(omitted))));
    Some(Draft {
        tools: Vec::new(),
        entries,
        ..history.clone()
    })
}

/// Lowers the output limit to the room the request being sent leaves; a summary near its target
/// still gets up to 32000 tokens.
fn fit_output(draft: &mut Draft, model: &Model, size: Option<Estimate>) {
    let input = size.unwrap_or(Estimate {
        measured: 0,
        estimated: draft.tokens(),
    });
    let room = output_limit(model, input.measured, input.estimated, 0.0);
    let floor = SUMMARY_OUTPUT_FLOOR.min(output_limit(model, input.total(), 0, 0.0));
    draft.max_tokens = draft.max_tokens.min(room.max(floor));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// Before a step, when the prompt passed the ceiling.
    Auto(Estimate),
    /// The provider just rejected the prompt as too long.
    Overflow(Estimate),
    /// `/compact`.
    Manual,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Automatic compaction is off or not due.
    Skipped,
    Completed,
    Failed(SessionError),
}

/// A reply that cannot be used (never retried) or a provider failure.
enum Failure {
    Unusable(SessionError),
    Api(ApiError),
}

struct Call<'a> {
    shared: &'a Shared,
    session: &'a SessionRow,
    model: &'a Model,
    variant: Option<&'a str>,
    cancel: &'a CancellationToken,
    tokens: Tokens,
}

impl Call<'_> {
    async fn stream(&mut self, draft: &Draft) -> Result<String, Failure> {
        let (body, mut headers) = build(Request {
            model: self.model,
            variant: self.variant.and_then(|v| self.model.variant(v)),
            system: draft.system.clone(),
            tools: draft.tools.clone(),
            messages: draft.entries.clone(),
            max_tokens: draft.max_tokens,
            tools_disabled: false,
        });
        headers.extend(super::step_headers(self.shared, self.session));
        let mut stream = self
            .shared
            .gateway
            .messages(body, &headers)
            .await
            .map_err(Failure::Api)?;
        let mut assembly = Assembly::default();
        while let Some(item) = stream.next().await {
            match item.map_err(Failure::Api)? {
                StreamItem::Credits { charged, remaining } => {
                    self.shared
                        .credits
                        .record(&self.session.id, charged, remaining)
                }
                StreamItem::Event(event) => {
                    assembly.on_event(&event).map_err(Failure::Api)?;
                }
            }
        }
        let t = assembly.tokens;
        let _ = self.shared.db.add_tokens(
            &self.session.id,
            crate::storage::TokenTotals {
                input: t.input,
                output: t.output,
                reasoning: t.reasoning,
                cache_read: t.cache.read,
                cache_write: t.cache.write,
            },
        );
        self.tokens.input += t.input;
        self.tokens.output += t.output;
        self.tokens.reasoning += t.reasoning;
        self.tokens.cache.read += t.cache.read;
        self.tokens.cache.write += t.cache.write;
        if !assembly.finished {
            return Err(Failure::Api(ApiError::incomplete(
                "The provider response ended with an unknown finish reason.",
            )));
        }
        if assembly.finish() == "length" {
            return Err(Failure::Unusable(SessionError::new(
                "compaction.failed",
                "Compaction summary reached the output token limit",
            )));
        }
        Ok(assembly
            .parts
            .iter()
            .filter_map(|p| match p {
                super::message::Part::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect())
    }

    /// Sends the request with the summary prompt; a reply that skips the template gets one nudge.
    async fn send(
        &mut self,
        draft: &Draft,
        prompt: &str,
        size: Option<Estimate>,
    ) -> Result<String, Failure> {
        let grow = |extra: u64| {
            size.map(|s| Estimate {
                estimated: s.estimated + extra,
                ..s
            })
        };
        let mut prompted = draft.clone();
        prompted.entries.push(user(prompt.to_string()));
        fit_output(&mut prompted, self.model, grow(tokens(prompt)));
        let reply = self.stream(&prompted).await?;
        if filled(&reply) {
            return Ok(reply);
        }
        let mut nudged = prompted;
        nudged.entries.push(user(NUDGE.to_string()));
        fit_output(
            &mut nudged,
            self.model,
            grow(tokens(prompt) + tokens(NUDGE)),
        );
        let retry = self.stream(&nudged).await?;
        if filled(&retry) {
            return Ok(retry);
        }
        Err(Failure::Unusable(SessionError::new(
            "compaction.failed",
            if retry.trim().is_empty() {
                "Compaction produced no summary"
            } else {
                "Compaction summary did not match the required template"
            },
        )))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sent {
    Prepared,
    History,
    Text,
}

/// The system blocks and tools the session's next request would carry.
pub(crate) struct Prefix<'a> {
    pub system: &'a [String],
    pub tools: &'a [ToolDefinition],
}

/// Compacts the session if the trigger calls for it. The summary request is the normal request
/// plus the summary prompt while that fits; otherwise the history goes as a shortened text
/// transcript.
pub(crate) async fn compact(
    shared: &Shared,
    session: &SessionRow,
    selection: &Selection,
    prefix: Prefix<'_>,
    trigger: Trigger,
    cancel: &CancellationToken,
) -> Outcome {
    let settings = Settings::from_config(&shared.config);
    if trigger != Trigger::Manual && !settings.auto {
        return Outcome::Skipped;
    }
    let loaded = history(shared, &session.id);
    if let Trigger::Auto(estimate) = trigger {
        let ceiling = ceiling(selection.model.context, settings.buffer.as_ref());
        let just_compacted =
            matches!(loaded.messages.last(), Some(Message::Compaction(c)) if c.completed());
        if just_compacted || ceiling.is_none_or(|c| estimate.total() < c) {
            return Outcome::Skipped;
        }
    }
    let configured = shared
        .agents
        .get("compaction")
        .and_then(|a| a.model.as_deref())
        .and_then(ModelRef::parse)
        .and_then(|r| crate::model::resolve(&shared.models, &r).ok());
    let model = configured.unwrap_or_else(|| selection.model.clone());
    let same_model = model.id == selection.model.id;
    let cap = ceiling(model.context, settings.buffer.as_ref())
        .or_else(|| ceiling(Some(UNKNOWN_WINDOW), settings.buffer.as_ref()))
        .unwrap_or(UNKNOWN_WINDOW);
    let budget = match trigger {
        Trigger::Overflow(estimate) => {
            cap.min((estimate.total() as f64 * SHRINK_STEPS[0]).floor() as u64)
        }
        _ => cap,
    };
    let reason = if trigger == Trigger::Manual {
        "manual"
    } else {
        "auto"
    };
    let row = crate::ids::message();
    let started = json!({"time": {"created": now_ms()}, "status": "running", "reason": reason, "summary": ""});
    let seq = match shared
        .db
        .append_message(&session.id, &row, "compaction", &started)
    {
        Ok(seq) => seq,
        Err(error) => return Outcome::Failed(SessionError::new("unknown", error.to_string())),
    };
    let reference = ModelRef {
        provider: selection.reference.provider.clone(),
        id: model.id.clone(),
        variant: same_model
            .then(|| selection.reference.variant.clone())
            .flatten(),
    };
    let mut call = Call {
        shared,
        session,
        model: &model,
        variant: reference.variant.as_deref(),
        cancel,
        tokens: Tokens::default(),
    };
    let result = tokio::select! {
        result = summarize(&mut call, &loaded, selection, &prefix, trigger, budget, settings.keep) => result,
        _ = cancel.cancelled() => Err(SessionError::new(
            if trigger == Trigger::Manual { "aborted" } else { "compaction.interrupted" },
            if trigger == Trigger::Manual { "Compaction cancelled" } else { "Compaction was interrupted" },
        )),
    };
    let tokens = call.tokens;
    let mut data = CompactionData {
        time: super::message::Created { created: now_ms() },
        status: "completed".into(),
        reason: reason.into(),
        model: None,
        summary: None,
        native: None,
        error: None,
        cost: Some(0.0),
        tokens: Some(tokens),
        metadata: None,
    };
    let outcome = match result {
        Ok((summary, keep)) => {
            data.model = Some(StoredModel {
                id: reference.id.clone(),
                provider_id: reference.provider.clone(),
                variant: reference.variant.clone(),
            });
            data.summary = Some(summary);
            data.metadata = keep.map(|i| json!({"keepID": loaded.ids[i]}));
            Outcome::Completed
        }
        Err(error) => {
            data.status = "failed".into();
            data.error = Some(error.clone());
            Outcome::Failed(error)
        }
    };
    let _ = shared
        .db
        .update_message(&row, &super::message::encode(&data));
    if outcome == Outcome::Completed {
        // The values the model has seen become the baseline of the history the checkpoint starts.
        let _ = shared.db.instruction_new_epoch(&session.id, seq);
    }
    crate::log::info(
        "compaction",
        &[
            ("session", &session.id),
            ("reason", reason),
            ("status", &data.status),
        ],
    );
    outcome
}

async fn summarize(
    call: &mut Call<'_>,
    loaded: &History,
    selection: &Selection,
    prefix: &Prefix<'_>,
    trigger: Trigger,
    budget: u64,
    keep: u64,
) -> Result<(String, Option<usize>), SessionError> {
    let messages = &loaded.messages;
    let nothing = || SessionError::new("compaction.unavailable", "Nothing to compact yet");
    let keep_at = split(messages, keep).ok_or_else(nothing)?;
    let previous = previous(messages);
    let prompt = build_prompt(
        previous.is_some(),
        previous.is_some_and(|p| {
            p.summary
                .as_deref()
                .is_some_and(|s| s.contains(LEGACY_HEADING))
        }),
    );
    let target = Target {
        provider: &selection.reference.provider,
        id: &call.model.id,
    };
    let prepared = Draft {
        system: prefix.system.to_vec(),
        tools: prefix.tools.to_vec(),
        entries: lower(messages, &target),
        max_tokens: output_limit(call.model, budget, 0, 0.0),
    };
    let history = Draft {
        entries: match keep_at {
            Some(index) => lower(&messages[..index], &target),
            None => prepared.entries.clone(),
        },
        ..prepared.clone()
    };
    // Sized with the count that decided to compact, while the measured request is unchanged. An
    // overflow already proved the count low.
    let measured = match trigger {
        Trigger::Overflow(_) => None,
        _ => {
            let anchored =
                anchor(messages, &selection.reference.provider).and_then(|i| match &messages[i] {
                    Message::Assistant(a) => Some(a),
                    _ => None,
                });
            anchored
                .filter(|a| {
                    a.model.id == call.model.id
                        && a.model.variant == selection.reference.variant
                        && a.agent == selection.agent.id
                })
                .map(|_| estimate_prompt(messages, &target, prefix.system, prefix.tools))
        }
    };
    let overhead = (tokens(&prompt) + tokens(NUDGE)) as f64;
    let fit = match trigger {
        Trigger::Overflow(_) => budget,
        _ => budget.max(summary_room(call.model.context, prepared.max_tokens)),
    };
    let mut target_size = budget as f64 - overhead;
    let mut fit = fit as f64 - overhead;
    let mut rejections = 0;
    let mut rejected: Option<u64> = None;
    let mut as_text = false;
    let mut retry = Retry::default();
    loop {
        let size_measured = if rejections == 0 { measured } else { None };
        let unchanged = if as_text {
            None
        } else if size_measured.map_or_else(|| prepared.tokens(), |m| m.total()) as f64 <= fit {
            Some((prepared.clone(), Sent::Prepared))
        } else if history.tokens() as f64 <= target_size {
            Some((history.clone(), Sent::History))
        } else {
            None
        };
        let request = unchanged
            .or_else(|| {
                flatten_and_drop_oldest(&history, previous, target_size).map(|d| (d, Sent::Text))
            })
            .or_else(|| match (rejections, as_text) {
                (0, true) => flatten_and_drop_oldest(&history, previous, f64::INFINITY)
                    .map(|d| (d, Sent::Text)),
                (0, false) => Some((history.clone(), Sent::History)),
                _ => None,
            });
        let Some((draft, sent)) = request else {
            return Err(SessionError::new(
                "compaction.failed",
                "The compaction request cannot be reduced further without losing the latest exchange or checkpoint",
            ));
        };
        let size = if sent == Sent::Prepared {
            size_measured
        } else {
            None
        };
        let error = match call.send(&draft, &prompt, size).await {
            Ok(summary) => return Ok((summary, keep_at)),
            Err(Failure::Unusable(error)) => return Err(error),
            Err(Failure::Api(error)) => error,
        };
        let too_large = error.reason == Reason::PayloadTooLarge;
        if too_large && sent != Sent::Text {
            as_text = true;
            continue;
        }
        if too_large || error.reason == Reason::ContextOverflow {
            let Some(step) = SHRINK_STEPS.get(rejections) else {
                return Err(error.to_session_error());
            };
            let first =
                *rejected.get_or_insert_with(|| size.map_or_else(|| draft.tokens(), |s| s.total()));
            if size.is_some() {
                as_text = true;
            }
            target_size = (first as f64 * step).floor();
            fit = target_size;
            rejections += 1;
            continue;
        }
        match retry.next(&error) {
            Some(delay) => tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = call.cancel.cancelled() => return Err(error.to_session_error()),
            },
            None => return Err(error.to_session_error()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceilings() {
        assert_eq!(ceiling(Some(1_000_000), None), Some(850_000));
        assert_eq!(ceiling(Some(200_000), None), Some(170_000));
        assert_eq!(ceiling(Some(50_000), None), Some(34_000));
        assert_eq!(ceiling(Some(20_000), None), Some(17_000));
        assert_eq!(ceiling(Some(200_000), Some(&json!("20%"))), Some(160_000));
        assert_eq!(ceiling(Some(200_000), Some(&json!(50_000))), Some(150_000));
        assert_eq!(ceiling(Some(200_000), Some(&json!(300_000))), Some(170_000));
        assert_eq!(ceiling(None, None), None);
    }

    #[test]
    fn token_estimate() {
        assert_eq!(tokens(""), 0);
        assert_eq!(tokens("abcdef"), 2);
        assert_eq!(tokens("abcdefghij"), 3);
    }

    #[test]
    fn prompt_and_template_detection() {
        let first = build_prompt(false, false);
        assert!(first.starts_with("You MUST summarize the conversation above"));
        assert!(first.ends_with("or other commentary."));
        let update = build_prompt(true, true);
        assert!(update.starts_with("Update the existing checkpoint"));
        assert!(update.contains("earlier format"));
        assert!(filled("intro\n## Objective\n- x"));
        assert!(!filled("# Objective"));
    }

    fn turn(role: &str, text: &str) -> Entry {
        Entry::Turn(json!({"role": role, "content": [{"type": "text", "text": text}]}))
    }

    #[test]
    fn transcript_levels() {
        let call = Entry::Turn(json!({"role": "assistant", "content": [
            {"type": "text", "text": "Reading."},
            {"type": "tool_use", "id": "t", "name": "read", "input": {"filePath": "a"}},
        ]}));
        let result = Entry::Turn(json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "t", "content": "x".repeat(2500)},
        ]}));
        assert_eq!(flatten(&turn("user", "hi"), 0), "[User]: hi");
        assert_eq!(
            flatten(&call, 0),
            "[Assistant]: Reading.\n[Assistant tool call]: read({\"filePath\":\"a\"})"
        );
        assert!(flatten(&result, 1).contains("\n[… 500 characters omitted …]\n"));
        assert_eq!(flatten(&result, 2), "[Tool result omitted]");
        assert_eq!(flatten(&call, 3), "");
        assert_eq!(flatten(&turn("user", "hi"), 4), "");

        let draft = Draft {
            system: vec!["s".into()],
            tools: Vec::new(),
            entries: vec![
                turn("user", "old question"),
                call,
                result,
                turn("user", "new"),
            ],
            max_tokens: 1,
        };
        let whole = flatten_and_drop_oldest(&draft, None, 10_000.0).unwrap();
        let Entry::Turn(text) = &whole.entries[0] else {
            panic!()
        };
        assert!(
            text["content"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with("[User]: old question\n[Assistant]: Reading.")
        );
        let tight = flatten_and_drop_oldest(&draft, None, 14.0).unwrap();
        let Entry::Turn(text) = &tight.entries[0] else {
            panic!()
        };
        assert_eq!(
            text["content"][0]["text"],
            "[1 older exchange omitted]\n\n[User]: new"
        );
        assert!(flatten_and_drop_oldest(&draft, None, 0.0).is_none());
    }
}
