//! Usage limits as the gateway reports them (`UsageLimitsWire`): a 5-hour and a weekly window per
//! account, shared with Anysola Chat, shown as percentages and reset times, and the banked resets
//! a person can spend when a window is full.

use serde_json::Value;

/// One window: how much of it is used (0 to 100, rounded down) and when it ends (epoch ms; `None`
/// while it hasn't started).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Window {
    pub percent: u32,
    pub resets_at: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowId {
    FiveHour,
    Weekly,
}

impl WindowId {
    pub fn parse(text: &str) -> Option<WindowId> {
        match text {
            "five_hour" => Some(WindowId::FiveHour),
            "weekly" => Some(WindowId::Weekly),
            _ => None,
        }
    }

    /// `5-hour` or `Weekly`, as the gateway's limit message names them.
    pub fn label(self) -> &'static str {
        match self {
            WindowId::FiveHour => "5-hour",
            WindowId::Weekly => "Weekly",
        }
    }
}

/// A banked reset: spent on demand, gone at `expires_at`; `both` clears the weekly window too.
#[derive(Debug, Clone, PartialEq)]
pub struct Banked {
    pub id: String,
    pub expires_at: Option<u64>,
    pub both: bool,
}

/// `GET /limits`, `POST /limits/redeem` and `/me`'s `limits`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Limits {
    pub plan: Option<String>,
    pub allowed: bool,
    pub blocked_by: Option<WindowId>,
    pub five_hour: Window,
    pub weekly: Window,
    /// Soonest to expire first.
    pub bank: Vec<Banked>,
}

/// The closing `anysola_usage` event of a model call: what it took of the 5-hour window (percent,
/// a float so small turns add up) and both windows after it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UsageEvent {
    pub five_hour_percent_used: f64,
    pub five_hour: Window,
    pub weekly: Window,
}

/// RFC 3339 to epoch milliseconds.
pub fn parse_time(text: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .and_then(|at| u64::try_from(at.timestamp_millis()).ok())
}

impl Window {
    pub fn from_wire(value: Option<&Value>) -> Window {
        let Some(value) = value else {
            return Window::default();
        };
        Window {
            percent: value
                .get("percent")
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite())
                .map_or(0, |n| n.clamp(0.0, 1000.0) as u32),
            resets_at: value
                .get("resetsAt")
                .and_then(Value::as_str)
                .and_then(parse_time),
        }
    }

    /// The window as it stands at `now`: one that already ended counts as empty.
    pub fn at(self, now: u64) -> Window {
        match self.resets_at {
            Some(at) if at <= now => Window::default(),
            _ => self,
        }
    }
}

