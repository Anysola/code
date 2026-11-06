//! The Anysola account commands: `/usage`, `/devices`, `/budget`, `/account`, `/feedback`, the
//! budget gate before a prompt and the banked-reset offer after a full usage window.

use anysola_core::gateway::account::Method;
use anysola_core::gateway::limits::{Limits, resets_in};

use super::{App, Msg};
use crate::dialog::{Confirm, ConfirmKind, Dialog, Opt};
use crate::menus::anysola::{self as account, Budget, UsageView};
use crate::state::Variant;
use crate::transcript::model::{Item, Part};

impl App {
    /// Account commands; false when `id` is not one.
    pub(super) fn account_command(&mut self, id: &str, arguments: Option<&str>) -> bool {
        match id {
            "anysola.usage" => self.open_usage(7),
            "anysola.devices" => self.open_devices(),
            "anysola.budget" => self.budget(arguments.unwrap_or_default()),
            "anysola.account" => self.open_url(account::ACCOUNT_URL),
            "anysola.feedback" => self.open_url(account::FEEDBACK_URL),
            _ => return false,
        }
        true
    }

    fn usage_view(&self, days: u32) -> UsageView {
        UsageView {
            days,
            session: self
                .state
                .session()
                .and_then(|s| s.usage.as_ref())
                .map(|c| (c.turns, c.total)),
            zone: self.state.zone,
            now: self.state.now.max(super::now()),
        }
    }

    pub(super) fn open_usage(&mut self, days: u32) {
        self.push(Dialog::Body(account::usage_dialog(
            &self.usage_view(days),
            None,
        )));
        let (core, tx) = (self.core.clone(), self.tx.clone());
        tokio::spawn(async move {
            let reply = core
                .account(Method::GET, &format!("/usage?days={days}"))
                .await;
            let _ = tx.send(Msg::Apply(Box::new(move |app: &mut App| {
                if let Ok(r) = &reply
                    && r.ok()
                    && let Some(limits) = r.body.get("limits").and_then(Limits::from_wire)
                {
                    app.set_limits(limits);
                }
                let body = account::usage_dialog(&app.usage_view(days), Some(&reply));
                if let Some(Dialog::Body(b)) = app.state.dialogs.last_mut()
                    && b.kind == body.kind
                {
                    *b = body;
                }
            })));
        });
    }

    fn open_devices(&mut self) {
        self.push(Dialog::Body(account::devices_pending(None)));
        let (core, tx) = (self.core.clone(), self.tx.clone());
        tokio::spawn(async move {
            let reply = core.account(Method::GET, "/tokens").await;
            let _ = tx.send(Msg::Apply(Box::new(move |app: &mut App| {
                let pending = account::devices_pending(None);
                if app.state.dialogs.last() != Some(&Dialog::Body(pending)) {
                    return;
                }
                app.state.dialogs.pop();
                match &reply {
                    Ok(r) if r.ok() => {
                        let select = account::devices_dialog(&r.body, app.state.zone);
                        app.push(Dialog::Select(select));
                    }
                    other => {
                        let body = account::devices_pending(Some(account::error_text(other)));
                        app.push(Dialog::Body(body));
                    }
                }
            })));
        });
    }

    pub(super) fn choose_device(&mut self, opt: &Opt) {
        if opt.title.ends_with(" (this device)") {
            return self.toast(
                None,
                "This device: sign out with `anysola logout`.".into(),
                Variant::Info,
            );
        }
        let confirm = super::dialogs::revoke_confirm(opt.value.clone(), &opt.title);
        self.push(confirm);
    }

    pub(super) fn revoke_device(&mut self, id: String) {
        let (core, tx) = (self.core.clone(), self.tx.clone());
        tokio::spawn(async move {
            let route = format!("/tokens/{}/revoke", encode(&id));
            let reply = core.account(Method::POST, &route).await;
            let _ = tx.send(Msg::Apply(Box::new(move |app: &mut App| {
                match &reply {
                    Ok(r) if r.ok() => {
                        app.toast(None, "Device signed out".into(), Variant::Success)
                    }
                    // [fix] §B6
                    Ok(r) if r.status == 404 => app.toast(
                        None,
                        format!(
                            "No signed-in device with ID {id}. Run `anysola devices` to list them."
                        ),
                        Variant::Error,
                    ),
                    other => app.toast(None, account::error_text(other), Variant::Error),
                }
                app.state.dialogs.clear();
                app.open_devices();
            })));
        });
    }

