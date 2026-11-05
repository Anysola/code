//! Programs run through `execute` against 0.5.1's captured results, the catalog with an extra
//! namespace, and the opencode tools' effects.

use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use super::execute::ExecuteTool;
use super::{Called, Inventory, Namespace, NamespaceTool, catalog, opencode};
use crate::permission::{Effect, Rule};
use crate::storage::SessionRow;
use crate::tool::registry::Registry;
use crate::tool::test_support::{TestContext, context};
use crate::tool::{
    DefinitionEnv, Tool, ToolContent, ToolContext, ToolError, ToolErrorKind, ToolOutput,
};

fn execute_tool() -> ExecuteTool {
    ExecuteTool::new(Inventory::new(vec![opencode::namespace()]))
}

async fn run(t: &TestContext, code: &str) -> ToolOutput {
    execute_tool()
        .execute(json!({"code": code}), &t.ctx)
        .await
        .expect("execute returns a result")
}

fn text(output: &ToolOutput) -> &str {
    match &output.content[0] {
        ToolContent::Text { text } => text,
        ToolContent::File { .. } => "",
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn captured_results() {
    let dir = tempfile::tempdir().unwrap();
    let t = context(dir.path());
    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/results.json")).unwrap();
    let mut mismatches = Vec::new();
    for case in &cases {
        let code = case["code"].as_str().unwrap();
        let output = run(&t, code).await;
        if text(&output) != case["expected"].as_str().unwrap() {
            mismatches.push(format!("{code}\n  got: {}", text(&output)));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn call_rows_and_errors_in_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let t = context(dir.path());
    let output = run(
        &t,
        "await tools.opencode.worktree_list(); await tools.opencode.models({query: 'x'}); return tools.opencode.nope()",
    )
    .await;
    assert_eq!(
        output.metadata["toolCalls"],
        json!([
            {"tool": "opencode.worktree_list", "status": "completed"},
            {"tool": "opencode.models", "status": "completed", "input": {"query": "x"}},
        ])
    );
    assert_eq!(output.metadata["error"], json!(true));
}

#[tokio::test(flavor = "multi_thread")]
async fn sessions_are_renamed_and_moved() {
    let dir = tempfile::tempdir().unwrap();
    let t = context(dir.path());
    std::fs::create_dir(dir.path().join("dest")).unwrap();
    t.ctx
        .host
        .db()
        .create_session(&SessionRow {
            id: "ses_test".into(),
            project_id: "global".into(),
            parent_id: None,
            fork_session_id: None,
            fork_boundary: None,
            slug: "s".into(),
            directory: dir.path().display().to_string(),
            title: "Old".into(),
            version: "v".into(),
            agent: None,
            model: None,
            time_created: 0,
            time_updated: 0,
        })
        .unwrap();
    let output = run(
        &t,
        "return await tools.opencode.session_rename({title: ' New '})",
    )
    .await;
    assert_eq!(
        text(&output),
        "{\n  \"sessionID\": \"ses_test\",\n  \"title\": \"New\"\n}"
    );
    let output = run(
        &t,
        "return await tools.opencode.session_move({directory: 'dest'})",
    )
    .await;
    assert_eq!(
        text(&output),
        "{\n  \"sessionID\": \"ses_test\",\n  \"directory\": \"dest\"\n}"
    );
    let session = t.ctx.host.db().session("ses_test").unwrap().unwrap();
    assert_eq!(session.title, "New");
    let moved = dir.path().join("dest").display().to_string();
    assert_eq!(session.directory, moved);
    assert_eq!(
        t.notifier.messages.lock().unwrap()[0].1,
        format!("The working directory has been changed to {moved}.")
    );
    let output = run(
        &t,
        "return await tools.opencode.session_move({directory: 'nowhere'})",
    )
    .await;
    assert_eq!(text(&output), "Unable to move session to nowhere");
}

#[tokio::test(flavor = "multi_thread")]
async fn interrupting_stops_a_busy_program() {
    let dir = tempfile::tempdir().unwrap();
    let t = context(dir.path());
    let cancel = t.ctx.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();
    });
    let error = execute_tool()
        .execute(json!({"code": "for (;;) {}"}), &t.ctx)
        .await
        .unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Interrupted);
}

#[tokio::test(flavor = "multi_thread")]
async fn engine_differences_keep_the_shape() {
    let dir = tempfile::tempdir().unwrap();
    let t = context(dir.path());
    let cases = [
        (
            "(1)()",
            "TypeError: The called value is not a function. (line 1, col ",
        ),
        (
            "let x; return x.y.z",
            "TypeError: Cannot read properties of undefined (reading 'y'). (line 1, col ",
        ),
        (
            "function f() { return f() }; f()",
            "Execution exceeded the maximum nesting depth.",
        ),
        (
            "const a = {}; a.self = a; return a",
            "TypeError: Converting circular structure to JSON.",
        ),
        (
            "try { await tools.opencode.nope() } catch (e) { throw e }",
            "Unknown tool 'opencode.nope'.\nUse search to find available tools.",
        ),
    ];
    for (code, prefix) in cases {
        let output = run(&t, code).await;
        assert!(
            text(&output).starts_with(prefix),
            "{code}: {}",
            text(&output)
        );
    }
}

struct Echo {
    name: &'static str,
    description: &'static str,
    input: Value,
    output: Value,
}

impl NamespaceTool for Echo {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        self.description
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
        _: &'a ToolContext,
    ) -> BoxFuture<'a, Result<Called, ToolError>> {
        async move { Ok(Called::output(input)) }.boxed()
    }
}

