//! Opening dialogs for commands and acting on what is chosen in them.

use anysola_core::model::ModelRef;
use anysola_core::session::inbox::Delivery;
use serde_json::Value;

use super::{App, Msg, now};
use crate::dialog::{
    Body, BodyKind, Confirm, ConfirmKind, Dialog, Export, Kind, Prompt, PromptKind, Select, Size,
    Tone, line, list_limit,
};
use crate::keymap::Stroke;
use crate::menus::lists::{self, Connection, McpRow, ModelRow, SessionItem};
use crate::menus::{palette, settings};
use crate::state::{Route, Variant};
use crate::term;
use crate::transcript::model::Item;

impl App {
    pub(super) fn push(&mut self, dialog: Dialog) {
        self.state.dialogs.push(dialog);
    }

    fn limit(&self) -> usize {
        list_limit(self.state.height)
    }

    fn top_select(&mut self) -> Option<&mut Select> {
        match self.state.dialogs.last_mut() {
            Some(Dialog::Select(s)) => Some(s),
            _ => None,
        }
    }

    /// Commands that open a dialog; false when `id` is not one.
    pub(super) fn open_dialog(&mut self, id: &str, current: Option<String>) -> bool {
        match id {
            "command.palette.show" => {
                let cx = self.palette_context();
                let select = palette::dialog(&self.keymap, &cx);
                self.push(Dialog::Select(select));
            }
            "session.list" => self.open_sessions(false),
            "open.menu" => self.open_menu(),
            "model.list" => self.open_models(),
            "agent.list" => {
                let loaded = self.core.loaded();
                let rows: Vec<(String, String)> = self
                    .state
                    .agents
                    .iter()
                    .map(|id| {
                        let description = loaded
                            .agents
                            .list
                            .iter()
                            .find(|a| &a.id == id)
                            .and_then(|a| a.description.clone())
                            .unwrap_or_default();
                        (id.clone(), description)
                    })
                    .collect();
                self.push(Dialog::Select(lists::agents(&rows, &self.state.agent)));
            }
            "variant.list" => {
                let variants = self
                    .state
                    .model
                    .as_ref()
                    .map(|m| m.variants.clone())
                    .unwrap_or_default();
                if variants.is_empty() {
                    self.toast(
                        Some("No variants available".into()),
                        "The current model does not support any variants.".into(),
                        Variant::Info,
                    );
                } else {
                    let select = lists::variants(&variants, self.state.variant.as_deref());
                    self.push(Dialog::Select(select));
                }
            }
            "mcp.list" => {
                let select = lists::mcps(&self.mcp_rows());
                self.push(Dialog::Select(select));
            }
            "opencode.status" => self.push(Dialog::Body(lists::status(&self.mcp_rows()))),
            "provider.connect" => {
                let connection = if anysola_core::gateway::env_token().is_some() {
                    Connection::Env
                } else if self.core.signed_in() {
                    Connection::Login
                } else {
                    Connection::None
                };
                self.push(Dialog::Select(lists::connect(connection)));
            }
            "opencode.settings" => self.push(Dialog::Select(settings::dialog(&self.tui.value, 0))),
            "theme.switch" => {
                let select = lists::themes(&self.catalog.names(), &self.theme.name);
                self.push(Dialog::Select(select));
                let limit = self.limit();
                if let Some(s) = self.top_select() {
                    s.reveal(limit);
                }
            }
            "help.show" => {
                let key = self
                    .keymap
                    .first("command.palette.show")
                    .unwrap_or_else(|| "ctrl+p".into());
                self.push(Dialog::Body(lists::help(&key)));
            }
            "opencode.debug" => {
                let info = lists::DebugInfo {
                    version: self.state.version.clone(),
                    date: anysola_core::time::iso(now()),
                    os: describe_os(),
                    terminal: describe_terminal(),
                    session: current,
                    model: self
                        .state
                        .model
                        .as_ref()
                        .map(|m| format!("{}/{}", m.provider, m.id)),
                };
                let mut body = lists::debug(&info);
                body.size = crate::dialog::Size::Large;
                self.push(Dialog::Body(body));
            }
            "opencode.update" => self.check_update(true),
            "docs.open" => self.open_url("https://www.anysola.com/code"),
            "session.timeline" | "session.fork" => {
                let Some(session) = self.state.session() else {
                    return true;
                };
                let messages: Vec<(String, String, u64)> = session
                    .items
                    .iter()
                    .filter_map(|item| match item {
                        Item::User {
                            id, text, created, ..
                        } if !id.is_empty() => Some((id.clone(), text.clone(), *created)),
                        _ => None,
                    })
                    .collect();
                let select = if id == "session.timeline" {
                    lists::timeline(&messages, self.state.zone)
                } else {
                    lists::fork(&messages, self.state.zone)
                };
                self.push(Dialog::Select(select));
            }
            "session.rename" => {
                let Some(session) = self.state.session() else {
                    return true;
                };
                let prompt = rename_prompt(&session.id, &session.title);
                self.push(Dialog::Prompt(prompt));
            }
            "session.export" => self.push(Dialog::Export(Export {
                json: false,
                thinking: true,
                tools: true,
                sanitize: false,
                focus: 0,
            })),
            "prompt.skills" => {
                let loaded = self.core.loaded();
                let mut skills: Vec<(String, String)> = loaded
                    .skills
                    .iter()
                    .map(|s| (s.name.clone(), s.description.clone().unwrap_or_default()))
                    .collect();
                skills.sort_by_key(|(n, _)| n.to_lowercase());
                self.push(Dialog::Select(lists::skills(&skills)));
            }
            "session.aside" if current.is_none() => {
                self.toast(None, "Open a session first".into(), Variant::Warning)
            }
            "session.aside" => self.push(Dialog::Prompt(Prompt {
                kind: PromptKind::Btw,
                title: "/btw".into(),
                placeholder: "Ask anything".into(),
                text: String::new(),
                busy: None,
            })),
            "session.move" => {
                let home = std::path::Path::new(&self.state.home);
                let show = |d: &str| super::abbreviate(std::path::Path::new(d), home);
                let others: Vec<String> = self
                    .core
                    .worktrees()
                    .iter()
                    .map(|w| show(&w.directory))
                    .filter(|d| *d != show(&self.state.directory))
                    .collect();
                let select = lists::worktrees(
                    &show(&self.state.directory),
                    &others,
                    self.state.session().is_some(),
                );
                self.push(Dialog::Select(select));
            }
            _ => return false,
        }
        true
    }

