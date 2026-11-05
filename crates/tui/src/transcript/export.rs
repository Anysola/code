//! The session as a markdown transcript (`/export`, `Copy session transcript`).

use super::model::{Item, Part, Status};

pub struct Header<'a> {
    pub id: &'a str,
    pub title: &'a str,
    /// `toLocaleString()` of the creation and update times.
    pub created: String,
    pub updated: String,
}

pub fn markdown(header: &Header, items: &[Item], thinking: bool, tools: bool) -> String {
    let body: Vec<String> = items
        .iter()
        .filter_map(|item| match item {
            Item::User { text, .. } => Some(format!("## User\n\n{text}")),
            Item::Shell {
                command, output, ..
            } => Some(format!("## Shell\n\n```\n$ {command}\n{output}\n```")),
            Item::Assistant(a) => {
                let content: Vec<String> = a
                    .parts
                    .iter()
                    .filter_map(|part| match part {
                        Part::Text { text, .. } => Some(text.clone()),
                        Part::Reasoning { text, .. } => {
                            thinking.then(|| format!("_Thinking:_\n\n{text}"))
                        }
                        Part::Tool(tool) if tools => {
                            let input = match &tool.input {
                                serde_json::Value::String(s) => s.clone(),
                                other => serde_json::to_string_pretty(other).unwrap_or_default(),
                            };
                            let output = match (&tool.error, tool.status) {
                                (Some(error), _) => error.clone(),
                                (None, Status::Streaming) => String::new(),
                                _ => tool.output.clone(),
                            };
                            Some(format!(
                                "**Tool: {}**\n\n**Input:**\n```json\n{input}\n```\n\n{output}",
                                tool.name
                            ))
                        }
                        Part::Tool(_) => None,
                    })
                    .collect();
                (!content.is_empty()).then(|| format!("## Assistant\n\n{}", content.join("\n\n")))
            }
            _ => None,
        })
        .collect();
    format!(
        "# {}\n\n**Session ID:** {}\n**Created:** {}\n**Updated:** {}\n\n---\n\n{}\n",
        header.title,
        header.id,
        header.created,
        header.updated,
        body.join("\n\n---\n\n")
    )
}
