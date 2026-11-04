//! Session titles: generated in the background from the first message, by the title agent's model
//! (the gateway names a small model), falling back once to the session's model.

use std::sync::Arc;

use serde_json::json;

use super::message::Message;
use super::request::{Request, build};
use crate::api::Shared;
use crate::events::Event;
use crate::gateway::StreamItem;
use crate::gateway::retry::Retry;
use crate::model::{Model, ModelRef};
use crate::storage::SessionRow;

const MAX_LENGTH: usize = 100;
const MAX_CONTEXT_LENGTH: usize = 8_000;
const MAX_FIRST_MESSAGE_LENGTH: usize = 2_000;
/// Variants that minimize reasoning, in preference order.
const MINIMAL_VARIANTS: [&str; 3] = ["none", "minimal", "low"];

fn message_prompt(text: &str) -> String {
    format!(
        "Here is the first message of the conversation:\n\n<message>\n{text}\n</message>\n\nReply with only the title."
    )
}

fn conversation_prompt(text: &str) -> String {
    format!(
        "Here is the conversation so far:\n\n<conversation>\n{text}\n</conversation>\n\nReply with only the title."
    )
}

/// The first `n` UTF-16 units, as JavaScript's `slice` counts.
fn head(text: &str, n: usize) -> String {
    let units: Vec<u16> = text.encode_utf16().take(n).collect();
    String::from_utf16_lossy(&units)
}

/// The last `n` UTF-16 units.
fn tail(text: &str, n: usize) -> String {
    let units: Vec<u16> = text.encode_utf16().collect();
    String::from_utf16_lossy(&units[units.len().saturating_sub(n)..])
}