    /// A click on the footer's directory label: `Working directory` actions.
    pub(super) fn working_directory_menu(&mut self) {
        let options = vec![
            crate::dialog::Opt::new("Copy path", "location.copy")
                .description(self.state.directory.clone()),
            crate::dialog::Opt::new("Open folder", "location.open")
                .description("in system file manager"),
            crate::dialog::Opt::new("Workspaces", "session.move")
                .description("to another working directory"),
        ];
        let select =
            crate::dialog::Select::new(Kind::WorkingDirectory, "Working directory", options);
        self.push(Dialog::Select(select));
    }

    pub(super) fn palette_context(&self) -> palette::Context {
        let session = self.state.session();
        palette::Context {
            session: session.is_some(),
            sessions_exist: !self.state.recent.is_empty(),
            connected: self.core.signed_in(),
            variants: self
                .state
                .model
                .as_ref()
                .is_some_and(|m| !m.variants.is_empty()),
            revert: session.is_some_and(|s| s.reverted),
            stash: !self.stash.is_empty(),
            prompt_text: !self.state.composer.is_empty(),
            tabs: true,
            terminal_open: false,
            queued: session.is_some_and(|s| !s.queued.is_empty()),
            editor_context: self.editor.is_some(),
        }
    }

    pub(super) fn mcp_rows(&self) -> Vec<McpRow> {
        self.core
            .mcp()
            .map(|m| m.servers())
            .unwrap_or_default()
            .into_iter()
            .map(|s| McpRow {
                name: s.name,
                status: s.status,
            })
            .collect()
    }

    fn open_models(&mut self) {
        let loaded = self.core.loaded();
        let rows: Vec<ModelRow> = loaded
            .models
            .iter()
            .map(|m| ModelRow {
                provider: "anysola".into(),
                id: m.id.clone(),
                name: m.name.clone(),
                note: m.note.clone(),
                estimate: m.credits_per_turn,
            })
            .collect();
        let current = self.state.model.as_ref().map(|m| m.id.as_str());
        let select = lists::models(&rows, current, &self.models.favorite, &self.models.recent);
        self.push(Dialog::Select(select));
    }

    pub(super) fn open_sessions(&mut self, all: bool) {
        let rows = self.core.sessions(all, "", 50).unwrap_or_default();
        let select = self.sessions_select(rows, all);
        self.push(Dialog::Select(select));
    }

    fn sessions_select(
        &self,
        rows: Vec<anysola_core::storage::SessionSummary>,
        all: bool,
    ) -> Select {
        let directory = std::path::Path::new(&self.state.directory);
        let project = directory
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let items: Vec<SessionItem> = rows
            .into_iter()
            .map(|s| {
                let marker = self.state.sessions.get(&s.id).and_then(|open| {
                    if open.permission.is_some() {
                        Some('!')
                    } else if open.question.is_some() {
                        Some('?')
                    } else {
                        None
                    }
                });
                let elsewhere =
                    (!std::path::Path::new(&s.directory).starts_with(directory)).then(|| {
                        std::path::Path::new(&s.directory)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    });
                SessionItem {
                    id: s.id,
                    title: s.title,
                    updated: s.time_updated,
                    elsewhere,
                    marker,
                }
            })
            .collect();
        let project = (!all).then_some(project.as_str());
        lists::sessions(&items, project, self.state.now.max(now()), self.state.zone)
    }

