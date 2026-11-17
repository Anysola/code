//! Signing in and out, and the account's plan and windows (`GET /me`), as the CLI's `login`,
//! `logout` and `status` do them.

use anysola_core::gateway::account::Method;
use std::sync::LazyLock;

use anysola_core::gateway::{Gateway, Token, env_token};
use serde_json::{Value, json};
use tauri::State;
use tokio::sync::watch;

use crate::{Answer, App, text};

/// Counts sign-ins started or cancelled: a wait for an earlier one stops when it moves.
static ATTEMPT: LazyLock<watch::Sender<u64>> = LazyLock::new(|| watch::channel(0).0);

const ENV_REJECTED: &str = "ANYSOLA_TOKEN is set on this computer and was rejected. Update or remove it, then start the app again.";

/// The core words its sign-in errors for the terminal.
fn for_window(error: String) -> String {
    error.replace("Run `anysola login` again.", "Sign in again.")
}

#[tauri::command]
pub async fn auth_status(app: State<'_, App>) -> Answer {
    let token = Token::resolve(app.db()?).map_err(text)?;
    Ok(json!({"signedIn": token.is_some()}))
}

/// Starts the device sign-in: the window shows the code and opens the address.
#[tauri::command]
pub async fn login_start(app: State<'_, App>) -> Answer {
    // The variable wins over any stored login: signing in would change nothing.
    if env_token().is_some() {
        return Err(ENV_REJECTED.into());
    }
    let login = Gateway::new(None).start_device_login().await?;
    let started = json!({"url": login.url, "code": login.user_code});
    ATTEMPT.send_modify(|n| *n += 1);
    *app.login.lock().await = Some(login);
    Ok(started)
}

/// Gives the sign-in up: the wait returns `signedIn: false`.
#[tauri::command]
pub async fn login_cancel(app: State<'_, App>) -> Answer {
    ATTEMPT.send_modify(|n| *n += 1);
    *app.login.lock().await = None;
    Ok(Value::Null)
}

/// Waits for the browser approval and stores the token.
#[tauri::command]
pub async fn login_wait(app: State<'_, App>) -> Answer {
    let mut attempt = ATTEMPT.subscribe();
    let Some(login) = app.login.lock().await.take() else {
        return Ok(json!({"signedIn": false}));
    };
    let gateway = Gateway::new(None);
    let credential = tokio::select! {
        done = gateway.finish_device_login(&login) => done.map_err(for_window)?,
        _ = attempt.changed() => return Ok(json!({"signedIn": false})),
    };
    let db = app.db()?;
    // One stored login: an expired one left beside it would be what the next sign-out uncovers.
    if let Some((_, old)) = db.anysola_credential().map_err(text)? {
        let _ = Gateway::new(Some(Token::Login(old))).revoke_self().await;
    }
    db.remove_integration_credentials("anysola").map_err(text)?;
    db.insert_anysola_login(&credential).map_err(text)?;
    app.drop_cores().await;
    Ok(json!({"signedIn": true}))
}

/// The plan and both windows; `expired` when the token no longer works.
#[tauri::command]
pub async fn account(app: State<'_, App>) -> Answer {
    let Some(token) = Token::resolve(app.db()?).map_err(text)? else {
        return Ok(json!({"expired": true}));
    };
    let env = matches!(token, Token::Env(_));
    let reply = Gateway::new(Some(token))
        .account_call(Method::GET, "/me", None)
        .await?;
    if reply.status == 401 {
        return Ok(json!({"expired": true, "rejected": env.then_some(ENV_REJECTED)}));
    }
    if !reply.ok() {
        return Err(format!(
            "Account details are unavailable (HTTP {}).",
            reply.status
        ));
    }
    let body = reply.body;
    Ok(json!({
        "expired": false,
        "plan": body.get("plan"),
        "included": !body.get("access").is_some_and(Value::is_null),
        "fiveHour": body.pointer("/limits/fiveHour"),
        "weekly": body.pointer("/limits/weekly"),
    }))
}

/// Revokes the stored login and removes it, as `anysola logout`; `ANYSOLA_TOKEN` stays in use.
#[tauri::command]
pub async fn logout(app: State<'_, App>) -> Answer {
    let db = app.db()?;
    if let Some((_, token)) = db.anysola_credential().map_err(text)? {
        Gateway::new(Some(Token::Login(token)))
            .revoke_self()
            .await
            .map_err(|e| format!("Couldn’t revoke the token: {e}. It was kept."))?;
        db.remove_integration_credentials("anysola").map_err(text)?;
    }
    app.drop_cores().await;
    let token = Token::resolve(db).map_err(text)?;
    Ok(json!({"signedIn": token.is_some()}))
}
