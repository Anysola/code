//! A schedule's `if`/`until` check: a shell command or a confined Code Mode program run before
//! firing, with no model call. Checks are approved once at creation; when they run later nobody is
//! present, so anything beyond what was approved is refused.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use regex::Regex;
use serde_json::{Value, json};

use crate::api::Shared;
use crate::codemode::{Called, Inventory, Namespace, NamespaceTool, direct_name, path_of};
use crate::permission::{PermissionError, PermissionGate, PermissionRequest};
use crate::session::gate::{RuleGate, Scope};
use crate::storage::SessionRow;
use crate::tool::{ToolContext, ToolError, access};

pub const TIMEOUT_MS: u64 = 30_000;
const MAX_OUTPUT_BYTES: usize = 32 * 1024;
const MAX_TOOL_CALLS: usize = 20;

const FORBIDDEN_CODE_TOOLS: &[&str] = &[
    "opencode.schedule_create",
    "opencode.schedule_pause",
    "opencode.schedule_resume",
    "opencode.schedule_delete",
    "opencode.session_move",
    "opencode.session_rename",
    "opencode.worktree_create",
    "opencode.worktree_remove",
];

pub struct Run {
    pub message_id: String,
    pub now: i64,
    pub checks: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Checked {
        wake: bool,
        state: Value,
        message: Option<String>,
    },
    Failed {
        error: String,
        /// A broken check (bad syntax, unknown tool, refused approval) pauses at once.
        invalid: bool,
    },
}

fn failed(error: impl Into<String>) -> Outcome {
    Outcome::Failed {
        error: error.into(),
        invalid: false,
    }
}

/// Syntax checks made before asking for permissions or running anything.
pub fn validate(condition: &Value) -> Result<(), String> {
    if condition["type"] == "code" {
        let paths = scan_tools(condition["code"].as_str().unwrap_or_default())?;
        return match paths
            .iter()
            .find(|p| FORBIDDEN_CODE_TOOLS.contains(&p.as_str()))
        {
            Some(path) => Err(format!(
                "Tool {path} cannot be used inside a schedule check"
            )),
            None => Ok(()),
        };
    }
    if condition["run"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        return Err("A shell check needs a command".into());
    }
    let predicates = ["is", "matches", "changes"]
        .iter()
        .filter(|k| condition.get(**k).is_some())
        .count();
    if predicates > 1 {
        return Err("Choose at most one of is, matches or changes".into());
    }
    if let Some(pattern) = condition["matches"].as_str() {
        fancy_regex::Regex::new(pattern)
            .map_err(|e| format!("Invalid regular expression {}: {e}", Value::from(pattern)))?;
    }
    Ok(())
}

pub fn describe(condition: &Value) -> String {
    if condition["type"] == "shell" {
        return condition["run"].as_str().unwrap_or_default().to_string();
    }
    let first = condition["code"]
        .as_str()
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty());
    match first {
        Some(line) => format!("code ({})", line.chars().take(60).collect::<String>()),
        None => "code".into(),
    }
}

