//! Forking a session and its budget: a fork copies the conversation (all of it, or what came
//! before one message) into a new session listed like any other; the budget is the share of the
//! 5-hour window the session may use, read back with what it has used so far.
//!
//! A fork works in the same folder as its source: no file is put back to how it was at the
//! message it was forked from. It copies what the window shows: with a rewind staged, the
//! conversation up to the rewind's message, as if the rewind were final.

use anysola_core::api::{Core, CoreError};
use anysola_core::events::UsageSnapshot;
use anysola_core::session::message::Message;
use anysola_core::usage::MAX_BUDGET;
use serde_json::{Value, json};
use tauri::{AppHandle, State};

use crate::{Answer, App, core_for, session_json, text};

/// A session's usage as the window reads it, from a command and in the `usage` event alike.
/// `turn`, `total` and `budget` are percents of the 5-hour window; `fiveHour` and `weekly` are
/// the account's windows after the last model call this process saw, null before one.
pub(crate) fn usage_json(snapshot: &UsageSnapshot) -> Value {
    json!({
        "sessionId": snapshot.session_id,
        "turn": snapshot.turn,
        "total": snapshot.total,
        "turns": snapshot.turns,
        "budget": snapshot.budget,
        "fiveHour": snapshot.latest.as_ref().map(|u| u.five_hour.percent),
        "weekly": snapshot.latest.as_ref().map(|u| u.weekly.percent),
    })
}

/// A budget the core would take: a whole percent from 1 to [`MAX_BUDGET`], or `null` for none.
/// The core clears a budget out of range; here anything else is refused (a fraction, a string,
/// a number out of range), so a slip never removes the limit. NaN arrives as `null` from
/// JavaScript: the window checks its number before sending it.
fn budget_arg(budget: &Value) -> Result<Option<u64>, String> {
    let refused =
        || format!("A budget is a whole percent, 1 to {MAX_BUDGET}, of the 5-hour window.");
    match budget {
        Value::Null => Ok(None),
        Value::Number(n) => match n.as_u64() {
            Some(b) if (1..=MAX_BUDGET).contains(&b) => Ok(Some(b)),
            _ => Err(refused()),
        },
        _ => Err(refused()),
    }
}

/// The session as a root the window lists; a subagent's is not forked or budgeted on its own.
fn root(core: &Core, session_id: &str) -> Result<(), String> {
    let row = core
        .session(session_id)
        .map_err(text)?
        .ok_or("This session no longer exists.")?;
    if row.parent_id.is_some() {
        return Err("A subagent’s session can’t be changed on its own.".into());
    }
    Ok(())
}

/// Where the fork ends: before `before`, a message the user sent that the window shows (a
/// `user` item of the transcript, as `revert` and `resend` take), never the first one (nothing
/// comes before it); with a rewind staged at `staged`, no later than it. `ids`: the session's
/// user messages, oldest first. The core would fork all of it for an id it doesn't know.
fn fork_point(
    ids: &[String],
    before: Option<&str>,
    staged: Option<&str>,
) -> Result<Option<String>, String> {
    let at = |id: &str| ids.iter().position(|x| x == id);
    let shown = match staged {
        // Rewinds start at a user message; one elsewhere is not cut here, and not guessed at.
        Some(staged) => at(staged).ok_or("Redo the rewind or send a message before branching.")?,
        None => ids.len(),
    };
    match before {
        None => Ok(staged.map(str::to_string)),
        Some(before) => match at(before) {
            Some(0) => Err("There is nothing before the first message to fork.".into()),
            Some(i) if i < shown => Ok(Some(before.to_string())),
            _ => Err("That message is no longer in the session.".into()),
        },
    }
}

fn fork_error(error: CoreError) -> String {
    match error {
        CoreError::ForkEmpty(_) => "There is nothing before that message to fork.".into(),
        CoreError::SessionNotFound => "This session no longer exists.".into(),
        other => crate::plain_error(other),
    }
}

