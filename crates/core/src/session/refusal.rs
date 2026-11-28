//! Declined answers: the model's safety checks refused a request (`stop_reason: "refusal"`),
//! before any output or part way through it. The turn fails and says so, and the prompt it
//! declined is left out of every later request: carried along, it would be declined again and
//! again, and the session would be stuck.

use std::borrow::Borrow;

use serde_json::Value;

use super::message::{AssistantData, Message, Part};
use crate::gateway::error::SessionError;

/// The error kind of a declined answer (`gateway::error` maps a content policy rejection to it too).
pub const KIND: &str = "provider.content-filter";

/// Set on a declined reply stored before 1.6.1 once a request has left its prompt out: from then
/// on it is left out for good, whatever comes after it.
const MARK: &str = "declined";

/// What the model said about a refusal (`stop_details` of `message_delta`). Either field may be
/// null or missing; a refusal is known by its `stop_reason` alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Refusal {
    /// `cyber`, `bio`, `frontier_llm`, `general_harms`, `reasoning_extraction`, or another one.
    pub category: Option<String>,
    pub explanation: Option<String>,
}

/// What was declined, which decides what the person is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Declined {
    /// The prompt itself, before anything of its turn stayed: it is left out from now on.
    Prompt,
    /// A later step of a turn whose earlier steps stay in the session.
    MidTurn,
    /// A side question (`/btw`).
    Question,
    /// A compaction's summary.
    Summary,
}

impl Refusal {
    pub fn from_details(details: Option<&Value>) -> Refusal {
        let text = |key: &str| {
            details
                .and_then(|d| d.get(key))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_string)
        };
        Refusal {
            category: text("category"),
            explanation: text("explanation"),
        }
    }

    /// The category in words; `None` for no category or one this version does not know.
    pub fn topic(&self) -> Option<&'static str> {
        Some(match self.category.as_deref()? {
            "cyber" => "cybersecurity",
            "bio" => "biology",
            "frontier_llm" => "AI model development",
            "general_harms" => "the usage policy",
            "reasoning_extraction" => "reasoning extraction",
            _ => return None,
        })
    }

    /// What the person reads: the model, the category when there is one, and what to do.
    pub fn message(&self, model: &str, what: Declined) -> String {
        let citing = self
            .topic()
            .map(|topic| format!(", citing {topic}"))
            .unwrap_or_default();
        match what {
            Declined::Prompt => format!(
                "{model} declined to answer this message{citing}. It won’t be sent again in this session; rephrase it, or start a new session if this keeps happening."
            ),
            Declined::MidTurn => format!(
                "{model} declined to continue{citing}. Something earlier in this turn may have caused it, so later messages in this session may be declined too; if they are, start a new session."
            ),
            Declined::Question => {
                format!(
                    "{model} declined to answer this question{citing}. Try asking it another way."
                )
            }
            Declined::Summary => format!(
                "{model} declined to summarize this conversation{citing}, so nothing was compacted. If the session can’t go on without it, start a new one."
            ),
        }
    }

    pub fn error(&self, model: &str, what: Declined) -> SessionError {
        SessionError::new(KIND, self.message(model, what))
    }
}

/// A declined reply with nothing in it: refused before any output, or its output dropped when it
/// was refused.
fn empty_refusal(step: &AssistantData) -> bool {
    step.finish.as_deref() == Some("content-filter")
        && step.content.iter().all(|part| match part {
            Part::Text { text, .. } => text.trim().is_empty(),
            Part::Reasoning { text, native, .. } => text.trim().is_empty() && native.is_none(),
            Part::Tool { .. } => false,
        })
}

/// Recorded as a declined reply by 1.6.1 or later (its error), or marked once a request left its
/// prompt out ([`mark`]).
fn marked(step: &AssistantData) -> bool {
    step.error.as_ref().is_some_and(|e| e.kind == KIND)
        || step.extra.get(MARK).and_then(Value::as_bool) == Some(true)
}