fn without_comments(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut chars = code.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            out.push(c);
            if c == '\\' {
                out.extend(chars.next());
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('/', Some('/')) => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
                out.push(' ');
            }
            ('"' | '\'' | '`', _) => {
                quote = Some(c);
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// The `namespace.tool` paths a program calls as `tools.<ns>.<tool>(...)` or
/// `tools["ns"].tool(...)`; any other use of `tools` cannot be checked ahead of time.
pub fn scan_tools(code: &str) -> Result<Vec<String>, String> {
    let code = without_comments(code);
    let segment = r#"(?:\.\s*([A-Za-z_$][\w$]*)|\[\s*"([^"]+)"\s*\]|\[\s*'([^']+)'\s*\])"#;
    let call =
        Regex::new(&format!(r"(^|[^\w$.])tools\s*{segment}\s*{segment}\s*\(")).expect("pattern");
    let any = Regex::new(r"(^|[^\w$.])tools\b").expect("pattern");
    let mut paths = Vec::new();
    for found in call.captures_iter(&code) {
        let pick = |a: usize| (a..a + 3).find_map(|i| found.get(i)).map(|m| m.as_str());
        let (Some(namespace), Some(tool)) = (pick(2), pick(5)) else {
            continue;
        };
        let path = path_of(namespace, tool);
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    if any.find_iter(&code).count() != call.find_iter(&code).count() {
        return Err(
            "Code Mode checks must call tools statically, as tools.<namespace>.<tool>(...)".into(),
        );
    }
    Ok(paths)
}

/// The tools a code check calls and the permission actions they need.
fn resolve(ctx: &ToolContext, paths: &[String]) -> Result<(Inventory, Vec<String>), String> {
    let inventory = ctx.host.code_mode(&ctx.agent).unwrap_or_default();
    let mut namespaces: Vec<Namespace> = Vec::new();
    let mut actions: Vec<String> = Vec::new();
    for path in paths {
        if FORBIDDEN_CODE_TOOLS.contains(&path.as_str()) {
            return Err(format!(
                "Tool {path} cannot be used inside a schedule check"
            ));
        }
        let (namespace, tool) = inventory
            .find(path)
            .ok_or_else(|| format!("Unknown Code Mode tool: {path}"))?;
        if tool.permission() == Some("edit") {
            return Err(format!(
                "Tool {path} cannot be used inside a schedule check"
            ));
        }
        let action = tool
            .permission()
            .map(str::to_string)
            .unwrap_or_else(|| direct_name(&namespace.name, tool.name()));
        if !actions.contains(&action) {
            actions.push(action);
        }
        match namespaces.iter_mut().find(|n| n.name == namespace.name) {
            Some(existing) => existing.tools.push(tool.clone()),
            None => namespaces.push(Namespace {
                name: namespace.name.clone(),
                description: namespace.description.clone(),
                direct: false,
                tools: vec![tool.clone()],
            }),
        }
    }
    Ok((
        Inventory::new(namespaces.into_iter().map(Arc::new).collect()),
        actions,
    ))
}

/// Asks, while the user is present, for what the check will need unattended; nothing is saved.
pub async fn authorize(
    ctx: &ToolContext,
    name: Option<&str>,
    when: &Value,
    check: &Value,
) -> Result<(), String> {
    let condition = &check["condition"];
    let mut metadata =
        json!({"schedule": name, "mode": check["mode"], "when": when, "unattended": true});
    if condition["type"] == "shell" {
        metadata["run"] = condition["run"].clone();
        let run = condition["run"].as_str().unwrap_or_default();
        let (shell, target) = shell_target(ctx);
        return crate::tool::shell::authorize(ctx, run, &shell, &target)
            .await
            .map_err(|e| e.message);
    }
    let code = condition["code"].as_str().unwrap_or_default();
    let paths = scan_tools(code)?;
    metadata["code"] = json!(code);
    metadata["tools"] = json!(paths);
    let denied = |action: &str| ctx.permissions.denies(action, &["*".to_string()]);
    if denied("execute") {
        return Err("Permission denied: execute".into());
    }
    let (_, actions) = resolve(ctx, &paths)?;
    // Fail fast on any explicit deny before asking the user to approve earlier tools.
    if let Some(action) = actions.iter().find(|a| denied(a)) {
        return Err(format!("Permission denied: {action}"));
    }
    let asked = if actions.is_empty() {
        vec!["execute".to_string()]
    } else {
        actions
    };
    for action in asked {
        ctx.ask(PermissionRequest {
            action,
            resources: vec!["*".into()],
            save: Vec::new(),
            metadata: metadata.clone(),
        })
        .await
        .map_err(|e| e.message)?;
    }
    Ok(())
}

fn shell_target(ctx: &ToolContext) -> (String, access::Target) {
    let shell = crate::tool::shell::select::resolve(ctx.configured_shell.as_deref());
    let directory = ctx.directory.to_string_lossy().into_owned();
    let target = access::resolve(ctx, &directory, Some(access::Kind::Directory));
    (shell, target)
}

/// Allows what the check was approved for at creation unless a rule now denies it; anything
/// else would need someone to approve it.
struct Unattended {
    inner: Arc<dyn PermissionGate>,
    approved: Vec<String>,
}

impl PermissionGate for Unattended {
    fn ask(&self, request: PermissionRequest) -> BoxFuture<'_, Result<(), PermissionError>> {
        let result = if self.inner.denies(&request.action, &request.resources) {
            Err(PermissionError::Blocked {
                action: request.action,
                reason: None,
            })
        } else if self.approved.contains(&request.action)
            || self.inner.allows(&request.action, &request.resources)
        {
            Ok(())
        } else {
            Err(PermissionError::Blocked {
                reason: Some(format!(
                    "Permission {} needs approval, and nobody is present to give it",
                    request.action
                )),
                action: request.action,
            })
        };
        futures_util::future::ready(result).boxed()
    }

    fn allows(&self, action: &str, resources: &[String]) -> bool {
        !self.inner.denies(action, resources)
            && (self.approved.iter().any(|a| a == action) || self.inner.allows(action, resources))
    }

    fn denies(&self, action: &str, resources: &[String]) -> bool {
        self.inner.denies(action, resources)
    }
}

/// Runs one check under the unattended rules and a 30 s cap.
pub async fn evaluate(ctx: &ToolContext, check: &Value, state: &Value, run: &Run) -> Outcome {
    let condition = &check["condition"];
    let stop = ctx.cancel.child_token();
    let mut ctx = ctx.clone();
    ctx.cancel = stop.clone();
    ctx.message_id = run.message_id.clone();
    let checked = async {
        if condition["type"] == "shell" {
            ctx.permissions = Arc::new(Unattended {
                inner: ctx.permissions.clone(),
                approved: vec!["shell".into(), "external_directory".into()],
            });
            run_shell(&ctx, condition, state).await
        } else {
            run_code(&mut ctx, condition, state, run).await
        }
    };
    match tokio::time::timeout(Duration::from_millis(TIMEOUT_MS), checked).await {
        Ok(outcome) => outcome,
        Err(_) => {
            stop.cancel();
            failed(format!("Check timed out after {TIMEOUT_MS}ms"))
        }
    }
}

fn capped(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_OUTPUT_BYTES)]).into_owned();
    text.strip_suffix('\u{FFFD}')
        .map(str::to_string)
        .unwrap_or(text)
}

