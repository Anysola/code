//! The `write` tool: creates or overwrites a file, keeping an existing UTF-8 BOM.

use std::io::ErrorKind;
use std::path::Path;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

use super::access::{self, Kind};
use super::edit::diff::{FileDiff, Status};
use super::edit::mutation::{self, ReadFailure};
use super::{
    DefinitionEnv, Tool, ToolContext, ToolDefinition, ToolError, ToolOutput, ToolResult,
    parse_input,
};
use crate::embedded_def;
use crate::permission::PermissionRequest;

embedded_def!(def, "write");

pub struct WriteTool;

#[derive(Deserialize)]
struct Input {
    path: String,
    content: String,
}

impl Tool for WriteTool {
    fn name(&self) -> &'static str {
        "write"
    }

    fn permission(&self) -> &'static str {
        "edit"
    }

    fn definition(&self, _: &DefinitionEnv) -> ToolDefinition {
        let def = def();
        ToolDefinition {
            name: self.name().into(),
            description: def.description.clone(),
            input_schema: def.input_schema.clone(),
        }
    }

    fn execute<'a>(&'a self, input: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolResult> {
        async move {
            let input: Input = parse_input(self.name(), &def().input_schema, input)?;
            write(&input, ctx).await
        }
        .boxed()
    }
}

async fn write(input: &Input, ctx: &ToolContext) -> ToolResult {
    let failed = || ToolError::execution(format!("Unable to write {}", input.path));
    let target = access::resolve(ctx, &input.path, Some(Kind::File));
    access::authorize_external(ctx, &[&target]).await?;
    let current = match mutation::read_text(&target.absolute).await {
        Ok(file) => Some(file.text),
        Err(ReadFailure::Missing) => None,
        Err(_) => return Err(failed()),
    };
    let (content_bom, content) = mutation::split_bom(&input.content);
    let status = if current.is_some() {
        Status::Modified
    } else {
        Status::Added
    };
    let preview = FileDiff::new(
        &target.resource,
        current.as_deref().unwrap_or(""),
        content,
        status,
    );
    ctx.ask(PermissionRequest {
        action: "edit".into(),
        resources: vec![target.resource.clone()],
        save: vec!["*".into()],
        metadata: json!({ "files": [preview.to_value()] }),
    })
    .await?;

    let _guard = ctx
        .locks
        .lock(&mutation::lock_key(&target.absolute).await)
        .await;
    let existing_bom = head_bom(&target.absolute).await.map_err(|_| failed())?;
    let bom = existing_bom == Some(true) || content_bom;
    mutation::write_file(&target.absolute, &mutation::join_bom(content, bom))
        .await
        .map_err(|_| failed())?;
    let verb = if existing_bom.is_some() {
        "Wrote"
    } else {
        "Created"
    };
    Ok(ToolOutput::text(format!(
        "{verb} file successfully: {}",
        target.resource
    )))
}

/// Whether the file starts with a UTF-8 BOM, or `None` when it does not exist.
async fn head_bom(path: &Path) -> std::io::Result<Option<bool>> {
    match tokio::fs::metadata(path).await {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => return Err(ErrorKind::InvalidInput.into()),
        Err(error) if mutation::is_missing(&error) => return Ok(None),
        Err(error) => return Err(error),
    }
    let mut head = Vec::with_capacity(3);
    tokio::fs::File::open(path)
        .await?
        .take(3)
        .read_to_end(&mut head)
        .await?;
    Ok(Some(head == [0xEF, 0xBB, 0xBF]))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::permission::PermissionError;
    use crate::tool::test_support::context;

    async fn run(dir: &Path, input: Value) -> (ToolResult, Vec<PermissionRequest>) {
        let test = context(dir);
        let result = WriteTool.execute(input, &test.ctx).await;
        let asked = test.gate.asked.lock().unwrap().clone();
        (result, asked)
    }

    fn text(output: ToolOutput) -> String {
        match &output.content[..] {
            [crate::tool::ToolContent::Text { text }] => text.clone(),
            other => panic!("unexpected content {other:?}"),
        }
    }

    #[tokio::test]
    async fn creates_with_parents() {
        let dir = tempfile::tempdir().unwrap();
        let (result, asked) = run(
            dir.path(),
            json!({"path": "new/deep/n.txt", "content": "a\r\nb"}),
        )
        .await;
        assert_eq!(
            text(result.unwrap()),
            "Created file successfully: new/deep/n.txt"
        );
        assert_eq!(
            std::fs::read(dir.path().join("new/deep/n.txt")).unwrap(),
            b"a\r\nb"
        );
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].action, "edit");
        assert_eq!(asked[0].resources, ["new/deep/n.txt"]);
        let file = &asked[0].metadata["files"][0];
        assert_eq!(file["status"], "added");
        assert_eq!(file["file"], "new/deep/n.txt");
        assert_eq!(file["additions"], 2);
    }

    #[tokio::test]
    async fn overwrites_and_keeps_bom() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "\u{FEFF}old\n").unwrap();
        let (result, asked) = run(
            dir.path(),
            json!({"path": "a.txt", "content": "\u{FEFF}\u{FEFF}new\n"}),
        )
        .await;
        assert_eq!(text(result.unwrap()), "Wrote file successfully: a.txt");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "\u{FEFF}new\n"
        );
        let file = &asked[0].metadata["files"][0];
        assert_eq!(file["status"], "modified");
        assert_eq!(
            (file["additions"].clone(), file["deletions"].clone()),
            (json!(1), json!(1))
        );
    }

    #[tokio::test]
    async fn adds_bom_from_content_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "old").unwrap();
        run(dir.path(), json!({"path": "a.txt", "content": "plain"}))
            .await
            .0
            .unwrap();
        assert_eq!(std::fs::read(dir.path().join("a.txt")).unwrap(), b"plain");
        run(dir.path(), json!({"path": "a.txt", "content": "\u{FEFF}x"}))
            .await
            .0
            .unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("a.txt")).unwrap(),
            "\u{FEFF}x".as_bytes()
        );
    }

    #[tokio::test]
    async fn directory_fails_before_asking() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let (result, asked) = run(dir.path(), json!({"path": "sub", "content": "x"})).await;
        let error = result.unwrap_err();
        assert_eq!(error.message, "Unable to write sub");
        assert_eq!(error.type_tag(), "tool.execution");
        assert!(asked.is_empty());
    }

    #[tokio::test]
    async fn external_asks_directory_first() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let path = outside.path().join("o.txt");
        let (result, asked) = run(
            dir.path(),
            json!({"path": path.to_string_lossy(), "content": "x"}),
        )
        .await;
        let resource = access::slash(&path);
        assert_eq!(
            text(result.unwrap()),
            format!("Created file successfully: {resource}")
        );
        assert_eq!(asked[0].action, "external_directory");
        assert_eq!(
            asked[0].resources,
            [access::slash(&outside.path().join("*"))]
        );
        assert_eq!(asked[1].action, "edit");
        assert_eq!(asked[1].resources, [resource]);
    }

    #[tokio::test]
    async fn permission_block_keeps_its_message() {
        let dir = tempfile::tempdir().unwrap();
        let test = context(dir.path());
        *test.gate.answer.lock().unwrap() = Some(PermissionError::Blocked {
            action: "edit".into(),
            reason: None,
        });
        let error = WriteTool
            .execute(json!({"path": "a.txt", "content": "x"}), &test.ctx)
            .await
            .unwrap_err();
        assert_eq!(error.message, "Permission denied: edit");
        assert_eq!(error.type_tag(), "permission.rejected");
        assert!(!dir.path().join("a.txt").exists());
    }
}