/// Thinking a request replays with its signature, which the provider checks against the history
/// before it.
fn signed_thinking(step: &AssistantData) -> bool {
    step.content.iter().any(|part| match part {
        Part::Reasoning {
            native: Some(native),
            ..
        } => {
            native
                .get("signature")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
                || native.get("redactedData").is_some()
        }
        _ => false,
    })
}

/// What a reply at `at` answered directly: the user messages right before it (a prompt and the
/// notes steered with it), the hook context rows UserPromptSubmit added for them, and a
/// compaction that completed in between (its summary was written with the prompt in the request,
/// so it goes with the prompt; the next step compacts again without it). What came with the input
/// but is not part of it stays: a background completion, a plan reminder, a session-start hook's
/// context, instruction updates, a failed compaction. Empty when the reply answered no user
/// message (a later step of the turn, a turn a completion woke): then only the reply is left out.
struct Answered {
    rows: Vec<usize>,
    /// The input opens its turn: no step of the same turn came before it. A steer that arrived
    /// after a step is part of a turn that already ran.
    opens_turn: bool,
}

fn answered<M: Borrow<Message>>(messages: &[M], at: usize) -> Answered {
    let mut run = Vec::new();
    let mut compactions = Vec::new();
    let mut opens_turn = true;
    for before in (0..at).rev() {
        match messages[before].borrow() {
            Message::User(_) => run.push(before),
            Message::Synthetic(data)
                if data.description.as_deref() == Some(crate::hooks::fire::CONTEXT) =>
            {
                run.push(before)
            }
            Message::Compaction(c) if c.completed() => compactions.push(before),
            Message::Synthetic(_) | Message::System(_) | Message::Compaction(_) => {}
            Message::Assistant(_) => {
                opens_turn = false;
                break;
            }
            _ => break,
        }
    }
    let oldest = run
        .iter()
        .rposition(|i| matches!(messages[*i].borrow(), Message::User(_)));
    let Some(oldest) = oldest else {
        return Answered {
            rows: Vec::new(),
            opens_turn: false,
        };
    };
    run.truncate(oldest + 1);
    let first = run[oldest];
    run.extend(compactions.into_iter().filter(|i| *i > first));
    Answered {
        rows: run,
        opens_turn,
    }
}

/// Whether a declined reply at `at` (with nothing in it) answered a message the person sent that
/// opened its turn: then that message is what was declined, and it is left out from now on.
pub fn answers_prompt<M: Borrow<Message>>(messages: &[M], at: usize) -> bool {
    let answered = answered(messages, at);
    !answered.rows.is_empty() && answered.opens_turn
}

/// The messages every request leaves out (true at their index): each declined reply with nothing
/// in it and the input it answered directly. They stay stored and shown. Dropping such an exchange
/// leaves everything before it as it was sent, so the thinking blocks of earlier steps stay valid.
///
/// A reply stored before 1.6.1 is left out only while no later step carries signed thinking: such
/// a step was written with that prompt in its request. Once a request leaves it out, [`mark`]
/// keeps it so for the steps that follow.
pub fn left_out<M: Borrow<Message>>(messages: &[M]) -> Vec<bool> {
    let mut out = vec![false; messages.len()];
    let mut thinking_after = false;
    for at in (0..messages.len()).rev() {
        let Message::Assistant(step) = messages[at].borrow() else {
            continue;
        };
        if empty_refusal(step) && (marked(step) || !thinking_after) {
            out[at] = true;
            for before in answered(messages, at).rows {
                out[before] = true;
            }
        }
        thinking_after |= signed_thinking(step);
    }
    out
}

/// Of the replies [`left_out`] leaves out, those no request has marked yet (stored before 1.6.1
/// or imported): their indexes.
pub fn unmarked<M: Borrow<Message>>(messages: &[M], left_out: &[bool]) -> Vec<usize> {
    (0..messages.len())
        .filter(|at| left_out[*at])
        .filter(|at| matches!(messages[*at].borrow(), Message::Assistant(step) if !marked(step)))
        .collect()
}

