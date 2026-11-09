//! Commands (keymap ids and `/` names), prompt submission, `!` shell runs, the external editor
//! and suspend.

use anysola_core::api::{Prompt, Target};
use anysola_core::model::ModelRef;
use anysola_core::session::TurnOptions;
use anysola_core::session::inbox::Delivery;
use anysola_core::session::message::InvokedSkill;
use ratatui::Terminal;
use serde_json::Value;

use super::{App, Msg, now};
use crate::composer::{Composer, Entry, Mode};
use crate::keymap::slash::SLASH;
use crate::state::{Route, Variant};
use crate::term::{self, Backend};
use crate::transcript::model::{Item, Part};

/// Work that needs the terminal itself, run by the loop after the key that asked for it.
pub(super) enum Deferred {
    Editor,
    Suspend,
}

fn s(text: &str) -> Value {
    Value::String(text.into())
}

fn title_of(id: &str) -> String {
    SLASH
        .iter()
        .find(|s| s.id == id)
        .map(|s| s.title.to_string())
        .or_else(|| {
            crate::keymap::DEFAULTS
                .iter()
                .find(|(d, _, _)| *d == id)
                .map(|(_, _, description)| description.to_string())
        })
        .unwrap_or_else(|| id.to_string())
}

impl App {
    fn current(&self) -> Option<String> {
        self.state.session().map(|s| s.id.clone())
    }

