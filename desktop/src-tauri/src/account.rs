//! Signing in and out, the account's plan and windows (`GET /me`), its devices, usage and banked
//! resets.

use std::collections::HashMap;
use std::sync::LazyLock;

use anysola_core::gateway::account::Method;
use anysola_core::gateway::limits::Limits;

use anysola_core::gateway::{Gateway, Token, env_token};
use serde_json::{Value, json};
use tauri::State;
use tokio::sync::watch;

use crate::{Answer, App, text};

/// Counts sign-ins started or cancelled: a wait for an earlier one stops when it moves.
static ATTEMPT: LazyLock<watch::Sender<u64>> = LazyLock::new(|| watch::channel(0).0);

const ENV_REJECTED: &str = "ANYSOLA_TOKEN is set on this computer and was rejected. Update or remove it, then start the app again.";

const OFFLINE: &str = "Can’t reach Anysola. Check your connection.";

/// The core's account errors as the window says them.
fn for_window(error: String) -> String {
    if error.starts_with("Cannot reach ") || error == "Login failed: network error" {
        return OFFLINE.into();
    }
    if let Some(why) = error.strip_prefix("Could not start login: ") {
        return format!("Couldn’t start signing in: {why}");
    }
    error
}

#[tauri::command]
pub async fn auth_status(app: State<'_, App>) -> Answer {
    // The app's data didn't open: signing in can't help.
    if let Some(fault) = app.fault() {
        return Ok(json!({"signedIn": false, "fault": fault}));
    }
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
    let login = Gateway::new(None)
        .start_device_login()
        .await
        .map_err(for_window)?;
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

/// The signed-in gateway, or `None` when there is no token.
fn gateway(app: &App) -> Result<Option<(Gateway, bool)>, String> {
    Ok(Token::resolve(app.db()?).map_err(text)?.map(|token| {
        let env = matches!(token, Token::Env(_));
        (Gateway::new(Some(token)), env)
    }))
}

const SIGNED_OUT: &str = "You’re signed out. Sign in again.";

/// An account route that must succeed; its errors as the window shows them.
async fn call(app: &App, method: Method, route: &str) -> Result<Value, String> {
    let (gateway, env) = gateway(app)?.ok_or(SIGNED_OUT)?;
    let reply = gateway
        .account_call(method, route, None)
        .await
        .map_err(for_window)?;
    match reply.status {
        401 if env => Err(ENV_REJECTED.into()),
        401 => Err("Your sign-in has expired. Sign in again.".into()),
        404 => Err("Anysola doesn’t offer this yet. Update the app, or try again later.".into()),
        _ if !reply.ok() => Err(reply.error().1),
        _ => Ok(reply.body),
    }
}

/// `GET /me` as the window's `Account`.
pub(crate) fn account_json(body: &Value) -> Value {
    let limits = body.get("limits").filter(|l| l.is_object());
    let now = anysola_core::time::now_ms();
    let bank: Vec<Value> = limits
        .and_then(Limits::from_wire)
        .map(|l| {
            l.bank_at(now)
                .into_iter()
                .map(|b| {
                    json!({
                        "id": b.id,
                        "expiresAt": b.expires_at.map(anysola_core::time::iso),
                        "scope": if b.both { "both" } else { "five_hour" },
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let blocked = limits
        .and_then(|l| l.get("blockedBy"))
        .and_then(Value::as_str)
        .filter(|w| matches!(*w, "five_hour" | "weekly"));
    let event = body.get("event").filter(|e| e.is_object()).map(|e| {
        let mode = match e.get("mode").and_then(Value::as_str) {
            Some("unlimited") => "unlimited",
            _ => "multiplier",
        };
        json!({
            "mode": mode,
            "label": e.get("label").and_then(Value::as_str).unwrap_or_default(),
            "until": e.get("until").and_then(Value::as_str),
        })
    });
    json!({
        "expired": false,
        "plan": body.get("plan"),
        "included": !body.get("access").is_some_and(Value::is_null),
        "fiveHour": limits.and_then(|l| l.get("fiveHour")),
        "weekly": limits.and_then(|l| l.get("weekly")),
        "blockedBy": blocked,
        "bank": bank,
        "requestsToday": body.pointer("/usageToday/requests").and_then(Value::as_f64).map(|n| n as u64),
        "event": event,
    })
}

/// The plan and both windows; `expired` when the token no longer works.
#[tauri::command]
pub async fn account(app: State<'_, App>) -> Answer {
    let Some((gateway, env)) = gateway(&app)? else {
        return Ok(json!({"expired": true}));
    };
    let reply = gateway
        .account_call(Method::GET, "/me", None)
        .await
        .map_err(for_window)?;
    if reply.status == 401 {
        return Ok(json!({"expired": true, "rejected": env.then_some(ENV_REJECTED)}));
    }
    if !reply.ok() {
        return Err(format!(
            "Anysola couldn’t load your account (HTTP {}). Try again in a moment.",
            reply.status
        ));
    }
    Ok(account_json(&reply.body))
}

/// `GET /tokens` as the window's `Device` list.
fn devices_json(body: &Value) -> Value {
    let devices: Vec<Value> = body
        .get("tokens")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let field = |key: &str| t.get(key).and_then(Value::as_str);
            let name = field("name")
                .filter(|n| !n.is_empty())
                .unwrap_or("Unnamed device");
            Some(json!({
                "id": field("id")?,
                "name": name,
                "client": field("client").unwrap_or_default(),
                "kind": if field("kind") == Some("ci") { "ci" } else { "device" },
                "createdAt": field("createdAt").unwrap_or_default(),
                "lastUsedAt": field("lastUsedAt"),
                "expiresAt": field("expiresAt"),
                "current": t.get("current") == Some(&Value::Bool(true)),
            }))
        })
        .collect();
    Value::Array(devices)
}

/// The account's signed-in devices and CI tokens.
#[tauri::command]
pub async fn devices(app: State<'_, App>) -> Answer {
    Ok(devices_json(&call(&app, Method::GET, "/tokens").await?))
}

/// `encodeURIComponent`, for an id in a path.
fn encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Signs a device out; never this one, which signing out does properly.
#[tauri::command]
pub async fn device_revoke(app: State<'_, App>, id: String) -> Answer {
    let list = devices_json(&call(&app, Method::GET, "/tokens").await?);
    let device = list
        .as_array()
        .and_then(|l| l.iter().find(|d| d["id"] == id.as_str()))
        .ok_or("That device is already signed out.")?;
    if device["current"] == true {
        return Err("This is the device you’re on. Use Sign out instead.".into());
    }
    call(
        &app,
        Method::POST,
        &format!("/tokens/{}/revoke", encode(&id)),
    )
    .await?;
    Ok(devices_json(&call(&app, Method::GET, "/tokens").await?))
}

/// `GET /usage?days=` as the window's `Usage`, with the models' names where the core knows them.
fn usage_json(body: &Value, days: u32, names: &HashMap<String, String>) -> Value {
    let number = |v: Option<&Value>| v.and_then(Value::as_f64).unwrap_or(0.0) as u64;
    let rows = |key: &str| {
        body.get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let by_day: Vec<Value> = rows("byDay")
        .iter()
        .map(|r| json!({"date": r.get("date").and_then(Value::as_str).unwrap_or_default(), "requests": number(r.get("requests"))}))
        .collect();
    let by_model: Vec<Value> = rows("byModel")
        .iter()
        .map(|r| {
            let model = r.get("model").and_then(Value::as_str).unwrap_or_default();
            let id = model.strip_prefix("anysola/").unwrap_or(model);
            let name = names.get(id).map_or(id, String::as_str);
            json!({"model": model, "name": name, "requests": number(r.get("requests"))})
        })
        .collect();
    json!({
        "days": body.get("days").and_then(Value::as_u64).unwrap_or(days as u64),
        "requests": number(body.pointer("/totals/requests")),
        "cacheHitRate": body.pointer("/totals/cacheHitRate").and_then(Value::as_f64),
        "today": number(body.pointer("/today/requests")),
        "byDay": by_day,
        "byModel": by_model,
    })
}

/// Requests over the last `days` days (1 to 90).
#[tauri::command]
pub async fn usage(app: State<'_, App>, days: u32) -> Answer {
    let days = days.clamp(1, 90);
    let body = call(&app, Method::GET, &format!("/usage?days={days}")).await?;
    // Names from any running core: they all read the same catalog.
    let mut names = HashMap::new();
    if let Some((_, core)) = app.cores_running().await.into_iter().next() {
        for model in core.loaded().models.iter() {
            names.insert(model.id.clone(), model.name.clone());
        }
    }
    Ok(usage_json(&body, days, &names))
}

/// Spends the banked reset that expires first, then answers as `account` does.
#[tauri::command]
pub async fn limits_redeem(app: State<'_, App>) -> Answer {
    let (gateway, _) = gateway(&app)?.ok_or(SIGNED_OUT)?;
    let reply = gateway
        .account_call(Method::POST, "/limits/redeem", None)
        .await
        .map_err(for_window)?;
    match reply.status {
        409 => return Err(reply.error().1),
        401 => return Err("Your sign-in has expired. Sign in again.".into()),
        _ if !reply.ok() => return Err(format!("The reset wasn’t used: {}", reply.error().1)),
        _ => {}
    }
    account(app).await
}

/// Revokes the stored login and removes it; `ANYSOLA_TOKEN` stays in use.
#[tauri::command]
pub async fn logout(app: State<'_, App>) -> Answer {
    let db = app.db()?;
    if let Some((_, token)) = db.anysola_credential().map_err(text)? {
        Gateway::new(Some(Token::Login(token)))
            .revoke_self()
            .await
            .map_err(|e| match for_window(e) {
                e if e == OFFLINE => format!("{OFFLINE} You’re still signed in."),
                e => format!("Couldn’t sign out ({e}). You’re still signed in; try again."),
            })?;
        db.remove_integration_credentials("anysola").map_err(text)?;
    }
    app.drop_cores().await;
    let token = Token::resolve(db).map_err(text)?;
    Ok(json!({"signedIn": token.is_some()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn me_becomes_the_account() {
        let body = json!({
            "plan": "pro", "access": "paid",
            "limits": {"plan": "pro", "allowed": false, "blockedBy": "five_hour",
                       "fiveHour": {"percent": 100, "resetsAt": "2099-01-01T00:00:00Z"},
                       "weekly": {"percent": 40, "resetsAt": null},
                       "bank": [{"id": "b2", "expiresAt": "2099-03-01T00:00:00Z", "scope": "both"},
                                {"id": "b1", "expiresAt": "2099-02-01T00:00:00Z", "scope": "five_hour"},
                                {"id": "old", "expiresAt": "2000-01-01T00:00:00Z", "scope": "both"}]},
            "usageToday": {"requests": 12},
            "event": {"mode": "unlimited", "label": "Launch week", "until": null},
        });
        let account = account_json(&body);
        assert_eq!(account["blockedBy"], "five_hour");
        assert_eq!(account["requestsToday"], 12);
        assert_eq!(account["fiveHour"]["percent"], 100);
        assert_eq!(
            account["bank"].as_array().unwrap().len(),
            2,
            "an expired reset is left out"
        );
        assert_eq!(
            account["bank"][0],
            json!({"id": "b1", "expiresAt": "2099-02-01T00:00:00.000Z", "scope": "five_hour"})
        );
        assert_eq!(
            account["event"],
            json!({"mode": "unlimited", "label": "Launch week", "until": null})
        );
        let plain = account_json(&json!({"plan": "free", "access": null, "event": null}));
        assert_eq!(plain["included"], false);
        assert_eq!(plain["event"], Value::Null);
        assert_eq!(plain["bank"], json!([]));
        assert_eq!(plain["requestsToday"], Value::Null);
    }

    #[test]
    fn devices_and_usage() {
        let devices = devices_json(&json!({"tokens": [
            {"id": "t1", "name": "", "client": "anysola-code/1.0.6", "kind": "device",
             "createdAt": "2026-11-01T00:00:00Z", "lastUsedAt": null, "expiresAt": null, "current": true},
            {"id": "t2", "name": "CI", "client": "ci", "kind": "ci", "createdAt": "x",
             "lastUsedAt": "y", "expiresAt": "z", "current": false},
        ]}));
        assert_eq!(devices[0]["name"], "Unnamed device");
        assert_eq!(devices[0]["current"], true);
        assert_eq!(devices[1]["kind"], "ci");
        let names = HashMap::from([(
            "claude-sonnet-5-5".to_string(),
            "Claude Sonnet 5.5".to_string(),
        )]);
        let usage = usage_json(
            &json!({"days": 7, "totals": {"requests": 42, "cacheHitRate": 0.5},
                    "byDay": [{"date": "2026-11-16", "requests": 2}],
                    "byModel": [{"model": "claude-sonnet-5-5", "requests": 40}, {"model": "x", "requests": 2}],
                    "today": {"requests": 1}}),
            7,
            &names,
        );
        assert_eq!(usage["requests"], 42);
        assert_eq!(usage["cacheHitRate"], 0.5);
        assert_eq!(usage["byModel"][0]["name"], "Claude Sonnet 5.5");
        assert_eq!(usage["byModel"][1]["name"], "x");
        assert_eq!(usage["byDay"][0]["date"], "2026-11-16");
        assert_eq!(usage["today"], 1);
    }

    #[test]
    fn errors_read_for_the_window() {
        assert_eq!(
            for_window("Cannot reach https://api.anysola.com".into()),
            OFFLINE
        );
        assert_eq!(for_window("Login failed: network error".into()), OFFLINE);
        assert_eq!(
            for_window("Could not start login: busy".into()),
            "Couldn’t start signing in: busy"
        );
        assert_eq!(for_window("Sign-in was denied.".into()), "Sign-in was denied.");
    }

    #[test]
    fn ids_are_encoded() {
        assert_eq!(encode("a b/c"), "a%20b%2Fc");
    }
}
