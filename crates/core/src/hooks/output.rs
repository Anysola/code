//! What one hook's run means for its event: exit code 0 (stdout plain text or one JSON object),
//! 2 (blocking, stderr is the reason) or anything else (a non-blocking error).

use serde_json::{Map, Value};

use super::Event;

/// Every string taken from a hook is cut to this many characters.
pub const MAX_CHARS: usize = 10_000;
const CUT: &str = "\n[… cut: the hook's output was longer than 10000 characters]";

/// How a hook process ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Ran {
    Exited {
        code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    /// The command could not start.
    NotStarted(String),
    TimedOut {
        seconds: u64,
    },
    /// The hook exited 0 but printed more than can be read: its answer is unknown.
    OutputTooLarge,
    /// The turn was interrupted; the hook was killed and means nothing.
    Interrupted,
}

/// What one hook said, read for its event.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Verdict {
    /// Exit 2, `decision: "block"`, or a PreToolUse `deny`: the reason.
    pub block: Option<String>,
    /// PreToolUse `permissionDecision: "ask"`.
    pub ask: bool,
    /// Context for the model.
    pub context: Vec<String>,
    /// Shown to the user (`systemMessage`, SessionStart's stderr on exit 2).
    pub notice: Option<String>,
    /// `continue: false`, with its `stopReason` (possibly empty).
    pub stop: Option<String>,
    /// A non-blocking error, shown to the user.
    pub error: Option<String>,
}

/// `text` cut to [`MAX_CHARS`] characters, with a marker when it was longer.
pub fn cap(text: &str) -> String {
    match text.char_indices().nth(MAX_CHARS) {
        Some((at, _)) => format!("{}{CUT}", &text[..at]),
        None => text.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stdout {
    Plain(String),
    Json(Map<String, Value>),
    /// Starts like a JSON object but is not exactly one.
    Invalid,
}

/// Stdout is JSON only when it is exactly one JSON object.
pub fn parse_stdout(stdout: &str) -> Stdout {
    let trimmed = stdout.trim();
    if !trimmed.starts_with('{') {
        return Stdout::Plain(trimmed.to_string());
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(map)) => Stdout::Json(map),
        _ => Stdout::Invalid,
    }
}

fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
}

fn text(map: &Map<String, Value>, key: &str) -> Option<String> {
    map.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(cap)
}

/// A PreToolUse hook that could not be run at all (it did not start, or the shell exited 126 or
/// 127: not executable, not found) blocks the call: the model can cause that (by moving the
/// project, for one), so a guard must not fail open. The reason names the command, so a user with
/// a broken hook sees what to fix. `worktree`: the session works in its own worktree there (a
/// command that exits 126/127 there likely names a folder Git ignores, which isn't copied in), and
/// the reason says so. `None` for everything else, which [`interpret`] reads.
pub fn unrunnable(
    event: Event,
    command: &str,
    ran: &Ran,
    worktree: Option<&std::path::Path>,
) -> Option<String> {
    if event != Event::PreToolUse {
        return None;
    }
    let mut hint = String::new();
    let detail = match ran {
        Ran::NotStarted(error) => error.clone(),
        Ran::Exited {
            code: Some(code @ (126 | 127)),
            stderr,
            ..
        } => {
            if let Some(worktree) = worktree {
                hint = format!(
                    " This session runs in its own worktree ({}): folders Git ignores, like .venv or node_modules, aren't copied there. Create them with a worktree setup command (Settings › Worktree; $ANYSOLA_MAIN_FOLDER is set), or use an absolute path in the hook.",
                    worktree.display()
                );
            }
            match first_line(stderr) {
                "" => format!("exit code {code}"),
                line => format!("exit code {code}: {line}"),
            }
        }
        _ => return None,
    };
    // The command and the shell's line are cut first, so the hint always gets through.
    let reason = cap(&format!(
        "The PreToolUse hook \"{command}\" could not be run ({detail}), so the call was blocked."
    ));
    Some(format!("{reason}{hint}"))
}

impl Event {
    /// Whether exit code 2 or `decision: "block"` means a block (for PreCompact: of a manual
    /// compaction only, see `fire::pre_compact`). The others show exit 2's stderr as a notice.
    pub fn can_block(self) -> bool {
        !matches!(
            self,
            Event::SessionStart | Event::Notification | Event::SessionEnd
        )
    }
}