async fn run_shell(ctx: &ToolContext, condition: &Value, state: &Value) -> Outcome {
    let run = condition["run"].as_str().unwrap_or_default();
    let (shell, target) = shell_target(ctx);
    if let Err(error) = crate::tool::shell::authorize(ctx, run, &shell, &target).await {
        return failed(error.message);
    }
    let mut command = tokio::process::Command::new(&shell);
    command
        .args(crate::tool::shell::select::args(&shell, run))
        .current_dir(&target.absolute)
        .env_clear()
        .envs(ctx.env.iter().cloned())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let output = match command.output().await {
        Ok(output) => output,
        Err(error) => return failed(error.to_string()),
    };
    let (stdout, stderr) = (capped(&output.stdout), capped(&output.stderr));
    let code = output.status.code().unwrap_or(-1);
    let trimmed = stdout.trim();
    let message = (!trimmed.is_empty()).then(|| trimmed.to_string());
    let woken = |wake: bool, state: Value| Outcome::Checked {
        wake,
        state,
        message: message.clone().filter(|_| wake),
    };
    let predicate = condition.get("is").is_some()
        || condition.get("matches").is_some()
        || condition["changes"] == true;
    if !predicate {
        return woken(code == 0, Value::Null);
    }
    if code != 0 {
        let detail = Some(stderr.trim())
            .filter(|d| !d.is_empty())
            .unwrap_or(trimmed);
        return failed(if detail.is_empty() {
            format!("Command exited with code {code}")
        } else {
            format!("Command exited with code {code}: {detail}")
        });
    }
    if let Some(expected) = condition["is"].as_str() {
        return woken(trimmed == expected.trim(), Value::Null);
    }
    if let Some(pattern) = condition["matches"].as_str() {
        let matched = fancy_regex::Regex::new(pattern)
            .ok()
            .and_then(|r| r.is_match(&stdout).ok())
            .unwrap_or(false);
        return woken(matched, Value::Null);
    }
    let previous = state["stdout"].as_str();
    woken(
        previous.is_some_and(|p| p != stdout),
        json!({"stdout": stdout}),
    )
}

/// Counts a program's tool calls against the check's cap.
struct Counted {
    tool: Arc<dyn NamespaceTool>,
    left: Arc<AtomicUsize>,
}

impl NamespaceTool for Counted {
    fn name(&self) -> &str {
        self.tool.name()
    }

    fn description(&self) -> &str {
        self.tool.description()
    }