/// Marks a declined reply as left out for good ([`left_out`]).
pub fn mark(step: &mut AssistantData) {
    step.extra.insert(MARK.into(), Value::Bool(true));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user(text: &str) -> Message {
        Message::decode("user", &json!({"time": {"created": 1}, "text": text}))
    }

    fn synthetic(text: &str) -> Message {
        Message::decode("synthetic", &json!({"time": {"created": 1}, "text": text}))
    }

    fn context(text: &str) -> Message {
        Message::decode(
            "synthetic",
            &json!({"time": {"created": 1}, "text": text, "description": crate::hooks::fire::CONTEXT}),
        )
    }

    fn compacted(status: &str) -> Message {
        Message::decode(
            "compaction",
            &json!({"time": {"created": 1}, "status": status, "reason": "auto", "summary": "S"}),
        )
    }

    fn system() -> Message {
        Message::decode("system", &json!({"time": {"created": 1}, "text": "rules"}))
    }

    fn idle() -> Message {
        Message::decode(
            "idle",
            &json!({"time": {"created": 1}, "outcome": "failed"}),
        )
    }

    fn step(finish: &str, content: Value, error: Option<&str>) -> Message {
        let mut data = json!({"time": {"created": 1, "completed": 2}, "agent": "build",
            "model": {"id": "claude-haiku-5-5", "providerID": "anysola"},
            "content": content, "finish": finish});
        if let Some(kind) = error {
            data["error"] = json!({"type": kind, "message": "declined"});
        }
        Message::decode("assistant", &data)
    }

    fn declined() -> Message {
        step("content-filter", json!([]), Some(KIND))
    }

    fn old_declined() -> Message {
        step("content-filter", json!([]), None)
    }

    fn answer(text: &str) -> Message {
        step("stop", json!([{"type": "text", "text": text}]), None)
    }

    fn thought(text: &str) -> Message {
        step(
            "stop",
            json!([{"type": "reasoning", "text": "hm", "native": {"signature": "sig"}},
                   {"type": "text", "text": text}]),
            None,
        )
    }

    fn left(messages: &[Message]) -> Vec<usize> {
        let out = left_out(messages);
        (0..out.len()).filter(|i| out[*i]).collect()
    }

    #[test]
    fn categories_in_words() {
        let of = |c: Option<&str>| {
            Refusal::from_details(Some(&json!({"type": "refusal", "category": c})))
                .message("Claude Haiku 5.5", Declined::Prompt)
        };
        assert_eq!(
            of(Some("cyber")),
            "Claude Haiku 5.5 declined to answer this message, citing cybersecurity. It won’t be sent again in this session; rephrase it, or start a new session if this keeps happening."
        );
        assert!(of(Some("bio")).contains(", citing biology."));
        assert!(of(Some("frontier_llm")).contains(", citing AI model development."));
        assert!(of(Some("general_harms")).contains(", citing the usage policy."));
        assert!(of(Some("reasoning_extraction")).contains(", citing reasoning extraction."));
        // Null, missing, empty and unknown say nothing about a category.
        for none in [of(None), of(Some("")), of(Some("something_new"))] {
            assert!(
                none.starts_with("Claude Haiku 5.5 declined to answer this message. It won’t"),
                "{none}"
            );
        }
        assert_eq!(Refusal::from_details(None), Refusal::default());
        assert_eq!(
            Refusal::from_details(Some(&json!({"category": "bio", "explanation": " why "}))),
            Refusal {
                category: Some("bio".into()),
                explanation: Some("why".into())
            }
        );
    }

    #[test]
    fn a_declined_prompt_and_its_reply_are_left_out() {
        let messages = vec![
            user("hi"),
            answer("hello"),
            idle(),
            user("bad"),
            declined(),
            idle(),
            user("next"),
        ];
        assert_eq!(left(&messages), vec![3, 4]);
        assert!(answers_prompt(&messages, 4));
    }

    #[test]
    fn what_came_with_the_prompt_goes_with_it() {
        // A steered note promoted with the prompt, a hook's context after it, an instruction update
        // recorded before the step: the update stays, the rest goes. What came before the prompt
        // (a background completion, a plan reminder, a session-start hook's context) stays, and so
        // does a background completion promoted after it.
        let messages = vec![
            answer("earlier"),
            idle(),
            synthetic("background subagent done"),
            synthetic("plan reminder"),
            context("session start"),
            user("queued"),
            user("steered"),
            synthetic("background shell done"),
            context("hook context"),
            system(),
            declined(),
        ];
        assert_eq!(left(&messages), vec![5, 6, 8, 10]);
        assert!(answers_prompt(&messages, 10));
    }

    #[test]
    fn a_steer_after_a_step_is_left_out_but_the_turn_is_not_a_declined_prompt() {
        let tool = step(
            "tool-calls",
            json!([{"type": "tool", "id": "t", "name": "shell",
                    "state": {"status": "completed", "input": {}, "content": [{"type": "text", "text": "x"}]},
                    "time": {"created": 1}}]),
            None,
        );
        let messages = vec![
            user("build it"),
            tool,
            user("also this"),
            synthetic("background shell done"),
            declined(),
        ];
        assert_eq!(left(&messages), vec![2, 4]);
        assert!(!answers_prompt(&messages, 4));
    }

    #[test]
    fn a_turn_a_completion_woke_keeps_its_input() {
        let messages = vec![
            answer("earlier"),
            idle(),
            synthetic("background shell done"),
            declined(),
        ];
        assert_eq!(left(&messages), vec![3]);
        assert!(!answers_prompt(&messages, 3));
    }

    #[test]
    fn a_compaction_written_in_the_declined_turn_goes_with_the_prompt() {
        let messages = vec![
            answer("earlier"),
            idle(),
            user("bad"),
            compacted("completed"),
            declined(),
        ];
        assert_eq!(left(&messages), vec![2, 3, 4]);
        // One before the prompt, and a failed one, stay.
        let messages = vec![
            compacted("completed"),
            user("bad"),
            compacted("failed"),
            declined(),
        ];
        assert_eq!(left(&messages), vec![1, 3]);
    }

    #[test]
    fn a_refusal_after_tool_steps_drops_nothing_before_it() {
        let tool = step(
            "tool-calls",
            json!([{"type": "tool", "id": "t", "name": "read",
                    "state": {"status": "completed", "input": {}, "content": [{"type": "text", "text": "x"}]},
                    "time": {"created": 1}}]),
            None,
        );
        let messages = vec![user("go"), tool, declined()];
        assert_eq!(left(&messages), vec![2]);
        assert!(!answers_prompt(&messages, 2));
        // A reply with output is not left out, nor what it answered.
        let partial = step(
            "content-filter",
            json!([{"type": "text", "text": "Sure, first"}]),
            None,
        );
        assert_eq!(left(&[user("go"), partial]), Vec::<usize>::new());
        // A continuation note before it is no message of the person's: it stays.
        let messages = vec![
            user("go"),
            answer("cut"),
            synthetic("Continue."),
            declined(),
        ];
        assert_eq!(left(&messages), vec![3]);
        assert!(!answers_prompt(&messages, 3));
    }

    #[test]
    fn an_old_reply_stays_once_signed_thinking_followed_it() {
        // Stored before 1.6.1: left out while nothing signed came after it.
        let messages = vec![user("bad"), old_declined(), idle(), user("next")];
        assert_eq!(left(&messages), vec![0, 1]);
        assert_eq!(unmarked(&messages, &left_out(&messages)), vec![1]);
        // An old version answered later with that prompt in the request: it stays.
        let messages = vec![
            user("bad"),
            old_declined(),
            idle(),
            user("next"),
            thought("answer"),
        ];
        assert_eq!(left(&messages), Vec::<usize>::new());
        // Marked, or recorded by 1.6.1: left out whatever follows.
        let Message::Assistant(mut old) = old_declined() else {
            unreachable!()
        };
        mark(&mut old);
        let messages = vec![
            user("bad"),
            Message::Assistant(old),
            idle(),
            user("next"),
            thought("answer"),
        ];
        assert_eq!(left(&messages), vec![0, 1]);
        assert!(unmarked(&messages, &left_out(&messages)).is_empty());
        let messages = vec![user("bad"), declined(), user("next"), thought("answer")];
        assert_eq!(left(&messages), vec![0, 1]);
    }
}
