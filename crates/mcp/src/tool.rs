//! An MCP server's tool as a namespace tool, and its `tools/call` result as the model sees it.

use std::sync::{Arc, Weak};

use anysola_core::codemode::{Called, NamespaceTool, direct_name, sanitize};
use anysola_core::permission::PermissionRequest;
use anysola_core::tool::{ToolContent, ToolContext, ToolError};
use base64::Engine;
use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::runtime::Runtime;

pub struct McpTool {
    runtime: Weak<Runtime>,
    server: String,
    name: String,
    description: String,
    input: Value,
    /// `{}` (`Promise<unknown>`) when the server declares no output schema.
    output: Value,
    declares_output: bool,
}

impl McpTool {
    pub fn new(runtime: Weak<Runtime>, server: &str, tool: &Value) -> McpTool {
        McpTool {
            runtime,
            server: server.to_string(),
            name: tool["name"].as_str().unwrap_or_default().to_string(),
            description: tool["description"].as_str().unwrap_or_default().to_string(),
            input: tool
                .get("inputSchema")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
            output: tool
                .get("outputSchema")
                .cloned()
                .unwrap_or_else(|| json!({})),
            declares_output: tool.get("outputSchema").is_some(),
        }
    }
}

impl NamespaceTool for McpTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> &Value {
        &self.input
    }

    fn output_schema(&self) -> Option<&Value> {
        Some(&self.output)
    }

    fn call<'a>(
        &'a self,
        input: Value,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<Called, ToolError>> {
        async move {
            ctx.ask(PermissionRequest {
                action: direct_name(&sanitize(&self.server), &self.name),
                resources: vec!["*".into()],
                save: vec!["*".into()],
                metadata: json!({"input": input}),
            })
            .await?;
            let Some(runtime) = self.runtime.upgrade() else {
                return Err(ToolError::execution(format!(
                    "MCP server \"{}\" is not available",
                    self.server
                )));
            };
            let result = runtime
                .call_tool(&self.server, &self.name, input, &ctx.session_id)
                .await
                .map_err(ToolError::execution)?;
            map_result(&result, self.declares_output, &ctx.paths.tool_output_dir())
                .map_err(ToolError::execution)
        }
        .boxed()
    }
}

fn textual(mime: &str) -> bool {
    mime.is_empty()
        || mime.starts_with("text/")
        || mime == "application/json"
        || mime.ends_with("+json")
        || mime == "application/xml"
        || mime.ends_with("+xml")
        || mime == "application/javascript"
        || mime == "application/x-javascript"
}

