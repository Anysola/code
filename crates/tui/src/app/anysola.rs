//! The Anysola account commands: `/credits`, `/usage`, `/devices`, `/budget`, `/account`,
//! `/feedback`, and the budget gate before a prompt.

use anysola_core::gateway::account::Method;

use super::{App, Msg};
use crate::dialog::{Dialog, Opt};
use crate::menus::anysola::{self as account, Budget, CreditsView};
use crate::state::Variant;

impl App {
    /// Account commands; false when `id` is not one.
    pub(super) fn account_command(&mut self, id: &str, arguments: Option<&str>) -> bool {
        match id {
            "anysola.credits" => {
                self.fetch_account();
                let body = self.credits_body();
                self.push(Dialog::Body(body));
            }
            "anysola.usage" => self.open_usage(7),
            "anysola.devices" => self.open_devices(),
            "anysola.budget" => self.budget(arguments.unwrap_or_default()),
            "anysola.account" => self.open_url(account::ACCOUNT_URL),
            "anysola.feedback" => self.open_url(account::FEEDBACK_URL),
            _ => return false,
        }
        true
    }

    pub(super) fn credits_body(&self) -> crate::dialog::Body {
        let session = self
            .state
            .session()
            .and_then(|s| s.credits.as_ref())
            .map(|c| (c.turns, c.total));
        let me = match (&self.state.account.me, &self.state.account.error) {
            (Some(me), _) => Ok(me),
            (None, error) => Err(error.as_deref()),
        };
        account::credits_dialog(&CreditsView {
            me,
            balance: self.state.remaining(),
            session,
            low: self.state.low_balance().is_some(),
            zone: self.state.zone,
            now: self.state.now.max(super::now()),
        })
    }

    pub(super) fn open_usage(&mut self, days: u32) {
        self.push(Dialog::Body(account::usage_dialog(
            days,
            None,
            self.state.zone,
        )));
        let (core, tx) = (self.core.clone(), self.tx.clone());
        tokio::spawn(async move {
            let reply = core
                .account(Method::GET, &format!("/usage?days={days}"))
                .await;
            let _ = tx.send(Msg::Apply(Box::new(move |app: &mut App| {
                let body = account::usage_dialog(days, Some(&reply), app.state.zone);
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
                    "Open a session first, or set one for new sessions: /budget default <credits>"
                        .into(),
                    Variant::Warning,
                );
            }
            (Budget::Show, Some(id)) => {
                let c = self.core.credits(&id);
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
                    s.credits = Some(self.core.credits(&s.id));
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
        let c = self.core.credits(session);
        match c.budget {
            Some(budget) if c.total >= budget => {
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
        let c = self.core.credits(&id);
        let changed = match (choice, c.budget) {
            ("raise", Some(b)) => self
                .core
                .set_budget(&id, Some(c.total.max(b) + b))
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
            s.credits = Some(self.core.credits(&id));
        }
        self.gate_passed = true;
        self.state.composer.set(&prompt);
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
