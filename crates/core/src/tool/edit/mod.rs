//! The `edit` tool: exact find-and-replace with line-ending and typographic tolerance.

pub(crate) mod diff;
mod hint;
mod matcher;
pub(crate) mod mutation;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};

use super::access::{self, Kind};
use super::{
    DefinitionEnv, Tool, ToolContext, ToolDefinition, ToolError, ToolOutput, ToolResult,
    parse_input,
};
use crate::embedded_def;
use crate::permission::PermissionRequest;
use diff::{FileDiff, Status};
use mutation::ReadFailure;

embedded_def!(def, "edit");

pub struct EditTool;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Input {
    path: String,
    old_string: String,
    new_string: String,
    replace_all: Option<bool>,
}

impl Tool for EditTool {
    fn name(&self) -> &'static str {
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
            edit(&input, ctx).await
        }
        .boxed()
    }
}

async fn edit(input: &Input, ctx: &ToolContext) -> ToolResult {
    let path = &input.path;
    if input.old_string == input.new_string {
        return Err(ToolError::execution(
            "No changes to apply: oldString and newString are identical.",
        ));
    }
    if input.old_string.is_empty() {
        return Err(ToolError::execution(
            "oldString must not be empty. Use write to create or overwrite a file.",
        ));
    }
    let failed = || ToolError::execution(format!("Unable to edit {path}"));
    let target = access::resolve(ctx, path, Some(Kind::File));
    // The whole read-match-ask-write runs under the file's lock, so parallel edits all apply.
    let _guard = ctx
        .locks
        .lock(&mutation::lock_key(&target.absolute).await)
        .await;
    access::authorize_external(ctx, &[&target]).await?;

    let original =
        mutation::read_text(&target.absolute)
            .await
            .map_err(|failure| match failure {
                ReadFailure::Missing => ToolError::execution(format!("File not found: {path}")),
                ReadFailure::Directory => {
                    ToolError::execution(format!("Path is a directory, not a file: {path}"))
                }
                ReadFailure::NotUtf8 => ToolError::execution(format!(
                    "File is not valid UTF-8: {path}. Editing it would replace every byte that is not UTF-8, so it was left unchanged."
                )),
                ReadFailure::Failed => failed(),
            })?;
    let source = &original.text;
    let matches = matcher::find(source, &input.old_string, &input.new_string);
    let replace_all = input.replace_all == Some(true);
    let count = matches.ranges.len();
    let replaced = matches.apply(source, if replace_all { count } else { 1 });
    let preview = (count == 1 || (count > 1 && replace_all))
        .then(|| json!({ "files": [FileDiff::new(&target.resource, source, &replaced, Status::Modified).to_value()] }));
    ctx.ask(PermissionRequest {
        action: "edit".into(),
        resources: vec![target.resource.clone()],
        save: vec!["*".into()],
        metadata: preview.unwrap_or(Value::Null),
    })
    .await?;

    if count == 0 {
        // The hint shows file contents, so only when reading this file is already allowed.
        let readable = ctx
            .permissions
            .allows("read", std::slice::from_ref(&target.resource));
        let hint = readable
            .then(|| hint::miss_hint(source, &input.old_string, &input.new_string))
            .flatten()
            .map(|hint| format!("\n\n{hint}"))
            .unwrap_or_default();
        return Err(ToolError::execution(format!(
            "Could not find oldString in {path}. It must match exactly, including whitespace and indentation.{hint}"
        )));
    }
    if count > 1 && !replace_all {
        return Err(ToolError::execution(format!(
            "Found {count} matches for oldString, but expected exactly one. Add more surrounding context to make oldString unique, or set replaceAll to true to replace every occurrence."
        )));
    }
    // An endings-only edit always changes bytes (its region is oldString itself), so only a
    // normalized match can leave the file as it was.
    if replaced == *source {
        return Err(ToolError::execution(format!(
            "No changes to apply: the edit leaves {path} unchanged. Line endings follow the file; to change them, write \\r explicitly in oldString or newString."
        )));
    }

    let (replacement_bom, after) = mutation::split_bom(&replaced);
    let bom = original.bom || replacement_bom;
    mutation::write_file(&target.absolute, &mutation::join_bom(after, bom))
        .await
        .map_err(|_| failed())?;
    let formatted = mutation::format(ctx, &target.absolute, bom).await;
    let after = formatted.as_deref().unwrap_or(after);
    let file = FileDiff::new(&target.resource, source, after, Status::Modified);
    let plural = if count == 1 { "" } else { "s" };
    Ok(ToolOutput::text(format!(
        "Edited {} ({count} replacement{plural})",
        target.resource
    ))
    .with_metadata("files", json!([file.to_value()])))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;

    use super::*;
    use crate::tool::ToolContent;
    use crate::tool::test_support::context;

    async fn run(dir: &Path, input: Value) -> (ToolResult, Vec<PermissionRequest>) {
        let test = context(dir);
        let result = EditTool.execute(input, &test.ctx).await;
        let asked = test.gate.asked.lock().unwrap().clone();
        (result, asked)
    }

    async fn error(dir: &Path, input: Value) -> String {
        let error = run(dir, input).await.0.unwrap_err();
        assert_eq!(error.type_tag(), "tool.execution");
        error.message
    }

    fn text(output: &ToolOutput) -> &str {
        match &output.content[..] {
            [ToolContent::Text { text }] => text,
            other => panic!("unexpected content {other:?}"),
        }
    }

    fn setup(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            std::fs::write(dir.path().join(name), content).unwrap();
        }
        dir
    }

    fn read(dir: &tempfile::TempDir, name: &str) -> String {
        std::fs::read_to_string(dir.path().join(name)).unwrap()
    }

    #[tokio::test]
    async fn edits_once_with_metadata() {
        let dir = setup(&[("e.ts", "const x = 0\nconst y = 1\n")]);
        let (result, asked) = run(
            dir.path(),
            json!({"path": "e.ts", "oldString": "const y = 1", "newString": "const y = 2"}),
        )
        .await;
        let output = result.unwrap();
        assert_eq!(text(&output), "Edited e.ts (1 replacement)");
        assert_eq!(read(&dir, "e.ts"), "const x = 0\nconst y = 2\n");
        let patch = "Index: e.ts\n===================================================================\n--- e.ts\n+++ e.ts\n@@ -1,2 +1,2 @@\n const x = 0\n-const y = 1\n+const y = 2\n";
        let files = json!([{"file": "e.ts", "patch": patch, "status": "modified", "additions": 1, "deletions": 1}]);
        assert_eq!(Value::Object(output.metadata), json!({ "files": files }));
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].action, "edit");
        assert_eq!(asked[0].resources, ["e.ts"]);
        assert_eq!(asked[0].metadata, json!({ "files": files }));
    }

    #[tokio::test]
    async fn replace_all() {
        let dir = setup(&[("dup.txt", "a\nb\na\n")]);
        let (result, _) = run(
            dir.path(),
            json!({"path": "dup.txt", "oldString": "a", "newString": "c", "replaceAll": true}),
        )
        .await;
        assert_eq!(text(&result.unwrap()), "Edited dup.txt (2 replacements)");
        assert_eq!(read(&dir, "dup.txt"), "c\nb\nc\n");
    }

    #[tokio::test]
    async fn multiple_matches_fail_after_asking_without_preview() {
        let dir = setup(&[("dup.txt", "a\nb\na\n")]);
        let (result, asked) = run(
            dir.path(),
            json!({"path": "dup.txt", "oldString": "a", "newString": "c"}),
        )
        .await;
        assert_eq!(
            result.unwrap_err().message,
            "Found 2 matches for oldString, but expected exactly one. Add more surrounding context to make oldString unique, or set replaceAll to true to replace every occurrence."
        );
        assert_eq!(asked[0].metadata, Value::Null);
        assert_eq!(read(&dir, "dup.txt"), "a\nb\na\n");
    }

    #[tokio::test]
    async fn input_errors_come_first() {
        let dir = setup(&[]);
        assert_eq!(
            error(
                dir.path(),
                json!({"path": "x", "oldString": "a", "newString": "a"})
            )
            .await,
            "No changes to apply: oldString and newString are identical."
        );
        assert_eq!(
            error(
                dir.path(),
                json!({"path": "x", "oldString": "", "newString": "a"})
            )
            .await,
            "oldString must not be empty. Use write to create or overwrite a file."
        );
    }

    #[tokio::test]
    async fn missing_file_and_directory() {
        let dir = setup(&[]);
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        assert_eq!(
            error(
                dir.path(),
                json!({"path": "none.txt", "oldString": "a", "newString": "b"})
            )
            .await,
            "File not found: none.txt"
        );
        assert_eq!(
            error(
                dir.path(),
                json!({"path": "sub", "oldString": "a", "newString": "b"})
            )
            .await,
            "Path is a directory, not a file: sub"
        );
    }

    #[tokio::test]
    async fn misses_with_and_without_hints() {
        let dir = setup(&[("e.ts", "const x = 0\nconst y = 1\n")]);
        let miss = "Could not find oldString in e.ts. It must match exactly, including whitespace and indentation.";
        assert_eq!(
            error(
                dir.path(),
                json!({"path": "e.ts", "oldString": "nothing here", "newString": "b"})
            )
            .await,
            miss
        );
        assert_eq!(
            error(
                dir.path(),
                json!({"path": "e.ts", "oldString": "const y  =  1;", "newString": "b"})
            )
            .await,
            miss
        );
        assert_eq!(
            error(
                dir.path(),
                json!({"path": "e.ts", "oldString": "const  y = 1\nconst q", "newString": "b"})
            )
            .await,
            format!("{miss}\n\nThe closest lines in the file:\n2: const y = 1")
        );
        let dir = setup(&[("e.ts", "export const greeting = 'hello'\n")]);
        assert_eq!(
            error(
                dir.path(),
                json!({"path": "e.ts", "oldString": "const greeting = 'hi'", "newString": "const greeting = 'hello'"})
            )
            .await,
            format!("{miss}\n\nnewString is already in the file, so this edit may already be applied.")
        );
    }

    #[tokio::test]
    async fn hint_needs_read_permission() {
        let dir = setup(&[("e.ts", "const y = 1\n")]);
        let test = context(dir.path());
        struct DenyRead(std::sync::Arc<crate::tool::test_support::RecordingGate>);
        impl crate::permission::PermissionGate for DenyRead {
            fn ask(
                &self,
                request: PermissionRequest,
            ) -> futures_util::future::BoxFuture<'_, Result<(), crate::permission::PermissionError>>
            {
                self.0.ask(request)
            }
            fn allows(&self, action: &str, _: &[String]) -> bool {
                action != "read"
            }
        }
        let mut ctx = test.ctx.clone();
        ctx.permissions = std::sync::Arc::new(DenyRead(test.gate.clone()));
        let error = EditTool
            .execute(
                json!({"path": "e.ts", "oldString": "const  y = 1", "newString": "b"}),
                &ctx,
            )
            .await
            .unwrap_err();
        assert_eq!(
            error.message,
            "Could not find oldString in e.ts. It must match exactly, including whitespace and indentation."
        );
    }

    #[tokio::test]
    async fn no_op_edit() {
        let dir = setup(&[("f.txt", "say 'x'\n")]);
        assert_eq!(
            error(dir.path(), json!({"path": "f.txt", "oldString": "say \u{2018}x\u{2019}", "newString": "say 'x'"})).await,
            "No changes to apply: the edit leaves f.txt unchanged. Line endings follow the file; to change them, write \\r explicitly in oldString or newString."
        );
    }

    #[tokio::test]
    async fn bom_is_kept_and_not_doubled() {
        let dir = setup(&[("b.txt", "\u{FEFF}hello\n")]);
        run(
            dir.path(),
            json!({"path": "b.txt", "oldString": "hello", "newString": "\u{FEFF}bye"}),
        )
        .await
        .0
        .unwrap();
        assert_eq!(read(&dir, "b.txt"), "\u{FEFF}bye\n");
    }

    #[tokio::test]
    async fn non_utf8_file_is_refused_and_untouched() {
        let dir = setup(&[]);
        let bytes = b"caf\xE9 au lait\n";
        std::fs::write(dir.path().join("l.txt"), bytes).unwrap();
        assert_eq!(
            error(
                dir.path(),
                json!({"path": "l.txt", "oldString": "au", "newString": "with"})
            )
            .await,
            "File is not valid UTF-8: l.txt. Editing it would replace every byte that is not UTF-8, so it was left unchanged."
        );
        assert_eq!(std::fs::read(dir.path().join("l.txt")).unwrap(), bytes);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn replaces_atomically_through_symlinks_keeping_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = setup(&[("real.sh", "echo one\n")]);
        let real = dir.path().join("real.sh");
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o751)).unwrap();
        std::os::unix::fs::symlink(&real, dir.path().join("link.sh")).unwrap();
        run(
            dir.path(),
            json!({"path": "link.sh", "oldString": "one", "newString": "two"}),
        )
        .await
        .0
        .unwrap();
        assert!(
            std::fs::symlink_metadata(dir.path().join("link.sh"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(read(&dir, "real.sh"), "echo two\n");
        let mode = std::fs::metadata(&real).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o751);
        let mut names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["link.sh", "real.sh"]);
    }

    #[tokio::test]
    async fn crlf_file_stays_crlf() {
        let dir = setup(&[("c.txt", "one\r\ntwo\r\n")]);
        run(
            dir.path(),
            json!({"path": "c.txt", "oldString": "one\n", "newString": "one\nnew\n"}),
        )
        .await
        .0
        .unwrap();
        assert_eq!(read(&dir, "c.txt"), "one\r\nnew\r\ntwo\r\n");
    }

    #[tokio::test]
    async fn typographic_and_trailing_whitespace_tiers() {
        let dir = setup(&[
            ("q.ts", "let z = \u{201C}hi\u{201D}\n"),
            ("w.txt", "foo   \nbar\n"),
        ]);
        let (result, _) = run(
            dir.path(),
            json!({"path": "q.ts", "oldString": "let z = \"hi\"", "newString": "let z = \"yo\""}),
        )
        .await;
        assert_eq!(text(&result.unwrap()), "Edited q.ts (1 replacement)");
        assert_eq!(read(&dir, "q.ts"), "let z = \"yo\"\n");
        run(
            dir.path(),
            json!({"path": "w.txt", "oldString": "foo\nbar", "newString": "baz"}),
        )
        .await
        .0
        .unwrap();
        assert_eq!(read(&dir, "w.txt"), "baz\n");
    }

    #[tokio::test]
    async fn parallel_edits_all_apply() {
        let lines: String = (0..13).map(|n| format!("line {n}\n")).collect();
        let dir = setup(&[("p.txt", &lines)]);
        let test = context(dir.path());
        let edits = (0..13).map(|n| {
            EditTool.execute(
                json!({"path": "p.txt", "oldString": format!("line {n}\n"), "newString": format!("LINE {n}\n")}),
                &test.ctx,
            )
        });
        for result in futures_util::future::join_all(edits).await {
            result.unwrap();
        }
        assert_eq!(read(&dir, "p.txt"), lines.replace("line", "LINE"));
    }

    #[tokio::test]
    async fn external_path_uses_absolute_resource() {
        let dir = setup(&[]);
        let outside = setup(&[("o.txt", "x\n")]);
        let path = outside.path().join("o.txt");
        let (result, asked) = run(
            dir.path(),
            json!({"path": path.to_string_lossy(), "oldString": "x", "newString": "y"}),
        )
        .await;
        let resource = access::slash(&path);
        assert_eq!(
            text(&result.unwrap()),
            format!("Edited {resource} (1 replacement)")
        );
        assert_eq!(asked[0].action, "external_directory");
        assert_eq!(asked[1].action, "edit");
        assert_eq!(asked[1].resources, [resource]);
    }
}
