//! Generic output bounding for tools that do not bound themselves (no `truncated` metadata): keep
//! the first 2000 lines / 51200 bytes, save the full text under `~/.anysola/tool-output/`, and
//! append a marker block. Lengths inside one result are counted in UTF-16 units, as 0.5.1 did.

use std::path::Path;

use serde_json::Value;

use super::{OutputLimits, ToolContent, ToolOutput};
use crate::text::{utf16_len, utf16_prefix};

pub fn bound(output: ToolOutput, limits: OutputLimits, directory: &Path) -> ToolOutput {
    if output.metadata.contains_key("truncated") {
        return output;
    }
    let ToolOutput {
        content,
        mut metadata,
    } = output;
    let text = content
        .iter()
        .filter_map(|item| match item {
            ToolContent::Text { text } => Some(text.as_str()),
            ToolContent::File { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut lines: Vec<&str> = text.split('\n').collect();
    if text.ends_with('\n') {
        lines.pop();
    }
    if lines.len() <= limits.max_lines && text.len() <= limits.max_bytes {
        metadata.insert("truncated".into(), Value::Bool(false));
        return ToolOutput { content, metadata };
    }
    let mut kept: Vec<&str> = Vec::new();
    let mut bytes = 0;
    for line in lines.iter().take(limits.max_lines) {
        let size = line.len() + usize::from(!kept.is_empty());
        if bytes + size > limits.max_bytes {
            break;
        }
        kept.push(line);
        bytes += size;
    }
    let file = directory.join(crate::ids::tool_output());
    let _ = std::fs::create_dir_all(directory);
    let _ = std::fs::write(&file, &text);
    let shown = if kept.is_empty() {
        "0 lines".to_string()
    } else {
        format!("lines 1-{}", kept.len())
    };
    let marker = format!(
        "[showing {shown} of {}; full output saved to {}]",
        lines.len(),
        file.display()
    );

    let mut remaining = utf16_len(&kept.join("\n"));
    let mut bounded = Vec::new();
    let mut seen_text = false;
    let mut marked = false;
    for item in content {
        let ToolContent::Text { text } = &item else {
            bounded.push(item);
            continue;
        };
        if seen_text && remaining > 0 {
            remaining -= 1;
        }
        seen_text = true;
        let length = utf16_len(text);
        if remaining >= length {
            remaining -= length;
            bounded.push(item);
            continue;
        }
        if remaining > 0 {
            bounded.push(ToolContent::text(utf16_prefix(text, remaining)));
        }
        if !marked {
            bounded.push(ToolContent::text(marker.clone()));
        }
        remaining = 0;
        marked = true;
    }
    if !marked {
        bounded.push(ToolContent::text(marker));
    }
    metadata.insert("truncated".into(), Value::Bool(true));
    metadata.insert(
        "outputPath".into(),
        Value::String(file.display().to_string()),
    );
    ToolOutput {
        content: bounded,
        metadata,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_output_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let out = bound(
            ToolOutput::text("a\nb\n"),
            OutputLimits::default(),
            dir.path(),
        );
        assert_eq!(out.content, vec![ToolContent::text("a\nb\n")]);
        assert_eq!(out.metadata["truncated"], Value::Bool(false));
    }

    #[test]
    fn long_output_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let text: String = (1..=10).map(|i| format!("line{i}\n")).collect();
        let out = bound(
            ToolOutput::text(text),
            OutputLimits {
                max_lines: 3,
                max_bytes: 51200,
            },
            dir.path(),
        );
        assert_eq!(out.content.len(), 2);
        assert_eq!(out.content[0], ToolContent::text("line1\nline2\nline3"));
        let ToolContent::Text { text } = &out.content[1] else {
            panic!()
        };
        assert!(text.starts_with("[showing lines 1-3 of 10; full output saved to "));
        assert_eq!(out.metadata["truncated"], Value::Bool(true));
    }
}
