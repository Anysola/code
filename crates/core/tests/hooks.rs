//! Hooks: commands the user configures run around tool calls, prompts, stops and session starts,
//! and can block, add context or keep a turn going, never widen what the agent may do.
#![cfg(unix)]

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anysola_core::api::{Core, Prompt, Target};
use anysola_core::codemode::{Called, Namespace, NamespaceTool};
use anysola_core::events::Event;
use anysola_core::hooks::{HookOutcome, HookScope};
use anysola_core::session::gate::Reply as Answer;
use anysola_core::session::inbox::Delivery;
use anysola_core::session::message::{Message, Part, ToolState};
use anysola_core::session::{Outcome, TurnOptions};
use anysola_core::tool::{ToolContext, ToolError};
use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Recorded, Reply};
use common::*;
use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

fn prompt(text: &str) -> Prompt {
    Prompt {
        text: text.into(),
        ..Default::default()
    }
}

fn last(request: &Recorded) -> String {
    request.json()["messages"]
        .as_array()
        .and_then(|m| m.last())
        .map(Value::to_string)
        .unwrap_or_default()
}

fn call(request: &Recorded, name: &str, input: Value) -> Reply {
    let model = request.json()["model"].as_str().unwrap().to_string();
    Reply::Sse {
        body: Stream::new()
            .tool("toolu_1", name, input)
            .finish(&model, Some(1)),
    }
}

/// A model that makes one tool call per prompt, then answers `done`.
fn calls_once(name: &'static str, input: Value) -> FakeGateway {
    FakeGateway::start(move |request, _| {
        if request.is_title() || last(request).contains("tool_result") {
            return text(request, "done");
        }
        call(request, name, input.clone())
    })
}

fn model_requests(gateway: &FakeGateway) -> Vec<Recorded> {
    gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect()
}

/// `{"<event>": [{"matcher": …, "hooks": [{"type": "command", "command": …}]}]}`.
fn hooks(entries: &[(&str, Option<&str>, &str)]) -> Value {
    let mut out = serde_json::Map::new();
    for (event, matcher, command) in entries {
        let mut group = json!({"hooks": [{"type": "command", "command": command}]});
        if let Some(matcher) = matcher {
            group["matcher"] = json!(matcher);
        }
        out.entry(event.to_string())
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .unwrap()
            .push(group);
    }
    Value::Object(out)
}

fn user_config(env: &Env, config: Value) {
    env.home.write(".anysola/config.json", &config.to_string());
}

fn quote(path: &Path) -> String {
    format!("'{}'", path.display())
}

/// The stored tool parts of the session, in order.
fn tool_parts(core: &Core, session_id: &str) -> Vec<(String, ToolState)> {
    core.db()
        .messages(session_id)
        .unwrap()
        .into_iter()
        .filter_map(|row| match Message::decode(&row.kind, &row.data) {
            Message::Assistant(step) => Some(step.content),
            _ => None,
        })
        .flatten()
        .filter_map(|part| match part {
            Part::Tool { name, state, .. } => Some((name, state)),
            _ => None,
        })
        .collect()
}

fn hook_entries(state: &ToolState) -> Vec<Value> {
    let metadata = match state {
        ToolState::Completed { metadata, .. } | ToolState::Error { metadata, .. } => metadata,
        _ => return Vec::new(),
    };
    metadata
        .get("hooks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn drain_events(events: &mut tokio::sync::broadcast::Receiver<Event>) -> Vec<Event> {
    let mut out = Vec::new();
    while let Ok(event) = events.try_recv() {
        out.push(event);
    }
    out
}

#[test]
fn pre_tool_use_exit_two_blocks_a_shell_call() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() || last(request).contains("tool_result") {
            return text(request, "done");
        }
        call(request, "shell", json!({"command": "touch made"}))
    });
    let env = env(&gateway.url);
    let payload = env.home.home().join("payload.json");
    user_config(
        &env,
        json!({"hooks": hooks(&[(
            "PreToolUse",
            Some("Bash"),
            &format!("cat > {}; echo 'no touching here' >&2; exit 2", quote(&payload)),
        )])}),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let mut events = core.subscribe();
    let (session, _) = core.open(&Target::New, false).unwrap();
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert_eq!(outcome, Outcome::Succeeded);
    assert!(
        !env.home.project().join("made").exists(),
        "the command did not run"
    );
    let requests = model_requests(&gateway);
    assert_eq!(requests.len(), 2);
    assert!(
        last(&requests[1]).contains("no touching here"),
        "{}",
        last(&requests[1])
    );
    let sent = read_json(&payload);
    assert_eq!(sent["hook_event_name"], "PreToolUse");
    assert_eq!(sent["tool_name"], "Bash");
    assert_eq!(sent["anysola_tool"], "shell");
    assert_eq!(sent["tool_input"]["command"], "touch made");
    assert_eq!(sent["tool_use_id"], "toolu_1");
    assert_eq!(sent["session_id"], session.id.as_str());
    assert_eq!(sent["cwd"], env.home.project().display().to_string());
    assert_eq!(sent["permission_mode"], "default");
    assert_eq!(sent["agent"], "build");
    assert_eq!(sent["transcript_path"], "");
    assert!(sent.get("agent_id").is_none());
    let parts = tool_parts(&core, &session.id);
    let entries = hook_entries(&parts[0].1);
    assert!(matches!(parts[0].1, ToolState::Error { .. }));
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0]["event"], "PreToolUse");
    assert_eq!(entries[0]["outcome"], "blocked");
    assert_eq!(entries[0]["message"], "no touching here");
    let hook_events: Vec<Event> = drain_events(&mut events)
        .into_iter()
        .filter(|e| matches!(e, Event::Hook { .. }))
        .collect();
    assert!(
        matches!(&hook_events[..], [Event::Hook { outcome: HookOutcome::Blocked, event, .. }] if event == "PreToolUse"),
        "{hook_events:?}"
    );
}

