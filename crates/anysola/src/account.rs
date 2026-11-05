//! Account commands: `login`, `logout`, `status`, `usage`, `devices`, and the sign-in before the
//! first interactive start.

use std::io::IsTerminal;

use anysola_core::gateway::account::{Method, Reply};
use anysola_core::gateway::format::{credits, usage_lines};
use anysola_core::gateway::{Gateway, Token, env_token};
use anysola_core::paths::Paths;
use anysola_core::storage::Db;
use anysola_core::time::short_local;
use serde_json::Value;

use crate::clack;
use crate::cli::{DevicesCommand, UsageArgs, UsageFormat};

const APPS_URL: &str = "https://account.anysola.com/apps";
const NOT_SIGNED_IN: &str = "Not signed in. Run `anysola login`.";
const EXPIRED: &str = "The token was revoked or expired. Run `anysola login --force`.";

fn open_db() -> Result<Db, String> {
    Db::open(&Paths::from_env().database()).map_err(|e| e.to_string())
}

/// `asc_FFFF...FFFF`: the first 8 and last 4 characters.
pub fn mask(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    let head: String = chars.iter().take(8).collect();
    let tail: String = chars[chars.len().saturating_sub(4)..].iter().collect();
    format!("{head}...{tail}")
}

/// Runs the device flow and stores the token; prints the framed progress lines.
pub async fn sign_in(db: &Db) -> Result<(), String> {
    let gateway = Gateway::new(None);
    let spinner = clack::Spinner::start("Starting authorization...");
    let login = match gateway.start_device_login().await {
        Ok(login) => {
            spinner.stop("Authorization started", true);
            login
        }
        Err(error) => {
            spinner.stop("Authentication failed", false);
            return Err(error);
        }
    };
    clack::info(&login.instructions());
    clack::info(&login.url);
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        open_browser(&login.url);
    }
    clack::step("Waiting for authorization...");
    let credential = gateway.finish_device_login(&login).await?;
    db.insert_anysola_login(&credential)
        .map_err(|e| e.to_string())?;
    clack::success("Connected to Anysola");
    Ok(())
}

