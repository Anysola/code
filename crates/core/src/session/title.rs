//! Session titles: generated in the background from the first message, by the title agent's model
//! (the gateway names a small model), falling back once to the session's model.

use std::sync::Arc;

use serde_json::json;

use super::request::{Request, build};
use crate::api::Shared;
use crate::events::Event;
use crate::gateway::StreamItem;
use crate::gateway::retry::Retry;
use crate::model::{Model, ModelRef};
use crate::storage::SessionRow;

const MAX_LENGTH: usize = 100;

fn prompt(text: &str) -> String {
    format!(
        "Here is the first message of the conversation:\n\n<message>\n{text}\n</message>\n\nReply with only the title."
    )
}

/// First non-empty line, at most 100 characters.
fn clean(answer: &str) -> Option<String> {
    let line = answer.lines().map(str::trim).find(|l| !l.is_empty())?;
    let chars: Vec<char> = line.chars().collect();
    Some(if chars.len() <= MAX_LENGTH {
        line.to_string()
    } else {
        format!("{}...", chars[..MAX_LENGTH - 3].iter().collect::<String>())
    })
}

/// The title agent's model with its minimal variant, else the session model.
fn candidates(shared: &Shared, session: &SessionRow) -> Vec<(Model, Option<String>)> {
    let mut out = Vec::new();
    let title_model = shared
        .agents
        .get("title")
        .and_then(|a| a.model.as_deref())
        .and_then(ModelRef::parse);
    if let Some(model) = title_model.and_then(|r| crate::model::resolve(&shared.models, &r).ok()) {
        let variant = ["none", "minimal", "low"]
            .into_iter()
            .find(|v| model.variant(v).is_some())
            .map(str::to_string);
        out.push((model, variant));
    }
    let primary = session
        .model
        .as_ref()
        .and_then(ModelRef::from_json)
        .or_else(|| shared.config.model().and_then(ModelRef::parse))
        .and_then(|r| crate::model::resolve(&shared.models, &r).ok());
    if let Some(primary) = primary
        && !out.iter().any(|(m, _)| m.id == primary.id)
    {
        out.push((primary, None));
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
        messages: vec![
            json!({"role": "user", "content": [{"type": "text", "text": prompt(text)}]}),
        ],
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
    clean(&answer)
}

pub(crate) async fn generate(shared: Arc<Shared>, session: SessionRow, text: String) {
    for (model, variant) in candidates(&shared, &session) {
        if let Some(title) = ask(&shared, &session, &model, variant.as_deref(), &text).await {
            let current = shared.db.session(&session.id).ok().flatten();
            if current.is_some_and(|c| c.title == session.title)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleaning() {
        assert_eq!(
            clean("\n  Fake title  \nmore").as_deref(),
            Some("Fake title")
        );
        assert_eq!(clean(&"x".repeat(120)).unwrap().len(), 100);
        assert!(clean("  \n").is_none());
    }
}