    fn budget(&mut self, arguments: &str) {
        let session = self.state.session().map(|s| s.id.clone());
        let parsed = account::parse_budget(arguments);
        let result = match (&parsed, session) {
            (Budget::Usage(text), _) => {
                return self.toast(None, text.to_string(), Variant::Warning);
            }
            (Budget::ShowDefault, _) => Ok(account::default_text(self.core.default_budget())),
            (Budget::SetDefault(limit), _) => {
                self.core.set_default_budget(*limit).map(|()| match limit {
                    Some(_) => account::default_text(*limit),
                    None => "Default budget removed".into(),
                })
            }
            (_, None) => {
                return self.toast(
                    None,
                    "Open a session first, or set one for new sessions: /budget default <percent>"
                        .into(),
                    Variant::Warning,
                );
            }
            (Budget::Show, Some(id)) => {
                let c = self.core.usage(&id);
                return self.toast(None, account::budget_text(c.total, c.budget), Variant::Info);
            }
            (Budget::Set(n), Some(id)) => self
                .core
                .set_budget(&id, Some(*n))
                .map(|c| account::budget_text(c.total, c.budget)),
            (Budget::Off, Some(id)) => self
                .core
                .set_budget(&id, None)
                .map(|_| "Budget removed".to_string()),
        };
        match result {
            Ok(text) => {
                if let Some(s) = self.state.session_mut() {
                    s.usage = Some(self.core.usage(&s.id));
                }
                self.toast(None, text, Variant::Success)
            }
            Err(error) => self.toast(
                None,
                format!("Budget unavailable ({error})."),
                Variant::Error,
            ),
        }
    }

    /// Before a new turn: a spent budget asks first; true when the prompt may go.
    pub(super) fn budget_allows(&mut self, session: &str, prompt: &str) -> bool {
        if std::mem::take(&mut self.gate_passed) {
            return true;
        }
        let c = self.core.usage(session);
        match c.budget {
            Some(budget) if c.total >= budget as f64 => {
                let select = account::budget_gate(c.total, budget, prompt.to_string());
                self.push(Dialog::Select(select));
                false
            }
            _ => true,
        }
    }

    pub(super) fn budget_gate_choice(&mut self, choice: &str, prompt: String) {
        let Some(id) = self.state.session().map(|s| s.id.clone()) else {
            return;
        };
        let c = self.core.usage(&id);
        let changed = match (choice, c.budget) {
            ("raise", Some(b)) => self
                .core
                .set_budget(&id, Some(account::raised_budget(c.total, b)))
                .map(drop),
            ("remove", _) => self.core.set_budget(&id, None).map(drop),
            ("stop", _) => {
                self.state.composer.set(&prompt);
                return;
            }
            _ => Ok(()),
        };
        if let Err(error) = changed {
            self.toast(
                None,
                format!("Budget unavailable ({error})."),
                Variant::Error,
            );
        }
        if let Some(s) = self.state.session_mut() {
            s.usage = Some(self.core.usage(&id));
        }
        self.gate_passed = true;
        self.state.composer.set(&prompt);
        self.submit(false);
    }
}

impl App {
    /// New limits from `/limits`, `/usage` or a redeem; warns once per window near its end.
    pub(super) fn set_limits(&mut self, limits: Limits) {
        self.state.account.plan = limits.plan.clone().or(self.state.account.plan.take());
        self.state.account.limits = Some(limits);
        self.warn_limits();
    }