    fn input_schema(&self) -> &Value {
        self.tool.input_schema()
    }

    fn output_schema(&self) -> Option<&Value> {
        self.tool.output_schema()
    }

    fn permission(&self) -> Option<&str> {
        self.tool.permission()
    }

    fn call<'a>(
        &'a self,
        input: Value,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<Called, ToolError>> {
        let spent = self
            .left
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_err();
        if spent {
            let message = format!("A schedule check can make at most {MAX_TOOL_CALLS} tool calls");
            return futures_util::future::ready(Err(ToolError::execution(message))).boxed();
        }
        self.tool.call(input, ctx)
    }
}

fn invalid_error(message: &str) -> bool {
    [
        "needs approval",
        "Unknown tool",
        "SyntaxError",
        "is not callable",
        "Invalid input",
        "expects at most",
    ]
    .iter()
    .any(|m| message.contains(m))
}

async fn run_code(ctx: &mut ToolContext, condition: &Value, state: &Value, run: &Run) -> Outcome {
    let code = condition["code"].as_str().unwrap_or_default();
    let paths = match scan_tools(code) {
        Ok(paths) => paths,
        Err(error) => {
            return Outcome::Failed {
                error,
                invalid: true,
            };
        }
    };
    let (inventory, actions) = match resolve(ctx, &paths) {
        Ok(resolved) => resolved,
        Err(error) => return failed(error),
    };
    let mut approved = vec!["execute".to_string()];
    approved.extend(actions);
    let gate = Unattended {
        inner: ctx.permissions.clone(),
        approved,
    };
    if let Some(action) = gate
        .approved
        .iter()
        .find(|a| gate.denies(a, &["*".to_string()]))
    {
        return failed(format!("Permission denied: {action}"));
    }
    ctx.permissions = Arc::new(gate);
    let left = Arc::new(AtomicUsize::new(MAX_TOOL_CALLS));
    let counted = Inventory::new(
        inventory
            .namespaces()
            .iter()
            .map(|n| {
                Arc::new(Namespace {
                    name: n.name.clone(),
                    description: n.description.clone(),
                    direct: false,
                    tools: n
                        .tools
                        .iter()
                        .map(|tool| {
                            Arc::new(Counted {
                                tool: tool.clone(),
                                left: left.clone(),
                            }) as Arc<dyn NamespaceTool>
                        })
                        .collect(),
                })
            })
            .collect(),
    );
    let bindings = format!(
        "const state = {state}; const now = {now}; const runs = {runs}; const input = {input};\n",
        now = run.now,
        runs = run.checks,
        input = json!({"state": state, "now": run.now, "runs": run.checks}),
    );
    let result = crate::codemode::execute::run(&counted, format!("{bindings}{code}"), ctx).await;
    let output = match result {
        Ok(output) => output,
        Err(error) => return failed(error.message),
    };
    let text = output
        .content
        .iter()
        .find_map(|c| match c {
            crate::tool::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let text = text
        .split("\n\nLogs:\n")
        .next()
        .unwrap_or_default()
        .to_string();
    if output.metadata.get("error") == Some(&Value::Bool(true)) {
        let invalid = invalid_error(&text);
        return Outcome::Failed {
            error: text,
            invalid,
        };
    }
    if text.len() > MAX_OUTPUT_BYTES {
        return failed(format!("Check output exceeded {MAX_OUTPUT_BYTES} bytes"));
    }
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if let Value::Bool(wake) = value {
        return Outcome::Checked {
            wake,
            state: state.clone(),
            message: None,
        };
    }
    let shaped = value["wake"]
        .as_bool()
        .filter(|_| value.is_object() && value.get("message").is_none_or(Value::is_string));
    let Some(wake) = shaped else {
        return Outcome::Failed {
            error: format!(
                "A code check must return a boolean or {{ wake, state?, message? }}: {text}"
            ),
            invalid: true,
        };
    };
    let state = value.get("state").cloned().unwrap_or_else(|| state.clone());
    let message = value["message"]
        .as_str()
        .map(str::trim)
        .filter(|m| !m.is_empty() && wake)
        .map(str::to_string);
    Outcome::Checked {
        wake,
        state,
        message,
    }
}

/// A tool context for running `session`'s checks from the firing loop.
pub(crate) fn unattended(
    shared: &Arc<Shared>,
    session: &SessionRow,
) -> Result<ToolContext, String> {
    let selection = crate::session::select(shared, session).map_err(|e| e.message)?;
    let mut configured = selection.agent.permissions.clone();
    configured.extend(
        shared
            .db
            .session_permissions(&session.id)
            .unwrap_or_default(),
    );
    let gate = Arc::new(RuleGate {
        session_id: session.id.clone(),
        scope: Arc::new(Scope {
            configured,
            project_id: shared.project.id.clone(),
            policies: Arc::new(selection.sources.config.policies()),
        }),
        asks: shared.asks.clone(),
    });
    Ok(ToolContext {
        session_id: session.id.clone(),
        message_id: String::new(),
        call_id: String::new(),
        agent: selection.agent.id.clone(),
        directory: session.directory.clone().into(),
        worktree: shared.project.worktree.clone(),
        project_id: shared.project.id.clone(),
        paths: shared.paths.clone(),
        permissions: gate,
        cancel: tokio_util::sync::CancellationToken::new(),
        env: shared.env.clone(),
        configured_shell: selection.sources.config.shell().map(str::to_string),
        limits: selection.sources.config.tool_output(),
        locks: shared.locks.clone(),
        notifier: shared.notifier.clone(),
        web_search: Arc::new(crate::search::GatewaySearch::new(
            shared.gateway.clone(),
            shared.credits.clone(),
        )),
        host: shared.clone(),
        session: Arc::new(crate::session::host::Host {
            shared: shared.clone(),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_are_scanned_statically() {
        assert_eq!(
            scan_tools("// tools.x.y()\nconst t = await tools.slack.read_thread({ id });\nreturn tools[\"my-srv\"].get({})").unwrap(),
            ["slack.read_thread", "my-srv.get"]
        );
        assert!(scan_tools("const f = tools.slack; return f.read({})").is_err());
        assert_eq!(
            validate(
                &json!({"type": "code", "code": "await tools.opencode.schedule_delete({id})"})
            )
            .unwrap_err(),
            "Tool opencode.schedule_delete cannot be used inside a schedule check"
        );
        assert_eq!(
            validate(&json!({"type": "shell", "run": "x", "is": "a", "changes": true}))
                .unwrap_err(),
            "Choose at most one of is, matches or changes"
        );
        assert_eq!(
            describe(&json!({"type": "code", "code": "\n  return true\n"})),
            "code (return true)"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shell_checks_wake_on_their_predicates() {
        let dir = tempfile::tempdir().unwrap();
        let test = crate::tool::test_support::context(dir.path());
        let run = Run {
            message_id: "msg_x".into(),
            now: 0,
            checks: 0,
        };
        let check = |condition: Value| json!({"mode": "if", "condition": condition});
        let outcome = evaluate(
            &test.ctx,
            &check(json!({"type": "shell", "run": "echo ready"})),
            &Value::Null,
            &run,
        )
        .await;
        assert_eq!(
            outcome,
            Outcome::Checked {
                wake: true,
                state: Value::Null,
                message: Some("ready".into())
            }
        );
        let changes = check(json!({"type": "shell", "run": "echo a", "changes": true}));
        let first = evaluate(&test.ctx, &changes, &Value::Null, &run).await;
        assert_eq!(
            first,
            Outcome::Checked {
                wake: false,
                state: json!({"stdout": "a\n"}),
                message: None
            }
        );
        let changed = evaluate(&test.ctx, &changes, &json!({"stdout": "b\n"}), &run).await;
        assert!(matches!(changed, Outcome::Checked { wake: true, .. }));
        let failing = check(json!({"type": "shell", "run": "echo oops >&2; exit 3", "is": "x"}));
        assert_eq!(
            evaluate(&test.ctx, &failing, &Value::Null, &run).await,
            Outcome::Failed {
                error: "Command exited with code 3: oops".into(),
                invalid: false
            }
        );
        // What was approved at creation runs although the rules alone would ask.
        *test.gate.answer.lock().unwrap() = Some(PermissionError::Declined);
        let refused = evaluate(
            &test.ctx,
            &check(json!({"type": "shell", "run": "true"})),
            &Value::Null,
            &run,
        )
        .await;
        assert!(
            matches!(refused, Outcome::Checked { wake: true, .. }),
            "approved actions run: {refused:?}"
        );
    }
}