    pub(super) fn run_command(&mut self, id: &str, arguments: Option<&str>) {
        let session = self.current();
        match (id, session) {
            ("app.exit", _) => self.quit = true,
            ("session.new", _) => {
                self.state.route = Route::Home;
                self.refresh_recent();
            }
            ("session.clear", Some(current)) => {
                self.state.tabs.retain(|t| t.session_id != current);
                self.state.route = Route::Home;
                self.refresh_recent();
            }
            ("session.clear", None) => self.state.composer.clear(),
            ("location.reload", _) => {
                let core = self.core.clone();
                tokio::spawn(async move { core.refresh().await });
            }
            ("session.compact", Some(current)) => {
                if let Some(s) = self.state.sessions.get_mut(&current)
                    && s.busy
                {
                    s.compact_queued = true;
                    return;
                }
                self.compact(&current);
            }
            ("session.undo", Some(current)) => self.undo(&current),
            ("session.redo", Some(current)) => {
                if self.core.unrevert(&current).is_ok() {
                    self.reload(&current);
                }
            }
            ("session.rename", Some(current)) if arguments.is_some() => {
                let title = arguments.unwrap_or_default().trim().to_string();
                if title.is_empty() {
                    let (core, tx) = (self.core.clone(), self.tx.clone());
                    tokio::spawn(async move {
                        let _ = core.regenerate_title(&current).await;
                        let _ = tx.send(Msg::Reload(current));
                    });
                } else if self.core.set_title(&current, &title).is_ok() {
                    if let Some(tab) = self.state.tabs.iter_mut().find(|t| t.session_id == current)
                    {
                        tab.title = title.clone();
                    }
                    if let Some(s) = self.state.sessions.get_mut(&current) {
                        s.title = title;
                    }
                }
            }
            ("messages.copy", Some(_)) => self.copy_last(),
            ("session.copy.id", Some(current)) => {
                term::copy(&current);
                self.toast(
                    None,
                    "Session ID copied to clipboard!".into(),
                    Variant::Info,
                );
            }
            ("session.background", Some(current)) => {
                self.core.background(&current);
            }
            ("agent.cycle" | "agent.cycle.reverse", _) => {
                let agents = &self.state.agents;
                if !agents.is_empty() {
                    let at = self.state.agent_index(&self.state.agent);
                    let next = if id == "agent.cycle" {
                        at + 1
                    } else {
                        at + agents.len() - 1
                    };
                    self.state.agent = agents[next % agents.len()].clone();
                }
            }
            ("variant.cycle", _) => {
                if let Some(model) = &self.state.model {
                    let variants = &model.variants;
                    self.state.variant = match &self.state.variant {
                        None => variants.first().cloned(),
                        Some(v) => variants
                            .iter()
                            .position(|x| x == v)
                            .and_then(|i| variants.get(i + 1))
                            .cloned(),
                    };
                    let id = model.id.clone();
                    let variant = self.state.variant.clone();
                    self.models.set_variant(&id, variant.as_deref());
                }
            }
            ("model.cycle_favorite" | "model.cycle_favorite_reverse", _) => {
                self.cycle_model(id == "model.cycle_favorite", true)
            }
            ("model.cycle_recent" | "model.cycle_recent_reverse", _) => {
                self.cycle_model(id == "model.cycle_recent", false)
            }
            ("session.sidebar.toggle", _) => {
                // 0.5.1 toggles what the right pane shows, so opening it also closes the
                // terminal pane and a docked diff.
                let open = !crate::view::sidebar_shown(&self.state);
                self.state.settings.sidebar = open;
                self.state.sidebar_open = open;
                if open {
                    self.shown = None;
                    self.state.pane = None;
                    if crate::view::diff_docked(&self.state) {
                        self.state.diff = None;
                    }
                }
                let value = if open { "auto" } else { "hide" };
                let _ = self
                    .tui
                    .set(&["session", "sidebar"], Value::String(value.into()));
            }
            (
                "session.message.next"
                | "session.message.previous"
                | "session.message.user.next"
                | "session.message.user.previous"
                | "session.messages_last_user",
                Some(_),
            ) => self.jump_to_message(id),
            (
                "session.page.up"
                | "session.page.down"
                | "session.half.page.up"
                | "session.half.page.down"
                | "session.line.up"
                | "session.line.down"
                | "session.first"
                | "session.last",
                Some(_),
            ) => {
                let view = self.state.height as isize / 2;
                let rows = match id {
                    "session.page.up" => view,
                    "session.page.down" => -view,
                    "session.half.page.up" => view / 2,
                    "session.half.page.down" => -view / 2,
                    "session.line.up" => 1,
                    "session.line.down" => -1,
                    "session.first" => {
                        if let Some(s) = self.state.session_mut().filter(|s| s.floor > 0) {
                            s.history_loading = true;
                            return;
                        }
                        isize::MAX / 4
                    }
                    _ => isize::MIN / 4,
                };
                self.scroll(rows);
            }
            ("prompt.editor", _) => self.deferred = Some(Deferred::Editor),
            ("terminal.suspend", _) if cfg!(unix) => self.deferred = Some(Deferred::Suspend),
            ("prompt.queue", _) => self.submit(true),
            ("session.child.first", Some(_)) => self.open_dock(),
            ("session.parent", Some(_)) => self.go_parent(),
            ("session.child.next", Some(_)) => self.go_sibling(1),
            ("session.child.previous", Some(_)) => self.go_sibling(-1),
            ("session.cd", _) if arguments.is_some_and(|a| a.trim().is_empty()) => {
                self.toast(None, "Directory is required".into(), Variant::Warning);
            }
            ("session.cd", _) => self.change_directory(arguments.unwrap_or_default()),
            ("session.aside", Some(_)) if arguments.is_some_and(|a| !a.trim().is_empty()) => {
                self.btw(arguments.unwrap_or_default().trim().to_string())
            }
            ("session.copy", Some(_)) => {
                let text = self.transcript(true, true);
                term::copy(&text);
                self.toast(
                    None,
                    "Session transcript copied to clipboard!".into(),
                    Variant::Success,
                );
            }
            ("prompt.stash", _) if !self.state.composer.is_empty() => {
                self.stash.push(self.state.composer.expanded(), now());
                self.state.composer.clear();
            }
            ("prompt.editor_context.clear", _) => self.editor = None,
            ("session.tab.close", Some(current)) => {
                self.state.tabs.retain(|t| t.session_id != current);
                self.closed.push(current);
                self.state.route = Route::Home;
                self.refresh_recent();
            }
            ("session.tab.next", _) => self.cycle_tab(1, false),
            ("session.tab.previous", _) => self.cycle_tab(-1, false),
            ("session.tab.next_unread", _) => self.cycle_tab(1, true),
            ("session.tab.previous_unread", _) => self.cycle_tab(-1, true),
            (select, _) if select.starts_with("session.tab.select.") => {
                if let Ok(number) = select["session.tab.select.".len()..].parse() {
                    self.select_tab(number);
                }
            }
            ("session.tab.reopen", _) => {
                if let Some(id) = self.closed.pop() {
                    self.open_session(&id);
                }
            }
            ("theme.switch_mode", _) => {
                let mode = match self.theme.mode {
                    crate::theme::Mode::Dark => crate::theme::Mode::Light,
                    crate::theme::Mode::Light => crate::theme::Mode::Dark,
                };
                if self.theme.modes.contains(&mode)
                    && let Ok(theme) = self.catalog.load(&self.theme.name, mode)
                {
                    self.theme = theme;
                }
            }
            ("theme.mode.lock", _) => {
                let locked = self.tui.theme_mode().is_some();
                let value = match (locked, self.theme.mode) {
                    (true, _) => s("system"),
                    (false, crate::theme::Mode::Dark) => s("dark"),
                    (false, crate::theme::Mode::Light) => s("light"),
                };
                let _ = self.tui.set(&["theme", "mode"], value);
            }
            ("prompt.images.view", _) => self.view_images(crate::preview::Set::Composer, 0),
            ("prompt.paste", _) => self.paste_clipboard(),
            ("session.queued_prompts", Some(current)) => self.open_queue(&current),
            ("diff.open", _) if self.state.diff.is_some() => self.state.diff = None,
            ("diff.open", _) => self.open_diff(crate::diffview::Source::All),
            ("prompt.stash.pop", _) => {
                if let Some(text) = self.stash.pop() {
                    self.state.composer.set(&text);
                }
            }
            ("prompt.stash.list", _) if !self.stash.is_empty() => self.open_stash(),
            (id, _) if self.toggle(id) || self.pane_command(id) => {}
            (id, session) => {
                if !self.open_dialog(id, session) && !self.account_command(id, arguments) {
                    self.toast(
                        None,
                        format!("{} is not available here", title_of(id)),
                        Variant::Info,
                    );
                }
            }
        }
    }

