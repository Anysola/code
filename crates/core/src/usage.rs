//! Usage per session: what this turn and the session took of the account's 5-hour window (the sum
//! of the gateway's `fiveHourPercentUsed`), both windows as the last usage event left them, and
//! the session budget in percent of the 5-hour window. Totals survive restarts in KV
//! `anysola:usage:<id>` as `{used, turns, budget}`.
//!
//! Credit budgets and totals from 0.5.x and the 1.0 previews (`anysola:session:<id>`,
//! `anysola:budget:default`) are not read: a credit count is no percentage, so they are dropped.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::events::{Event, UsageSnapshot};
use crate::gateway::limits::{UsageEvent, Window};
use crate::storage::{Db, StorageError};

const DEFAULT_KEY: &str = "anysola:budget:five-hour";

/// The highest budget: the whole 5-hour window.
pub const MAX_BUDGET: u64 = 100;

#[derive(Debug, Clone, Default)]
struct State {
    latest: Option<UsageEvent>,
    turn: f64,
    total: f64,
    turns: u64,
    open: bool,
    budget: Option<u64>,
}

pub struct Usage {
    db: Arc<Db>,
    events: broadcast::Sender<Event>,
    sessions: Mutex<HashMap<String, State>>,
}

/// A whole percent from 1 to 100, else no budget.
fn budget_of(value: Option<&Value>) -> Option<u64> {
    value
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v >= 1.0 && *v <= MAX_BUDGET as f64)
        .map(|v| v.floor() as u64)
}

