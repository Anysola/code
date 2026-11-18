//! Hooks: what they did (notices from `Event::Hook`), the approval of a project file's hooks,
//! and the `/hooks` list.

use anysola_core::api::HookDocument;
use anysola_core::hooks::HookOutcome;

use super::{App, now};
use crate::dialog::{BodyKind, Dialog, Kind};
use crate::menus::lists;
use crate::state::Variant;
use crate::transcript::model::{self, Item};

/// The approval waits this long after the last key, so it never opens under typing.
const QUIET_MS: u64 = 1500;

/// No key came in for a moment.
fn quiet(typed: u64, now: u64) -> bool {
    now >= typed.saturating_add(QUIET_MS)
}

/// The first non-empty line of a hook's text, without escapes.
fn first_line(text: &str) -> String {
    crate::text::strip_ansi(text)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| crate::text::truncate(&crate::text::visible(l), 300))
        .unwrap_or_default()
}

impl App {
    /// A project file's path as the user reads it: relative to the project, else `~/…`.
    fn show_source(&self, path: &str) -> String {
        crate::text::visible(&self.source_text(path))
    }

    fn source_text(&self, path: &str) -> String {
        if let Some(rest) = path
            .strip_prefix(&self.state.directory)
            .and_then(|r| r.strip_prefix(['/', '\\']))
        {
            return rest.to_string();
        }
        let home = std::path::Path::new(&self.state.home);
        let shown = super::abbreviate(std::path::Path::new(path), home);
        if shown.is_empty() {
            path.to_string()
        } else {
            shown
        }
    }

    /// Reads which project files wait for an answer (at start, after a directory change and on
    /// `HooksChanged`), closes an approval no longer needed, and refreshes an open `/hooks`.
    pub(super) fn refresh_hook_asks(&mut self) {
        let documents = self.core.hook_documents();
        let loaded = self.core.loaded();
        let configured = &loaded.config.hooks;
        // `disable_hooks` in a project file is ignored by the core: only the user's turns
        // project hooks off.
        let off = configured.disabled_by_user || anysola_core::env::flag("DISABLE_HOOKS");
        let undecided: Vec<&HookDocument> = documents
            .iter()
            .filter(|d| d.decision.is_none() && !off)
            .collect();
        // Answered elsewhere or changed since it was shown: a changed file is asked again.
        if let Some(Dialog::Body(b)) = self.state.dialogs.last()
            && let BodyKind::HookTrust {
                path,
                digest,
                review: false,
                ..
            } = &b.kind
            && !undecided
                .iter()
                .any(|d| &d.path == path && &d.digest == digest)
        {
            self.state.dialogs.pop();
        }
        self.hook_asks = undecided
            .into_iter()
            .filter(|d| {
                !self
                    .hook_asked
                    .contains(&(d.path.clone(), d.digest.clone()))
            })
            .cloned()
            .collect();
        self.refresh_hooks_dialog();
    }

    /// The approval can open but waits for the keyboard to go quiet: the loop wakes for it.
    pub(super) fn hook_ask_waiting(&self) -> bool {
        !self.hook_asks.is_empty() && !quiet(self.typed, now())
    }

    /// Opens the next approval when nothing else has the keyboard: no dialog or diff, no
    /// permission or question, nothing typed in the composer, no key for a moment.
    pub(super) fn offer_hooks(&mut self) {
        if self.hook_asks.is_empty()
            || !self.state.dialogs.is_empty()
            || self.state.diff.is_some()
            || !self.state.composer.is_empty()
            || !quiet(self.typed, now())
            || self
                .state
                .session()
                .is_some_and(|s| s.permission.is_some() || s.question.is_some())
        {
            return;
        }
        let document = self.hook_asks.remove(0);
        self.hook_asked
            .insert((document.path.clone(), document.digest.clone()));
        let shown = self.show_source(&document.path);
        let body = lists::hook_trust(&document, &shown, false, now());
        self.push(Dialog::Body(body));
    }

    /// `/hooks`.
    pub(super) fn open_hooks(&mut self) {
        let select = self.hooks_select();
        self.push(Dialog::Select(select));
    }

    fn hooks_select(&self) -> crate::dialog::Select {
        let loaded = self.core.loaded();
        let configured = &loaded.config.hooks;
        let off = if anysola_core::env::flag("DISABLE_HOOKS") {
            Some("off: ANYSOLA_DISABLE_HOOKS")
        } else if configured.disabled_by_user {
            Some("off: disable_hooks")
        } else {
            None
        };
        let config = anysola_core::paths::Paths::from_env().config_file();
        let config = self.show_source(&config.display().to_string());
        let show = |path: &str| self.show_source(path);
        lists::hooks(
            &self.core.hooks(),
            &self.core.hook_documents(),
            &show,
            &config,
            off,
        )
    }