/// Best effort: the URL is already on screen.
pub fn open_browser(url: &str) {
    let mut command = if cfg!(windows) {
        let mut command = std::process::Command::new("cmd");
        command.args(["/c", "start", ""]);
        command
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else {
        std::process::Command::new("xdg-open")
    };
    let _ = command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Revokes a stored login with its own token.
async fn revoke(token: &str) -> Result<(), String> {
    Gateway::new(Some(Token::Login(token.to_string())))
        .revoke_self()
        .await
}

pub async fn login(force: bool) -> i32 {
    clack::intro("Sign in to Anysola");
    let result = async {
        if env_token().is_some() && !force {
            clack::info("Signed in through ANYSOLA_TOKEN; unset it to use a device login.");
            return Ok("Done");
        }
        let db = open_db()?;
        if let Some((id, token)) = db.anysola_credential().map_err(|e| e.to_string())? {
            if !force {
                clack::info(&format!(
                    "Already signed in ({}). Use `anysola login --force` for a new token.",
                    mask(&token)
                ));
                return Ok("Done");
            }
            if let Err(error) = revoke(&token).await {
                clack::warn(&format!(
                    "Could not revoke the old token ({error}); revoke it at {APPS_URL}"
                ));
            }
            db.remove_credential(&id).map_err(|e| e.to_string())?;
        }
        sign_in(&db).await.map(|()| "Signed in")
    }
    .await;
    match result {
        Ok(outro) => {
            clack::outro(outro);
            0
        }
        Err(error) => {
            clack::failed(&error);
            1
        }
    }
}

pub async fn logout() -> i32 {
    clack::intro("Sign out of Anysola");
    let stored = match open_db().and_then(|db| {
        let stored = db.anysola_credential().map_err(|e| e.to_string())?;
        Ok((db, stored))
    }) {
        Ok(found) => found,
        Err(error) => {
            clack::failed(&error);
            return 1;
        }
    };
    let (db, Some((id, token))) = stored else {
        clack::info(if env_token().is_some() {
            "Signed in through ANYSOLA_TOKEN; unset it to sign out."
        } else {
            "Not signed in."
        });
        clack::outro("Done");
        return 0;
    };
    if let Err(error) = revoke(&token).await {
        clack::error(&format!("Could not revoke the token: {error}"));
        clack::info(&format!(
            "The token was kept. Try again, or revoke it at {APPS_URL}."
        ));
        return 1;
    }
    if let Err(error) = db.remove_credential(&id) {
        clack::failed(&error.to_string());
        return 1;
    }
    if env_token().is_some() {
        clack::info("ANYSOLA_TOKEN is still set and stays in use; unset it to sign out.");
    }
    clack::outro("Signed out. The token is revoked and removed from this machine.");
    0
}

/// The signed-in gateway client, or the "not signed in" text.
fn signed_in() -> Result<Gateway, String> {
    let db = open_db()?;
    match Token::resolve(&db).map_err(|e| e.to_string())? {
        Some(token) => Ok(Gateway::new(Some(token))),
        None => Err(NOT_SIGNED_IN.into()),
    }
}

/// Calls an account route; the error is the line to print.
async fn account_call(method: Method, route: &str) -> Result<Reply, String> {
    let reply = signed_in()?.account_call(method, route, None).await?;
    match reply.status {
        401 => Err(EXPIRED.into()),
        404 => Err("This Anysola server does not offer this yet.".into()),
        _ if !reply.ok() => Err(reply
            .body
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("Request failed (HTTP {})", reply.status))),
        _ => Ok(reply),
    }
}

/// Prints an account command's outcome: stdout on success, the bare error on stderr.
fn report(result: Result<String, String>) -> i32 {
    match result {
        Ok(text) => {
            print!("{text}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn number(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_f64).filter(|n| n.is_finite())
}

fn text(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// The `anysola status` rows from `GET /me`.
fn status_lines(me: &Value) -> Vec<(&'static str, String)> {
    let mut lines = Vec::new();
    if let Some(plan) = text(me.get("plan")) {
        let mut chars = plan.chars();
        let first = chars.next().map(|c| c.to_uppercase().to_string());
        lines.push((
            "Plan",
            format!("{}{}", first.unwrap_or_default(), chars.as_str()),
        ));
    }
    let balance = me.get("credits");
    if let Some(available) = number(balance.and_then(|c| c.get("available"))) {
        let renews = text(balance.and_then(|c| c.get("renewsAt")))
            .map(|at| format!(", renews {}", at.chars().take(10).collect::<String>()))
            .unwrap_or_default();
        lines.push((
            "Credits",
            format!("{} available{renews}", credits(available)),
        ));
    }
    let free = me.get("freeQuota").filter(|f| f.is_object());
    if let Some(daily) = number(free.and_then(|f| f.get("dailyTurns"))) {
        let free = free.expect("dailyTurns came from it");
        let used = number(free.get("turns"))
            .or_else(|| number(free.get("usedTurns")))
            .unwrap_or(0.0);
        lines.push((
            "Free quota",
            format!("{}/{} turns today", credits(used), credits(daily)),
        ));
    }
    if let Some(today) = me.get("usageToday").filter(|t| t.is_object()) {
        let count = |key| credits(number(today.get(key)).unwrap_or(0.0));
        lines.push((
            "Today",
            format!(
                "{} requests · {} credits",
                count("requests"),
                count("credits")
            ),
        ));
    }
    let event = me.get("event");
    if let Some(label) = text(event.and_then(|e| e.get("label"))) {
        let until = text(event.and_then(|e| e.get("until")))
            .and_then(short_local)
            .map(|at| format!(" until {at}"))
            .unwrap_or_default();
        lines.push(("Event", format!("{label}{until}")));
    }
    lines
}

pub async fn status() -> i32 {
    let gateway = match signed_in() {
        Ok(gateway) => gateway,
        Err(error) => return report(Err(error)),
    };
    let token = gateway
        .token()
        .map(|t| t.secret().to_string())
        .unwrap_or_default();
    let reply = match gateway.account_call(Method::GET, "/me", None).await {
        Ok(reply) => reply,
        Err(error) => return report(Err(error)),
    };
    if reply.status == 401 {
        return report(Err(EXPIRED.into()));
    }
    let mut lines = if reply.ok() {
        status_lines(&reply.body)
    } else {
        eprintln!("Could not load status: HTTP {}", reply.status);
        Vec::new()
    };
    lines.push(("Token", mask(&token)));
    let width = lines.iter().map(|(key, _)| key.len()).max().unwrap_or(0) + 2;
    let text: String = lines
        .iter()
        .map(|(key, value)| format!("{key:<width$}{value}\n"))
        .collect();
    report(Ok(text))
}

pub async fn usage(args: UsageArgs) -> i32 {
    let result = account_call(Method::GET, &format!("/usage?days={}", args.days))
        .await
        .map(|reply| match args.format {
            UsageFormat::Json => format!(
                "{}\n",
                serde_json::to_string_pretty(&reply.body).unwrap_or_default()
            ),
            UsageFormat::Default => {
                let mut lines = vec![format!("Usage, last {} days", args.days), String::new()];
                lines.extend(usage_lines(&reply.body, &chrono::Local));
                lines.iter().map(|l| format!("{l}\n")).collect()
            }
        });
    report(result)
}

/// `encodeURIComponent`.
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

fn devices_table(body: &Value) -> String {
    let tokens = body
        .get("tokens")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if tokens.is_empty() {
        return "No devices are signed in.\n".into();
    }
    let field = |token: &Value, key: &str| {
        token
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let mut rows = vec![["  id", "name", "client", "last used", "expires"].map(str::to_string)];
    for token in &tokens {
        let current = if token.get("current") == Some(&Value::Bool(true)) {
            "* "
        } else {
            "  "
        };
        let name = field(token, "name");
        rows.push([
            format!("{current}{}", field(token, "id")),
            if name.is_empty() {
                "Unnamed device".into()
            } else {
                name
            },
            field(token, "client"),
            short_local(&field(token, "lastUsedAt")).unwrap_or_else(|| "never".into()),
            short_local(&field(token, "expiresAt")).unwrap_or_default(),
        ]);
    }
    let widths: Vec<usize> = (0..5)
        .map(|i| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0))
        .collect();
    let mut out = String::new();
    for row in &rows {
        let line: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(i, cell)| format!("{cell}{}", " ".repeat(widths[i] - cell.chars().count())))
            .collect();
        out.push_str(line.join("   ").trim_end());
        out.push('\n');
    }
    out.push_str("\n* this device. Sign one out with `anysola devices revoke <id>`.\n");
    out
}

pub async fn devices(command: Option<DevicesCommand>) -> i32 {
    let result = match command {
        None => account_call(Method::GET, "/tokens")
            .await
            .map(|reply| devices_table(&reply.body)),
        Some(DevicesCommand::Revoke { id }) => {
            match account_call(Method::POST, &format!("/tokens/{}/revoke", encode(&id))).await {
                Ok(_) => Ok(format!("Device {id} is signed out.\n")),
                Err(error) if error == "This Anysola server does not offer this yet." => {
                    Err(format!(
                        "No signed-in device with ID {id}. Run `anysola devices` to list them."
                    ))
                }
                Err(error) => Err(error),
            }
        }
    };
    report(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn masks_like_string_slice() {
        assert_eq!(mask(&format!("asc_{}", "F".repeat(40))), "asc_FFFF...FFFF");
        assert_eq!(mask("x"), "x...x");
        assert_eq!(mask("abcdef"), "abcdef...cdef");
    }

    #[test]
    fn free_quota_reads_turns_then_used_turns() {
        let line = |me: Value| {
            status_lines(&me)
                .into_iter()
                .find(|(k, _)| *k == "Free quota")
        };
        assert_eq!(
            line(json!({"freeQuota": {"turns": 3, "dailyTurns": 30}}))
                .unwrap()
                .1,
            "3/30 turns today"
        );
        assert_eq!(
            line(json!({"freeQuota": {"usedTurns": 4, "dailyTurns": 30}}))
                .unwrap()
                .1,
            "4/30 turns today"
        );
        assert!(line(json!({"freeQuota": null})).is_none());
    }

    #[test]
    fn uri_component() {
        assert_eq!(encode("tok 1/é"), "tok%201%2F%C3%A9");
    }
}