    /// `Open`: recent sessions of every project not in a tab, then every known project.
    fn open_menu(&mut self) {
        let home = std::path::PathBuf::from(&self.state.home);
        let projects: Vec<(String, String)> = self.core.projects(50).unwrap_or_default();
        let name_of = |directory: &str| {
            std::path::Path::new(directory)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| directory.to_string())
        };
        let now = now();
        let sessions: Vec<lists::OpenSession> = self
            .core
            .sessions(true, "", 50)
            .unwrap_or_default()
            .into_iter()
            .filter(|s| !self.state.tabs.iter().any(|t| t.session_id == s.id))
            .take(8)
            .map(|s| lists::OpenSession {
                project: name_of(&s.directory),
                ago: short_ago(s.time_updated, now),
                id: s.id,
                title: s.title,
            })
            .collect();
        let current = self.core.directory().to_path_buf();
        let mut seen = std::collections::HashSet::new();
        let projects: Vec<lists::OpenProject> = projects
            .into_iter()
            .filter(|(_, worktree)| worktree != "/" && seen.insert(worktree.clone()))
            .map(|(_, worktree)| {
                let path = std::path::Path::new(&worktree);
                lists::OpenProject {
                    name: name_of(&worktree),
                    shown: super::abbreviate(path, &home),
                    git: path.join(".git").exists(),
                    current: current.starts_with(path),
                    directory: worktree,
                }
            })
            .collect();
        let select = lists::open(&sessions, &projects, self.state.width as usize);
        self.push(Dialog::Select(select));
    }

    /// Keys while a dialog is open.
    pub(super) fn dialog_key(&mut self, stroke: &Stroke) {
        let limit = self.limit();
        let key = stroke.key.as_str();
        let close = key == "escape" || (stroke.ctrl && key == "c");
        let Some(top) = self.state.dialogs.last_mut() else {
            return;
        };
        match top {
            Dialog::Select(s) => {
                if close {
                    return self.close_dialog();
                }
                match (key, stroke.ctrl) {
                    ("up", _) | ("p", true) | ("k", true) => s.step(-1, limit),
                    ("down", _) | ("n", true) | ("j", true) => s.step(1, limit),
                    ("pageup", _) => s.jump(-(limit as isize), limit),
                    ("pagedown", _) => s.jump(limit as isize, limit),
                    ("home", _) => s.jump(isize::MIN / 2, limit),
                    ("end", _) => s.jump(isize::MAX / 2, limit),
                    ("return", _) => return self.choose_option(),
                    _ => {
                        if self.select_action(stroke) {
                            return;
                        }
                        let Some(Dialog::Select(s)) = self.state.dialogs.last_mut() else {
                            return;
                        };
                        let mut query = s.query.clone();
                        if key == "backspace" {
                            query.pop();
                        } else if let Some(text) = typed(stroke) {
                            query.push_str(&text);
                        } else {
                            return;
                        }
                        s.set_query(query, limit);
                    }
                }
                self.after_move();
            }
            Dialog::Body(b) => {
                if close {
                    return self.close_dialog();
                }
                match key {
                    "up" | "k" => b.scroll = b.scroll.saturating_sub(1),
                    "down" | "j" => b.scroll = (b.scroll + 1).min(b.lines.len().saturating_sub(1)),
                    "left" | "tab" if !b.buttons.is_empty() => {
                        b.active = (b.active + b.buttons.len() - 1) % b.buttons.len()
                    }
                    "right" if !b.buttons.is_empty() => b.active = (b.active + 1) % b.buttons.len(),
                    _ => self.body_key(stroke),
                }
            }
            Dialog::Prompt(p) => {
                if close {
                    return self.close_dialog();
                }
                if p.busy.is_some() {
                    return;
                }
                match key {
                    "return" => self.submit_prompt(),
                    "backspace" => {
                        p.text.pop();
                    }
                    _ => {
                        if let Some(text) = typed(stroke) {
                            p.text.push_str(&text);
                        }
                    }
                }
            }
            Dialog::Confirm(k) => {
                if close {
                    return self.close_dialog();
                }
                match key {
                    "left" | "right" | "tab" => k.yes = !k.yes,
                    "return" if k.yes => self.confirmed(),
                    "return" => self.close_dialog(),
                    _ => {}
                }
            }
            Dialog::Export(e) => {
                if close {
                    return self.close_dialog();
                }
                let count = e.fields() + 2;
                match key {
                    "down" | "tab" if !stroke.shift => e.focus = (e.focus + 1) % count,
                    "up" | "backtab" | "tab" => e.focus = (e.focus + count - 1) % count,
                    "space" | "return" | "left" | "right" => match e.focus {
                        0 => {
                            e.json = !e.json;
                        }
                        1 if e.json => e.sanitize = !e.sanitize,
                        1 => e.thinking = !e.thinking,
                        2 if !e.json => e.tools = !e.tools,
                        focus if key == "return" || key == "space" => {
                            let copy = focus == e.fields();
                            let export = e.clone();
                            self.export(&export, copy);
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
    }

    fn close_dialog(&mut self) {
        if let Some(Dialog::Select(Select {
            kind: Kind::Themes { original },
            ..
        })) = self.state.dialogs.pop()
        {
            self.apply_theme(&original, false);
        }
    }

    /// Live effects of moving in a list: theme preview, the MCP action label.
    fn after_move(&mut self) {
        let Some(Dialog::Select(s)) = self.state.dialogs.last() else {
            return;
        };
        match &s.kind {
            Kind::Themes { original } => {
                let name = match s.current() {
                    Some(o) => o.value.clone(),
                    None if s.query.is_empty() => original.clone(),
                    None => return,
                };
                self.apply_theme(&name, false);
            }
            Kind::Mcps => {
                let rows = self.mcp_rows();
                if let Some(s) = self.top_select() {
                    lists::mcp_actions(s, &rows);
                }
            }
            _ => {}
        }
    }

    pub(super) fn apply_theme(&mut self, name: &str, save: bool) {
        if let Ok(theme) = self.catalog.load(name, self.theme.mode) {
            self.theme = theme;
        }
        if save {
            let _ = self.tui.set(&["theme", "name"], Value::String(name.into()));
        }
    }

    /// Ctrl keys and other per-dialog actions of a list; true when handled.
    fn select_action(&mut self, stroke: &Stroke) -> bool {
        let Some(Dialog::Select(s)) = self.state.dialogs.last_mut() else {
            return false;
        };
        let value = s.current().map(|o| o.value.clone());
        match (&s.kind, stroke.ctrl, stroke.key.as_str()) {
            (Kind::Sessions { .. }, true, "d") => {
                let Some(id) = value else { return true };
                if s.confirm.as_deref() != Some(&id) {
                    s.confirm = Some(id);
                    return true;
                }
                let all = matches!(s.kind, Kind::Sessions { all: true });
                match self.core.delete_session(&id) {
                    Ok(_) => {
                        self.forget_session(&id);
                        self.state.dialogs.pop();
                        self.open_sessions(all);
                    }
                    Err(error) => self.toast(
                        None,
                        format!("Failed to delete session: {error}"),
                        Variant::Error,
                    ),
                }
            }
            (Kind::Sessions { .. }, true, "r") => {
                let Some(o) = s.current() else { return true };
                let prompt = rename_prompt(&o.value, &o.title);
                self.push(Dialog::Prompt(prompt));
            }
            (Kind::Sessions { all }, true, "a") => {
                let all = !*all;
                self.state.dialogs.pop();
                self.open_sessions(all);
            }
            (Kind::Models, true, "f") => {
                let Some(value) = value else { return true };
                let id = value.split_once('/').map_or(value.as_str(), |(_, id)| id);
                self.models.toggle_favorite(id);
                self.state.dialogs.pop();
                self.open_models();
            }
            (Kind::Models, true, "a") => {
                self.state.dialogs.pop();
                self.open_dialog("provider.connect", None);
            }
            (Kind::Settings, _, "left" | "right") => {
                let delta = if stroke.key == "left" { -1 } else { 1 };
                self.change_setting(value, delta);
            }
            (Kind::Mcps, _, "space") => {
                if let (Some(name), Some(mcp)) = (value, self.core.mcp()) {
                    let row = self.mcp_rows().into_iter().find(|r| r.name == name);
                    use anysola_core::mcp::Status;
                    match row.map(|r| r.status) {
                        Some(Status::Connected | Status::Pending) => mcp.disconnect(&name),
                        Some(Status::NeedsAuth { .. }) => self.mcp_sign_in(name),
                        _ => mcp.connect(&name),
                    }
                }
            }
            (Kind::Stash, true, "d") => {
                let Some(id) = value else { return true };
                if s.confirm.as_deref() != Some(&id) {
                    s.confirm = Some(id);
                    return true;
                }
                if let Ok(index) = id.parse::<usize>() {
                    self.stash.remove(index);
                }
                self.state.dialogs.pop();
                if !self.stash.is_empty() {
                    self.open_stash();
                }
            }
            (Kind::Queue { session }, true, "d") => {
                let session = session.clone();
                if let Some(id) = value {
                    let _ = self.core.cancel_pending(&id);
                    if let Some(s) = self.state.sessions.get_mut(&session) {
                        s.queued.retain(|q| *q != id);
                    }
                }
                self.state.dialogs.pop();
                self.open_queue(&session);
            }
            (Kind::Worktrees, true, "a") => self.worktree_prompt(),
            (Kind::Worktrees, true, "m") => self.worktree_move(value),
            (Kind::Worktrees, true, "d") => self.worktree_delete_key(),
            (Kind::Worktrees, true, "r") => {
                self.state.dialogs.pop();
                self.open_dialog("session.move", None);
            }
            _ => return false,
        }
        true
    }

    fn change_setting(&mut self, value: Option<String>, delta: i32) {
        let Some(index) = value.and_then(|v| v.parse::<usize>().ok()) else {
            return;
        };
        let all = settings::all();
        let setting = &all[index];
        let next = setting.step(
            &setting.value(&self.tui.value),
            delta,
            &self.catalog.names(),
        );
        let _ = self.tui.set(setting.path, next.clone());
        if setting.path == ["theme", "name"]
            && let Some(name) = next.as_str()
        {
            self.apply_theme(name, false);
        }
        if setting.path == ["theme", "mode"] {
            let mode = match next.as_str() {
                Some("light") => crate::theme::Mode::Light,
                Some("dark") => crate::theme::Mode::Dark,
                _ => self.theme.mode,
            };
            if let Ok(theme) = self.catalog.load(&self.theme.name, mode) {
                self.theme = theme;
            }
        }
        self.state.settings = self.tui.settings();
        self.state.auto = self.tui.auto_accept();
        let label = setting.label(&next);
        if let Some(s) = self.top_select()
            && let Some(o) = s.options.get_mut(index)
        {
            o.footer = label;
        }
    }

    /// Enter on a list row.
    fn choose_option(&mut self) {
        let Some(Dialog::Select(s)) = self.state.dialogs.last() else {
            return;
        };
        let Some(opt) = s.current().cloned() else {
            return;
        };
        let kind = s.kind.clone();
        let value = opt.value.clone();
        match kind {
            Kind::Palette => {
                self.state.dialogs.pop();
                if let Some(index) = value.strip_prefix("setting:") {
                    let focus = index.parse().unwrap_or(0);
                    self.push(Dialog::Select(settings::dialog(&self.tui.value, focus)));
                    let limit = self.limit();
                    if let Some(s) = self.top_select() {
                        s.reveal(limit);
                    }
                    return;
                }
                self.run_command(&value, None);
            }
            Kind::Models => {
                self.state.dialogs.pop();
                let Some(reference) = ModelRef::parse(&value) else {
                    return;
                };
                self.state.model = self.model_choice(&reference);
                self.models.used(&reference.id);
                if reference.variant.is_none() && self.models.variant(&reference.id).is_some() {
                    self.state.variant = self.remembered_variant(&reference.id);
                }
                let variants = self
                    .state
                    .model
                    .as_ref()
                    .map(|m| m.variants.clone())
                    .unwrap_or_default();
                let valid = self
                    .state
                    .variant
                    .as_ref()
                    .is_none_or(|v| variants.contains(v));
                if !valid {
                    self.state.variant = None;
                }
                if !variants.is_empty() && !valid {
                    self.open_dialog("variant.list", None);
                }
            }
            Kind::Variants => {
                self.state.dialogs.pop();
                self.state.variant = (value != "default").then_some(value);
                if let Some(id) = self.state.model.as_ref().map(|m| m.id.clone()) {
                    let variant = self.state.variant.clone();
                    self.models.set_variant(&id, variant.as_deref());
                }
            }
            Kind::Agents => {
                self.state.dialogs.pop();
                self.state.agent = value;
            }
            Kind::Sessions { .. } => {
                self.state.dialogs.clear();
                self.open_session(&value);
            }
            Kind::Themes { .. } => {
                self.state.dialogs.pop();
                self.apply_theme(&value, true);
            }
            Kind::Settings => self.change_setting(Some(value), 1),
            Kind::Mcps => {
                use anysola_core::mcp::Status;
                let status = self
                    .mcp_rows()
                    .into_iter()
                    .find(|r| r.name == value)
                    .map(|r| r.status);
                if let Some(Status::Failed { error } | Status::NeedsAuth { error }) = status {
                    let state = match self.mcp_rows().into_iter().find(|r| r.name == value) {
                        Some(McpRow {
                            status: Status::NeedsAuth { .. },
                            ..
                        }) => "needs_auth",
                        _ => "failed",
                    };
                    let details = format!(
                        "Status: {state}\nConfiguration: mcp.servers.{value}\n\n{}",
                        if error.is_empty() {
                            "Unknown MCP connection error"
                        } else {
                            &error
                        }
                    );
                    let lines = details.lines().map(|l| line(l, Tone::Base)).collect();
                    let mut body = Body::new(
                        BodyKind::Error {
                            details: details.clone(),
                        },
                        &format!("MCP server: {value}"),
                        lines,
                    );
                    body.actions = vec![
                        crate::dialog::Action::left("copy details", "c"),
                        crate::dialog::Action::right("scroll", "↑/↓"),
                    ];
                    self.push(Dialog::Body(body));
                }
            }
            Kind::Timeline => {
                let select = lists::message_actions(&value);
                self.push(Dialog::Select(select));
            }
            Kind::PendingSteer {
                session,
                inbox,
                item,
            } => {
                self.state.dialogs.clear();
                if value == "queue" {
                    let _ = self.core.set_delivery(&inbox, Delivery::Queue);
                } else {
                    let _ = self.core.cancel_pending(&inbox);
                    if let Some(s) = self.state.sessions.get_mut(&session) {
                        s.queued.retain(|q| *q != inbox);
                        if matches!(s.items.get(item), Some(Item::User { id, .. }) if id.is_empty())
                        {
                            s.items.remove(item);
                        }
                    }
                }
            }
            Kind::MessageActions { message } => {
                self.state.dialogs.clear();
                self.message_action(&value, &message);
            }
            Kind::Fork => {
                self.state.dialogs.clear();
                let before = (!value.is_empty()).then_some(value.as_str());
                self.fork(before);
            }
            Kind::WorkingDirectory => {
                self.state.dialogs.pop();
                let directory = self.state.directory.clone();
                match value.as_str() {
                    "location.copy" => {
                        term::copy(&directory);
                        self.toast(None, "Path copied to clipboard".into(), Variant::Info);
                    }
                    "location.open" => {
                        term::open_url(&directory);
                    }
                    _ => self.run_command("session.move", None),
                }
            }
            Kind::DiffSource => self.diff_source_chosen(&value),
            Kind::DiffBase => self.diff_base_chosen(value),
            Kind::Skills => {
                self.state.dialogs.pop();
                self.state
                    .composer
                    .insert_atom(&format!("@{value}"), crate::composer::AtomKind::Skill);
                self.state.composer.insert(" ");
            }
            Kind::Open => {
                self.state.dialogs.clear();
                if let Some(id) = value.strip_prefix("session:") {
                    self.open_elsewhere(id);
                } else if let Some(directory) = value.strip_prefix("project:")
                    && !opt.current
                {
                    self.relocate(directory.into(), None, false);
                }
            }
            Kind::Worktrees => {
                self.state.dialogs.pop();
                let current = std::path::PathBuf::from(&self.state.directory);
                let home = std::path::PathBuf::from(&self.state.home);
                if !opt.current
                    && let Some(directory) = super::resolve_directory(&value, &current, &home)
                {
                    self.relocate(directory, None, false);
                }
            }
            Kind::Connect => {
                self.state.dialogs.pop();
                self.connect_anysola();
            }
            Kind::Devices => self.choose_device(&opt),
            Kind::BudgetGate { prompt } => {
                self.state.dialogs.pop();
                self.budget_gate_choice(&value, prompt);
            }
            Kind::Queue { .. } => {
                self.state.dialogs.pop();
            }
            Kind::Stash => {
                self.state.dialogs.pop();
                if let Some(text) = value
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| self.stash.remove(index))
                {
                    self.state.composer.set(&text);
                }
            }
        }
    }

    fn message_action(&mut self, action: &str, message: &str) {
        let Some(current) = self.state.session().map(|s| s.id.clone()) else {
            return;
        };
        let text = self.state.session().and_then(|s| {
            s.items.iter().find_map(|i| match i {
                Item::User { id, text, .. } if id == message => Some(text.clone()),
                _ => None,
            })
        });
        match action {
            "revert" => match self.core.revert(&current, message) {
                Ok(_) => {
                    if let Some(text) = text {
                        self.state.composer.set(&text);
                    }
                    self.reload(&current);
                }
                Err(error) => self.toast(None, error.to_string(), Variant::Error),
            },
            "copy" => {
                term::copy(&text.unwrap_or_default());
                self.toast(None, "Message copied to clipboard!".into(), Variant::Info);
            }
            "fork" => self.fork(Some(message)),
            _ => self.jump_to(message),
        }
    }

    /// Scrolls the transcript so the user message is in view.
    fn jump_to(&mut self, message: &str) {
        let width = crate::view::session_content_width(&self.state);
        let Some(session) = self.state.session() else {
            return;
        };
        let before: Vec<Item> = session
            .items
            .iter()
            .take_while(|i| !matches!(i, Item::User { id, .. } if id == message))
            .cloned()
            .collect();
        let mut partial = session.clone();
        partial.items = before;
        let above = crate::view::transcript_rows(&self.state, &partial, &self.theme, width);
        let total = crate::view::transcript_rows(&self.state, session, &self.theme, width);
        if let Some(s) = self.state.session_mut() {
            s.scroll = total.saturating_sub(above + 1);
        }
    }

    fn fork(&mut self, before: Option<&str>) {
        let Some(current) = self.state.session().map(|s| s.id.clone()) else {
            return;
        };
        let restore = before.and_then(|m| {
            self.state.session().and_then(|s| {
                s.items.iter().find_map(|i| match i {
                    Item::User { id, text, .. } if id == m => Some(text.clone()),
                    _ => None,
                })
            })
        });
        match self.core.fork_session(&current, before) {
            Ok(row) => {
                self.open_session(&row.id);
                if let Some(text) = restore {
                    self.state.composer.set(&text);
                }
                self.toast(None, "Forked session".into(), Variant::Success);
            }
            Err(error) => self.toast(None, error.to_string(), Variant::Error),
        }
    }

    /// Drops a deleted session from tabs and memory.
    fn forget_session(&mut self, id: &str) {
        self.state.tabs.retain(|t| t.session_id != id);
        self.state.sessions.remove(id);
        if self.state.route == Route::Session(id.to_string()) {
            self.state.route = Route::Home;
        }
        self.refresh_recent();
    }

    fn submit_prompt(&mut self) {
        let Some(Dialog::Prompt(p)) = self.state.dialogs.last().cloned() else {
            return;
        };
        let text = p.text.trim().to_string();
        match p.kind {
            PromptKind::Rename { session } => {
                if text.is_empty() {
                    return;
                }
                self.state.dialogs.pop();
                match self.core.set_title(&session, &text) {
                    Ok(()) => {
                        if let Some(tab) =
                            self.state.tabs.iter_mut().find(|t| t.session_id == session)
                        {
                            tab.title = text.clone();
                        }
                        if let Some(s) = self.state.sessions.get_mut(&session) {
                            s.title = text;
                        }
                        if let Some(Dialog::Select(Select {
                            kind: Kind::Sessions { all },
                            ..
                        })) = self.state.dialogs.last()
                        {
                            let all = *all;
                            self.state.dialogs.pop();
                            self.open_sessions(all);
                        }
                        self.refresh_recent();
                    }
                    Err(error) => self.toast(
                        None,
                        format!("Failed to rename session: {error}"),
                        Variant::Error,
                    ),
                }
            }
            PromptKind::NewWorktree => self.create_worktree(text),
            PromptKind::Btw => {
                self.state.dialogs.pop();
                if !text.is_empty() {
                    self.btw(text);
                }
            }
        }
    }

    fn confirmed(&mut self) {
        let Some(Dialog::Confirm(k)) = self.state.dialogs.pop() else {
            return;
        };
        match k.kind {
            ConfirmKind::Revoke { id } => self.revoke_device(id),
            ConfirmKind::DeleteWorktree { directory } => {
                self.delete_worktree(directory.into(), true)
            }
        }
    }

    fn body_key(&mut self, stroke: &Stroke) {
        let Some(Dialog::Body(b)) = self.state.dialogs.last() else {
            return;
        };
        let kind = b.kind.clone();
        let button = b.buttons.get(b.active).cloned();
        match (kind, stroke.key.as_str()) {
            (BodyKind::Goal, "p" | "r" | "x") => self.goal_key(stroke),
            (BodyKind::Help, "return") => {
                self.state.dialogs.pop();
            }
            (BodyKind::Debug { report }, "return") => {
                term::copy(&report);
                self.toast(None, "Debug info copied to clipboard".into(), Variant::Info);
                if let Some(Dialog::Body(b)) = self.state.dialogs.last_mut() {
                    b.actions = vec![crate::dialog::Action::right("✓", "copied")];
                }
            }
            (BodyKind::Image { index }, "left" | "right") => {
                let count = self.state.composer.attachments().len().max(1);
                let next = if stroke.key == "left" {
                    index + count - 1
                } else {
                    index + 1
                };
                self.view_image(next);
            }
            (BodyKind::Usage { days }, "t") => {
                let days = if days == 7 { 30 } else { 7 };
                self.state.dialogs.pop();
                self.open_usage(days);
            }
            (BodyKind::ShellOutput { .. }, "end") => {
                let rows = self.state.height.saturating_sub(10) as usize;
                if let Some(Dialog::Body(b)) = self.state.dialogs.last_mut() {
                    b.scroll = b.lines.len().saturating_sub(rows.min(b.lines.len()));
                }
            }
            (BodyKind::Execute { code, .. }, "c") => {
                if !code.is_empty() {
                    term::copy(&code);
                    self.toast(None, "Copied to clipboard".into(), Variant::Success);
                }
            }
            (BodyKind::Execute { output, .. }, "o") => {
                if !output.is_empty() {
                    term::copy(&output);
                    self.toast(None, "Copied to clipboard".into(), Variant::Success);
                }
            }
            (BodyKind::Exported { path: text } | BodyKind::Error { details: text }, "c") => {
                term::copy(&text);
                self.toast(None, "Copied to clipboard".into(), Variant::Success);
            }
            (BodyKind::Update, "return") => match button.as_deref() {
                Some("Update") => self.install_update(),
                Some("Restart") => {
                    self.restart = true;
                    self.quit = true;
                }
                _ => {
                    self.state.dialogs.pop();
                }
            },
            (_, "return") => {
                self.state.dialogs.pop();
            }
            _ => {}
        }
    }

    fn export(&mut self, options: &Export, copy: bool) {
        let Some(current) = self.state.session().map(|s| s.id.clone()) else {
            return;
        };
        let text = if options.json {
            match self.core.export(&current) {
                Ok(Some(mut value)) => {
                    if options.sanitize {
                        anysola_core::storage::sanitize(&mut value);
                    }
                    format!("{value:#}")
                }
                _ => return self.toast(None, "Failed to export session".into(), Variant::Error),
            }
        } else {
            self.transcript(options.thinking, options.tools)
        };
        self.state.dialogs.pop();
        if copy {
            term::copy(&text);
            return self.toast(None, "Copied to clipboard".into(), Variant::Success);
        }
        let extension = if options.json { "json" } else { "md" };
        let path =
            std::env::temp_dir().join(format!("session-{}.{extension}", anysola_core::ids::part()));
        match std::fs::write(&path, text) {
            Ok(()) => {
                let path = path.display().to_string();
                let mut body = Body::new(
                    BodyKind::Exported { path: path.clone() },
                    "Session exported",
                    vec![line(path, Tone::Base)],
                );
                body.buttons = vec!["Close".into()];
                self.push(Dialog::Body(body));
            }
            Err(_) => self.toast(None, "Failed to export session".into(), Variant::Error),
        }
    }

    /// The session as markdown (`/export`, `Copy session transcript`).
    pub(super) fn transcript(&self, thinking: bool, tools: bool) -> String {
        let Some(session) = self.state.session() else {
            return String::new();
        };
        let row = self.core.session(&session.id).ok().flatten();
        let local = |ms: u64| {
            use chrono::TimeZone;
            self.state
                .zone
                .timestamp_millis_opt(ms as i64)
                .single()
                .map(|at| at.format("%-m/%-d/%Y, %-I:%M:%S %p").to_string())
                .unwrap_or_default()
        };
        let header = crate::transcript::export::Header {
            id: &session.id,
            title: &session.title,
            created: local(row.as_ref().map_or(0, |r| r.time_created)),
            updated: local(row.as_ref().map_or(0, |r| r.time_updated)),
        };
        crate::transcript::export::markdown(&header, &session.items, thinking, tools)
    }

    /// `/btw`: asks the question with the session's context and shows the answer.
    pub(super) fn btw(&mut self, question: String) {
        let Some(id) = self.state.session().map(|s| s.id.clone()) else {
            return self.toast(None, "Open a session first".into(), Variant::Warning);
        };
        self.state.asides += 1;
        let (core, tx) = (self.core.clone(), self.tx.clone());
        tokio::spawn(async move {
            let result = core.aside(&id, &question).await;
            let _ = tx.send(Msg::Apply(Box::new(move |app: &mut App| {
                app.state.asides -= 1;
                match result {
                    Ok(answer) => {
                        let width = Size::Large.width().saturating_sub(6) as usize;
                        let lines =
                            crate::transcript::markdown::render(&answer, width, &app.theme, false)
                                .into_iter()
                                .map(|spans| {
                                    spans
                                        .into_iter()
                                        .map(|span| (span.content.into_owned(), Tone::Base))
                                        .collect()
                                })
                                .collect();
                        let mut body = Body::new(BodyKind::Btw { answer }, "/btw", lines);
                        body.title.push((format!(" {question}"), Tone::Muted));
                        body.size = Size::Large;
                        body.centered = true;
                        body.actions = vec![
                            crate::dialog::Action::left("copy", "c"),
                            crate::dialog::Action::right("scroll", "j/k ↑/↓"),
                        ];
                        app.push(Dialog::Body(body));
                    }
                    Err(error) => app.toast(None, error, Variant::Error),
                }
            })));
        });
    }

    /// The `Execute` dialog of a call: status, numbered code and output.
    pub(super) fn open_execute(&mut self, call_id: &str) {
        use crate::transcript::model::{Item, Part, Status};
        let Some(tool) = self.state.session().and_then(|s| {
            s.items.iter().rev().find_map(|item| match item {
                Item::Assistant(a) => a.parts.iter().find_map(|p| match p {
                    Part::Tool(t) if t.call_id == call_id => Some(t.clone()),
                    _ => None,
                }),
                _ => None,
            })
        }) else {
            return;
        };
        let code = tool.str("code").unwrap_or_default().to_string();
        let output = crate::text::strip_ansi(tool.output.trim());
        let failed = tool.metadata.get("error") == Some(&serde_json::Value::Bool(true))
            || tool.status == Status::Failed;
        let (status, tone) = match tool.status {
            Status::Streaming => ("Receiving code…", Tone::Muted),
            Status::Running => ("Running", Tone::Muted),
            _ if failed => ("Failed", Tone::Error),
            _ => ("Completed", Tone::Muted),
        };
        let digits = code.lines().count().to_string().len();
        let mut lines: Vec<crate::dialog::Line> = vec![vec![("Code".into(), Tone::Bold)]];
        if code.is_empty() {
            lines.push(vec![("Waiting for code…".into(), Tone::Muted)]);
        }
        for (i, line) in code.lines().enumerate() {
            lines.push(vec![
                (format!("{:>digits$} ", i + 1), Tone::Muted),
                (line.to_string(), Tone::Base),
            ]);
        }
        lines.push(Vec::new());
        lines.push(vec![("Output".into(), Tone::Bold)]);
        if output.is_empty() {
            let text = if tool.status == Status::Completed {
                "No output"
            } else {
                "Waiting for output…"
            };
            lines.push(vec![(text.into(), Tone::Muted)]);
        }
        let output_tone = if failed { Tone::Error } else { Tone::Base };
        lines.extend(output.lines().map(|l| vec![(l.to_string(), output_tone)]));
        let mut body = Body::new(
            BodyKind::Execute {
                code,
                output: output.clone(),
            },
            "execute",
            lines,
        );
        body.title.push((format!("  {status}"), tone));
        body.size = Size::Large;
        body.actions = vec![
            crate::dialog::Action::left("copy code", "c"),
            crate::dialog::Action::left("copy output", "o"),
            crate::dialog::Action::right("scroll", "↑/↓"),
        ];
        self.push(Dialog::Body(body));
    }

    /// `Image <i> of <n>` over the composer's image attachments (no inline graphics: the name).
    pub(super) fn view_image(&mut self, index: usize) {
        let images: Vec<String> = self
            .state
            .composer
            .attachments()
            .into_iter()
            .filter(|(mime, _, _)| mime.starts_with("image/"))
            .map(|(_, _, name)| name)
            .collect();
        if images.is_empty() {
            return;
        }
        let index = index % images.len();
        let mut body = Body::new(
            BodyKind::Image { index },
            &format!("Image {} of {}", index + 1, images.len()),
            vec![line("No preview", Tone::Error)],
        );
        body.size = Size::XLarge;
        body.footer = vec![
            ("← previous  ".into(), Tone::Muted),
            (images[index].clone(), Tone::Base),
            ("  next →".into(), Tone::Muted),
        ];
        if let Some(Dialog::Body(Body {
            kind: BodyKind::Image { .. },
            ..
        })) = self.state.dialogs.last()
        {
            self.state.dialogs.pop();
        }
        self.push(Dialog::Body(body));
    }

    /// A click on the user message at `index`: its actions, or for a prompt still waiting to
    /// steer the running turn, `Pending steer` (`routes/session/index.tsx:2384-2407`).
    pub(super) fn click_user(&mut self, index: usize) {
        let Some(session) = self.state.session() else {
            return;
        };
        let pending_before = session.items[..index.min(session.items.len())]
            .iter()
            .filter(|i| matches!(i, Item::User { id, .. } if id.is_empty()))
            .count();
        let (id, session_id) = match session.items.get(index) {
            Some(Item::User { id, .. }) => (id.clone(), session.id.clone()),
            _ => return,
        };
        if !id.is_empty() {
            let select = lists::message_actions(&id);
            self.push(Dialog::Select(select));
            return;
        }
        let rows = self.core.pending(&session_id).unwrap_or_default();
        let Some(row) = rows.iter().filter(|r| r.kind == "user").nth(pending_before) else {
            return;
        };
        if row.delivery != "steer" {
            return;
        }
        let options = vec![
            crate::dialog::Opt::new("Move to queue", "queue"),
            crate::dialog::Opt::new("Delete", "cancel"),
        ];
        self.push(Dialog::Select(Select::new(
            Kind::PendingSteer {
                session: session_id,
                inbox: row.id.clone(),
                item: index,
            },
            "Pending steer",
            options,
        )));
    }

    /// Prompts waiting for the running turn; `ctrl+d` takes one back.
    pub(super) fn open_queue(&mut self, session: &str) {
        let rows = self.core.pending(session).unwrap_or_default();
        let options = rows
            .iter()
            .filter(|r| r.kind == "user")
            .map(|r| {
                let text = r
                    .payload
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                let first = text.lines().next().unwrap_or_default();
                crate::dialog::Opt::new(crate::text::truncate(first, 60), &r.id).footer(&r.delivery)
            })
            .collect();
        let mut select = Select::new(
            Kind::Queue {
                session: session.to_string(),
            },
            "Queued prompts",
            options,
        );
        select.actions = vec![crate::dialog::Action::left("remove", "ctrl+d")];
        self.push(Dialog::Select(select));
    }

    pub(super) fn open_stash(&mut self) {
        let now = now();
        let options = self
            .stash
            .entries()
            .iter()
            .enumerate()
            .rev()
            .map(|(i, entry)| {
                let text = &entry.prompt;
                let first = text.lines().next().unwrap_or_default().trim();
                let lines = text.lines().count();
                let mut opt =
                    crate::dialog::Opt::new(crate::text::truncate(first, 50), i.to_string());
                opt.description = crate::stash::age(entry.timestamp, now);
                if lines > 1 {
                    opt.footer = format!("~{lines} lines");
                }
                opt
            })
            .collect();
        let mut select = Select::new(Kind::Stash, "Stash", options);
        select.selected = 0;
        select.actions = vec![crate::dialog::Action::left("delete", "ctrl+d")];
        self.push(Dialog::Select(select));
    }

    pub(super) fn open_url(&mut self, url: &str) {
        if term::open_url(url) {
            self.toast(None, url.to_string(), Variant::Info);
        } else {
            self.toast(None, format!("Open {url}"), Variant::Info);
        }
    }

    /// `/update`: checks `GET /version` and shows the dialog (or only the home notice).
    pub(super) fn check_update(&mut self, dialog: bool) {
        if dialog {
            self.push(Dialog::Body(lists::update(&lists::Update::Checking)));
        }
        let tx = self.tx.clone();
        let version = self.state.version.clone();
        tokio::spawn(async move {
            let gateway = anysola_core::gateway::Gateway::new(None);
            let info = gateway.version(std::time::Duration::from_secs(5)).await;
            let newer = info
                .map(|i| i.latest)
                .filter(|latest| anysola_core::version::compare(latest, &version).is_gt());
            let _ = tx.send(Msg::Apply(Box::new(move |app: &mut App| {
                app.state.update = newer.clone();
                if dialog {
                    let state = match newer {
                        Some(v) => lists::Update::Available(v),
                        None => lists::Update::Current,
                    };
                    app.replace_update(&state);
                }
            })));
        });
    }

    fn replace_update(&mut self, state: &lists::Update) {
        if let Some(Dialog::Body(b)) = self.state.dialogs.last_mut()
            && b.kind == BodyKind::Update
        {
            *b = lists::update(state);
        }
    }

    /// Runs `anysola update` (the same install command) and offers a restart.
    fn install_update(&mut self) {
        let Some(version) = self.state.update.clone() else {
            return;
        };
        self.replace_update(&lists::Update::Installing(version.clone()));
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = match std::env::current_exe() {
                Ok(exe) => tokio::process::Command::new(exe)
                    .arg("update")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::piped())
                    .output()
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|out| {
                        if out.status.success() {
                            Ok(())
                        } else {
                            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
                        }
                    }),
                Err(error) => Err(error.to_string()),
            };
            let _ = tx.send(Msg::Apply(Box::new(move |app: &mut App| match result {
                Ok(()) => {
                    app.installed = Some(version);
                    app.replace_update(&lists::Update::Installed);
                }
                Err(error) => app.replace_update(&lists::Update::Failed(error)),
            })));
        });
    }

    fn mcp_sign_in(&mut self, name: String) {
        let Some(mcp) = self.core.mcp() else {
            return;
        };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = mcp.authorize(&name).await;
            match result {
                Ok(sign_in) => {
                    let url = sign_in.url.clone();
                    let _ = tx.send(Msg::Apply(Box::new(move |app: &mut App| {
                        app.open_url(&url);
                    })));
                    let done = sign_in.done.await.unwrap_or(Err("cancelled".into()));
                    let _ = tx.send(Msg::Apply(Box::new(move |app: &mut App| match done {
                        Ok(()) => app.toast(None, format!("Connected {name}"), Variant::Success),
                        Err(error) => app.toast(None, error, Variant::Error),
                    })));
                }
                Err(error) => {
                    let _ = tx.send(Msg::Toast(None, error, Variant::Error));
                }
            }
        });
    }

    /// `/connect` → Anysola: the device sign-in in a dialog.
    fn connect_anysola(&mut self) {
        let mut body = Body::new(
            BodyKind::Status,
            "Anysola",
            vec![line("Starting authorization…", Tone::Muted)],
        );
        body.size = crate::dialog::Size::Medium;
        self.push(Dialog::Body(body));
        let (tx, db) = (self.tx.clone(), self.db_path.clone());
        tokio::spawn(async move {
            let gateway = anysola_core::gateway::Gateway::new(None);
            let set = |lines: Vec<crate::dialog::Line>| {
                let _ = tx.send(Msg::Apply(Box::new(move |app: &mut App| {
                    if let Some(Dialog::Body(b)) = app.state.dialogs.last_mut()
                        && b.title == line("Anysola", Tone::Bold)
                    {
                        b.lines = lines;
                    }
                })));
            };
            let login = match gateway.start_device_login().await {
                Ok(login) => login,
                Err(error) => return set(vec![line(error, Tone::Error)]),
            };
            term::open_url(&login.url);
            set(vec![
                line(login.instructions(), Tone::Base),
                line(login.url.clone(), Tone::Accent),
                Vec::new(),
                line("Waiting for authorization…", Tone::Muted),
            ]);
            let stored = match gateway.finish_device_login(&login).await {
                Ok(credential) => anysola_core::storage::Db::open(&db)
                    .and_then(|db| db.insert_anysola_login(&credential).map(drop))
                    .map_err(|e| e.to_string()),
                Err(error) => Err(error),
            };
            let _ = tx.send(Msg::Apply(Box::new(move |app: &mut App| {
                app.state.dialogs.pop();
                match stored {
                    Ok(()) => {
                        app.toast(None, "Connected Anysola".into(), Variant::Success);
                        // The core holds the credential it started with; a fresh one picks up
                        // the new sign-in without leaving the app.
                        let directory = std::path::PathBuf::from(&app.state.directory);
                        let session = app.state.session().map(|s| s.id.clone());
                        app.relocate(directory, session, false);
                    }
                    Err(error) => app.toast(None, error, Variant::Error),
                }
            })));
        });
    }
}