    /// A toast the first time a window reaches 80% (once per window, keyed on its reset time).
    pub(super) fn warn_limits(&mut self) {
        let now = self.state.now.max(super::now());
        for (window, w) in self.state.limit_warnings() {
            let key = (window, w.resets_at);
            if self.limit_warned.contains(&key) {
                continue;
            }
            self.limit_warned.push(key);
            let text = if w.percent >= 100 {
                format!("{} limit reached.", window.label())
            } else {
                format!(
                    "{}% of the {} limit used.",
                    w.percent,
                    window.label().to_lowercase()
                )
            };
            let resets = format!(" Resets {}.", resets_in(w.resets_at, now));
            self.toast(None, format!("{text}{resets} /usage"), Variant::Warning);
        }
    }

    /// The session's turn was refused for a full usage window: show the gateway's message, and
    /// offer a banked reset when the bank holds one.
    pub(super) fn usage_limit(&mut self, session_id: &str, message: String) {
        self.alert(session_id, &message, crate::sound::Sound::Error);
        let shown = self.state.session().is_some_and(|s| s.id == session_id);
        let (core, tx) = (self.core.clone(), self.tx.clone());
        let session = session_id.to_string();
        tokio::spawn(async move {
            let reply = core.account(Method::GET, "/limits").await;
            let limits = match &reply {
                Ok(r) if r.ok() => Limits::from_wire(&r.body),
                _ => None,
            };
            let _ = tx.send(Msg::Apply(Box::new(move |app: &mut App| {
                let now = app.state.now.max(super::now());
                let offer = limits.as_ref().filter(|_| shown).and_then(|limits| {
                    account::banked_reset_message(&message, limits, now, app.state.zone)
                });
                if let Some(limits) = limits {
                    app.set_limits(limits);
                }
                match offer {
                    Some(text) => app.push(Dialog::Confirm(Confirm {
                        kind: ConfirmKind::BankedReset { session },
                        title: "Use a banked reset?".into(),
                        message: text,
                        button: "Use banked reset".into(),
                        yes: true,
                    })),
                    None => app.toast(Some("Usage limit".into()), message, Variant::Error),
                }
            })));
        });
    }

    /// Spends the banked reset that expires first, then sends the refused turn again.
    pub(super) fn redeem_and_retry(&mut self, session: String) {
        let (core, tx) = (self.core.clone(), self.tx.clone());
        tokio::spawn(async move {
            let reply = core.account(Method::POST, "/limits/redeem").await;
            let _ = tx.send(Msg::Apply(Box::new(move |app: &mut App| match &reply {
                Ok(r) if r.ok() => {
                    if let Some(limits) = Limits::from_wire(&r.body) {
                        app.limit_warned.clear();
                        app.set_limits(limits);
                    }
                    app.toast(None, "Banked reset used".into(), Variant::Success);
                    app.retry_turn(&session);
                }
                other => app.toast(
                    Some("Banked reset".into()),
                    account::error_text(other),
                    Variant::Error,
                ),
            })));
        });
    }

    /// Sends the last prompt of `session` again. A turn refused before anything ran is undone and
    /// resubmitted as it was; one that already ran tools continues with `Continue.`, so their work
    /// is kept.
    fn retry_turn(&mut self, session: &str) {
        if self.state.session().is_none_or(|s| s.id != session) {
            return;
        }
        let Some(items) = self.state.sessions.get(session).map(|s| &s.items) else {
            return;
        };
        let Some(at) = items.iter().rposition(|i| matches!(i, Item::User { .. })) else {
            return;
        };
        let ran = items[at + 1..].iter().any(|item| match item {
            Item::Assistant(a) => a.parts.iter().any(|p| matches!(p, Part::Tool(_))),
            _ => false,
        });
        let Item::User { id, text, .. } = &items[at] else {
            return;
        };
        let (id, text) = (id.clone(), text.clone());
        if ran {
            self.state.composer.set("Continue.");
        } else {
            match self.core.revert(session, &id) {
                Ok(_) => {
                    self.reload(session);
                    self.state.composer.set(&text);
                }
                Err(error) => return self.toast(None, error.to_string(), Variant::Error),
            }
        }
        self.submit(false);
    }
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