    /// Palette toggles of a `tui.json` setting; false when `id` is not one.
    fn toggle(&mut self, id: &str) -> bool {
        let (path, values): (&[&str], &[Value]) = match id {
            "terminal.title.toggle" => (
                &["terminal", "title"],
                &[Value::Bool(true), Value::Bool(false)],
            ),
            "app.toggle.animations" => (&["animations"], &[Value::Bool(true), Value::Bool(false)]),
            "app.toggle.file_context" => (
                &["prompt", "editor"],
                &[Value::Bool(true), Value::Bool(false)],
            ),
            "app.toggle.paste_summary" => (&["prompt", "paste"], &[s("compact"), s("full")]),
            "app.toggle.diffwrap" => (&["diffs", "wrap"], &[s("word"), s("none")]),
            "session.toggle.thinking" => (&["session", "thinking"], &[s("hide"), s("show")]),
            "session.toggle.scrollbar" => (
                &["session", "scrollbar"],
                &[Value::Bool(false), Value::Bool(true)],
            ),
            "session.toggle.exploration_grouping" => {
                (&["session", "grouping"], &[s("auto"), s("none")])
            }
            "session.verbosity.cycle" => (
                &["session", "verbosity"],
                &[s("medium"), s("high"), s("low")],
            ),
            _ => return false,
        };
        let current = path
            .iter()
            .try_fold(&self.tui.value, |v, k| v.get(k))
            .cloned()
            .unwrap_or_else(|| values[0].clone());
        let at = values.iter().position(|v| *v == current).unwrap_or(0);
        let next = values[(at + 1) % values.len()].clone();
        let _ = self.tui.set(path, next);
        self.state.settings = self.tui.settings();
        true
    }