/// A Code Mode namespace standing in for an MCP server: `tools.fake.touch()` creates a file.
struct Touch(PathBuf);

impl NamespaceTool for Touch {
    fn name(&self) -> &str {
        "touch"
    }

    fn description(&self) -> &str {
        "Creates the marker file."
    }

    fn input_schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| json!({"type": "object", "properties": {}}))
    }

    fn call<'a>(
        &'a self,
        _: Value,
        _: &'a ToolContext,
    ) -> BoxFuture<'a, Result<Called, ToolError>> {
        async move {
            std::fs::write(&self.0, "x").unwrap();
            Ok(Called::output(json!("touched")))
        }
        .boxed()
    }
}

#[test]
fn pre_tool_use_blocks_calls_from_a_code_mode_program() {
    let gateway = calls_once(
        "execute",
        json!({"code": "return await tools.fake.touch({})"}),
    );
    let env = env(&gateway.url);
    let marker = env.home.home().join("touched");
    let execute_ran = env.home.home().join("execute-hook-ran");
    user_config(
        &env,
        json!({"hooks": hooks(&[
            ("PreToolUse", Some("mcp__fake__touch"), "echo 'not from a program' >&2; exit 2"),
            ("PreToolUse", Some("execute"), &format!("touch {}", quote(&execute_ran))),
        ])}),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    core.set_namespace(Arc::new(Namespace {
        name: "fake".into(),
        description: None,
        direct: false,
        tools: vec![Arc::new(Touch(marker.clone()))],
    }));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert!(!marker.exists(), "the program's call did not run");
    assert!(execute_ran.exists(), "`execute` fires its own hooks");
    let requests = model_requests(&gateway);
    assert!(last(&requests[1]).contains("not from a program"));
    let parts = tool_parts(&core, &session.id);
    let entries = hook_entries(&parts[0].1);
    assert!(
        entries
            .iter()
            .any(|e| e["outcome"] == "blocked" && e["message"] == "not from a program"),
        "{entries:?}"
    );
}

#[test]
fn pre_tool_use_blocks_calls_from_a_subagent() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        let child = request.header("x-opencode-parent-session-id").is_some();
        let answered = last(request).contains("tool_result");
        match (child, answered) {
            (true, false) => call(request, "shell", json!({"command": "touch child-made"})),
            (false, false) => call(
                request,
                "subagent",
                json!({"agent": "general", "description": "Touch", "prompt": "touch it"}),
            ),
            _ => text(request, "done"),
        }
    });
    let env = env(&gateway.url);
    let payload = env.home.home().join("payload.json");
    user_config(
        &env,
        json!({"hooks": hooks(&[(
            "PreToolUse",
            Some("Bash"),
            &format!("cat > {}; echo 'not in a subagent either' >&2; exit 2", quote(&payload)),
        )])}),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert!(!env.home.project().join("child-made").exists());
    let child_answer = model_requests(&gateway)
        .into_iter()
        .find(|r| {
            r.header("x-opencode-parent-session-id").is_some() && last(r).contains("tool_result")
        })
        .expect("the child got its tool result");
    assert!(last(&child_answer).contains("not in a subagent either"));
    let sent = read_json(&payload);
    assert_eq!(sent["session_id"], session.id.as_str(), "the root session");
    assert_eq!(sent["agent_type"], "general");
    let child = core.children(&session.id).unwrap();
    assert_eq!(sent["agent_id"], child[0].0.as_str());
}