impl Limits {
    /// Lenient: `None` only when `value` is not an object with both windows.
    pub fn from_wire(value: &Value) -> Option<Limits> {
        if !value.get("fiveHour").is_some_and(Value::is_object)
            || !value.get("weekly").is_some_and(Value::is_object)
        {
            return None;
        }
        let mut bank: Vec<Banked> = value
            .get("bank")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        Some(Banked {
                            id: item.get("id")?.as_str()?.to_string(),
                            expires_at: item
                                .get("expiresAt")
                                .and_then(Value::as_str)
                                .and_then(parse_time),
                            both: item.get("scope").and_then(Value::as_str) == Some("both"),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        bank.sort_by_key(|b| b.expires_at.unwrap_or(u64::MAX));
        let blocked_by = value
            .get("blockedBy")
            .and_then(Value::as_str)
            .and_then(WindowId::parse);
        Some(Limits {
            plan: value
                .get("plan")
                .and_then(Value::as_str)
                .map(str::to_string),
            allowed: value
                .get("allowed")
                .and_then(Value::as_bool)
                .unwrap_or(blocked_by.is_none()),
            blocked_by,
            five_hour: Window::from_wire(value.get("fiveHour")),
            weekly: Window::from_wire(value.get("weekly")),
            bank,
        })
    }

    pub fn window(&self, id: WindowId) -> Window {
        match id {
            WindowId::FiveHour => self.five_hour,
            WindowId::Weekly => self.weekly,
        }
    }

    /// Takes a usage event's windows; the bank and plan stay.
    pub fn apply(&mut self, event: &UsageEvent) {
        self.five_hour = event.five_hour;
        self.weekly = event.weekly;
    }

    /// The bank entries still valid at `now`.
    pub fn bank_at(&self, now: u64) -> Vec<&Banked> {
        self.bank
            .iter()
            .filter(|b| b.expires_at.is_none_or(|at| at > now))
            .collect()
    }
}

impl UsageEvent {
    pub fn from_wire(value: &Value) -> Option<UsageEvent> {
        let used = value
            .get("fiveHourPercentUsed")
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite() && *n >= 0.0)?;
        Some(UsageEvent {
            five_hour_percent_used: used,
            five_hour: Window::from_wire(value.get("fiveHour")),
            weekly: Window::from_wire(value.get("weekly")),
        })
    }
}

/// The gateway's `humanDuration`: `12 min`, `2 h`, `2 h 5 min`, `3 d`, `3 d 4 h`.
pub fn human_duration(ms: u64) -> String {
    let minutes = ms.div_ceil(60_000).max(1);
    if minutes < 60 {
        return format!("{minutes} min");
    }
    let (hours, rest) = (minutes / 60, minutes % 60);
    if hours < 24 {
        return if rest > 0 {
            format!("{hours} h {rest} min")
        } else {
            format!("{hours} h")
        };
    }
    let (days, h) = (hours / 24, hours % 24);
    if h > 0 {
        format!("{days} d {h} h")
    } else {
        format!("{days} d")
    }
}

/// `in 2 h`, or `soon` when the time is unknown or past.
pub fn resets_in(resets_at: Option<u64>, now: u64) -> String {
    match resets_at {
        Some(at) if at > now => format!("in {}", human_duration(at - now)),
        _ => "soon".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_the_wire_shape() {
        let limits = Limits::from_wire(&json!({
            "plan": "pro", "allowed": false, "blockedBy": "five_hour",
            "fiveHour": {"percent": 100, "resetsAt": "2026-11-06T12:00:00.000Z"},
            "weekly": {"percent": 41, "resetsAt": null},
            "bank": [
                {"id": "b2", "expiresAt": "2026-12-01T00:00:00Z", "scope": "both"},
                {"id": "b1", "expiresAt": "2026-11-20T00:00:00Z", "scope": "five_hour"}
            ]
        }))
        .unwrap();
        assert_eq!(limits.plan.as_deref(), Some("pro"));
        assert!(!limits.allowed);
        assert_eq!(limits.blocked_by, Some(WindowId::FiveHour));
        assert_eq!(limits.five_hour.percent, 100);
        assert_eq!(
            limits.five_hour.resets_at,
            parse_time("2026-11-06T12:00:00Z")
        );
        assert_eq!(limits.weekly.resets_at, None);
        assert_eq!(limits.bank[0].id, "b1", "soonest expiry first");
        assert!(limits.bank[1].both);
        assert_eq!(Limits::from_wire(&json!({"credits": 3})), None);
    }

    #[test]
    fn usage_event() {
        let event = UsageEvent::from_wire(&json!({"type": "anysola_usage", "fiveHourPercentUsed": 1.25,
            "fiveHour": {"percent": 43, "resetsAt": "2026-11-06T12:00:00Z"}, "weekly": {"percent": 9, "resetsAt": null}}))
        .unwrap();
        assert_eq!(event.five_hour_percent_used, 1.25);
        assert_eq!(event.five_hour.percent, 43);
        assert_eq!(event.weekly.percent, 9);
        assert_eq!(
            UsageEvent::from_wire(&json!({"type": "anysola_usage"})),
            None
        );
    }

    #[test]
    fn durations_match_the_gateway() {
        assert_eq!(human_duration(1), "1 min");
        assert_eq!(human_duration(59 * 60_000), "59 min");
        assert_eq!(human_duration(2 * 3_600_000), "2 h");
        assert_eq!(human_duration(2 * 3_600_000 + 5 * 60_000), "2 h 5 min");
        assert_eq!(human_duration(3 * 86_400_000), "3 d");
        assert_eq!(human_duration(3 * 86_400_000 + 4 * 3_600_000), "3 d 4 h");
        assert_eq!(resets_in(None, 5), "soon");
        assert_eq!(resets_in(Some(10), 5), "in 1 min");
    }
}
