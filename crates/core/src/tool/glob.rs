//! The `glob` tool: file paths matching a glob, as `rg --files --glob=<pattern>` listed them (see
//! [`super::search`]).

use std::path::PathBuf;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::access::{self, Kind};
use super::search::{self, DEFAULT_LIMIT, EXCLUDE_GIT};
use super::{
    DefinitionEnv, Tool, ToolContext, ToolDefinition, ToolError, ToolOutput, ToolResult,
    parse_input,
};
use crate::embedded_def;
use crate::permission::PermissionRequest;

embedded_def!(def, "glob");

/// Positive globs override rg's hidden-file filter, so hidden entries are excluded explicitly.
const EXCLUDE_HIDDEN: &str = "!**/.*";

pub struct GlobTool;

#[derive(Deserialize)]
struct Input {
    pattern: String,
    path: Option<String>,
    hidden: Option<bool>,
    limit: Option<usize>,
}

fn ask_metadata(path: Option<&str>, input: &Input) -> Value {
    let mut map = Map::new();
    map.insert("root".into(), path.unwrap_or(".").into());
    let fields = [
        ("path", path.map(Value::from)),
        ("hidden", input.hidden.map(Value::from)),
        ("limit", input.limit.map(Value::from)),
    ];
    for (key, value) in fields {
        if let Some(value) = value {
            map.insert(key.into(), value);
        }
    }
    Value::Object(map)
}

fn format_files(files: &[PathBuf], truncated: bool) -> String {
    let mut lines: Vec<String> = if files.is_empty() {
        vec!["No files found".into()]
    } else {
        files.iter().map(|f| f.display().to_string()).collect()
    };
    if truncated {
        lines.push(String::new());
        lines.push(search::truncation_notice(files.len()));
    }
    lines.join("\n")
}

impl Tool for GlobTool {
    fn name(&self) -> &'static str {
        "glob"
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
            // Models sometimes send the strings "undefined" / "null" for an omitted path.
            let path = input
                .path
                .as_deref()
                .filter(|path| !matches!(*path, "undefined" | "null"));
            let shown_path = path.unwrap_or(".");
            let target = access::resolve(ctx, shown_path, Some(Kind::Directory));
            access::authorize_external(ctx, &[&target]).await?;
            ctx.ask(PermissionRequest {
                action: self.name().into(),
                resources: vec![input.pattern.clone()],
                save: vec!["*".into()],
                metadata: ask_metadata(path, &input),
            })
            .await?;
            let metadata = std::fs::metadata(&target.absolute).map_err(|_| {
                ToolError::execution(format!("Search path does not exist: {shown_path}"))
            })?;
            if !metadata.is_dir() {
                return Err(ToolError::execution(format!(
                    "Search path is not a directory: {shown_path}"
                )));
            }
            let limit = input.limit.unwrap_or(DEFAULT_LIMIT);
            let hidden = input.hidden == Some(true);
            let pattern = input.pattern;
            let root = target.absolute;
            let mut files = search::run(move |stop| {
                let mut globs = vec![pattern.as_str()];
                if !hidden {
                    globs.push(EXCLUDE_HIDDEN);
                }
                globs.push(EXCLUDE_GIT);
                match search::overrides(&root, &globs) {
                    Some(overrides) => search::files(&root, overrides, hidden, stop)
                        .take(limit + 1)
                        .collect(),
                    None => Vec::new(),
                }
            })
            .await?;
            let truncated = files.len() > limit;
            files.truncate(limit);
            Ok(ToolOutput::text(format_files(&files, truncated))
                .with_metadata("count", files.len())
                .with_metadata("truncated", truncated))
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;

    use super::*;
    use crate::tool::ToolContent;
    use crate::tool::search::fixture;
    use crate::tool::test_support::context;

    async fn glob(dir: &Path, input: Value) -> ToolResult {
        let test = context(dir);
        GlobTool.execute(input, &test.ctx).await
    }

    async fn text(dir: &Path, input: Value) -> String {
        let out = glob(dir, input).await.unwrap();
        let [ToolContent::Text { text }] = out.content.as_slice() else {
            panic!("one text block expected")
        };
        text.clone()
    }

    #[tokio::test]
    async fn captured_listings() {
        let dir = tempfile::tempdir().unwrap();
        fixture::project(dir.path());
        let d = dir.path().display();
        assert_eq!(
            text(dir.path(), json!({"pattern": "**/*.ts"})).await,
            format!("{d}/src/x.ts")
        );
        assert_eq!(
            text(dir.path(), json!({"pattern": "**/*.ts", "hidden": true})).await,
            format!("{d}/.hid/h.ts\n{d}/src/x.ts")
        );
        assert_eq!(
            text(dir.path(), json!({"pattern": "*.none"})).await,
            "No files found"
        );
        assert_eq!(
            text(dir.path(), json!({"pattern": "**/*", "limit": 1})).await,
            format!(
                "{d}/a.txt\n\n(Results are truncated: showing first 1 results. Consider using a more specific path or pattern.)"
            )
        );
    }

    #[tokio::test]
    async fn positive_glob_reincludes_ignored_directory() {
        let dir = tempfile::tempdir().unwrap();
        fixture::project(dir.path());
        let d = dir.path().display();
        assert_eq!(
            text(dir.path(), json!({"pattern": "**/*"})).await,
            format!("{d}/a.txt\n{d}/ign/i.ts\n{d}/src/x.ts")
        );
        // Hidden entries stay out unless asked for, even when the glob names them.
        assert_eq!(
            text(dir.path(), json!({"pattern": ".gitignore"})).await,
            "No files found"
        );
        assert_eq!(
            text(
                dir.path(),
                json!({"pattern": "**/*", "hidden": true, "path": "undefined"})
            )
            .await,
            format!("{d}/.gitignore\n{d}/.hid/h.ts\n{d}/a.txt\n{d}/ign/i.ts\n{d}/src/x.ts")
        );
    }

    #[tokio::test]
    async fn captured_errors_and_metadata() {
        let dir = tempfile::tempdir().unwrap();
        fixture::project(dir.path());
        let error = glob(dir.path(), json!({"pattern": "*", "path": "a.txt"}))
            .await
            .unwrap_err();
        assert_eq!(error.type_tag(), "tool.execution");
        assert_eq!(error.message, "Search path is not a directory: a.txt");
        let error = glob(dir.path(), json!({"pattern": "*", "path": "nope"}))
            .await
            .unwrap_err();
        assert_eq!(error.message, "Search path does not exist: nope");
        let out = glob(dir.path(), json!({"pattern": "**/*.ts"}))
            .await
            .unwrap();
        assert_eq!(
            Value::Object(out.metadata),
            json!({"count": 1, "truncated": false})
        );
        // An invalid glob made rg fail without output.
        assert_eq!(
            text(dir.path(), json!({"pattern": "a{b"})).await,
            "No files found"
        );
    }

    #[tokio::test]
    async fn permission_request() {
        let dir = tempfile::tempdir().unwrap();
        let test = context(dir.path());
        GlobTool
            .execute(json!({"pattern": "*.rs", "hidden": false}), &test.ctx)
            .await
            .unwrap();
        let asked = test.gate.asked.lock().unwrap();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].action, "glob");
        assert_eq!(asked[0].resources, ["*.rs"]);
        assert_eq!(asked[0].save, ["*"]);
        assert_eq!(asked[0].metadata, json!({"root": ".", "hidden": false}));
    }
}