#[test]
fn a_json_deny_blocks_and_allow_never_widens() {
    let allow = r#"echo '{"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "allow"}}'"#;
    let deny = r#"echo '{"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": "denied by json"}}'"#;
    // A JSON deny blocks.
    {
        let gateway = calls_once("shell", json!({"command": "touch made"}));
        let env = env(&gateway.url);
        user_config(
            &env,
            json!({"hooks": hooks(&[("PreToolUse", Some("Bash"), deny)])}),
        );
        let runtime = runtime();
        let core = runtime.block_on(common::core(&env));
        let (session, _) = core.open(&Target::New, false).unwrap();
        runtime
            .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
            .unwrap();
        assert!(!env.home.project().join("made").exists());
        assert!(last(&model_requests(&gateway)[1]).contains("denied by json"));
    }
    // "allow" does not lift a deny rule, nor a policy's denial.
    for extra in [
        json!({"permissions": [{"action": "shell", "resource": "*", "effect": "deny"}]}),
        json!({"policies": [{"action": "tool.use", "resource": "shell:*", "effect": "deny"}]}),
    ] {
        let gateway = calls_once("shell", json!({"command": "touch made"}));
        let env = env(&gateway.url);
        let mut config = extra.clone();
        config["hooks"] = hooks(&[("PreToolUse", Some("Bash"), allow)]);
        user_config(&env, config);
        let runtime = runtime();
        let core = runtime.block_on(common::core(&env));
        let (session, _) = core.open(&Target::New, false).unwrap();
        runtime
            .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
            .unwrap();
        assert!(!env.home.project().join("made").exists(), "{extra}");
    }
    // "allow" does not skip an ask; "ask" asks where the rules allow.
    for (rules, hook) in [
        (
            json!([{"action": "shell", "resource": "*", "effect": "ask"}]),
            allow.to_string(),
        ),
        (
            json!([{"action": "shell", "resource": "*", "effect": "allow"}]),
            r#"echo '{"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "ask"}}'"#.to_string(),
        ),
    ] {
        let gateway = calls_once("shell", json!({"command": "touch made"}));
        let env = env(&gateway.url);
        user_config(
            &env,
            json!({"permissions": rules, "hooks": hooks(&[("PreToolUse", Some("Bash"), &hook)])}),
        );
        let runtime = runtime();
        let core = runtime.block_on(common::core(&env));
        let (session, _) = core.open(&Target::New, false).unwrap();
        let mut events = core.subscribe();
        let answering = core.clone();
        let asked = runtime.spawn(async move {
            loop {
                if let Ok(Event::PermissionAsked {
                    request_id, action, ..
                }) = events.recv().await
                {
                    answering.reply_permission(&request_id, Answer::Reject(Some("no".into())));
                    return action;
                }
            }
        });
        runtime
            .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
            .unwrap();
        let action = runtime
            .block_on(async { tokio::time::timeout(Duration::from_secs(5), asked).await })
            .expect("the user was asked")
            .unwrap();
        assert_eq!(action, "shell");
        assert!(!env.home.project().join("made").exists());
    }
}

#[test]
fn post_tool_use_feedback_and_context_reach_the_model_and_the_part() {
    let gateway = calls_once("shell", json!({"command": "echo hello-output"}));
    let env = env(&gateway.url);
    let payload = env.home.home().join("post.json");
    let context = r#"echo '{"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": "formatted 1 file"}}'"#;
    user_config(
        &env,
        json!({"hooks": hooks(&[
            ("PostToolUse", Some("Bash"), &format!("cat > {}; echo 'lint failed' >&2; exit 2", quote(&payload))),
            ("PostToolUse", Some("Bash"), context),
        ])}),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    let result = last(&model_requests(&gateway)[1]);
    assert!(result.contains("hello-output"), "{result}");
    assert!(
        result.contains("PostToolUse hook feedback: lint failed"),
        "{result}"
    );
    assert!(
        result.contains("PostToolUse hook context: formatted 1 file"),
        "{result}"
    );
    let sent = read_json(&payload);
    assert!(
        sent["tool_response"]["output"]
            .as_str()
            .unwrap()
            .contains("hello-output")
    );
    assert!(sent["duration_ms"].is_u64());
    let parts = tool_parts(&core, &session.id);
    assert!(matches!(parts[0].1, ToolState::Completed { .. }));
    let outcomes: Vec<Value> = hook_entries(&parts[0].1)
        .iter()
        .map(|e| e["outcome"].clone())
        .collect();
    assert_eq!(outcomes, [json!("feedback"), json!("context")]);
}

#[test]
fn user_prompt_submit_adds_context_or_blocks_only_that_prompt() {
    let gateway = FakeGateway::start(|request, _| text(request, "answered"));
    let env = env(&gateway.url);
    let block = r#"if grep -q secret; then echo 'no secrets in prompts' >&2; exit 2; fi; echo 'ctx-from-hook'"#;
    user_config(
        &env,
        json!({"hooks": hooks(&[("UserPromptSubmit", None, block)])}),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("hello"), TurnOptions::default()))
        .unwrap();
    let lines = transcript(&model_requests(&gateway)[0]);
    let at = lines.iter().position(|l| l == "user: hello").unwrap();
    assert!(
        lines[at..].iter().any(|l| l.contains("ctx-from-hook")),
        "{lines:?}"
    );
    let synthetic = core
        .db()
        .messages(&session.id)
        .unwrap()
        .into_iter()
        .filter(|r| r.kind == "synthetic" && r.data["description"] == "Hook context")
        .count();
    assert_eq!(synthetic, 1);

    let later = core
        .enqueue(&session.id, prompt("later"), Delivery::Queue)
        .unwrap();
    let mut events = core.subscribe();
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("my secret"), TurnOptions::default()))
        .unwrap();
    let Outcome::Failed(error) = outcome else {
        panic!("blocked: {outcome:?}")
    };
    assert_eq!(error.kind, "hook.blocked");
    assert_eq!(error.message, "no secrets in prompts");
    assert!(drain_events(&mut events).iter().any(|e| matches!(
        e,
        Event::Failed { unsent: true, error, .. } if error.kind == "hook.blocked"
    )));
    let texts: Vec<String> = core
        .db()
        .messages(&session.id)
        .unwrap()
        .iter()
        .filter(|r| r.kind == "user")
        .map(|r| r.data["text"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        texts,
        ["hello"],
        "the blocked prompt is not in the conversation"
    );
    let pending = core.pending(&session.id).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, later, "the queued prompt is still queued");
    assert_eq!(model_requests(&gateway).len(), 1);
}