/// Media inline when the read tool would accept it, textual resources as text, other binaries
/// saved to the tool-output directory.
fn media(data: &str, mime_type: &str, uri: Option<&str>, saved: &std::path::Path) -> ToolContent {
    let mime = mime_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    if anysola_core::media::READABLE.contains(&mime.as_str()) {
        return ToolContent::File {
            uri: format!("data:{mime_type};base64,{data}"),
            mime: mime_type.to_string(),
            name: None,
        };
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .unwrap_or_default();
    let label = uri.unwrap_or("content");
    if textual(&mime) {
        return ToolContent::text(format!(
            "[Resource {label} ({mime_type})]\n{}",
            String::from_utf8_lossy(&bytes)
        ));
    }
    let extension = mime
        .split('/')
        .nth(1)
        .map(|s| s.split('+').next().unwrap_or(s));
    let mut file = saved.join(anysola_core::ids::tool_output());
    if let Some(extension) = extension.filter(|e| !e.is_empty()) {
        file.set_extension(extension);
    }
    let _ = std::fs::create_dir_all(saved);
    let _ = std::fs::write(&file, &bytes);
    ToolContent::text(format!(
        "[Binary resource {label} ({mime_type}, {} bytes) saved to {}]",
        bytes.len(),
        file.display()
    ))
}

/// A `tools/call` result as the tool's output (the program's value) and content.
pub fn map_result(
    result: &Value,
    declares_output: bool,
    saved: &std::path::Path,
) -> Result<Called, String> {
    let parts = result["content"].as_array().cloned().unwrap_or_default();
    let text_of = |part: &Value| part["text"].as_str().map(str::to_string);
    if result["isError"] == json!(true) {
        let text = parts
            .iter()
            .filter(|p| p["type"] == "text")
            .filter_map(text_of)
            .collect::<Vec<_>>()
            .join("\n");
        let text = text.trim();
        return Err(if text.is_empty() {
            "MCP tool returned an error".to_string()
        } else {
            text.to_string()
        });
    }
    let content: Vec<ToolContent> = parts
        .iter()
        .filter_map(|part| {
            let s = |key: &str| part[key].as_str();
            match s("type")? {
                "text" => Some(ToolContent::text(s("text")?)),
                "image" | "audio" => Some(media(s("data")?, s("mimeType")?, None, saved)),
                "resource_link" => Some(ToolContent::text(s("uri")?)),
                "resource" => {
                    let resource = &part["resource"];
                    let r = |key: &str| resource[key].as_str();
                    match (r("text"), r("blob"), r("mimeType")) {
                        (Some(text), _, _) => Some(ToolContent::text(text)),
                        (None, Some(blob), Some(mime)) => Some(media(blob, mime, r("uri"), saved)),
                        _ => Some(ToolContent::text(r("uri").unwrap_or_default())),
                    }
                }
                _ => None,
            }
        })
        .collect();
    let text = content
        .iter()
        .filter_map(|c| match c {
            ToolContent::Text { text } => Some(text.as_str()),
            ToolContent::File { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let output = match result.get("structuredContent") {
        Some(structured) => structured.clone(),
        None if text.is_empty() => Value::Null,
        // Agents assume JSON returned as text is already an object.
        None if !declares_output && (text.starts_with('{') || text.starts_with('[')) => {
            serde_json::from_str(&text).unwrap_or(Value::String(text))
        }
        None => Value::String(text),
    };
    Ok(Called {
        output: Some(output),
        content,
    })
}

/// A server's listed tools as namespace tools that call back into the runtime.
pub fn tools(runtime: &Arc<Runtime>, server: &str, list: &[Value]) -> Vec<Arc<dyn NamespaceTool>> {
    list.iter()
        .map(|tool| {
            Arc::new(McpTool::new(Arc::downgrade(runtime), server, tool)) as Arc<dyn NamespaceTool>
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results() {
        let dir = tempfile::tempdir().unwrap();
        let call = |result: Value| map_result(&result, false, dir.path());
        let called = call(json!({"content": [{"type": "text", "text": "{\"a\":1}"}]})).unwrap();
        assert_eq!(called.output, Some(json!({"a": 1})));
        assert_eq!(
            call(json!({"isError": true, "content": []})),
            Err("MCP tool returned an error".into())
        );
        let called = call(json!({"content": [
            {"type": "image", "data": "aGk=", "mimeType": "image/png"},
            {"type": "resource", "resource": {"uri": "u", "blob": "aGk=", "mimeType": "text/plain"}},
            {"type": "resource_link", "uri": "file:///x", "name": "x"}
        ], "structuredContent": {"k": true}}))
        .unwrap();
        assert_eq!(called.output, Some(json!({"k": true})));
        assert_eq!(
            called.content[1..],
            [
                ToolContent::text("[Resource u (text/plain)]\nhi"),
                ToolContent::text("file:///x")
            ]
        );
        assert!(matches!(called.content[0], ToolContent::File { .. }));
        let saved =
            call(json!({"content": [{"type": "audio", "data": "aGk=", "mimeType": "audio/wav"}]}))
                .unwrap();
        let ToolContent::Text { text } = &saved.content[0] else {
            panic!()
        };
        assert!(text.starts_with("[Binary resource content (audio/wav, 2 bytes) saved to "));
    }
}