    /// `f2`/`shift+f2` walk the recent models, `model.cycle_favorite(_reverse)` the favourites;
    /// ids no longer offered are skipped, an unknown current model starts at either end.
    fn cycle_model(&mut self, forward: bool, favorites: bool) {
        let loaded = self.core.loaded();
        let list: Vec<String> = if favorites {
            &self.models.favorite
        } else {
            &self.models.recent
        }
        .iter()
        .filter(|id| loaded.models.iter().any(|m| m.id == **id))
        .cloned()
        .collect();
        if list.is_empty() {
            if favorites {
                self.toast(
                    None,
                    "Add a favorite model to use this shortcut".into(),
                    Variant::Info,
                );
            }
            return;
        }
        let current = self.state.model.as_ref().map(|m| m.id.clone());
        let count = list.len();
        let next = match current.and_then(|c| list.iter().position(|id| *id == c)) {
            Some(at) if forward => (at + 1) % count,
            Some(at) => (at + count - 1) % count,
            None if forward => 0,
            None => count - 1,
        };
        let provider = self
            .state
            .model
            .as_ref()
            .map_or_else(|| "anysola".to_string(), |m| m.provider.clone());
        let reference = ModelRef {
            provider,
            id: list[next].clone(),
            variant: None,
        };
        self.state.model = self.model_choice(&reference);
        self.state.variant = self.remembered_variant(&reference.id);
    }

    /// The variant last used with this model, while the model still offers it.
    pub(super) fn remembered_variant(&self, id: &str) -> Option<String> {
        let variant = self.models.variant(id).flatten()?;
        self.state
            .model
            .as_ref()
            .is_some_and(|m| m.variants.contains(&variant))
            .then_some(variant)
    }

    fn undo(&mut self, current: &str) {
        let Some(session) = self.state.sessions.get(current) else {
            return;
        };
        let last = session.items.iter().rev().find_map(|item| match item {
            Item::User { id, text, .. } => Some((id.clone(), text.clone())),
            _ => None,
        });
        let Some((message_id, text)) = last else {
            self.toast(None, "Nothing to undo".into(), Variant::Warning);
            return;
        };
        if session.busy {
            self.core.interrupt(current);
        }
        match self.core.revert(current, &message_id) {
            Ok(_) => {
                self.state.composer.set(&text);
                self.reload(current);
            }
            Err(error) => self.toast(None, error.to_string(), Variant::Error),
        }
    }