impl Usage {
    pub fn new(db: Arc<Db>, events: broadcast::Sender<Event>) -> Self {
        Usage {
            db,
            events,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn key(session_id: &str) -> String {
        format!("anysola:usage:{session_id}")
    }

    fn with<T>(&self, session_id: &str, f: impl FnOnce(&mut State) -> T) -> T {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let state = sessions
            .entry(session_id.to_string())
            .or_insert_with(|| self.load(session_id));
        f(state)
    }

    fn load(&self, session_id: &str) -> State {
        match self.db.kv_get(&Self::key(session_id)).ok().flatten() {
            Some(saved) => State {
                total: saved
                    .get("used")
                    .and_then(Value::as_f64)
                    .filter(|n| n.is_finite() && *n >= 0.0)
                    .unwrap_or(0.0),
                turns: saved.get("turns").and_then(Value::as_u64).unwrap_or(0),
                budget: budget_of(saved.get("budget")),
                ..State::default()
            },
            None => State {
                budget: self.default_budget(),
                ..State::default()
            },
        }
    }

    fn stored(state: &State) -> Value {
        json!({"used": state.total, "turns": state.turns, "budget": state.budget})
    }

    fn save(&self, session_id: &str, state: &State) {
        let _ = self.db.kv_set(&Self::key(session_id), &Self::stored(state));
    }

    fn publish(&self, session_id: &str, state: &State) {
        let _ = self.events.send(Event::Usage(snapshot(session_id, state)));
    }

    /// The budget new sessions start with.
    pub fn default_budget(&self) -> Option<u64> {
        budget_of(self.db.kv_get(DEFAULT_KEY).ok().flatten().as_ref())
    }

    pub fn set_default_budget(&self, budget: Option<u64>) -> Result<(), StorageError> {
        self.db.kv_set(DEFAULT_KEY, &json!(budget))
    }

    pub fn snapshot(&self, session_id: &str) -> UsageSnapshot {
        self.with(session_id, |state| snapshot(session_id, state))
    }

    /// Records a model call's usage event.
    pub fn record(&self, session_id: &str, event: &UsageEvent) {
        let state = self.with(session_id, |state| {
            if !state.open {
                state.turn = 0.0;
            }
            state.open = true;
            state.turn += event.five_hour_percent_used;
            state.total += event.five_hour_percent_used;
            state.latest = Some(*event);
            state.clone()
        });
        if event.five_hour_percent_used > 0.0 {
            self.save(session_id, &state);
        }
        self.publish(session_id, &state);
    }

    /// The session went idle: the turn is over.
    pub fn close_turn(&self, session_id: &str) {
        let closed = self.with(session_id, |state| {
            if !state.open {
                return None;
            }
            state.open = false;
            if state.turn > 0.0 {
                state.turns += 1;
            }
            Some(state.clone())
        });
        if let Some(state) = closed {
            if state.turn > 0.0 {
                self.save(session_id, &state);
            }
            self.publish(session_id, &state);
        }
    }

    /// Sets (or clears) the session's budget, a percent of the 5-hour window; it is persisted
    /// before it applies.
    pub fn set_budget(
        &self,
        session_id: &str,
        budget: Option<u64>,
    ) -> Result<UsageSnapshot, StorageError> {
        let mut state = self.with(session_id, |state| state.clone());
        state.budget = budget.filter(|b| (1..=MAX_BUDGET).contains(b));
        self.db
            .kv_set(&Self::key(session_id), &Self::stored(&state))?;
        let state = self.with(session_id, |current| {
            current.budget = state.budget;
            current.clone()
        });
        self.publish(session_id, &state);
        Ok(snapshot(session_id, &state))
    }
}

fn snapshot(session_id: &str, state: &State) -> UsageSnapshot {
    UsageSnapshot {
        session_id: session_id.to_string(),
        latest: state.latest,
        turn: state.turn,
        total: state.total,
        turns: state.turns,
        budget: state.budget,
    }
}

/// From this share of a window on, front ends warn.
pub const WARN_PERCENT: u32 = 80;

/// `5h 42%`, with `week 81%` when the weekly window is the fuller one or near its limit.
pub fn windows_text(five_hour: Window, weekly: Window) -> String {
    if weekly.percent > five_hour.percent || weekly.percent >= WARN_PERCENT {
        format!("5h {}% · week {}%", five_hour.percent, weekly.percent)
    } else {
        format!("5h {}%", five_hour.percent)
    }
}

/// `0.4%`, `2%`, `12%`: one decimal under 10, whole percents above.
pub fn percent(value: f64) -> String {
    let value = value.max(0.0);
    if value >= 9.95 {
        format!("{}%", value.round())
    } else {
        let tenths = (value * 10.0).round() / 10.0;
        if tenths.fract() == 0.0 {
            format!("{tenths}%")
        } else {
            format!("{tenths:.1}%")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(used: f64, percent: u32) -> UsageEvent {
        UsageEvent {
            five_hour_percent_used: used,
            five_hour: Window {
                percent,
                resets_at: None,
            },
            weekly: Window::default(),
        }
    }

    #[test]
    fn turn_accounting() {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(Db::open(&dir.path().join("a.db")).unwrap());
        let (tx, _rx) = broadcast::channel(16);
        let usage = Usage::new(db.clone(), tx);
        usage.record("s", &event(1.5, 40));
        usage.record("s", &event(0.25, 41));
        let snap = usage.snapshot("s");
        assert_eq!((snap.turn, snap.total), (1.75, 1.75));
        assert_eq!(snap.latest.unwrap().five_hour.percent, 41);
        usage.close_turn("s");
        assert_eq!(usage.snapshot("s").turns, 1);
        usage.record("s", &event(1.0, 42));
        assert_eq!(usage.snapshot("s").turn, 1.0, "a new turn starts fresh");
        assert_eq!(
            db.kv_get("anysola:usage:s").unwrap().unwrap(),
            json!({"used": 2.75, "turns": 1, "budget": null})
        );
        usage.set_budget("s", Some(20)).unwrap();
        let reloaded = Usage::new(db, broadcast::channel(1).0);
        assert_eq!(reloaded.snapshot("s").budget, Some(20));
    }

    #[test]
    fn credit_budgets_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(Db::open(&dir.path().join("a.db")).unwrap());
        db.kv_set(
            "anysola:session:s",
            &json!({"total": 40, "turns": 2, "budget": 500}),
        )
        .unwrap();
        db.kv_set("anysola:budget:default", &json!(500)).unwrap();
        let usage = Usage::new(db.clone(), broadcast::channel(1).0);
        let snap = usage.snapshot("s");
        assert_eq!((snap.total, snap.turns, snap.budget), (0.0, 0, None));
        assert_eq!(usage.default_budget(), None);
        db.kv_set(
            "anysola:usage:t",
            &json!({"used": 1, "turns": 1, "budget": 500}),
        )
        .unwrap();
        assert_eq!(usage.snapshot("t").budget, None, "over 100% is no budget");
    }

    #[test]
    fn window_text() {
        let w = |percent| Window {
            percent,
            resets_at: None,
        };
        assert_eq!(windows_text(w(42), w(18)), "5h 42%");
        assert_eq!(windows_text(w(42), w(43)), "5h 42% · week 43%");
        assert_eq!(windows_text(w(90), w(80)), "5h 90% · week 80%");
    }

    #[test]
    fn percent_text() {
        assert_eq!(percent(0.0), "0%");
        assert_eq!(percent(0.04), "0%");
        assert_eq!(percent(0.42), "0.4%");
        assert_eq!(percent(2.0), "2%");
        assert_eq!(percent(9.96), "10%");
        assert_eq!(percent(12.4), "12%");
    }

    #[test]
    fn a_budget_that_cannot_be_stored_does_not_apply() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.db");
        let db = Arc::new(Db::open(&path).unwrap());
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER full BEFORE INSERT ON kv BEGIN SELECT RAISE(FAIL, 'disk full'); END;",
            )
            .unwrap();
        let usage = Usage::new(db, broadcast::channel(1).0);
        let error = usage.set_budget("s", Some(5)).unwrap_err();
        assert!(error.to_string().contains("disk full"));
        assert_eq!(usage.snapshot("s").budget, None);
    }
}
