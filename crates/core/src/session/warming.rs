//! `warming`: keeps the provider's prompt cache of recently active root sessions warm with a
//! keep-alive generation after each idle interval, until the duration since the last request
//! runs out. Off unless configured.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::time::Instant;

use crate::api::Shared;
use crate::config::Config;
use crate::storage::SessionRow;

const PROMPT: &str =
    "This is a keep-alive request. Do not perform any work or use tools. Reply with exactly: OK";
const INTERVAL: Duration = Duration::from_secs(4 * 60);
const DURATION: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, PartialEq)]
struct Settings {
    prompt: String,
    interval: Duration,
    duration: Duration,
}

/// `warming: true` or `{prompt?, interval?, duration?}` (durations like `"4 minutes"`).
fn settings(value: Option<&Value>) -> Option<Settings> {
    let value = value?;
    let mut settings = Settings {
        prompt: PROMPT.into(),
        interval: INTERVAL,
        duration: DURATION,
    };
    match value {
        Value::Bool(true) => {}
        Value::Object(fields) => {
            if let Some(prompt) = fields.get("prompt").and_then(Value::as_str) {
                settings.prompt = prompt.into();
            }
            let duration = |key: &str| {
                fields
                    .get(key)
                    .and_then(Value::as_str)
                    .map(crate::time::parse_duration)
            };
            match (duration("interval"), duration("duration")) {
                (Some(None), _) | (_, Some(None)) => {
                    crate::log::warn(
                        "warming interval and duration must be finite positive durations",
                        &[],
                    );
                    return None;
                }
                (interval, length) => {
                    settings.interval = interval.flatten().unwrap_or(INTERVAL);
                    settings.duration = length.flatten().unwrap_or(DURATION);
                }
            }
        }
        _ => return None,
    }
    if settings.interval.is_zero() || settings.duration.is_zero() {
        crate::log::warn(
            "warming interval and duration must be finite positive durations",
            &[],
        );
        return None;
    }
    Some(settings)
}

struct Warm {
    /// Tells a later registration apart from the one a loop started with.
    id: u64,
    last: Instant,
    expires: Instant,
    settings: Settings,
}

#[derive(Default)]
pub(crate) struct Warming {
    sessions: Mutex<HashMap<String, Warm>>,
    /// Sessions with a step request in flight; a ping would compete with it.
    requesting: Mutex<HashSet<String>>,
    next_id: std::sync::atomic::AtomicU64,
}

impl Warming {
    /// The session's step ended (streamed or failed): the next interval counts from now.
    pub fn settled(&self, session_id: &str) {
        self.requesting
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session_id);
        if let Some(warm) = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(session_id)
        {
            warm.last = Instant::now();
        }
    }
}

/// What kind of request is about to go out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Request {
    /// A step: activity, and in flight until the step ends.
    Step,
    Compaction,
    /// The keep-alive generation itself.
    KeepAlive,
}

/// A request of the session is about to go out. Activity of a root session (re)starts its
/// warming window; the keep-alive request itself only picks up new settings.
pub(crate) fn context(
    shared: &Arc<Shared>,
    session: &SessionRow,
    config: &Config,
    request: Request,
) {
    let warming = &shared.warming;
    let keep_alive = request == Request::KeepAlive;
    if request == Request::Step {
        warming
            .requesting
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(session.id.clone());
    }
    if session.parent_id.is_some() {
        return;
    }
    let mut sessions = warming.sessions.lock().unwrap_or_else(|e| e.into_inner());
    let Some(settings) = settings(config.get("warming")) else {
        sessions.remove(&session.id);
        return;
    };
    let now = Instant::now();
    match sessions.get_mut(&session.id) {
        Some(active) => {
            if !keep_alive {
                active.last = now;
                active.expires = now + settings.duration;
            }
            active.settings = settings;
        }
        None if keep_alive => {}
        None => {
            let id = warming
                .next_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            sessions.insert(
                session.id.clone(),
                Warm {
                    id,
                    last: now,
                    expires: now + settings.duration,
                    settings,
                },
            );
            crate::log::info("scheduled session warming", &[("session", &session.id)]);
            tokio::spawn(keep_warm(shared.clone(), session.id.clone(), id));
        }
    }
}

enum Next {
    Sleep(Duration),
    Ping { prompt: String, last: Instant },
    Stop,
}

async fn keep_warm(shared: Arc<Shared>, session_id: String, id: u64) {
    loop {
        let next = {
            let mut sessions = shared
                .warming
                .sessions
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let Some(warm) = sessions.get(&session_id).filter(|w| w.id == id) else {
                return;
            };
            let now = Instant::now();
            let due = (warm.last + warm.settings.interval).min(warm.expires);
            if now < due {
                Next::Sleep(due - now)
            } else if now >= warm.expires {
                sessions.remove(&session_id);
                Next::Stop
            } else if shared
                .warming
                .requesting
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains(&session_id)
            {
                Next::Sleep(warm.settings.interval)
            } else {
                Next::Ping {
                    prompt: warm.settings.prompt.clone(),
                    last: warm.last,
                }
            }
        };
        match next {
            Next::Stop => return,
            Next::Sleep(delay) => tokio::time::sleep(delay).await,
            Next::Ping { prompt, last } => {
                crate::log::info("warming session", &[("session", &session_id)]);
                let session = shared.db.session(&session_id).ok().flatten();
                let result = match &session {
                    Some(session) => super::step::generate(&shared, session, &prompt)
                        .await
                        .map(drop),
                    None => return,
                };
                if let Err(super::step::GenerateError::Failed(error)) = result {
                    crate::log::warn(
                        "failed to warm session",
                        &[("session", &session_id), ("error", &error.message)],
                    );
                }
                let mut sessions = shared
                    .warming
                    .sessions
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if let Some(warm) = sessions.get_mut(&session_id)
                    && warm.id == id
                    && warm.last == last
                {
                    warm.last = Instant::now();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_from_config() {
        assert_eq!(settings(None), None);
        assert_eq!(settings(Some(&Value::Bool(false))), None);
        assert_eq!(
            settings(Some(&Value::Bool(true))),
            Some(Settings {
                prompt: PROMPT.into(),
                interval: INTERVAL,
                duration: DURATION
            })
        );
        let custom = settings(Some(&serde_json::json!({"interval": "90 seconds"}))).unwrap();
        assert_eq!(
            (custom.interval, custom.duration),
            (Duration::from_secs(90), DURATION)
        );
        assert_eq!(
            settings(Some(&serde_json::json!({"interval": "soon"}))),
            None
        );
        assert_eq!(
            settings(Some(&serde_json::json!({"duration": "0 minutes"}))),
            None
        );
    }
}