    fn copy_last(&mut self) {
        let Some(session) = self.state.session() else {
            return;
        };
        let last = session.items.iter().rev().find_map(|item| match item {
            Item::Assistant(a) => Some(a),
            _ => None,
        });
        let Some(assistant) = last else {
            return self.toast(None, "No assistant messages found".into(), Variant::Error);
        };
        let text: Vec<&str> = assistant
            .parts
            .iter()
            .filter_map(|p| match p {
                Part::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        if text.is_empty() {
            return self.toast(
                None,
                "No text content found in last assistant message".into(),
                Variant::Error,
            );
        }
        term::copy(&text.join("\n\n"));
        self.toast(None, "Message copied to clipboard!".into(), Variant::Info);
    }

    /// The session prompts go to: the open one, or a new one created on the spot (home).
    fn ensure_session(&mut self) -> Option<String> {
        if let Some(id) = self.current() {
            return Some(id);
        }
        match self.core.open(&Target::New, false) {
            Ok((row, _)) => {
                self.open_session(&row.id);
                Some(row.id)
            }
            Err(error) => {
                self.toast(
                    Some("Failed to set up session".into()),
                    error.to_string(),
                    Variant::Error,
                );
                None
            }
        }
    }

    /// Commits the composer's agent and model choice to the session.
    fn commit_choice(&mut self, id: &str) {
        let Ok(Some(row)) = self.core.session(id) else {
            return;
        };
        let loaded = self.core.loaded();
        let default = loaded.agents.default_agent().map(|a| a.id.as_str());
        if row.agent.as_deref().or(default) != Some(self.state.agent.as_str()) {
            let _ = self.core.select_agent(id, &self.state.agent);
        }
        if let Some(model) = &self.state.model {
            let reference = ModelRef {
                provider: model.provider.clone(),
                id: model.id.clone(),
                variant: self.state.variant.clone(),
            };
            let stored = row
                .model
                .as_ref()
                .and_then(ModelRef::from_json)
                .or_else(|| loaded.config.model().and_then(ModelRef::parse));
            if stored.as_ref() != Some(&reference)
                && let Err(error) = self.core.select_model(id, &reference)
            {
                self.toast(
                    None,
                    format!("Failed to switch model: {error}"),
                    Variant::Error,
                );
            }
        }
    }

    pub(super) fn submit(&mut self, queue: bool) {
        // The gate's pass is for this submit only, whichever way it goes.
        let passed = std::mem::take(&mut self.gate_passed);
        let text = self.state.composer.expanded();
        let trimmed = text.trim().to_string();
        let shell = self.state.composer.mode == Mode::Shell;
        if trimmed.is_empty() {
            return;
        }
        if !shell && matches!(trimmed.as_str(), "exit" | "quit" | ":q") {
            self.quit = true;
            return;
        }
        if shell {
            if queue {
                return self.toast(
                    None,
                    "This prompt cannot be queued".into(),
                    Variant::Warning,
                );
            }
            return self.shell(trimmed);
        }
        if let Some(rest) = trimmed.strip_prefix('/') {
            let (name, arguments) = rest.split_once(' ').unwrap_or((rest, ""));
            let tui = SLASH
                .iter()
                .find(|s| s.name == name || s.aliases.contains(&name));
            if let Some(command) = tui
                && (command.arguments || arguments.is_empty())
            {
                if queue && command.arguments {
                    return self.toast(
                        None,
                        "This prompt cannot be queued".into(),
                        Variant::Warning,
                    );
                }
                self.history.push(Entry {
                    text: trimmed.clone(),
                    shell: false,
                });
                self.state.composer.clear();
                self.state.popup = None;
                let arguments = command.arguments.then_some(arguments);
                return self.run_command(command.id, arguments);
            }
            if self.core.loaded().commands.get(name).is_some() {
                if self.submit_in_new_worktree() {
                    return;
                }
                return self.server_command(
                    name.to_string(),
                    arguments.to_string(),
                    trimmed.clone(),
                );
            }
        }
        if self.submit_in_new_worktree() {
            return;
        }
        self.prompt(trimmed, queue, passed);
    }

    fn server_command(&mut self, name: String, arguments: String, text: String) {
        let Some(id) = self.ensure_session() else {
            return;
        };
        self.commit_choice(&id);
        self.history.push(Entry { text, shell: false });
        self.state.composer.clear();
        self.state.popup = None;
        if let Some(s) = self.state.sessions.get_mut(&id) {
            s.busy = true;
        }
        let (core, tx) = (self.core.clone(), self.tx.clone());
        tokio::spawn(async move {
            let result = core
                .command(&id, &name, &arguments, TurnOptions::default())
                .await;
            let error = result.err().map(|e| e.to_string());
            let _ = tx.send(Msg::Done {
                session_id: id,
                error,
            });
        });
    }

    fn prompt(&mut self, text: String, queue: bool, passed: bool) {
        let Some(id) = self.ensure_session() else {
            return;
        };
        let busy = self.state.sessions.get(&id).is_some_and(|s| s.busy);
        if !busy && !queue && !self.budget_allows(&id, passed) {
            return;
        }
        self.commit_choice(&id);
        if let Some(selection) = self.editor.as_ref().filter(|_| !self.editor_sent)
            && self.tui.editor_context()
        {
            let note = crate::editor::reminder(selection);
            if let Err(error) = self.core.admit_note(&id, &note) {
                return self.toast(
                    Some("Failed to send editor context".into()),
                    error.to_string(),
                    Variant::Error,
                );
            }
            self.editor_sent = true;
        }
        let loaded = self.core.loaded();
        let skills = self
            .state
            .composer
            .skills()
            .iter()
            .filter_map(|mention| {
                let name = mention.trim_start_matches('@');
                let skill = loaded
                    .skills
                    .iter()
                    .find(|s| s.name == name || s.id == name)?;
                Some(InvokedSkill::new(skill, mention))
            })
            .collect();
        let prompt = Prompt {
            text: text.clone(),
            files: self.state.composer.attachments(),
            skills,
        };
        self.history.push(Entry {
            text: text.clone(),
            shell: false,
        });
        let images = self.state.composer.images();
        let draft = self.state.composer.clone();
        self.state.composer.clear();
        self.state.popup = None;
        let agent = self.state.agent.clone();
        let Some(session) = self.state.sessions.get_mut(&id) else {
            return;
        };
        session.scroll = 0;
        // Turns the transcript had before this prompt; a failed prompt that never landed
        // leaves it at that and gets its draft back.
        let users = user_count(&session.items);
        session.items.push(Item::User {
            id: String::new(),
            text,
            agent,
            files: Vec::new(),
            images,
            created: now(),
        });
        if session.busy || queue {
            let delivery = if queue {
                Delivery::Queue
            } else {
                Delivery::Steer
            };
            match self.core.enqueue(&id, prompt, delivery) {
                Ok(pending) => session.queued.push(pending),
                Err(error) => {
                    session.items.pop();
                    self.state.composer = draft;
                    self.toast(
                        Some("Failed to send prompt".into()),
                        error.to_string(),
                        Variant::Error,
                    );
                }
            }
            return;
        }
        session.busy = true;
        let (core, tx) = (self.core.clone(), self.tx.clone());
        tokio::spawn(async move {
            let error = core
                .prompt(&id, prompt, TurnOptions::default())
                .await
                .err()
                .map(|e| e.to_string());
            let failed = error.is_some();
            let _ = tx.send(Msg::Done {
                session_id: id.clone(),
                error,
            });
            if failed {
                // After `Done` reloaded the transcript from storage.
                let _ = tx.send(Msg::Apply(Box::new(move |app: &mut App| {
                    app.restore_unsent(&id, users, draft);
                })));
            }
        });
    }

    /// Puts a failed prompt back when it never reached the session (no new turn stored, none
    /// pending), into the composer if it is still empty, or the tab's draft.
    fn restore_unsent(&mut self, id: &str, users: usize, draft: Composer) {
        let stored = self
            .state
            .sessions
            .get(id)
            .is_some_and(|s| user_count(&s.items) > users);
        let pending = self.core.pending(id).map_or(true, |p| !p.is_empty());
        if stored || pending {
            return;
        }
        if self.state.route == Route::Session(id.to_string()) {
            if self.state.composer.is_empty() {
                self.state.composer = draft;
            }
        } else {
            self.drafts.entry(id.to_string()).or_insert(draft);
        }
    }

    fn shell(&mut self, command: String) {
        let Some(id) = self.ensure_session() else {
            return;
        };
        self.history.push(Entry {
            text: command.clone(),
            shell: true,
        });
        self.state.composer.clear();
        self.state.composer.mode = Mode::Normal;
        let (core, tx) = (self.core.clone(), self.tx.clone());
        tokio::spawn(async move {
            if let Err(error) = core.run_shell(&id, &command).await {
                let _ = tx.send(Msg::Toast(
                    Some("Failed to run shell command".into()),
                    error.to_string(),
                    Variant::Error,
                ));
            }
            let _ = tx.send(Msg::Reload(id));
        });
    }

    pub(super) fn run_deferred(&mut self, terminal: &mut Terminal<Backend>) {
        match self.deferred.take() {
            Some(Deferred::Editor) => self.editor(terminal),
            Some(Deferred::Suspend) => {
                term::restore();
                #[cfg(unix)]
                // SAFETY: raising a signal on our own process.
                unsafe {
                    libc::raise(libc::SIGTSTP);
                }
                let _ = term::resume(self.tui.mouse());
                let _ = terminal.clear();
            }
            None => {}
        }
    }

    fn compact(&mut self, session: &str) {
        if let Some(s) = self.state.sessions.get_mut(session) {
            s.busy = true;
            s.compact_queued = false;
        }
        let (core, tx) = (self.core.clone(), self.tx.clone());
        let session = session.to_string();
        tokio::spawn(async move {
            let error = core.compact(&session).await.err().map(|e| e.to_string());
            let _ = tx.send(Msg::Done {
                session_id: session,
                error,
            });
        });
    }

    /// A `/compact` asked mid-turn starts once the turn is over.
    pub(super) fn run_queued_compaction(&mut self, session: &str) {
        if self
            .state
            .sessions
            .get(session)
            .is_some_and(|s| s.compact_queued && !s.busy)
        {
            self.compact(session);
        }
    }

    fn editor(&mut self, terminal: &mut Terminal<Backend>) {
        let program = std::env::var("VISUAL")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| {
                std::env::var("EDITOR")
                    .ok()
                    .filter(|v| !v.trim().is_empty())
            });
        let Some(program) = program else {
            return self.toast(
                Some("Failed to open editor".into()),
                "Set $VISUAL or $EDITOR to open an external editor".into(),
                Variant::Error,
            );
        };
        let marks = self.state.composer.mention_marks();
        let file = std::env::temp_dir().join(format!("{}.md", now()));
        if std::fs::write(&file, self.state.composer.expanded()).is_err() {
            return;
        }
        term::restore();
        let mut parts = program.split_whitespace();
        let status = std::process::Command::new(anysola_core::which::program(
            parts.next().unwrap_or_default(),
        ))
        .args(parts)
        .arg(&file)
        .current_dir(self.core.directory())
        .status();
        let _ = term::resume(self.tui.mouse());
        let _ = terminal.clear();
        match status {
            Ok(status) if status.success() => {
                let content = std::fs::read_to_string(&file).unwrap_or_default();
                let content = match content.strip_suffix('\n') {
                    Some(single) if !single.contains('\n') => single.to_string(),
                    _ => content,
                };
                if !content.is_empty() {
                    self.state.composer.realign(&content, &marks);
                }
            }
            Ok(status) => {
                let detail = match status.code() {
                    Some(code) => format!("Editor exited with code {code}"),
                    None => "Editor exited with signal".to_string(),
                };
                self.toast(Some("Failed to open editor".into()), detail, Variant::Error);
            }
            Err(error) => self.toast(
                Some("Failed to open editor".into()),
                error.to_string(),
                Variant::Error,
            ),
        }
        let _ = std::fs::remove_file(&file);
    }

    /// Message navigation: puts the next or previous (user) message, or the last user message,
    /// at the top of the transcript.
    fn jump_to_message(&mut self, id: &str) {
        let Some(session) = self.state.session_mut() else {
            return;
        };
        let (messages, (total, height)) = {
            let cache = session.layout.0.lock().unwrap_or_else(|e| e.into_inner());
            (cache.messages.clone(), cache.viewport)
        };
        let top = total.saturating_sub(session.scroll + height);
        let users = id.contains("user");
        let mut candidates = messages
            .iter()
            .filter(|(_, user)| !users || *user)
            .map(|(row, _)| *row);
        let target = match id {
            "session.message.next" | "session.message.user.next" => {
                candidates.find(|row| *row > top)
            }
            "session.messages_last_user" => candidates.next_back(),
            _ => candidates.rev().find(|row| *row < top),
        };
        if let Some(row) = target {
            session.scroll = total.saturating_sub(row + height);
        }
    }
}

/// The user turns of a transcript.
fn user_count(items: &[Item]) -> usize {
    items
        .iter()
        .filter(|i| matches!(i, Item::User { .. }))
        .count()
}