/// First non-empty line.
fn first_line(answer: &str) -> Option<String> {
    answer
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

/// At most 100 characters, cut ones ending in `...`.
fn truncate(title: &str) -> String {
    if title.encode_utf16().count() <= MAX_LENGTH {
        title.to_string()
    } else {
        format!("{}...", head(title, MAX_LENGTH - 3))
    }
}

/// The prompt for a session: its first message while it is untitled, else the original request
/// and the most recent conversation within 8000 characters.
fn prompt(messages: &[Message], untitled: bool) -> Option<String> {
    let first = messages
        .iter()
        .position(|m| matches!(m, Message::User(_)))?;
    let Message::User(first_user) = &messages[first] else {
        return None;
    };
    if untitled {
        return Some(message_prompt(&first_user.text));
    }
    let original = format!(
        "Original request:\n{}",
        head(&first_user.text, MAX_FIRST_MESSAGE_LENGTH)
    );
    let recent = messages
        .iter()
        .enumerate()
        .filter_map(|(i, m)| match m {
            Message::User(u) if i != first => Some(format!("User: {}", u.text.trim())),
            Message::Assistant(a) => {
                let text = a
                    .content
                    .iter()
                    .filter_map(|p| match p {
                        super::message::Part::Text { text, .. } => Some(text.trim()),
                        _ => None,
                    })
                    .filter(|t| !t.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                (!text.is_empty()).then(|| format!("Assistant: {text}"))
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    if recent.is_empty() {
        return Some(conversation_prompt(&original));
    }
    let prefix = format!("{original}\n\nRecent conversation:\n");
    let budget = MAX_CONTEXT_LENGTH
        .saturating_sub(conversation_prompt("").encode_utf16().count())
        .saturating_sub(prefix.encode_utf16().count());
    Some(conversation_prompt(&format!(
        "{prefix}{}",
        tail(&recent, budget)
    )))
}

fn minimal(model: &Model) -> Option<String> {
    MINIMAL_VARIANTS
        .into_iter()
        .find(|v| model.variant(v).is_some())
        .map(str::to_string)
}

/// A variant below the selected one: `none` or `minimal`, or `low` when it ranks below it.
fn lower_variant(model: &Model, selected: Option<&str>) -> Option<String> {
    let rank = |id: Option<&str>| id.and_then(|id| model.variants.iter().position(|v| v.id == id));
    MINIMAL_VARIANTS
        .into_iter()
        .find(|v| {
            rank(Some(v)).is_some_and(|r| *v != "low" || rank(selected).is_some_and(|s| r < s))
        })
        .map(str::to_string)
}

/// The title agent's model (its variant, else a minimal one); else `small_model`; else the
/// session's model at a lower variant. The session's model is the one fallback.
fn candidates(shared: &Shared, session: &SessionRow) -> Vec<(Model, Option<String>)> {
    let resolve = |r: &ModelRef| {
        crate::model::resolve(
            &shared.models,
            &ModelRef {
                variant: None,
                ..r.clone()
            },
        )
        .ok()
    };
    let primary_ref = session
        .model
        .as_ref()
        .and_then(ModelRef::from_json)
        .or_else(|| shared.config.model().and_then(ModelRef::parse));
    let primary = primary_ref.as_ref().and_then(resolve);
    let preferred = match shared
        .agents
        .get("title")
        .and_then(|a| a.model.as_deref())
        .and_then(ModelRef::parse)
    {
        Some(r) => resolve(&r).map(|m| {
            let variant = r.variant.clone().or_else(|| minimal(&m));
            (m, variant)
        }),
        None => match shared
            .config
            .get("small_model")
            .and_then(|v| v.as_str())
            .and_then(ModelRef::parse)
            .and_then(|r| resolve(&r))
        {
            Some(small) => {
                let variant = minimal(&small);
                Some((small, variant))
            }
            None => primary.as_ref().and_then(|m| {
                lower_variant(m, primary_ref.as_ref().and_then(|r| r.variant.as_deref()))
                    .map(|v| (m.clone(), Some(v)))
            }),
        },
    };
    let fallback = primary.map(|m| (m, primary_ref.and_then(|r| r.variant)));
    let mut out: Vec<(Model, Option<String>)> = preferred.into_iter().collect();
    if let Some((model, variant)) = fallback
        && !out.iter().any(|(m, v)| m.id == model.id && *v == variant)
    {
        out.push((model, variant));
    }
    out
}

async fn ask(
    shared: &Shared,
    session: &SessionRow,
    model: &Model,
    variant: Option<&str>,
    text: &str,
) -> Option<String> {
    let (body, mut headers) = build(Request {
        model,
        variant: variant.and_then(|v| model.variant(v)),
        system: vec![crate::agent::PROMPT_TITLE.to_string()],
        tools: Vec::new(),
        messages: vec![super::lower::Entry::Turn(
            json!({"role": "user", "content": [{"type": "text", "text": text}]}),
        )],
        max_tokens: 32_000,
        tools_disabled: false,
    });
    headers.extend(super::step_headers(shared, session));
    let mut retry = Retry::default();
    let mut stream = loop {
        match shared.gateway.messages(body.clone(), &headers).await {
            Ok(stream) => break stream,
            Err(error) => tokio::time::sleep(retry.next_one_shot(&error)?).await,
        }
    };
    let mut assembly = super::stream::Assembly::default();
    while let Some(item) = stream.next().await {
        match item.ok()? {
            StreamItem::Credits { charged, remaining } => {
                shared.credits.record(&session.id, charged, remaining)
            }
            StreamItem::Event(event) => {
                assembly.on_event(&event).ok()?;
            }
        }
    }
    let answer: String = assembly
        .parts
        .iter()
        .filter_map(|p| match p {
            super::message::Part::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    first_line(&answer)
}

/// Names the session (or renames it, when it already has a title) unless its title changes
/// meanwhile.
pub(crate) async fn generate(shared: Arc<Shared>, session: SessionRow) {
    let messages: Vec<Message> = shared
        .db
        .messages(&session.id)
        .unwrap_or_default()
        .iter()
        .map(|r| Message::decode(&r.kind, &r.data))
        .collect();
    let Some(text) = prompt(&messages, super::is_fallback_title(&session)) else {
        return;
    };
    for (model, variant) in candidates(&shared, &session) {
        let Some(title) = ask(&shared, &session, &model, variant.as_deref(), &text).await else {
            continue;
        };
        let title = truncate(&title);
        let current = shared.db.session(&session.id).ok().flatten();
        if current.is_some_and(|c| c.title == session.title && c.title != title)
            && shared.db.set_title(&session.id, &title).is_ok()
        {
            let _ = shared.events.send(Event::TitleChanged {
                session_id: session.id.clone(),
                title,
            });
        }
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleaning() {
        assert_eq!(
            first_line("\n  Fake title  \nmore").as_deref(),
            Some("Fake title")
        );
        assert_eq!(truncate(&"x".repeat(120)).len(), 100);
        assert!(truncate(&"x".repeat(120)).ends_with("..."));
        assert!(first_line("  \n").is_none());
    }

    #[test]
    fn regeneration_prompt() {
        let user = |t: &str| Message::decode("user", &json!({"time": {"created": 1}, "text": t}));
        let reply = |t: &str| {
            Message::decode(
                "assistant",
                &json!({"time": {"created": 1}, "agent": "build", "model": {"id": "m", "providerID": "anysola"}, "content": [{"type": "text", "text": t}]}),
            )
        };
        let messages = vec![user("fix the bug"), reply(" Done. "), user("now tests")];
        assert_eq!(
            prompt(&messages, true).unwrap(),
            message_prompt("fix the bug")
        );
        assert_eq!(
            prompt(&messages, false).unwrap(),
            "Here is the conversation so far:\n\n<conversation>\nOriginal request:\nfix the bug\n\nRecent conversation:\nAssistant: Done.\n\nUser: now tests\n</conversation>\n\nReply with only the title."
        );
        let long = vec![user("a"), reply(&"x".repeat(9000))];
        let text = prompt(&long, false).unwrap();
        assert!(text.encode_utf16().count() <= MAX_CONTEXT_LENGTH);
        assert!(text.contains("Recent conversation:\nxxx"));
    }
}
