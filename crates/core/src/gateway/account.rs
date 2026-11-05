//! Account calls (`/me`, `/usage`, `/tokens`, revokes) and the device login flow
//! (`/device/code`, `/device/token`), with 0.5.1's header set A and error texts.

use std::time::{Duration, Instant};

pub use reqwest::Method;
use serde_json::{Value, json};

use super::Gateway;
use crate::version::VERSION;

/// A reply of an account route: status and the body parsed leniently (non-JSON → `{}`).
#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub body: Value,
}

impl Reply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// `{error, error_description}` of the device routes or the gateway's error envelope:
    /// the error code and a message.
    pub fn error(&self) -> (Option<String>, String) {
        let text = |v: Option<&Value>| {
            v.and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let error = self.body.get("error");
        let code = match error {
            Some(Value::String(code)) => Some(code.clone()),
            other => text(other.and_then(|e| e.get("type"))),
        };
        let message = text(self.body.get("error_description"))
            .or_else(|| text(error.and_then(|e| e.get("message"))))
            .or_else(|| text(self.body.get("message")))
            .or_else(|| code.clone())
            .unwrap_or_else(|| format!("HTTP {}", self.status));
        (code, message)
    }
}

/// A started device login: what to show and how to poll.
#[derive(Debug, Clone)]
pub struct DeviceLogin {
    pub user_code: String,
    pub url: String,
    device_code: String,
    interval: f64,
    deadline: Instant,
}

impl DeviceLogin {
    pub fn instructions(&self) -> String {
        format!("Open {} and enter {}", self.url, self.user_code)
    }
}

impl Gateway {
    /// `method route` with header set A; `Err` is the transport failure text.
    pub async fn account_call(
        &self,
        method: reqwest::Method,
        route: &str,
        body: Option<&Value>,
    ) -> Result<Reply, String> {
        let request = self.account(self.http.request(method, self.url(route)));
        let request = match body {
            Some(body) => request
                .header("content-type", "application/json")
                .body(crate::json::compact(body)),
            None => request,
        };
        let unreachable = || format!("Cannot reach {}", self.base);
        let response = request.send().await.map_err(|_| unreachable())?;
        let status = response.status().as_u16();
        let text = response.text().await.map_err(|_| unreachable())?;
        let body = serde_json::from_str::<Value>(&text)
            .ok()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        Ok(Reply { status, body })
    }

    /// `POST /tokens/self/revoke`; a 401 means the token is already dead, which counts as done.
    pub async fn revoke_self(&self) -> Result<(), String> {
        let reply = self
            .account_call(reqwest::Method::POST, "/tokens/self/revoke", None)
            .await?;
        if reply.ok() || reply.status == 401 {
            Ok(())
        } else {
            Err(format!("HTTP {}", reply.status))
        }
    }

    /// `POST /device/code` (no token).
    pub async fn start_device_login(&self) -> Result<DeviceLogin, String> {
        let hostname = gethostname::gethostname().to_string_lossy().into_owned();
        let body = json!({"client": format!("anysola-code/{VERSION}"), "deviceName": hostname});
        let reply = self
            .account_call(reqwest::Method::POST, "/device/code", Some(&body))
            .await?;
        if reply.status == 429 {
            return Err("Too many login attempts. Try again later.".into());
        }
        if !reply.ok() {
            return Err(format!("Could not start login: {}", reply.error().1));
        }
        let text = |key: &str| {
            reply
                .body
                .get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let number = |key: &str| reply.body.get(key).and_then(Value::as_f64);
        let (Some(device_code), Some(user_code), Some(url)) = (
            text("device_code"),
            text("user_code"),
            text("verification_uri"),
        ) else {
            return Err("Could not start login: unexpected response from the server.".into());
        };
        let interval = number("interval").unwrap_or(5.0).max(1.0);
        let lifetime = number("expires_in").unwrap_or(600.0).clamp(0.0, 900.0);
        Ok(DeviceLogin {
            user_code,
            url,
            device_code,
            interval,
            deadline: Instant::now() + Duration::from_secs_f64(lifetime),
        })
    }

    /// Polls `POST /device/token` until approval; returns the credential value to store.
    pub async fn finish_device_login(&self, login: &DeviceLogin) -> Result<Value, String> {
        let mut wait = login.interval;
        let mut failures = 0;
        loop {
            tokio::time::sleep(Duration::from_secs_f64(wait)).await;
            if Instant::now() >= login.deadline {
                return Err("The login code expired. Run `anysola login` again.".into());
            }
            let body = json!({"device_code": login.device_code});
            let reply = self
                .account_call(reqwest::Method::POST, "/device/token", Some(&body))
                .await
                .ok();
            match reply {
                Some(reply) if reply.ok() => return credential(&reply.body, &self.base),
                Some(reply) if reply.status < 500 => {
                    failures = 0;
                    let (code, message) = reply.error();
                    match code.as_deref() {
                        Some("authorization_pending") => {}
                        Some("slow_down") => wait += 5.0,
                        Some("expired_token") => {
                            return Err("The code expired before it was approved. Run `anysola login` again.".into());
                        }
                        Some("access_denied") => return Err("Login was denied.".into()),
                        _ => return Err(format!("Login failed: {message}")),
                    }
                }
                reply => {
                    // Network errors and 5xx: keep polling unless they repeat.
                    failures += 1;
                    if failures >= 3 {
                        return Err(match reply {
                            Some(reply) => format!("Login failed: {}", reply.error().1),
                            None => "Login failed: network error".into(),
                        });
                    }
                }
            }
        }
    }
}

/// The stored credential of an approved device login.
fn credential(body: &Value, api_url: &str) -> Result<Value, String> {
    let token = body
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|t| usable(t))
        .ok_or("Login failed: the server returned no usable token.")?;
    let expires = body
        .get("expires_at")
        .and_then(Value::as_str)
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.timestamp_millis())
        .unwrap_or_else(|| crate::time::now_ms() as i64 + 365 * 24 * 60 * 60 * 1000);
    let device = gethostname::gethostname().to_string_lossy().into_owned();
    Ok(json!({
        "type": "oauth", "methodID": "device", "refresh": "", "access": token, "expires": expires,
        "metadata": {"apiURL": api_url, "device": device}
    }))
}

/// `^asc_[0-9A-Za-z]+$`
fn usable(token: &str) -> bool {
    token
        .strip_prefix("asc_")
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_shape() {
        assert!(usable("asc_abc123"));
        assert!(!usable("asc_"));
        assert!(!usable("asc_a-b"));
        assert!(!usable("nope"));
    }

    #[test]
    fn error_texts() {
        let reply = |status, body| Reply { status, body };
        assert_eq!(
            reply(400, json!({"error": "slow_down", "interval": 10})).error(),
            (Some("slow_down".into()), "slow_down".into())
        );
        assert_eq!(
            reply(
                403,
                json!({"type": "error", "error": {"type": "permission_error", "message": "No."}})
            )
            .error(),
            (Some("permission_error".into()), "No.".into())
        );
        assert_eq!(reply(502, json!({})).error(), (None, "HTTP 502".into()));
    }
}