pub fn interpret(event: Event, ran: &Ran) -> Verdict {
    let mut verdict = Verdict::default();
    let (code, stdout, stderr) = match ran {
        Ran::Interrupted => return verdict,
        Ran::NotStarted(error) => {
            verdict.error = Some(cap(&format!("The hook could not start: {error}")));
            return verdict;
        }
        Ran::TimedOut { seconds } => {
            verdict.error = Some(format!("The hook timed out after {seconds} s"));
            return verdict;
        }
        Ran::OutputTooLarge => {
            let reason = "The hook's output was too large to read".to_string();
            // A guard whose answer cannot be read must not let the call through.
            if event == Event::PreToolUse {
                verdict.block = Some(reason);
            } else {
                verdict.error = Some(reason);
            }
            return verdict;
        }
        Ran::Exited {
            code,
            stdout,
            stderr,
        } => (*code, stdout, stderr),
    };
    match code {
        Some(0) => {}
        Some(2) => {
            let reason = stderr.trim();
            let reason = if reason.is_empty() {
                "Blocked by a hook".to_string()
            } else {
                cap(reason)
            };
            if !event.can_block() {
                verdict.notice = Some(reason);
            } else {
                verdict.block = Some(reason);
            }
            return verdict;
        }
        other => {
            let status = other.map_or_else(
                || "The hook was ended by a signal".to_string(),
                |c| format!("The hook exited with code {c}"),
            );
            let line = first_line(stderr);
            verdict.error = Some(cap(&if line.is_empty() {
                status
            } else {
                format!("{status}: {line}")
            }));
            return verdict;
        }
    }
    let map = match parse_stdout(stdout) {
        Stdout::Plain(plain) => {
            if !plain.is_empty() && matches!(event, Event::UserPromptSubmit | Event::SessionStart) {
                verdict.context.push(cap(&plain));
            }
            return verdict;
        }
        Stdout::Invalid => {
            verdict.error = Some(
                "The hook printed something that looks like JSON but is not one valid JSON object"
                    .into(),
            );
            return verdict;
        }
        Stdout::Json(map) => map,
    };
    verdict.notice = text(&map, "systemMessage");
    if matches!(event, Event::Notification | Event::SessionEnd) {
        // They never block, stop or add anything for the model: only a message for the user.
        return verdict;
    }
    if map.get("continue") == Some(&Value::Bool(false)) {
        verdict.stop = Some(text(&map, "stopReason").unwrap_or_default());
    }
    // Legacy PreToolUse "approve" is ignored: hooks never allow anything.
    if map.get("decision").and_then(Value::as_str) == Some("block") {
        verdict.block =
            Some(text(&map, "reason").unwrap_or_else(|| "Blocked by a hook".to_string()));
    }
    if let Some(specific) = map.get("hookSpecificOutput") {
        let Some(specific) = specific.as_object() else {
            verdict.error = Some("The hook's hookSpecificOutput is not an object".into());
            return verdict;
        };
        let named = specific.get("hookEventName").and_then(Value::as_str);
        if named != Some(event.as_str()) {
            verdict.error = Some(format!(
                "The hook's hookSpecificOutput is for {}, not {}; it was ignored",
                named.unwrap_or("no event"),
                event.as_str()
            ));
            return verdict;
        }
        if event == Event::PreToolUse {
            match specific.get("permissionDecision").and_then(Value::as_str) {
                Some("deny") => {
                    verdict.block = Some(
                        text(specific, "permissionDecisionReason")
                            .unwrap_or_else(|| "Denied by a hook".to_string()),
                    );
                }
                Some("ask") => verdict.ask = true,
                // "allow" is not honoured: rules, policies and the mode decide.
                _ => {}
            }
        }
        if let Some(context) = text(specific, "additionalContext")
            && event != Event::PreCompact
        {
            verdict.context.push(context);
        }
    }
    verdict
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn exited(code: i32, stdout: &str, stderr: &str) -> Ran {
        Ran::Exited {
            code: Some(code),
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    #[test]
    fn stdout_is_json_only_when_it_is_one_object() {
        assert_eq!(parse_stdout(" hello \n"), Stdout::Plain("hello".into()));
        assert_eq!(parse_stdout("[1]"), Stdout::Plain("[1]".into()));
        assert_eq!(
            parse_stdout("{\"a\": 1}\n"),
            Stdout::Json(json!({"a": 1}).as_object().unwrap().clone())
        );
        assert_eq!(parse_stdout("{\"a\": 1} {\"b\": 2}"), Stdout::Invalid);
        assert_eq!(parse_stdout("{not json"), Stdout::Invalid);
    }

    #[test]
    fn plain_stdout_is_context_only_for_prompts_and_session_start() {
        let ran = exited(0, "remember this\n", "");
        assert_eq!(
            interpret(Event::UserPromptSubmit, &ran).context,
            ["remember this"]
        );
        assert_eq!(
            interpret(Event::SessionStart, &ran).context,
            ["remember this"]
        );
        assert_eq!(interpret(Event::PreToolUse, &ran), Verdict::default());
        assert_eq!(interpret(Event::Stop, &ran), Verdict::default());
    }

    #[test]
    fn exit_two_blocks_with_stderr_except_on_session_start() {
        let ran = exited(2, "ignored", "  no rm -rf here\n");
        assert_eq!(
            interpret(Event::PreToolUse, &ran).block.as_deref(),
            Some("no rm -rf here")
        );
        let start = interpret(Event::SessionStart, &ran);
        assert_eq!(start.block, None);
        assert_eq!(start.notice.as_deref(), Some("no rm -rf here"));
        assert_eq!(
            interpret(Event::Stop, &exited(2, "", "")).block.as_deref(),
            Some("Blocked by a hook")
        );
    }

    #[test]
    fn other_failures_are_non_blocking_errors_with_the_first_stderr_line() {
        let verdict = interpret(Event::PreToolUse, &exited(1, "", "\nboom\nmore"));
        assert_eq!(verdict.block, None);
        assert_eq!(
            verdict.error.as_deref(),
            Some("The hook exited with code 1: boom")
        );
        assert!(
            interpret(Event::Stop, &Ran::TimedOut { seconds: 3 })
                .error
                .unwrap()
                .contains("timed out")
        );
        assert!(
            interpret(Event::Stop, &Ran::NotStarted("no such file".into()))
                .error
                .unwrap()
                .contains("no such file")
        );
        assert_eq!(
            interpret(Event::Stop, &Ran::Interrupted),
            Verdict::default()
        );
        let invalid = interpret(Event::PostToolUse, &exited(0, "{\"decision\": ", ""));
        assert!(invalid.error.is_some() && invalid.block.is_none());
    }

    #[test]
    fn json_answers() {
        let json = |value: Value| exited(0, &value.to_string(), "");
        let deny = json!({"hookSpecificOutput": {"hookEventName": "PreToolUse",
            "permissionDecision": "deny", "permissionDecisionReason": "not here"}});
        assert_eq!(
            interpret(Event::PreToolUse, &json(deny)).block.as_deref(),
            Some("not here")
        );
        let ask = json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "ask"}});
        assert!(interpret(Event::PreToolUse, &json(ask)).ask);
        let allow = json!({"hookSpecificOutput": {"hookEventName": "PreToolUse",
            "permissionDecision": "allow"}, "decision": "approve"});
        assert_eq!(
            interpret(Event::PreToolUse, &json(allow)),
            Verdict::default()
        );
        let block = json!({"decision": "block", "reason": "lint failed", "systemMessage": "Linted",
            "hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": "see above"}});
        let post = interpret(Event::PostToolUse, &json(block));
        assert_eq!(post.block.as_deref(), Some("lint failed"));
        assert_eq!(post.context, ["see above"]);
        assert_eq!(post.notice.as_deref(), Some("Linted"));
        let stop = json!({"continue": false, "stopReason": "enough"});
        assert_eq!(
            interpret(Event::Stop, &json(stop)).stop.as_deref(),
            Some("enough")
        );
        // A JSON answer's stdout is not context itself.
        let prompt = interpret(
            Event::UserPromptSubmit,
            &json(json!({"suppressOutput": true})),
        );
        assert!(prompt.context.is_empty());
    }

    #[test]
    fn notification_and_session_end_only_ever_show_a_message() {
        let json = |value: Value| exited(0, &value.to_string(), "");
        for event in [Event::Notification, Event::SessionEnd] {
            let two = interpret(event, &exited(2, "", "no"));
            assert_eq!(two.block, None);
            assert_eq!(two.notice.as_deref(), Some("no"));
            assert_eq!(
                interpret(event, &exited(0, "plain", "")),
                Verdict::default()
            );
            let everything = json(json!({"decision": "block", "reason": "r",
                "continue": false, "stopReason": "s", "systemMessage": "hi",
                "hookSpecificOutput": {"hookEventName": event.as_str(), "additionalContext": "c",
                    "permissionDecision": "deny"}}));
            let verdict = interpret(event, &everything);
            assert_eq!(
                verdict,
                Verdict {
                    notice: Some("hi".into()),
                    ..Verdict::default()
                },
                "{event:?}"
            );
        }
    }

    #[test]
    fn pre_compact_and_subagent_stop_block_like_stop() {
        for event in [Event::PreCompact, Event::SubagentStop] {
            assert_eq!(
                interpret(event, &exited(2, "", "not now")).block.as_deref(),
                Some("not now")
            );
            let block = json!({"decision": "block", "reason": "later"}).to_string();
            assert_eq!(
                interpret(event, &exited(0, &block, "")).block.as_deref(),
                Some("later")
            );
            // Plain stdout is not context for the model.
            assert!(interpret(event, &exited(0, "text", "")).context.is_empty());
        }
        let context = json!({"hookSpecificOutput": {"hookEventName": "PreCompact",
            "additionalContext": "c"}})
        .to_string();
        assert!(
            interpret(Event::PreCompact, &exited(0, &context, ""))
                .context
                .is_empty()
        );
        let context = context.replace("PreCompact", "SubagentStop");
        assert_eq!(
            interpret(Event::SubagentStop, &exited(0, &context, "")).context,
            ["c"]
        );
    }

    #[test]
    fn hook_specific_output_for_another_event_is_ignored() {
        let ran = exited(
            0,
            &json!({"hookSpecificOutput": {"hookEventName": "PostToolUse",
                "permissionDecision": "deny", "additionalContext": "x"}})
            .to_string(),
            "",
        );
        let verdict = interpret(Event::PreToolUse, &ran);
        assert_eq!(verdict.block, None);
        assert!(verdict.context.is_empty());
        assert!(verdict.error.unwrap().contains("PostToolUse"));
    }

    #[test]
    fn unreadable_output_blocks_a_tool_call_and_is_an_error_elsewhere() {
        let pre = interpret(Event::PreToolUse, &Ran::OutputTooLarge);
        assert!(pre.block.unwrap().contains("too large"));
        let post = interpret(Event::PostToolUse, &Ran::OutputTooLarge);
        assert!(post.block.is_none() && post.error.is_some());
    }

    #[test]
    fn a_guard_that_cannot_run_blocks_only_tool_calls() {
        let missing = exited(127, "", "sh: ./g16.sh: not found\n");
        let reason = unrunnable(Event::PreToolUse, "./g16.sh", &missing, None).unwrap();
        assert!(
            reason.contains("\"./g16.sh\"") && reason.contains("not found"),
            "{reason}"
        );
        assert!(!reason.contains("worktree"), "{reason}");
        let pre = Event::PreToolUse;
        assert!(unrunnable(pre, "x", &exited(126, "", ""), None).is_some());
        assert!(unrunnable(pre, "x", &Ran::NotStarted("no shell".into()), None).is_some());
        assert!(unrunnable(pre, "x", &exited(1, "", ""), None).is_none());
        assert!(unrunnable(pre, "x", &Ran::TimedOut { seconds: 1 }, None).is_none());
        assert!(unrunnable(Event::PostToolUse, "x", &missing, None).is_none());
        assert!(unrunnable(Event::Stop, "x", &Ran::NotStarted("e".into()), None).is_none());
    }

    /// In a worktree session the reason says why a relative tool may be missing there and what
    /// to do; the call stays blocked.
    #[test]
    fn a_guard_that_cannot_run_in_a_worktree_says_why() {
        let w = std::path::Path::new("/w/anysola-x");
        let missing = exited(127, "", "sh: .venv/bin/python: not found\n");
        let reason = unrunnable(
            Event::PreToolUse,
            ".venv/bin/python g.py",
            &missing,
            Some(w),
        );
        let reason = reason.expect("still blocks");
        assert!(reason.contains("could not be run"), "{reason}");
        assert!(reason.contains("own worktree (/w/anysola-x)"), "{reason}");
        assert!(reason.contains(".venv or node_modules"), "{reason}");
        assert!(reason.contains("$ANYSOLA_MAIN_FOLDER"), "{reason}");
        assert!(reason.contains("absolute path"), "{reason}");
        // A long command is cut, the hint is not.
        let long = "x".repeat(MAX_CHARS * 2);
        let reason = unrunnable(Event::PreToolUse, &long, &missing, Some(w)).unwrap();
        assert!(reason.ends_with("use an absolute path in the hook."), "cut");
        // Only "can't run" says it: a hook that didn't start (no shell) is another problem.
        let none = unrunnable(
            Event::PreToolUse,
            "x",
            &Ran::NotStarted("e".into()),
            Some(w),
        );
        assert!(!none.unwrap().contains("worktree"));
    }

    #[test]
    fn strings_are_capped() {
        let long = "é".repeat(MAX_CHARS + 5);
        let capped = cap(&long);
        assert!(capped.starts_with(&"é".repeat(MAX_CHARS)));
        assert!(capped.ends_with("characters]"));
        assert_eq!(cap("short"), "short");
        assert_eq!(cap(&"a".repeat(MAX_CHARS)), "a".repeat(MAX_CHARS));
        let verdict = interpret(Event::PreToolUse, &exited(2, "", &long));
        assert!(verdict.block.unwrap().chars().count() < MAX_CHARS + 100);
    }
}
