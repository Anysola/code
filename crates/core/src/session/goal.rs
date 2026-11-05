//! `/goal`: keeps a session working until an objective is met. Each time the session would go idle,
//! a reviewer generation judges the conversation against the objective; "continue" admits a
//! continuation into the same execution, anything else settles the goal. The goal lives in the KV
//! store under `goal:<sessionID>`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::broadcast;

use super::inbox::{self, Delivery};
use super::lower::Entry;
use super::message::{Message, Part};
use crate::api::Shared;
use crate::events::Event;
use crate::storage::Db;
use crate::time::now_ms;

const KICKOFF: &str = include_str!("../prompt/goal/kickoff.txt");
const CONTINUATION: &str = include_str!("../prompt/goal/continuation.txt");
const JUDGE: &str = include_str!("../prompt/goal/judge.txt");
const REMINDER: &str = include_str!("../prompt/goal/reminder.txt");

pub const MAX_OBJECTIVE: usize = 4_000;
pub const MAX_ROUNDS: u64 = 50;
pub const MAX_STALLS: u64 = 3;
/// How long an active goal may sit idle, waiting on background work that never reports, before it
/// pauses.
pub const WAIT_LIMIT: Duration = Duration::from_secs(30 * 60);
const USAGE: &str = "Usage: /goal <objective> | pause | resume | clear";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Active,
    Paused,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Goal {
    pub id: String,
    pub objective: String,
    pub status: Status,
    /// Why the goal last changed: the reviewer's reason, or what paused it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Continuations sent since the goal was set or last resumed.
    pub rounds: u64,
    pub max_rounds: u64,
    /// Rounds in a row without progress: no tool calls, or the same reason as the round before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stalls: Option<u64>,
    /// The process (`<pid>:<nonce>`) that armed the goal.
    pub boot: String,
    pub updated_at: u64,
}

fn key(session_id: &str) -> String {
    format!("{}{session_id}", crate::storage::GOAL_PREFIX)
}

/// The session's goal. An active goal whose process is gone reads as paused: nothing drives it.
pub fn read(db: &Db, events: &broadcast::Sender<Event>, session_id: &str) -> Option<Goal> {
    let goal: Goal = serde_json::from_value(db.kv_get(&key(session_id)).ok()??).ok()?;
    if goal.status == Status::Active && !crate::storage::owner_alive(&goal.boot) {
        settle(
            db,
            events,
            session_id,
            &goal,
            Status::Paused,
            "The server restarted while the goal was active",
        );
        return serde_json::from_value(db.kv_get(&key(session_id)).ok()??).ok();
    }
    Some(goal)
}

fn write(db: &Db, events: &broadcast::Sender<Event>, session_id: &str, goal: Option<&Goal>) {
    let stored = match goal {
        Some(goal) => db.kv_set(&key(session_id), &json!(goal)),
        None => db.kv_remove(&key(session_id)),
    };
    if stored.is_ok() {
        let _ = events.send(Event::GoalChanged {
            session_id: session_id.to_string(),
            goal: goal.cloned(),
        });
    }
}

/// The loop's writes only apply to the goal they were decided on: a `/goal pause`, `clear` or new
/// goal that landed meanwhile wins.
fn replace(
    db: &Db,
    events: &broadcast::Sender<Event>,
    session_id: &str,
    expected: &Goal,
    next: &Goal,
) -> bool {
    let current: Option<Goal> = db
        .kv_get(&key(session_id))
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok());
    let same = current.is_some_and(|c| {
        c.id == expected.id && c.updated_at == expected.updated_at && c.status == expected.status
    });
    if same {
        write(db, events, session_id, Some(next));
    }
    same
}

fn settle(
    db: &Db,
    events: &broadcast::Sender<Event>,
    session_id: &str,
    goal: &Goal,
    status: Status,
    reason: &str,
) {
    let next = Goal {
        status,
        reason: Some(reason.to_string()),
        updated_at: now_ms(),
        ..goal.clone()
    };
    replace(db, events, session_id, goal, &next);
}

fn pause_active(shared: &Shared, session_id: &str, reason: &str) {
    if let Some(goal) = read(&shared.db, &shared.events, session_id)
        && goal.status == Status::Active
    {
        settle(
            &shared.db,
            &shared.events,
            session_id,
            &goal,
            Status::Paused,
            reason,
        );
    }
}

