//! Messages as plain text, for the part of a compaction checkpoint kept verbatim and for sizing
//! what to keep.

use super::message::{Message, Part, ToolState};
use crate::tool::ToolContent;

const TOOL_OUTPUT_MAX_CHARS: usize = 1_250;

pub fn serialize_tool_content(content: &[ToolContent]) -> String {
    content
        .iter()
        .map(|item| match item {
            ToolContent::Text { text } => text.clone(),
            ToolContent::File { mime, name, .. } => match name {
                Some(name) => format!("[Attached {mime}: {name}]"),
                None => format!("[Attached {mime}]"),
            },
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// At most 1250 characters, then `\n[truncated]`.
pub fn truncate_tool_output(value: &str) -> String {
    match value.char_indices().nth(TOOL_OUTPUT_MAX_CHARS) {
        Some((end, _)) => format!("{}\n[truncated]", &value[..end]),
        None => value.to_string(),
    }
}

/// One message as the recent part of a checkpoint shows it; empty for what it leaves out.
pub fn message_to_text(message: &Message) -> String {
    match message {
        Message::User(data) => {
            let mut lines = vec![format!("[User]: {}", data.text)];
            for file in data.files.iter().flatten() {
                let name = file.name.clone().unwrap_or_else(|| {
                    file.source
                        .get("uri")
                        .and_then(|u| u.as_str())
                        .unwrap_or("inline attachment")
                        .to_string()
                });
                lines.push(format!("[Attached {}: {name}]", file.mime));
            }
            lines.join("\n")
        }
        Message::LocationSwitched(data) => format!(
            "[User]: The working directory has been changed to {}.",
            data.location.directory
        ),
        Message::Assistant(data) => data
            .content
            .iter()
            .flat_map(|part| match part {
                Part::Text { text, .. } => vec![format!("[Assistant]: {text}")],
                // Written out, summarized reasoning can read like an attempt to extract the model's thinking.
                Part::Reasoning { .. } => vec![],
                Part::Tool { name, state, .. } => {
                    let input = match state {
                        ToolState::Streaming { input } => input.clone(),
                        ToolState::Running { input, .. }
                        | ToolState::Completed { input, .. }
                        | ToolState::Error { input, .. } => crate::json::compact(input),
                    };
                    let call = format!("[Assistant tool call]: {name}({input})");
                    match state {
                        ToolState::Completed { content, .. } => vec![
                            call,
                            format!(
                                "[Tool result]: {}",
                                truncate_tool_output(&serialize_tool_content(content))
                            ),
                        ],
                        ToolState::Error { error, .. } => {
                            vec![call, format!("[Tool error]: {}", error.message)]
                        }
                        _ => vec![call],
                    }
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Message::Synthetic(data) => format!("[Synthetic context]: {}", data.text),
        Message::Skill(data) => format!("[Skill activated: {}]\n{}", data.name, data.text),
        Message::Shell(data) if !data.background() => format!(
            "[Shell]: {}\n{}",
            data.command,
            truncate_tool_output(data.output.as_ref().map_or("", |o| o.output.as_str()))
        ),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation() {
        assert_eq!(truncate_tool_output("abc"), "abc");
        let long = "é".repeat(1300);
        let cut = truncate_tool_output(&long);
        assert!(cut.ends_with("\n[truncated]"));
        assert_eq!(cut.chars().count(), 1250 + 12);
    }
}