#[test]
fn stop_continues_once_with_the_reason() {
    let gateway = FakeGateway::start(|request, _| text(request, "finished"));
    let env = env(&gateway.url);
    let seen = env.home.home().join("seen");
    let count = env.home.home().join("count");
    let command = format!(
        "grep -o '\"stop_hook_active\":[a-z]*' | cut -d: -f2 >> {seen}; \
         if [ ! -f {count} ]; then touch {count}; echo 'tests are failing' >&2; exit 2; fi",
        seen = quote(&seen),
        count = quote(&count),
    );
    user_config(&env, json!({"hooks": hooks(&[("Stop", None, &command)])}));
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert_eq!(outcome, Outcome::Succeeded);
    let requests = model_requests(&gateway);
    assert_eq!(requests.len(), 2);
    assert!(last(&requests[1]).contains("tests are failing"));
    let seen = std::fs::read_to_string(&seen).unwrap();
    assert_eq!(
        seen.split_whitespace().collect::<Vec<_>>(),
        ["false", "true"]
    );
    let stored = core
        .db()
        .messages(&session.id)
        .unwrap()
        .into_iter()
        .filter(|r| r.kind == "synthetic" && r.data["description"] == "Stop hook")
        .count();
    assert_eq!(stored, 1);
}

#[test]
fn stop_hooks_that_always_block_are_capped() {
    let gateway = FakeGateway::start(|request, _| text(request, "finished"));
    let env = env(&gateway.url);
    user_config(
        &env,
        json!({"hooks": hooks(&[("Stop", None, "echo 'again' >&2; exit 2")])}),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let mut events = core.subscribe();
    let (session, _) = core.open(&Target::New, false).unwrap();
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert_eq!(outcome, Outcome::Succeeded);
    assert_eq!(
        model_requests(&gateway).len(),
        9,
        "the prompt and 8 continuations"
    );
    assert!(drain_events(&mut events).iter().any(|e| matches!(
        e,
        Event::Hook { outcome: HookOutcome::Notice, event, message, .. }
            if event == "Stop" && message.contains("8 times")
    )));
}

#[test]
fn stop_does_not_fire_for_failed_turns_or_subagents() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if last(request).contains("fail please") {
            return Reply::Json {
                status: 400,
                body: json!({"type": "error", "error": {"type": "invalid_request_error", "message": "bad"}}),
                headers: Vec::new(),
            };
        }
        let child = request.header("x-opencode-parent-session-id").is_some();
        if !child && !last(request).contains("tool_result") {
            return call(
                request,
                "subagent",
                json!({"agent": "general", "description": "Look", "prompt": "look"}),
            );
        }
        text(request, "done")
    });
    let env = env(&gateway.url);
    let fired = env.home.home().join("fired");
    user_config(
        &env,
        json!({"hooks": hooks(&[("Stop", None, &format!("grep -o '\"session_id\":\"[^\"]*\"' >> {}", quote(&fired)))])}),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("fail please"), TurnOptions::default()))
        .unwrap();
    assert!(matches!(outcome, Outcome::Failed(_)), "{outcome:?}");
    assert!(!fired.exists(), "a failed turn fires no Stop hook");
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert_eq!(core.children(&session.id).unwrap().len(), 1);
    let lines = std::fs::read_to_string(&fired).unwrap();
    assert_eq!(lines.lines().count(), 1, "only the root session: {lines}");
}