/// One pass, so an objective that contains a placeholder is not expanded again.
fn render(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let name_len = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(after.len());
        let value = (name_len > 0 && after[name_len..].starts_with('}'))
            .then(|| values.iter().find(|(k, _)| *k == &after[..name_len]))
            .flatten();
        match value {
            Some((_, value)) => {
                out.push_str(value);
                rest = &after[name_len + 1..];
            }
            None => {
                out.push_str("${");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn truncate(text: &str, length: usize) -> String {
    let mut line = String::with_capacity(text.len());
    for c in text.chars() {
        if !c.is_whitespace() {
            line.push(c);
        } else if !line.ends_with(' ') {
            line.push(' ');
        }
    }
    if line.chars().count() > length {
        format!("{}…", line.chars().take(length - 1).collect::<String>())
    } else {
        line
    }
}

/// `/goal "Ship the release"` and `/goal Ship the release` set the same objective.
fn unquote(text: &str) -> String {
    let mut chars = text.chars();
    match (chars.next(), chars.next_back()) {
        (Some(first), Some(last))
            if [('"', '"'), ('\'', '\''), ('“', '”'), ('‘', '’')].contains(&(first, last)) =>
        {
            chars.as_str().trim().to_string()
        }
        _ => text.to_string(),
    }
}

fn objective_block(objective: &str) -> String {
    format!(
        "<untrusted_objective>\n{}\n</untrusted_objective>",
        escape(objective)
    )
}

fn continuation(goal: &Goal, feedback: &[Option<String>]) -> String {
    let feedback: Vec<&str> = feedback.iter().flatten().map(String::as_str).collect();
    render(
        CONTINUATION,
        &[
            ("objective", &escape(&goal.objective)),
            ("feedback", &feedback.join("\n")),
        ],
    )
}

fn describe(goal: &Goal) -> String {
    let status = match goal.status {
        Status::Active => "Goal active",
        Status::Paused => "Goal paused",
        Status::Complete => "Goal achieved",
    };
    let mut lines = vec![format!("{status}: {}", goal.objective)];
    lines.extend(goal.reason.clone().filter(|r| !r.is_empty()));
    lines.join("\n")
}

fn same_reason(previous: Option<&str>, next: &str) -> bool {
    let normalize = |text: &str| {
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    previous.is_some_and(|p| normalize(p) == normalize(next))
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Verdict {
    verdict: String,
    reason: String,
    #[serde(default)]
    next_step: Option<String>,
}

/// Reviewers sometimes wrap the JSON in a code fence or add a sentence around it.
fn parse_verdict(text: &str) -> Option<Verdict> {
    let (start, end) = (text.find('{')?, text.rfind('}')?);
    if end < start {
        return None;
    }
    let verdict: Verdict = serde_json::from_str(&text[start..=end]).ok()?;
    ["done", "continue", "blocked", "paused", "wait"]
        .contains(&verdict.verdict.as_str())
        .then_some(verdict)
}

fn metadata(goal: &Goal) -> Value {
    json!({"source": "goal", "goalID": goal.id})
}

/// What `/goal <arguments>` asks the session to do next.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Settled without a turn (pause, clear).
    Done,
    /// Start (or continue) the session with this synthetic message.
    Run {
        text: String,
        description: Option<String>,
        metadata: Value,
    },
}

/// `/goal <objective> | pause | resume | clear`. `Err` carries the text shown to the user.
pub(crate) fn command(
    shared: &Shared,
    session_id: &str,
    arguments: &str,
) -> Result<Command, String> {
    let (db, events) = (&shared.db, &shared.events);
    let text = unquote(arguments.trim());
    let goal = read(db, events, session_id);
    if text.is_empty() {
        return Err(match &goal {
            Some(goal) => format!("{}\n{USAGE}", describe(goal)),
            None => USAGE.to_string(),
        });
    }
    match text.to_lowercase().as_str() {
        "pause" => {
            let goal = goal
                .filter(|g| g.status == Status::Active)
                .ok_or("No active goal to pause.")?;
            // The user's command wins over the loop, so it writes without the loop's guard.
            let paused = Goal {
                status: Status::Paused,
                reason: Some("Paused by the user".into()),
                updated_at: now_ms(),
                ..goal
            };
            write(db, events, session_id, Some(&paused));
            Ok(Command::Done)
        }
        "resume" => {
            let goal = goal
                .filter(|g| g.status == Status::Paused)
                .ok_or("No paused goal to resume.")?;
            let resumed = Goal {
                id: goal.id,
                objective: goal.objective,
                status: Status::Active,
                reason: None,
                // The resume continuation is the first round of the new budget.
                rounds: 1,
                max_rounds: goal.max_rounds,
                stalls: None,
                boot: shared.owner.clone(),
                updated_at: now_ms(),
            };
            write(db, events, session_id, Some(&resumed));
            Ok(Command::Run {
                text: continuation(&resumed, &[Some("The user resumed the goal.".into())]),
                description: None,
                metadata: metadata(&resumed),
            })
        }
        "clear" => {
            goal.ok_or("No goal is set.")?;
            write(db, events, session_id, None);
            Ok(Command::Done)
        }
        _ => {
            let length = text.encode_utf16().count();
            if length > MAX_OBJECTIVE {
                return Err(format!(
                    "A goal objective is limited to {MAX_OBJECTIVE} characters (got {length})."
                ));
            }
            let created = Goal {
                id: uuid(),
                objective: text.clone(),
                status: Status::Active,
                reason: None,
                rounds: 0,
                max_rounds: MAX_ROUNDS,
                stalls: None,
                boot: shared.owner.clone(),
                updated_at: now_ms(),
            };
            write(db, events, session_id, Some(&created));
            // Titles come from the first user message, and a kickoff is not one, so name the
            // session here.
            if let Ok(Some(session)) = db.session(session_id)
                && super::is_fallback_title(&session)
                && db.set_title(session_id, &truncate(&text, 60)).is_ok()
            {
                let _ = events.send(Event::TitleChanged {
                    session_id: session_id.to_string(),
                    title: truncate(&text, 60),
                });
            }
            Ok(Command::Run {
                text: render(KICKOFF, &[("objective", &escape(&text))]),
                description: Some(format!("Goal: {}", truncate(&text, 120))),
                metadata: metadata(&created),
            })
        }
    }
}

fn uuid() -> String {
    let bytes: [u8; 16] = rand::random();
    let hex = hex::encode(bytes);
    let variant = ["8", "9", "a", "b"][(bytes[8] >> 6) as usize & 3];
    format!(
        "{}-{}-4{}-{variant}{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[13..16],
        &hex[17..20],
        &hex[20..]
    )
}

/// Until the next continuation restates the objective, a compacted history may not show it: the
/// reminder goes near the tail of the request, unrecorded, so the cached prefix stays warm.
pub(crate) fn remind(shared: &Shared, session_id: &str, entries: &mut Vec<Entry>) {
    let Some(goal) = read(&shared.db, &shared.events, session_id) else {
        return;
    };
    if goal.status != Status::Active {
        return;
    }
    let block = objective_block(&goal.objective);
    let user = |entry: &Entry| matches!(entry, Entry::Turn(m) if m["role"] == "user");
    let text = |message: &Value| {
        message["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|b| (b["type"] == "text").then(|| b["text"].as_str()).flatten())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    };
    let shown = entries.iter().any(|entry| match entry {
        Entry::Turn(message) if user(entry) => text(message).contains(&block),
        _ => false,
    });
    if shown {
        return;
    }
    let at = match entries.last() {
        Some(entry) if user(entry) => entries.len() - 1,
        _ => entries.len(),
    };
    let reminder = render(REMINDER, &[("objective", &escape(&goal.objective))]);
    entries.insert(
        at,
        Entry::Turn(json!({"role": "user", "content": [{"type": "text", "text": reminder}]})),
    );
}

/// Per-process loop state: reviewer overflows and the wait timers of idle goals.
#[derive(Default)]
pub(crate) struct Goals {
    /// One compaction per round when the reviewer's request is too large (`<goal id>:<rounds>`).
    compacted: Mutex<HashMap<String, String>>,
    waiting: Mutex<HashMap<String, tokio::task::AbortHandle>>,
}

fn reviewer_prompt(goal: &Goal, shells: &[String]) -> String {
    let background = if shells.is_empty() {
        "No background shells are running.".to_string()
    } else {
        let mut lines = vec!["Background shells still running:".to_string()];
        lines.extend(shells.iter().map(|c| format!("- {c}")));
        lines.push(String::new());
        lines.push("Answer \"wait\" only when the goal needs the result of one of these shells. Never wait for a long-running server such as a dev server.".into());
        lines.join("\n")
    };
    let wait = if shells.is_empty() {
        ""
    } else {
        "{\"verdict\":\"wait\",\"reason\":\"<the running shell whose result the goal needs>\"}\n"
    };
    render(
        JUDGE,
        &[
            ("objective", &escape(&goal.objective)),
            ("background", &background),
            ("wait", wait),
        ],
    )
}

enum Judged {
    Verdict(Verdict),
    Unusable,
    Overflow,
}

async fn judge(shared: &Arc<Shared>, session_id: &str, goal: &Goal, shells: &[String]) -> Judged {
    let prompt = reviewer_prompt(goal, shells);
    let Ok(Some(session)) = shared.db.session(session_id) else {
        return Judged::Unusable;
    };
    for _ in 0..2 {
        match super::step::generate(shared, &session, &prompt).await {
            Err(super::step::GenerateError::Overflow) => return Judged::Overflow,
            Err(super::step::GenerateError::Failed(error)) => {
                crate::log::warn(
                    "goal reviewer failed",
                    &[("session", session_id), ("error", &error.message)],
                );
            }
            Ok(text) => {
                // "wait" is only offered while a shell runs; without one nothing would wake the
                // session to judge again.
                if let Some(verdict) =
                    parse_verdict(&text).filter(|v| v.verdict != "wait" || !shells.is_empty())
                {
                    return Judged::Verdict(verdict);
                }
            }
        }
    }
    Judged::Unusable
}

fn compacted(shared: &Shared) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
    shared
        .goals
        .compacted
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Whether the model called any tool since the goal's last kickoff or continuation. Without that
/// message in view (after a compaction), the round counts as progress.
fn used_tools(shared: &Shared, session_id: &str, goal_id: &str) -> bool {
    let messages = super::compaction::history(shared, session_id).messages;
    let goal_message = |extra: &serde_json::Map<String, Value>| {
        extra
            .get("metadata")
            .and_then(|m| m.get("goalID"))
            .and_then(Value::as_str)
            == Some(goal_id)
    };
    let start = messages.iter().rposition(|m| match m {
        Message::User(data) => goal_message(&data.extra),
        Message::Synthetic(data) => goal_message(&data.extra),
        _ => false,
    });
    let Some(start) = start else {
        return true;
    };
    messages[start + 1..].iter().any(|m| match m {
        Message::Assistant(data) => data.content.iter().any(|p| matches!(p, Part::Tool { .. })),
        _ => false,
    })
}

/// The session would go idle with nothing pending. Returns whether input was admitted, which
/// continues the same execution.
pub(crate) async fn idle(shared: &Arc<Shared>, session_id: &str) -> bool {
    let (db, events) = (&shared.db, &shared.events);
    let Some(goal) = read(db, events, session_id) else {
        return false;
    };
    if goal.status != Status::Active {
        return false;
    }
    if goal.boot != shared.owner {
        settle(
            db,
            events,
            session_id,
            &goal,
            Status::Paused,
            "The server restarted while the goal was active",
        );
        return false;
    }
    // The session's own running subagent wakes it with its notice, and this runs again then.
    if super::subagent::has_running_child(shared, session_id) {
        return false;
    }
    if goal.rounds >= goal.max_rounds {
        let reason = format!("Reached the limit of {} rounds", goal.max_rounds);
        settle(db, events, session_id, &goal, Status::Paused, &reason);
        return false;
    }
    // Only the session's own shells wake it when they finish.
    let shells = crate::tool::shell::background_commands(session_id);
    let judged = judge(shared, session_id, &goal, &shells).await;
    if !matches!(judged, Judged::Overflow) {
        compacted(shared).remove(session_id);
    }
    // `/goal pause`, `clear` or a new goal may have landed while the reviewer ran.
    let Some(latest) = read(db, events, session_id)
        .filter(|latest| latest.id == goal.id && latest.status == Status::Active)
    else {
        return false;
    };
    let pause = |reason: &str| {
        settle(db, events, session_id, &latest, Status::Paused, reason);
        false
    };
    let verdict = match judged {
        Judged::Overflow => {
            let round = format!("{}:{}", latest.id, latest.rounds);
            let mut compacted = compacted(shared);
            if compacted.get(session_id) == Some(&round) {
                compacted.remove(session_id);
                drop(compacted);
                return pause("The goal reviewer's request is too large for the model");
            }
            compacted.insert(session_id.to_string(), round);
            drop(compacted);
            // The compaction runs before the session goes idle, then this runs again on the
            // smaller history.
            return inbox::enqueue_compaction(db, session_id).is_ok();
        }
        Judged::Unusable => return pause("The goal reviewer gave no usable verdict"),
        Judged::Verdict(verdict) => verdict,
    };
    match verdict.verdict.as_str() {
        "wait" => return false,
        "done" => {
            settle(
                db,
                events,
                session_id,
                &latest,
                Status::Complete,
                &verdict.reason,
            );
            return false;
        }
        "continue" => {}
        _ => return pause(&verdict.reason),
    }
    let stalled = same_reason(latest.reason.as_deref(), &verdict.reason)
        || !used_tools(shared, session_id, &latest.id);
    let stalls = if stalled {
        latest.stalls.unwrap_or(0) + 1
    } else {
        0
    };
    if stalls >= MAX_STALLS {
        return pause(&format!("Not making progress: {}", verdict.reason));
    }
    let next = Goal {
        rounds: latest.rounds + 1,
        stalls: Some(stalls),
        reason: Some(verdict.reason.clone()),
        updated_at: now_ms(),
        ..latest.clone()
    };
    if !replace(db, events, session_id, &latest, &next) {
        return false;
    }
    let text = continuation(
        &next,
        &[
            Some(verdict.reason),
            verdict.next_step.map(|step| format!("Next step: {step}")),
        ],
    );
    // No description: timelines hide the continuation, and the pinned goal shows that the loop is
    // still running.
    inbox::enqueue_synthetic(
        db,
        session_id,
        &text,
        None,
        Some(&metadata(&next)),
        Delivery::Steer,
    )
    .is_ok()
}

/// How an execution of the session ended, for the goal: success arms the wait timer, a failure or
/// the user's interrupt pauses the goal.
pub(crate) fn ended(shared: &Arc<Shared>, session_id: &str, outcome: &super::Outcome) {
    use crate::events::InterruptReason;
    match outcome {
        super::Outcome::Succeeded => watch(
            shared.db.clone(),
            shared.events.clone(),
            &shared.goals,
            session_id,
            WAIT_LIMIT,
        ),
        super::Outcome::Failed(error) => pause_active(
            shared,
            session_id,
            &format!("The session failed: {}", error.message),
        ),
        // Other interrupts keep the goal active; a later process sees the boot change.
        super::Outcome::Interrupted(InterruptReason::User) => {
            pause_active(shared, session_id, "Stopped by the user")
        }
        super::Outcome::Interrupted(_) => {}
    }
}

/// An execution of the session starts: it is no longer waiting.
pub(crate) fn started(shared: &Shared, session_id: &str) {
    if let Some(timer) = shared
        .goals
        .waiting
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(session_id)
    {
        timer.abort();
    }
}

/// A step anywhere below a waiting goal restarts its timer, so a healthy subagent working past
/// the limit does not pause the goal it works for.
pub(crate) fn touch(shared: &Arc<Shared>, session_id: &str) {
    if shared
        .goals
        .waiting
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_empty()
    {
        return;
    }
    let mut current = shared
        .db
        .session(session_id)
        .ok()
        .flatten()
        .and_then(|s| s.parent_id);
    while let Some(id) = current {
        let waiting = shared
            .goals
            .waiting
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&id);
        if waiting {
            watch(
                shared.db.clone(),
                shared.events.clone(),
                &shared.goals,
                &id,
                WAIT_LIMIT,
            );
            return;
        }
        current = shared
            .db
            .session(&id)
            .ok()
            .flatten()
            .and_then(|s| s.parent_id);
    }
}

/// An active goal only sits idle while it waits on background work, whose notice wakes the
/// session. If nothing runs in the session or below it for `limit`, the goal pauses.
fn watch(
    db: Arc<Db>,
    events: broadcast::Sender<Event>,
    goals: &Goals,
    session_id: &str,
    limit: Duration,
) {
    let Some(goal) = read(&db, &events, session_id).filter(|g| g.status == Status::Active) else {
        return;
    };
    let id = session_id.to_string();
    let timer = tokio::spawn(async move {
        tokio::time::sleep(limit).await;
        let reason = format!(
            "Nothing ran for {} minutes while the goal waited on background work",
            limit.as_secs() / 60
        );
        settle(&db, &events, &id, &goal, Status::Paused, &reason);
    });
    if let Some(previous) = goals
        .waiting
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(session_id.to_string(), timer.abort_handle())
    {
        previous.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn goal(objective: &str) -> Goal {
        Goal {
            id: "g1".into(),
            objective: objective.into(),
            status: Status::Active,
            reason: None,
            rounds: 0,
            max_rounds: MAX_ROUNDS,
            stalls: None,
            boot: format!("{}:x", std::process::id()),
            updated_at: 1,
        }
    }

    #[test]
    fn texts() {
        assert_eq!(
            render("a ${x} ${y} ${x}", &[("x", "${y}"), ("y", "2")]),
            "a ${y} 2 ${y}"
        );
        assert_eq!(render("${missing} $", &[]), "${missing} $");
        assert_eq!(unquote("\"Ship it\""), "Ship it");
        assert_eq!(unquote("“ Ship ”"), "Ship");
        assert_eq!(unquote("\"Ship’"), "\"Ship’");
        assert_eq!(unquote("Ship"), "Ship");
        assert_eq!(truncate("a  b\nc", 60), "a b c");
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(escape("<a & b>"), "&lt;a &amp; b&gt;");
        let judge = reviewer_prompt(&goal("x < y"), &[]);
        assert!(judge.contains("<untrusted_objective>\nx &lt; y\n</untrusted_objective>\n\nNo background shells are running.\n"));
        assert!(judge.contains(
            "{\"verdict\":\"paused\",\"reason\":\"<what the user asked for>\"}\n\nRules:"
        ));
        let judge = reviewer_prompt(&goal("x"), &["npm test".into()]);
        assert!(judge.contains("Background shells still running:\n- npm test\n\nAnswer \"wait\""));
        assert!(judge.contains("for>\"}\n{\"verdict\":\"wait\",\"reason\":\"<the running shell whose result the goal needs>\"}\n\nRules:"));
        let next = continuation(&goal("x"), &[Some("missing tests".into()), None]);
        assert!(next.contains("</untrusted_objective>\n\nmissing tests\n\nTake the next"));
        assert_eq!(
            describe(&Goal {
                reason: Some("why".into()),
                ..goal("x")
            }),
            "Goal active: x\nwhy"
        );
        assert!(same_reason(Some(" Tests  FAIL"), "tests fail"));
        assert!(!same_reason(None, "x"));
    }

    #[test]
    fn verdicts() {
        let verdict = parse_verdict(
            "```json\n{\"verdict\":\"continue\",\"reason\":\"r\",\"next_step\":\"s\"}\n```",
        )
        .unwrap();
        assert_eq!(
            (verdict.verdict.as_str(), verdict.next_step.as_deref()),
            ("continue", Some("s"))
        );
        assert_eq!(
            parse_verdict("{\"verdict\":\"maybe\",\"reason\":\"r\"}"),
            None
        );
        assert_eq!(parse_verdict("no json"), None);
        assert_eq!(parse_verdict("} {"), None);
    }

    #[test]
    fn stored_shape() {
        let stored = json!(goal("x"));
        assert_eq!(
            stored,
            json!({"id": "g1", "objective": "x", "status": "active", "rounds": 0, "maxRounds": 50,
                   "boot": format!("{}:x", std::process::id()), "updatedAt": 1})
        );
        let id = uuid();
        assert_eq!((id.len(), &id[14..15]), (36, "4"));
    }

    #[tokio::test(start_paused = true)]
    async fn waiting_goal_pauses_after_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(Db::open(&dir.path().join("db.sqlite")).unwrap());
        let (events, _) = broadcast::channel(16);
        let goals = Goals::default();
        db.kv_set(&key("ses_1"), &json!(goal("x"))).unwrap();
        watch(db.clone(), events.clone(), &goals, "ses_1", WAIT_LIMIT);
        tokio::time::sleep(WAIT_LIMIT - Duration::from_secs(1)).await;
        assert_eq!(read(&db, &events, "ses_1").unwrap().status, Status::Active);
        tokio::time::sleep(Duration::from_secs(2)).await;
        let paused = read(&db, &events, "ses_1").unwrap();
        assert_eq!(paused.status, Status::Paused);
        assert_eq!(
            paused.reason.as_deref(),
            Some("Nothing ran for 30 minutes while the goal waited on background work")
        );
    }

    #[test]
    fn a_goal_of_a_gone_process_reads_as_paused() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("db.sqlite")).unwrap();
        let (events, _) = broadcast::channel(16);
        let gone = Goal {
            boot: "999999999:x".into(),
            ..goal("x")
        };
        db.kv_set(&key("ses_1"), &json!(gone)).unwrap();
        let read = read(&db, &events, "ses_1").unwrap();
        assert_eq!(
            (read.status, read.reason.as_deref()),
            (
                Status::Paused,
                Some("The server restarted while the goal was active")
            )
        );
    }
}
