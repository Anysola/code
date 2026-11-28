//! `/btw`: a side question about a session, answered from its conversation so far and never
//! stored in it. One per session at a time; asking again cancels the one in flight.

use std::sync::atomic::{AtomicU64, Ordering};

use anysola_core::api::{AsideError, AsideKind, CancellationToken};
use serde_json::{Value, json};
use tauri::{AppHandle, State};

use crate::{Answer, App, core_for};

/// Tells one side question from the next of the same session.
static ASKED: AtomicU64 = AtomicU64::new(0);

/// `AsideResult` in bridge.ts: the answer, or why there is none.
fn error_json(error: &AsideError) -> Value {
    json!({"error": error})
}

#[tauri::command]
pub async fn aside(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
    question: String,
) -> Answer {
    let question = question.trim().to_string();
    if question.is_empty() {
        return Ok(error_json(&AsideError {
            kind: AsideKind::Other,
            message: "Type a question after /btw.".into(),
        }));
    }
    // Registered before the folder's core is awaited: closing the card while it starts cancels
    // this question too (and no request goes out).
    let n = ASKED.fetch_add(1, Ordering::Relaxed);
    let cancel = CancellationToken::new();
    // A newer question replaces the one in flight: that one ends as cancelled.
    if let Some((_, before)) = app
        .asides
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(session_id.clone(), (n, cancel.clone()))
    {
        before.cancel();
    }
    let forget = || {
        let mut asides = app.asides.lock().unwrap_or_else(|e| e.into_inner());
        if asides.get(&session_id).is_some_and(|(at, _)| *at == n) {
            asides.remove(&session_id);
        }
    };
    let core = match core_for(&handle, &app, &directory).await {
        Ok(core) => core,
        Err(message) => {
            forget();
            return Ok(error_json(&AsideError {
                kind: AsideKind::Other,
                message,
            }));
        }
    };
    // `Core::aside` returns `cancelled` at once for a token cancelled meanwhile.
    let answer = core.aside(&session_id, &question, &cancel).await;
    forget();
    Ok(match answer {
        Ok(text) => json!({"text": text}),
        Err(error) => error_json(&error),
    })
}

/// Cancels the session's side question in flight; false when none was.
#[tauri::command]
pub async fn aside_cancel(app: State<'_, App>, session_id: String) -> Answer {
    let pending = app
        .asides
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&session_id);
    Ok(json!(pending.map(|(_, cancel)| cancel.cancel()).is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_error_reaches_the_window_with_its_kind() {
        let error = AsideError {
            kind: AsideKind::RateLimit,
            message: "Too many requests.".into(),
        };
        assert_eq!(
            error_json(&error),
            json!({"error": {"kind": "rate_limit", "message": "Too many requests."}})
        );
    }
}