#[test]
fn stop_does_not_fire_for_an_interrupted_turn() {
    let gateway = calls_once("shell", json!({"command": "sleep 20"}));
    let env = env(&gateway.url);
    let fired = env.home.home().join("fired");
    user_config(
        &env,
        json!({"hooks": hooks(&[("Stop", None, &format!("touch {}", quote(&fired)))])}),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let interrupter = {
        let (core, id) = (core.clone(), session.id.clone());
        let mut events = core.subscribe();
        runtime.spawn(async move {
            while let Ok(event) = events.recv().await {
                if matches!(event, Event::ToolCalled { .. }) {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    core.interrupt(&id);
                    return;
                }
            }
        })
    };
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    runtime.block_on(interrupter).unwrap();
    assert!(matches!(outcome, Outcome::Interrupted(_)), "{outcome:?}");
    assert!(!fired.exists());
}

#[test]
fn session_start_is_startup_then_resume_and_adds_context() {
    let gateway = FakeGateway::start(|request, _| text(request, "hi"));
    let env = env(&gateway.url);
    let sources = env.home.home().join("sources");
    let command = format!(
        "grep -o '\"source\":\"[a-z]*\"' >> {}; echo 'start-context'",
        quote(&sources)
    );
    user_config(
        &env,
        json!({"hooks": hooks(&[("SessionStart", None, &command)])}),
    );
    let runtime = runtime();
    let session_id = {
        let core = runtime.block_on(common::core(&env));
        let (session, _) = core.open(&Target::New, false).unwrap();
        runtime
            .block_on(core.prompt(&session.id, prompt("one"), TurnOptions::default()))
            .unwrap();
        runtime
            .block_on(core.prompt(&session.id, prompt("two"), TurnOptions::default()))
            .unwrap();
        session.id
    };
    let first = transcript(&model_requests(&gateway)[0]);
    assert!(
        first.iter().any(|l| l.contains("start-context")),
        "{first:?}"
    );
    let core = runtime.block_on(common::core(&env));
    runtime
        .block_on(core.prompt(&session_id, prompt("three"), TurnOptions::default()))
        .unwrap();
    let sources = std::fs::read_to_string(&sources).unwrap();
    assert_eq!(
        sources.lines().collect::<Vec<_>>(),
        ["\"source\":\"startup\"", "\"source\":\"resume\""],
        "once per session and process"
    );
}

#[test]
fn project_hooks_need_approval_for_their_content() {
    let gateway = calls_once("shell", json!({"command": "true"}));
    let env = env(&gateway.url);
    let marker = env.home.home().join("project-hook-ran");
    let user_marker = env.home.home().join("user-hook-ran");
    let file = env.home.write(
        "proj/anysola.json",
        &json!({"hooks": hooks(&[("PreToolUse", Some("Bash"), &format!("touch {}", quote(&marker)))])})
            .to_string(),
    );
    user_config(
        &env,
        json!({"hooks": hooks(&[("PreToolUse", Some("Bash"), &format!("touch {}", quote(&user_marker)))])}),
    );
    // Neither the gateway's document nor ~/.claude contributes hooks.
    let mut remote = anysola_testkit::default_config(&gateway.url);
    remote["hooks"] = hooks(&[("Stop", None, "touch remote")]);
    gateway.set_config(remote);
    env.home.write(
        ".claude/settings.json",
        &json!({"hooks": hooks(&[("Stop", None, "touch claude")])}).to_string(),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let listed = core.hooks();
    assert_eq!(listed.len(), 2, "{listed:?}");
    assert_eq!(
        listed
            .iter()
            .map(|h| (h.scope, h.active))
            .collect::<Vec<_>>(),
        [(HookScope::User, true), (HookScope::Project, false)]
    );
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert!(user_marker.exists(), "user hooks run without approval");
    assert!(!marker.exists(), "an unapproved project hook never runs");
    let documents = core.hook_documents();
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].path, file.display().to_string());
    assert_eq!(documents[0].decision, None);
    let mut events = core.subscribe();
    core.trust_hooks(&documents[0].path, &documents[0].digest, true)
        .unwrap();
    assert!(drain_events(&mut events).contains(&Event::HooksChanged));
    runtime
        .block_on(core.prompt(&session.id, prompt("again"), TurnOptions::default()))
        .unwrap();
    assert!(marker.exists(), "approved");
    // Another command: undecided again.
    std::fs::remove_file(&marker).unwrap();
    std::fs::write(
        &file,
        json!({"hooks": hooks(&[("PreToolUse", Some("Bash"), &format!("touch {}; true", quote(&marker)))])})
            .to_string(),
    )
    .unwrap();
    runtime.block_on(core.reload());
    assert!(drain_events(&mut events).contains(&Event::HooksChanged));
    assert_eq!(core.hook_documents()[0].decision, None);
    runtime
        .block_on(core.prompt(&session.id, prompt("third"), TurnOptions::default()))
        .unwrap();
    assert!(
        marker.exists(),
        "the approved version runs until the change is reviewed"
    );
    // Another process runs nothing for undecided content.
    drop(core);
    std::fs::remove_file(&marker).unwrap();
    let core = runtime.block_on(common::core(&env));
    runtime
        .block_on(core.prompt(&session.id, prompt("fourth"), TurnOptions::default()))
        .unwrap();
    assert!(!marker.exists());
}