    /// Rebuilds an open `/hooks` (also under a review of one of its files), keeping the query
    /// and the row.
    fn refresh_hooks_dialog(&mut self) {
        let Some(at) = self
            .state
            .dialogs
            .iter()
            .position(|d| matches!(d, Dialog::Select(s) if s.kind == Kind::Hooks))
        else {
            return;
        };
        let mut fresh = self.hooks_select();
        if let Dialog::Select(old) = &self.state.dialogs[at] {
            let limit = crate::dialog::list_limit(self.state.height);
            let value = old.current().map(|o| o.value.clone());
            fresh.set_query(old.query.clone(), limit);
            if let Some(index) = fresh
                .matches()
                .iter()
                .position(|&i| Some(&fresh.options[i].value) == value.as_ref())
            {
                fresh.selected = index;
                fresh.reveal(limit);
            }
            lists::hook_actions(&mut fresh);
        }
        self.state.dialogs[at] = Dialog::Select(fresh);
    }

    /// Enter on a `/hooks` row: a project file's rows open its approval.
    pub(super) fn review_hooks(&mut self, value: &str) {
        let Some(path) = value.strip_prefix("doc:") else {
            return;
        };
        let Some(document) = self
            .core
            .hook_documents()
            .into_iter()
            .find(|d| d.path == path)
        else {
            return;
        };
        let shown = self.show_source(path);
        let body = lists::hook_trust(&document, &shown, true, now());
        self.push(Dialog::Body(body));
    }

    /// The answer of an approval, for the content it showed.
    pub(super) fn decide_hooks(&mut self, path: &str, digest: &str, allow: bool) {
        self.state.dialogs.pop();
        let shown = self.show_source(path);
        match self.core.trust_hooks(path, digest, allow) {
            Ok(()) if allow => {
                self.toast(None, format!("Hooks in {shown} will run."), Variant::Info)
            }
            Ok(()) => self.toast(
                None,
                format!("Hooks in {shown} will not run. Change this in /hooks."),
                Variant::Info,
            ),
            Err(error) => self.toast(
                Some("Failed to save your answer".into()),
                error.to_string(),
                Variant::Error,
            ),
        }
        self.refresh_hook_asks();
    }

    /// `Event::Hook`. Tool hooks show on their tool's row (from its metadata), so only the other
    /// events are shown from here: failures and messages as toasts, context and Stop-hook
    /// reasons as the notice their synthetic row shows once the transcript is reloaded. A
    /// blocked prompt is shown by the failed turn.
    pub(super) fn hook_event(
        &mut self,
        session_id: &str,
        event: &str,
        command: &str,
        outcome: HookOutcome,
        message: &str,
    ) {
        if matches!(event, "PreToolUse" | "PostToolUse") {
            return;
        }
        let description = match (event, outcome) {
            (_, HookOutcome::Error) => {
                let line = first_line(message);
                let text = if line.is_empty() {
                    "The hook failed; the work went on.".to_string()
                } else {
                    line
                };
                return self.toast(Some(format!("{event} hook failed")), text, Variant::Warning);
            }
            (_, HookOutcome::Notice) => {
                let title = (!command.is_empty()).then(|| format!("{event} hook"));
                return self.toast(title, first_line(message), Variant::Info);
            }
            ("Stop", HookOutcome::Blocked | HookOutcome::Context) => model::STOP_HOOK,
            ("UserPromptSubmit" | "SessionStart", HookOutcome::Context) => model::HOOK_CONTEXT,
            _ => return,
        };
        if let Some(notice) = model::hook_notice(description, message)
            && let Some(session) = self.state.sessions.get_mut(session_id)
        {
            session.items.push(Item::Notice(notice));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{QUIET_MS, first_line, quiet};

    #[test]
    fn the_approval_waits_for_the_keyboard_to_go_quiet() {
        assert!(!quiet(10_000, 10_000 + QUIET_MS - 1));
        assert!(quiet(10_000, 10_000 + QUIET_MS));
        assert!(quiet(0, 1_700_000_000_000));
    }

    #[test]
    fn first_line_skips_blank_lines_and_escapes() {
        assert_eq!(
            first_line("\n  \x1b[31mlint failed\x1b[0m\nmore"),
            "lint failed"
        );
        assert_eq!(first_line(""), "");
    }
}