fn rename_prompt(session: &str, title: &str) -> Prompt {
    Prompt {
        kind: PromptKind::Rename {
            session: session.into(),
        },
        title: "Rename session".into(),
        placeholder: "Session title".into(),
        text: title.into(),
        busy: None,
    }
}

/// The text a key types into a search or prompt field.
fn typed(stroke: &Stroke) -> Option<String> {
    if stroke.ctrl || stroke.alt || stroke.super_ {
        return None;
    }
    match stroke.key.as_str() {
        "space" => Some(" ".into()),
        key if key.chars().count() == 1 => Some(if stroke.shift {
            key.to_uppercase()
        } else {
            key.to_string()
        }),
        _ => None,
    }
}

/// `now`, `40m`, `5h`, `2d`, `3mo`, `1y`.
fn short_ago(ms: u64, now: u64) -> String {
    let minutes = now.saturating_sub(ms) / 60_000;
    match minutes {
        0 => "now".into(),
        1..60 => format!("{minutes}m"),
        60..1440 => format!("{}h", minutes / 60),
        1440..43_200 => format!("{}d", minutes / 1440),
        43_200..525_600 => format!("{}mo", minutes / 43_200),
        _ => format!("{}y", minutes / 525_600),
    }
}

fn describe_os() -> String {
    let name = match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("{name} {} ({arch})", release())
}