/// The namespace an MCP server `testsrv` with tools `echo` and `get.weather` contributes.
fn testsrv() -> Arc<Namespace> {
    Arc::new(Namespace {
        name: "testsrv".into(),
        description: None,
        tools: vec![
            Arc::new(Echo {
                name: "echo",
                description: "Echo the text back",
                input: json!({"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}),
                output: json!({}),
            }),
            Arc::new(Echo {
                name: "get.weather",
                description: "Weather (dotted name)",
                input: json!({"type": "object", "properties": {}}),
                output: json!({}),
            }),
        ],
    })
}

#[test]
fn catalog_with_another_namespace_matches_capture() {
    let registry = Registry::builtin(&crate::config::Config::default());
    registry.set_namespace(testsrv());
    let inventory = registry.code_mode(&[]).unwrap();
    let captured =
        include_str!("../../../../spec/verbatim/integrations/codemode-catalog-with-mcp.txt");
    let expected = &captured[..captured.find("\n\n<mcp_instructions>").unwrap()];
    assert_eq!(catalog::render(&inventory.summary()), expected);
}

#[tokio::test(flavor = "multi_thread")]
async fn other_namespaces_are_callable() {
    let dir = tempfile::tempdir().unwrap();
    let t = context(dir.path());
    let tool = ExecuteTool::new(Inventory::new(vec![opencode::namespace(), testsrv()]));
    let output = tool
        .execute(
            json!({"code": "return [await tools.testsrv.echo({text: 'hi'}), Object.keys(tools)]"}),
            &t.ctx,
        )
        .await
        .unwrap();
    assert_eq!(
        text(&output),
        "[\n  {\n    \"text\": \"hi\"\n  },\n  [\n    \"opencode\",\n    \"testsrv\"\n  ]\n]"
    );
}

#[test]
fn denied_execute_exposes_direct_tools() {
    let registry = Registry::builtin(&crate::config::Config::default());
    let rules = vec![Rule::new("execute", "*", Effect::Deny)];
    let env = DefinitionEnv {
        shell_name: "bash".into(),
        year: 2026,
        subagents: Vec::new(),
    };
    let names: Vec<String> = Registry::definitions(&registry.active(&rules), &env)
        .into_iter()
        .map(|d| d.name)
        .collect();
    assert_eq!(
        names,
        [
            "edit",
            "glob",
            "grep",
            "opencode_list_mcp_resources",
            "opencode_models",
            "opencode_read_mcp_resource",
            "opencode_session_move",
            "opencode_session_rename",
            "opencode_worktree_create",
            "opencode_worktree_list",
            "opencode_worktree_remove",
            "question",
            "read",
            "shell",
            "skill",
            "subagent",
            "webfetch",
            "websearch",
            "write",
        ]
    );
    assert!(registry.code_mode(&rules).is_none());
    let captured: Value = serde_json::from_str(include_str!(
        "../../../../spec/verbatim/tools/_captured-request-build-execute-denied.json"
    ))
    .unwrap();
    for definition in Registry::definitions(&registry.active(&rules), &env)
        .into_iter()
        .filter(|d| d.name.starts_with("opencode_"))
    {
        let entry = captured["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == definition.name.as_str())
            .unwrap();
        assert_eq!(entry["description"], definition.description.as_str());
        assert_eq!(entry["input_schema"], definition.input_schema);
    }
    let edit_denied = vec![Rule::new("edit", "*", Effect::Deny)];
    let inventory = registry.code_mode(&edit_denied).unwrap();
    assert_eq!(inventory.summary().total, 7);
}
