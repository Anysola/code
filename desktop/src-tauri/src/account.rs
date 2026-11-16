//! Signing in and out, and the account's plan and windows (`GET /me`), as the CLI's `login`,
//! `logout` and `status` do them.

use anysola_core::gateway::account::Method;
use anysola_core::gateway::{Gateway, Token};
use serde_json::{Value, json};
use tauri::State;

use crate::{Answer, App, text};

#[tauri::command]
pub async fn auth_status(app: State<'_, App>) -> Answer {
    let token = Token::resolve(app.db()?).map_err(text)?;
    Ok(json!({"signedIn": token.is_some()}))
}

/// Starts the device sign-in: the window shows the code and opens the address.
#[tauri::command]
pub async fn login_start(app: State<'_, App>) -> Answer {
    let login = Gateway::new(None).start_device_login().await?;
    let started = json!({"url": login.url, "code": login.user_code});
    *app.login.lock().await = Some(login);
    Ok(started)
}

/// Waits for the browser approval and stores the token.
#[tauri::command]
pub async fn login_wait(app: State<'_, App>) -> Answer {
    let login = app
        .login
        .lock()
        .await
        .take()
        .ok_or("No sign-in is in progress.")?;
    let credential = Gateway::new(None).finish_device_login(&login).await?;
    app.db()?.insert_anysola_login(&credential).map_err(text)?;
    app.drop_cores().await;
    Ok(json!({"signedIn": true}))
}

/// The plan and both windows; `expired` when the token no longer works.
#[tauri::command]
pub async fn account(app: State<'_, App>) -> Answer {
    let Some(token) = Token::resolve(app.db()?).map_err(text)? else {
        return Ok(json!({"expired": true}));
    };
    let reply = Gateway::new(Some(token))
        .account_call(Method::GET, "/me", None)
        .await?;
    if reply.status == 401 {
        return Ok(json!({"expired": true}));
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
    if let Some((id, token)) = db.anysola_credential().map_err(text)? {
        Gateway::new(Some(Token::Login(token)))
            .revoke_self()
            .await
            .map_err(|e| format!("Couldn’t revoke the token: {e}. It was kept."))?;
        db.remove_credential(&id).map_err(text)?;
    }
    app.drop_cores().await;
    let token = Token::resolve(db).map_err(text)?;
    Ok(json!({"signedIn": token.is_some()}))
}