#[test]
fn switches_turn_off_user_and_project_hooks() {
    let gateway = calls_once("shell", json!({"command": "true"}));
    let env = env(&gateway.url);
    let hook = hooks(&[("PreToolUse", Some("Bash"), "true")]);
    user_config(&env, json!({"hooks": hook, "disable_hooks": true}));
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    assert!(core.hooks().iter().all(|h| !h.active));
    drop(core);
    user_config(&env, json!({"hooks": hook}));
    // SAFETY: tests in this binary hold the environment lock while they run.
    unsafe { std::env::set_var("ANYSOLA_DISABLE_HOOKS", "1") };
    let core = runtime.block_on(common::core(&env));
    let off = core.hooks().iter().all(|h| !h.active);
    unsafe { std::env::remove_var("ANYSOLA_DISABLE_HOOKS") };
    assert!(off);
    // A project document can switch off project hooks, never the user's.
    env.home
        .write("proj/anysola.json", r#"{"disable_hooks": true}"#);
    let core = runtime.block_on(common::core(&env));
    assert!(core.hooks().iter().all(|h| h.active));
}

#[test]
fn timeouts_and_missing_commands_are_non_blocking_errors() {
    let gateway = calls_once("shell", json!({"command": "touch made"}));
    let env = env(&gateway.url);
    let config = json!({"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [
        {"type": "command", "command": "sleep 10", "timeout": 1},
        {"type": "command", "command": "/no/such/hook-command"},
    ]}]}});
    user_config(&env, config);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let mut events = core.subscribe();
    let (session, _) = core.open(&Target::New, false).unwrap();
    let started = Instant::now();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(8));
    assert!(env.home.project().join("made").exists(), "the call went on");
    let errors: Vec<String> = drain_events(&mut events)
        .into_iter()
        .filter_map(|e| match e {
            Event::Hook {
                outcome: HookOutcome::Error,
                message,
                ..
            } => Some(message),
            _ => None,
        })
        .collect();
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors.iter().any(|m| m.contains("timed out")));
    assert!(errors.iter().any(|m| m.contains("127")));
}

#[test]
fn interrupting_the_turn_kills_a_running_hook() {
    let gateway = calls_once("shell", json!({"command": "true"}));
    let env = env(&gateway.url);
    let pid_file = env.home.home().join("hook.pid");
    user_config(
        &env,
        json!({"hooks": hooks(&[("PreToolUse", Some("Bash"), &format!("echo $$ > {}; sleep 30", quote(&pid_file)))])}),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let interrupter = {
        let (core, id, pid_file) = (core.clone(), session.id.clone(), pid_file.clone());
        std::thread::spawn(move || {
            wait_for("the hook to start", || {
                std::fs::read_to_string(&pid_file).is_ok_and(|p| p.trim().parse::<i32>().is_ok())
            });
            core.interrupt(&id);
        })
    };
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    interrupter.join().unwrap();
    assert!(matches!(outcome, Outcome::Interrupted(_)), "{outcome:?}");
    let pid: i32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // SAFETY: signal 0 only checks whether the process group exists.
    wait_for("the hook's process group to die", || unsafe {
        libc::kill(-pid, 0) != 0
    });
}

#[test]
fn the_same_command_runs_once_and_hooks_run_concurrently() {
    let gateway = calls_once("shell", json!({"command": "true"}));
    let env = env(&gateway.url);
    let runs = env.home.home().join("runs");
    let once = format!("echo x >> {}", quote(&runs));
    let config = json!({"hooks": {"PreToolUse": [
        {"matcher": "Bash", "hooks": [{"type": "command", "command": once}]},
        {"matcher": "shell", "hooks": [{"type": "command", "command": once}]},
        {"hooks": [
            {"type": "command", "command": "sleep 1; true"},
            {"type": "command", "command": "sleep 1; :"},
            {"type": "command", "command": "sleep 1"},
        ]},
    ]}});
    user_config(&env, config);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let started = Instant::now();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(2900),
        "three one-second hooks ran side by side"
    );
    assert_eq!(std::fs::read_to_string(&runs).unwrap().lines().count(), 1);
}

