//! Anysola credits per session: what this turn and the session have spent, the balance after the
//! turn's cheapest-to-report request, and the session budget. Totals survive restarts in KV
//! `anysola:session:<id>` as `{total, turns, budget}`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::events::{CreditsSnapshot, Event};
use crate::storage::Db;

#[derive(Debug, Clone, Default)]
struct State {
    remaining: Option<u64>,
    turn: u64,
    total: u64,
    turns: u64,
    open: bool,
    budget: Option<u64>,
    /// Lowest balance reported during the open turn.
    turn_low: Option<u64>,
}

pub struct Credits {
    db: Arc<Db>,
    events: broadcast::Sender<Event>,
    sessions: Mutex<HashMap<String, State>>,
}

/// A positive whole number of credits, else no budget.
fn budget_of(value: Option<&Value>) -> Option<u64> {
    value
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v >= 1.0)
        .map(|v| v.floor() as u64)
}

impl Credits {
    pub fn new(db: Arc<Db>, events: broadcast::Sender<Event>) -> Self {
        Credits {
            db,
            events,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn key(session_id: &str) -> String {
        format!("anysola:session:{session_id}")
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
                total: saved.get("total").and_then(Value::as_u64).unwrap_or(0),
                turns: saved.get("turns").and_then(Value::as_u64).unwrap_or(0),
                budget: budget_of(saved.get("budget")),
                ..State::default()
            },
            None => State {
                budget: budget_of(
                    self.db
                        .kv_get("anysola:budget:default")
                        .ok()
                        .flatten()
                        .as_ref(),
                ),
                ..State::default()
            },
        }
    }

    fn save(&self, session_id: &str, state: &State) {
        let value = json!({"total": state.total, "turns": state.turns, "budget": state.budget});
        let _ = self.db.kv_set(&Self::key(session_id), &value);
    }

    fn publish(&self, session_id: &str, state: &State) {
        let _ = self
            .events
            .send(Event::Credits(snapshot(session_id, state)));
    }

    pub fn snapshot(&self, session_id: &str) -> CreditsSnapshot {
        self.with(session_id, |state| snapshot(session_id, state))
    }

    /// Records a charge. Within one turn the lowest reported balance wins, since concurrent
    /// requests (main and title) each report the balance before only their own charge.
    pub fn record(&self, session_id: &str, charged: Option<u64>, remaining: Option<u64>) {
        let state = self.with(session_id, |state| {
            if !state.open {
                state.turn = 0;
                state.turn_low = None;
            }
            state.open = true;
            state.turn += charged.unwrap_or(0);
            state.total += charged.unwrap_or(0);
            if let Some(remaining) = remaining {
                let low = state.turn_low.map_or(remaining, |low| low.min(remaining));
                state.turn_low = Some(low);
                state.remaining = Some(low);
            }
            state.clone()
        });
        if charged.unwrap_or(0) != 0 {
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
            if state.turn > 0 {
                state.turns += 1;
            }
            Some(state.clone())
        });
        if let Some(state) = closed {
            if state.turn > 0 {
                self.save(session_id, &state);
            }
            self.publish(session_id, &state);
        }
    }

    pub fn set_budget(&self, session_id: &str, budget: Option<u64>) -> CreditsSnapshot {
        let state = self.with(session_id, |state| {
            state.budget = budget.filter(|b| *b >= 1);
            state.clone()
        });
        self.save(session_id, &state);
        self.publish(session_id, &state);
        snapshot(session_id, &state)
    }
}

fn snapshot(session_id: &str, state: &State) -> CreditsSnapshot {
    CreditsSnapshot {
        session_id: session_id.to_string(),
        remaining: state.remaining,
        turn: state.turn,
        total: state.total,
        turns: state.turns,
        budget: state.budget,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_accounting() {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(Db::open(&dir.path().join("a.db")).unwrap());
        let (tx, _rx) = broadcast::channel(16);
        let credits = Credits::new(db.clone(), tx);
        credits.record("s", Some(2), Some(729));
        credits.record("s", Some(0), Some(731));
        let snap = credits.snapshot("s");
        assert_eq!((snap.turn, snap.total, snap.remaining), (2, 2, Some(729)));
        credits.close_turn("s");
        assert_eq!(credits.snapshot("s").turns, 1);
        credits.record("s", Some(1), Some(800));
        assert_eq!(
            credits.snapshot("s").remaining,
            Some(800),
            "a new turn starts fresh"
        );
        assert_eq!(
            db.kv_get("anysola:session:s").unwrap().unwrap(),
            json!({"total": 3, "turns": 1, "budget": null})
        );
        credits.set_budget("s", Some(5));
        let reloaded = Credits::new(db, broadcast::channel(1).0);
        assert_eq!(reloaded.snapshot("s").budget, Some(5));
    }
}