#[cfg(unix)]
fn release() -> String {
    // SAFETY: uname fills the zeroed struct; the strings are NUL-terminated.
    unsafe {
        let mut info: libc::utsname = std::mem::zeroed();
        if libc::uname(&mut info) != 0 {
            return String::new();
        }
        std::ffi::CStr::from_ptr(info.release.as_ptr())
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(not(unix))]
fn release() -> String {
    String::new()
}

fn describe_terminal() -> String {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let program = var("TERM_PROGRAM")
        .or_else(|| var("TERM"))
        .unwrap_or_else(|| "unknown".into());
    let version = var("TERM_PROGRAM_VERSION")
        .map(|v| format!(" {v}"))
        .unwrap_or_default();
    let multiplexer = if var("TMUX").is_some() {
        " in tmux"
    } else if var("STY").is_some() {
        " in screen"
    } else {
        ""
    };
    format!("{program}{version}{multiplexer}")
}

/// The confirmation before signing a device out.
pub(super) fn revoke_confirm(id: String, title: &str) -> Dialog {
    Dialog::Confirm(Confirm {
        kind: ConfirmKind::Revoke { id },
        title: "Revoke device".into(),
        message: format!("Sign out \"{title}\"? It will need to log in again."),
        button: "Revoke".into(),
        yes: true,
    })
}