#[test]
fn a_declined_call_still_ends_the_step_with_hooks_around_it() {
    let gateway = calls_once("shell", json!({"command": "touch made"}));
    let env = env(&gateway.url);
    let context = r#"echo '{"hookSpecificOutput": {"hookEventName": "PreToolUse", "additionalContext": "careful"}}'"#;
    user_config(
        &env,
        json!({
            "permissions": [{"action": "shell", "resource": "*", "effect": "ask"}],
            "hooks": hooks(&[("PreToolUse", Some("Bash"), context), ("PostToolUse", None, "echo 'seen' >&2; exit 2")]),
        }),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let mut events = core.subscribe();
    let answering = core.clone();
    runtime.spawn(async move {
        loop {
            if let Ok(Event::PermissionAsked { request_id, .. }) = events.recv().await {
                answering.reply_permission(&request_id, Answer::Reject(None));
                return;
            }
        }
    });
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert_eq!(
        outcome,
        Outcome::Interrupted(anysola_core::events::InterruptReason::Declined)
    );
    assert_eq!(model_requests(&gateway).len(), 1);
}

#[test]
fn continue_false_from_a_tool_hook_ends_the_turn_after_the_call() {
    let gateway = calls_once("shell", json!({"command": "touch made"}));
    let env = env(&gateway.url);
    let stop_ran = env.home.home().join("stop-ran");
    user_config(
        &env,
        json!({"hooks": hooks(&[
            ("PostToolUse", Some("Bash"), r#"echo '{"continue": false, "stopReason": "Paused by the build check"}'"#),
            ("Stop", None, &format!("touch {}", quote(&stop_ran))),
        ])}),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let mut events = core.subscribe();
    let (session, _) = core.open(&Target::New, false).unwrap();
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert_eq!(outcome, Outcome::Succeeded);
    assert!(env.home.project().join("made").exists(), "the call settled");
    assert_eq!(model_requests(&gateway).len(), 1, "no step after it");
    assert!(!stop_ran.exists(), "Stop hooks do not overrule it");
    assert!(drain_events(&mut events).iter().any(|e| matches!(
        e,
        Event::Hook { outcome: HookOutcome::Notice, message, .. } if message == "Paused by the build check"
    )));
}

#[test]
fn a_planted_file_path_does_not_hide_the_path_the_tool_writes() {
    let gateway = calls_once(
        "write",
        json!({"path": "protected/secret.txt", "file_path": "ok.txt", "content": "x"}),
    );
    let env = env(&gateway.url);
    let guard =
        r#"grep -q '"file_path":"[^"]*protected' && { echo 'protected path' >&2; exit 2; }; true"#;
    user_config(
        &env,
        json!({"hooks": hooks(&[("PreToolUse", Some("Write"), guard)])}),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert!(!env.home.project().join("protected/secret.txt").exists());
    assert!(last(&model_requests(&gateway)[1]).contains("protected path"));
}

#[test]
fn hooks_see_a_stringified_input_as_the_object_the_tool_runs() {
    let gateway = calls_once("shell", json!("{\"command\": \"touch made\"}"));
    let env = env(&gateway.url);
    let guard = r#"grep -q '"command":"touch made"' && { echo 'no touching' >&2; exit 2; }; true"#;
    user_config(
        &env,
        json!({"hooks": hooks(&[("PreToolUse", Some("Bash"), guard)])}),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert!(!env.home.project().join("made").exists());
    assert!(last(&model_requests(&gateway)[1]).contains("no touching"));
}

#[test]
fn a_moved_session_directory_does_not_stop_the_guards() {
    let gateway = calls_once("shell", json!({"command": "true"}));
    let env = env(&gateway.url);
    user_config(
        &env,
        json!({"hooks": hooks(&[("PreToolUse", Some("Bash"), "echo 'still guarded' >&2; exit 2")])}),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    std::fs::rename(env.home.project(), env.home.home().join("moved")).unwrap();
    let _ = runtime.block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()));
    let requests = model_requests(&gateway);
    assert!(requests.len() >= 2, "the call was answered");
    assert!(
        last(&requests[1]).contains("still guarded"),
        "{}",
        last(&requests[1])
    );
}

#[test]
fn a_weaker_copy_of_a_guard_does_not_weaken_it() {
    let gateway = calls_once("shell", json!({"command": "touch made"}));
    let env = env(&gateway.url);
    let guard = "sleep 2; echo 'slow guard' >&2; exit 2";
    // `ANYSOLA_CONFIG_CONTENT` is a user document that loads after the project's.
    let user = json!({"hooks": hooks(&[("PreToolUse", Some("Bash"), guard)])}).to_string();
    let project = json!({"hooks": {"PreToolUse": [{"matcher": "Bash",
        "hooks": [{"type": "command", "command": guard, "timeout": 1}]}]}});
    env.home.write("proj/anysola.json", &project.to_string());
    // SAFETY: tests in this binary hold the environment lock while they run.
    unsafe { std::env::set_var("ANYSOLA_CONFIG_CONTENT", &user) };
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let document = core.hook_documents().remove(0);
    core.trust_hooks(&document.path, &document.digest, true)
        .unwrap();
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    unsafe { std::env::remove_var("ANYSOLA_CONFIG_CONTENT") };
    assert!(!env.home.project().join("made").exists());
    assert!(last(&model_requests(&gateway)[1]).contains("slow guard"));
}

#[test]
fn approved_project_hooks_survive_an_edit_until_it_is_reviewed() {
    let gateway = calls_once("shell", json!({"command": "touch made"}));
    let env = env(&gateway.url);
    let guard = json!({"hooks": hooks(&[("PreToolUse", Some("Bash"), "echo 'approved guard' >&2; exit 2")])});
    let file = env.home.write("proj/anysola.json", &guard.to_string());
    let made = env.home.project().join("made");
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let document = core.hook_documents().remove(0);
    core.trust_hooks(&document.path, &document.digest, true)
        .unwrap();
    let (session, _) = core.open(&Target::New, false).unwrap();
    let blocked = |text: &str| {
        runtime
            .block_on(core.prompt(&session.id, prompt(text), TurnOptions::default()))
            .unwrap();
        let requests = model_requests(&gateway);
        last(&requests[requests.len() - 1]).contains("approved guard")
    };
    assert!(blocked("one"));
    // The agent rewrites the file: the approved guard keeps running, the change waits.
    std::fs::write(
        &file,
        json!({"disable_hooks": true, "hooks": hooks(&[("PreToolUse", Some("Bash"), "true")])})
            .to_string(),
    )
    .unwrap();
    runtime.block_on(core.reload());
    let mut events = core.subscribe();
    assert!(blocked("two"));
    assert!(!made.exists());
    assert!(blocked("three"));
    let notices = drain_events(&mut events)
        .into_iter()
        .filter(|e| matches!(e, Event::Hook { outcome: HookOutcome::Notice, message, .. } if message.contains("keep running")))
        .count();
    assert_eq!(notices, 1, "once per change");
    let listed = core.hook_documents();
    assert_eq!(listed[0].decision, None);
    assert!(listed[0].hooks.iter().all(|h| !h.active));
    // The file is deleted: still guarded, and listed as a document without hooks.
    std::fs::remove_file(&file).unwrap();
    runtime.block_on(core.reload());
    let listed = core.hook_documents();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].path, file.display().to_string());
    assert_eq!(listed[0].digest, anysola_core::hooks::removed_digest());
    assert!(listed[0].hooks.is_empty() && listed[0].decision.is_none());
    assert!(blocked("four"));
    // Accepting the removal ends it.
    core.trust_hooks(&listed[0].path, &listed[0].digest, true)
        .unwrap();
    assert!(core.hook_documents().is_empty());
    assert!(!blocked("five"));
    assert!(made.exists());
}

#[test]
fn global_config_writes_are_always_asked() {
    let target: Arc<std::sync::OnceLock<String>> = Arc::default();
    let path = target.clone();
    let gateway = FakeGateway::start(move |request, _| {
        if request.is_title() || last(request).contains("tool_result") {
            return text(request, "done");
        }
        call(
            request,
            "write",
            json!({"path": path.get().unwrap(), "content": "{\"hooks\": {}}"}),
        )
    });
    let env = env(&gateway.url);
    let config = env.home.home().join(".anysola/config.json");
    target.set(config.display().to_string()).unwrap();
    let original = json!({"permissions": [{"action": "*", "resource": "*", "effect": "allow"}]});
    user_config(&env, original.clone());
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let mut events = core.subscribe();
    let answering = core.clone();
    let asked = runtime.spawn(async move {
        loop {
            if let Ok(Event::PermissionAsked { request_id, .. }) = events.recv().await {
                answering.reply_permission(&request_id, Answer::Reject(Some("no".into())));
                return;
            }
        }
    });
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    runtime
        .block_on(async { tokio::time::timeout(Duration::from_secs(5), asked).await })
        .expect("the user was asked")
        .unwrap();
    assert_eq!(read_json(&config), original, "the config is unchanged");
}

#[test]
fn a_guard_whose_output_cannot_be_read_blocks() {
    let gateway = calls_once("shell", json!({"command": "touch made"}));
    let env = env(&gateway.url);
    user_config(
        &env,
        json!({"hooks": hooks(&[("PreToolUse", Some("Bash"), "head -c 1100000 /dev/zero | tr '\\0' a")])}),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert!(!env.home.project().join("made").exists());
    assert!(last(&model_requests(&gateway)[1]).contains("too large to read"));
}

#[test]
fn code_mode_hooks_see_the_repaired_input() {
    let gateway = calls_once(
        "execute",
        json!({"code": "return await tools.fake.touch('{\"target\": \"secret\"}')"}),
    );
    let env = env(&gateway.url);
    let marker = env.home.home().join("touched");
    let guard = r#"grep -q '"target":"secret"' && { echo 'not that one' >&2; exit 2; }; true"#;
    user_config(
        &env,
        json!({"hooks": hooks(&[("PreToolUse", Some("mcp__fake__touch"), guard)])}),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    core.set_namespace(Arc::new(Namespace {
        name: "fake".into(),
        description: None,
        direct: false,
        tools: vec![Arc::new(Touch(marker.clone()))],
    }));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert!(!marker.exists());
    assert!(last(&model_requests(&gateway)[1]).contains("not that one"));
}