/// Forks the session into a new one, whole or up to (not including) the user message `before`;
/// with a rewind staged, only what it leaves (see [`fork_point`]). Refused while a turn runs:
/// its reply is half written. Returns the new session's row; the core's `created` event only
/// refreshes the list.
#[tauri::command]
pub async fn fork_session(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
    before: Option<String>,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    root(&core, &session_id)?;
    if core.is_busy(&session_id) {
        return Err("Wait for the current turn to end, or stop it, before branching.".into());
    }
    let ids: Vec<String> = core
        .messages(&session_id)
        .map_err(text)?
        .into_iter()
        .filter(|(_, m)| matches!(m, Message::User(_)))
        .map(|(id, _)| id)
        .collect();
    let staged = core.revert_boundary(&session_id).map_err(text)?;
    let before = fork_point(&ids, before.as_deref(), staged.as_deref())?;
    let row = core
        .fork_session(&session_id, before.as_deref())
        .map_err(fork_error)?;
    Ok(session_json(&row))
}

/// Sets (`budget` 1 to 100) or clears (`null`) the session's budget. Returns the usage with
/// the new budget; a `usage` event carries the same.
#[tauri::command]
pub async fn set_budget(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
    budget: Value,
) -> Answer {
    let budget = budget_arg(&budget)?;
    let core = core_for(&handle, &app, &directory).await?;
    root(&core, &session_id)?;
    let snapshot = core.set_budget(&session_id, budget).map_err(text)?;
    Ok(usage_json(&snapshot))
}

/// What the session has used of the 5-hour window, and its budget.
#[tauri::command]
pub async fn session_usage(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    root(&core, &session_id)?;
    Ok(usage_json(&core.usage(&session_id)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anysola_core::gateway::limits::{UsageEvent, Window};

    #[test]
    fn budgets_out_of_range_or_not_numbers_are_refused_not_cleared() {
        assert_eq!(budget_arg(&Value::Null), Ok(None));
        assert_eq!(budget_arg(&json!(1)), Ok(Some(1)));
        assert_eq!(budget_arg(&json!(MAX_BUDGET)), Ok(Some(MAX_BUDGET)));
        for bad in [
            json!(0),
            json!(MAX_BUDGET + 1),
            json!(-5),
            json!(2.5),
            json!("10"),
            json!([]),
        ] {
            assert!(budget_arg(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_fork_point_is_a_shown_message_after_the_first() {
        let ids: Vec<String> = ["msg_a", "msg_b", "msg_c"].map(String::from).into();
        assert_eq!(fork_point(&ids, None, None), Ok(None));
        assert_eq!(
            fork_point(&ids, Some("msg_b"), None),
            Ok(Some("msg_b".into()))
        );
        assert_eq!(
            fork_point(&ids, Some("msg_a"), None),
            Err("There is nothing before the first message to fork.".into())
        );
        assert_eq!(
            fork_point(&ids, Some("msg_gone"), None),
            Err("That message is no longer in the session.".into())
        );
        // Rewound to msg_c: the whole fork stops there, and msg_c and after can't be picked.
        assert_eq!(
            fork_point(&ids, None, Some("msg_c")),
            Ok(Some("msg_c".into()))
        );
        assert_eq!(
            fork_point(&ids, Some("msg_b"), Some("msg_c")),
            Ok(Some("msg_b".into()))
        );
        assert_eq!(
            fork_point(&ids, Some("msg_c"), Some("msg_c")),
            Err("That message is no longer in the session.".into())
        );
        assert!(fork_point(&ids, None, Some("msg_x")).is_err());
    }

    #[test]
    fn an_empty_fork_says_so_plainly() {
        assert_eq!(
            fork_error(CoreError::ForkEmpty("ses_1".into())),
            "There is nothing before that message to fork."
        );
    }

    #[test]
    fn usage_carries_the_session_totals_and_budget() {
        let mut snapshot = UsageSnapshot {
            session_id: "ses_1".into(),
            latest: None,
            turn: 0.0,
            total: 0.0,
            turns: 0,
            budget: None,
        };
        assert_eq!(
            usage_json(&snapshot),
            json!({"sessionId": "ses_1", "turn": 0.0, "total": 0.0, "turns": 0, "budget": null,
                   "fiveHour": null, "weekly": null})
        );
        snapshot.latest = Some(UsageEvent {
            five_hour: Window {
                percent: 42,
                ..Default::default()
            },
            weekly: Window {
                percent: 7,
                ..Default::default()
            },
            five_hour_percent_used: 1.5,
        });
        snapshot.turn = 1.5;
        snapshot.total = 4.0;
        snapshot.turns = 2;
        snapshot.budget = Some(20);
        assert_eq!(
            usage_json(&snapshot),
            json!({"sessionId": "ses_1", "turn": 1.5, "total": 4.0, "turns": 2, "budget": 20,
                   "fiveHour": 42, "weekly": 7})
        );
    }
}
