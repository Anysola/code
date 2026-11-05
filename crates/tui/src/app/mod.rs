//! The interactive app: one loop over terminal input, core events and background results,
//! folding them into `State` and redrawing.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anysola_core::api::Core;
use anysola_core::gateway::account::Method;
use anysola_core::model::ModelRef;
use anysola_core::paths::Paths;
use crossterm::event::EventStream;
use futures_util::StreamExt;
use ratatui::Terminal;
use ratatui::buffer::Buffer;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};

use crate::composer::History;
use crate::keymap::{Keymap, Stroke};
use crate::settings::Tui;
use crate::state::{Account, ModelChoice, Recent, Route, Session, State, Toast, Variant};
use crate::term::{self, Backend};
use crate::theme::{Catalog, Theme};
use crate::transcript::model;

mod commands;
mod events;
mod keys;

/// How bare `anysola` was started.
#[derive(Debug, Default, Clone)]
pub struct Options {
    pub session: Option<String>,
    pub continue_last: bool,
    pub prompt: Option<String>,
    pub auto: bool,
}

/// Results of work the loop started in the background.
enum Msg {
    Account(Account),
    Done {
        session_id: String,
        error: Option<String>,
    },
    Reload(String),
    Toast(Option<String>, String, Variant),
}

/// The OSC 11 reply must arrive within this long; first frame waits for it.
const BACKGROUND_QUERY_MS: i32 = 100;
const TOAST_MS: u64 = 5000;

pub struct App {
    core: Core,
    pub state: State,
    theme: Theme,
    keymap: Keymap,
    tui: Tui,
    history: History,
    tx: mpsc::UnboundedSender<Msg>,
    /// Strokes of a pending leader sequence and when it expires.
    pending: Vec<Stroke>,
    pending_until: u64,
    /// The `@` file list, walked on first use.
    files: Option<Vec<String>>,
    /// A drag selection in screen cells: anchor and current end.
    selection: Option<((u16, u16), (u16, u16))>,
    /// The last frame, for copying selections.
    last: Buffer,
    /// The text the popup was hidden for with Esc; it stays hidden until the text changes.
    popup_hidden: Option<String>,
    title: String,
    deferred: Option<commands::Deferred>,
    quit: bool,
}

fn now() -> u64 {
    anysola_core::time::now_ms()
}

/// `~/my-app:master`: the directory with home abbreviated and the git branch.
fn location(directory: &Path, home: &Path) -> String {
    let shown = match directory.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => directory.display().to_string(),
    };
    let branch = directory.ancestors().find_map(|dir| {
        let git = dir.join(".git");
        let head = if git.is_dir() {
            std::fs::read_to_string(git.join("HEAD")).ok()?
        } else {
            let pointer = std::fs::read_to_string(&git).ok()?;
            let target = dir.join(pointer.trim().strip_prefix("gitdir:")?.trim());
            std::fs::read_to_string(target.join("HEAD")).ok()?
        };
        Some(
            head.trim()
                .strip_prefix("ref: refs/heads/")
                .map(str::to_string),
        )
    });
    match branch.flatten() {
        Some(branch) => format!("{shown}:{branch}"),
        None => shown,
    }
}

/// Custom theme directories: the old global config dir, `~/.anysola`, then `.opencode/` of
/// each ancestor up to home (outermost first).
fn theme_dirs(paths: &Paths, directory: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![paths.legacy_config.clone(), paths.data.clone()];
    let mut ancestors: Vec<PathBuf> = Vec::new();
    for dir in directory.ancestors() {
        ancestors.push(dir.join(".opencode"));
        if dir == paths.home || !directory.starts_with(&paths.home) {
            break;
        }
    }
    ancestors.reverse();
    dirs.extend(ancestors);
    dirs
}

pub async fn run(core: Core, options: Options) -> Result<(), String> {
    let paths = Paths::from_env();
    let tui = Tui::load(paths.data.join("tui.json"));
    let keymap = Keymap::from_config(&tui.value, cfg!(windows))?;
    let mode = tui
        .theme_mode()
        .unwrap_or_else(|| term::detect_mode(BACKGROUND_QUERY_MS));
    let mut catalog = Catalog::bundled();
    let mut theme_name = tui.theme_name();
    if catalog
        .discover(&theme_dirs(&paths, core.directory()))
        .is_err()
    {
        theme_name = crate::theme::DEFAULT.into();
    }
    let mut failure = None;
    let theme = match catalog.load(&theme_name, mode) {
        Ok(theme) => theme,
        Err(error) => {
            failure = Some((format!("Failed to load theme: {theme_name}"), error));
            catalog.load("opencode", mode)?
        }
    };
    let history = History::load(&paths.data.join("state").join("prompt-history.jsonl"));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut events = core.subscribe();
    let mut terminal = term::enter(tui.mouse()).map_err(|e| e.to_string())?;
    let mut app = App {
        state: State::new(anysola_core::version::VERSION),
        core,
        theme,
        keymap,
        tui,
        history,
        tx,
        pending: Vec::new(),
        pending_until: 0,
        files: None,
        selection: None,
        last: Buffer::empty(ratatui::layout::Rect::default()),
        popup_hidden: None,
        title: String::new(),
        deferred: None,
        quit: false,
    };
    app.init(&paths, &options);
    if let Some((title, message)) = failure {
        app.toast(Some(title), message, Variant::Error);
    }
    let mut input = EventStream::new();
    loop {
        app.draw(&mut terminal);
        if app.quit {
            break;
        }
        let tick = app.ticking();
        tokio::select! {
            Some(Ok(event)) = input.next() => {
                app.input(event);
                app.run_deferred(&mut terminal);
            }
            event = events.recv() => match event {
                Ok(event) => app.event(event),
                Err(broadcast::error::RecvError::Lagged(_)) => app.reload_all(),
                Err(broadcast::error::RecvError::Closed) => {}
            },
            Some(message) = rx.recv() => app.message(message),
            _ = tokio::time::sleep(Duration::from_millis(80)), if tick => {}
        }
    }
    term::restore();
    app.core.shutdown();
    Ok(())
}

impl App {
    fn init(&mut self, paths: &Paths, options: &Options) {
        let directory = self.core.directory().to_path_buf();
        self.state.location = location(&directory, &paths.home);
        self.state.directory = directory.display().to_string();
        self.state.home = paths.home.display().to_string();
        self.state.settings = self.tui.settings();
        self.state.auto = options.auto || self.tui.auto_accept();
        self.state.example = (now() / 7) as usize % 3;
        self.state.hints = crate::state::Hints {
            agents: self.keymap.first("agent.cycle"),
            commands: self.keymap.first("command.palette.show"),
            sessions: self.keymap.first("session.list"),
            background: self.keymap.first("session.background"),
            fullscreen: self.keymap.first("permission.prompt.fullscreen"),
        };
        let loaded = self.core.loaded();
        self.state.agents = loaded
            .agents
            .list
            .iter()
            .filter(|a| a.mode != anysola_core::agent::Mode::Subagent && !a.hidden)
            .map(|a| a.id.clone())
            .collect();
        if let Some(agent) = loaded.agents.default_agent() {
            self.state.agent = agent.id.clone();
        }
        self.state.model = loaded
            .config
            .model()
            .and_then(ModelRef::parse)
            .and_then(|r| self.model_choice(&r));
        self.refresh_recent();
        self.fetch_account();
        let target = match (&options.session, options.continue_last) {
            (Some(id), _) => Some(anysola_core::api::Target::Id(id.clone())),
            (None, true) => Some(anysola_core::api::Target::Continue),
            _ => None,
        };
        if let Some(target) = target {
            match self.core.open(&target, false) {
                Ok((row, _)) => self.open_session(&row.id),
                Err(error) => self.toast(None, error.to_string(), Variant::Error),
            }
        }
        if let Some(prompt) = &options.prompt {
            self.state.composer.set(prompt);
            self.submit(false);
        }
    }

    fn model_choice(&self, reference: &ModelRef) -> Option<ModelChoice> {
        let loaded = self.core.loaded();
        let model = loaded.models.iter().find(|m| m.id == reference.id)?;
        Some(ModelChoice {
            provider: reference.provider.clone(),
            id: model.id.clone(),
            name: model.name.clone(),
            variants: model.variants.iter().map(|v| v.id.clone()).collect(),
            context: model.context,
        })
    }

    fn ticking(&self) -> bool {
        let busy = self.state.sessions.values().any(|s| s.busy);
        busy || self.state.toast.is_some()
            || !self.pending.is_empty()
            || self.state.interrupt_armed.is_some()
    }

    fn draw(&mut self, terminal: &mut Terminal<Backend>) {
        self.state.now = now();
        if self
            .state
            .toast
            .as_ref()
            .is_some_and(|t| t.until <= self.state.now)
        {
            self.state.toast = None;
        }
        if self
            .state
            .interrupt_armed
            .is_some_and(|t| self.state.now > t + 5000)
        {
            self.state.interrupt_armed = None;
        }
        if !self.pending.is_empty() && self.state.now > self.pending_until {
            self.pending.clear();
        }
        if let Ok(size) = terminal.size() {
            self.state.width = size.width;
            self.state.height = size.height;
        }
        let title = if self.state.settings.title {
            term::title(self.state.session().map(|s| s.title.as_str()))
        } else {
            String::new()
        };
        if title != self.title {
            term::set_title(&title);
            self.title = title;
        }
        let selection = self.selection;
        let frame = terminal.draw(|frame| {
            let drawn = crate::view::draw(&self.state, &self.theme, frame.buffer_mut());
            if let Some((a, b)) = selection {
                highlight(frame.buffer_mut(), a, b);
            }
            if let Some((x, y)) = drawn.cursor {
                frame.set_cursor_position((x, y));
            }
        });
        if let Ok(frame) = frame {
            self.last = frame.buffer.clone();
        }
    }

    fn toast(&mut self, title: Option<String>, message: String, variant: Variant) {
        self.state.toast = Some(Toast {
            title,
            message,
            variant,
            until: now() + TOAST_MS,
        });
    }

    fn message(&mut self, message: Msg) {
        match message {
            Msg::Account(account) => self.state.account = account,
            Msg::Done { session_id, error } => {
                if let Some(session) = self.state.sessions.get_mut(&session_id) {
                    session.busy = false;
                }
                if let Some(error) = error {
                    self.toast(Some("Failed to send prompt".into()), error, Variant::Error);
                }
                self.reload(&session_id);
                self.fetch_account();
            }
            Msg::Reload(session_id) => self.reload(&session_id),
            Msg::Toast(title, message, variant) => self.toast(title, message, variant),
        }
    }

    fn fetch_account(&self) {
        let core = self.core.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let Ok(reply) = core.account(Method::GET, "/me").await else {
                return;
            };
            if !reply.ok() {
                return;
            }
            let me = reply.body;
            let credits = me.get("credits");
            let quota = me.get("freeQuota").filter(|q| q.is_object());
            let free = (me.get("access").and_then(Value::as_str) == Some("free"))
                .then_some(quota)
                .flatten()
                .map(|q| {
                    let used = q
                        .get("turns")
                        .or_else(|| q.get("usedTurns"))
                        .and_then(Value::as_u64);
                    (
                        used.unwrap_or(0),
                        q.get("dailyTurns").and_then(Value::as_u64).unwrap_or(0),
                    )
                });
            let account = Account {
                plan: me.get("plan").and_then(Value::as_str).map(str::to_string),
                renews: credits
                    .and_then(|c| c.get("renewsAt"))
                    .and_then(Value::as_str)
                    .and_then(crate::text::parse_iso),
                available: credits
                    .and_then(|c| c.get("available"))
                    .and_then(Value::as_f64),
                free,
            };
            let _ = tx.send(Msg::Account(account));
        });
    }

    fn refresh_recent(&mut self) {
        self.state.recent = self
            .core
            .recent_sessions(3)
            .unwrap_or_default()
            .into_iter()
            .map(|s| Recent {
                id: s.id,
                title: s.title,
                updated: s.time_updated,
            })
            .collect();
    }

    /// Loads a session into the state, adds its tab and shows it.
    fn open_session(&mut self, id: &str) {
        let Ok(Some(row)) = self.core.session(id) else {
            self.toast(None, format!("Session not found: {id}"), Variant::Error);
            return;
        };
        if !self.state.sessions.contains_key(id) {
            let session = Session {
                id: row.id.clone(),
                title: row.title.clone(),
                directory: row.directory.clone(),
                credits: Some(self.core.credits(id)),
                ..Session::default()
            };
            self.state.sessions.insert(id.to_string(), session);
            self.reload(id);
        }
        if !self.state.tabs.iter().any(|t| t.session_id == id) {
            self.state.tabs.push(crate::state::Tab {
                session_id: id.to_string(),
                title: row.title,
                attention: false,
            });
        }
        if let Some(agent) = row.agent {
            self.state.agent = agent;
        }
        if let Some(model) = row.model.as_ref().and_then(ModelRef::from_json)
            && let Some(choice) = self.model_choice(&model)
        {
            self.state.model = Some(choice);
            self.state.variant = model.variant;
        }
        self.state.route = Route::Session(id.to_string());
    }

    /// Rebuilds a session's transcript from storage (turn end, lag, revert).
    fn reload(&mut self, id: &str) {
        let boundary = self.core.revert_boundary(id).ok().flatten();
        let Ok(messages) = self.core.messages(id) else {
            return;
        };
        let loaded = self.core.loaded();
        let names = |model: &str| display_name(&loaded.models, model);
        let items = model::from_messages(messages, boundary.as_deref(), &names);
        let title = self.core.session(id).ok().flatten().map(|r| r.title);
        if let Some(session) = self.state.sessions.get_mut(id) {
            let awaiting = session.permission.is_some();
            session.items = items;
            if let Some(title) = title {
                session.title = title;
            }
            if awaiting && let Some(p) = &session.permission {
                model::mark_awaiting(&mut session.items, &p.action);
            }
        }
    }

    fn reload_all(&mut self) {
        let ids: Vec<String> = self.state.sessions.keys().cloned().collect();
        for id in ids {
            self.reload(&id);
        }
    }
}

/// A model's display name by id (the id itself when the catalog lacks it).
fn display_name(models: &[anysola_core::model::Model], id: &str) -> String {
    models
        .iter()
        .find(|m| m.id == id)
        .map_or_else(|| id.to_string(), |m| m.name.clone())
}

/// Inverts the cells of a selection (reading order from `a` to `b`).
fn highlight(buf: &mut Buffer, a: (u16, u16), b: (u16, u16)) {
    let (start, end) = if (a.1, a.0) <= (b.1, b.0) {
        (a, b)
    } else {
        (b, a)
    };
    let area = buf.area;
    for y in start.1..=end.1.min(area.bottom().saturating_sub(1)) {
        let from = if y == start.1 { start.0 } else { 0 };
        let to = if y == end.1 {
            end.0
        } else {
            area.right().saturating_sub(1)
        };
        for x in from..=to.min(area.right().saturating_sub(1)) {
            let cell = &mut buf[(x, y)];
            cell.modifier.toggle(ratatui::style::Modifier::REVERSED);
        }
    }
}

/// The text of a selection in a drawn frame, rows trimmed at the end.
pub fn selected_text(buf: &Buffer, a: (u16, u16), b: (u16, u16)) -> String {
    let (start, end) = if (a.1, a.0) <= (b.1, b.0) {
        (a, b)
    } else {
        (b, a)
    };
    let mut lines = Vec::new();
    for y in start.1..=end.1.min(buf.area.bottom().saturating_sub(1)) {
        let from = if y == start.1 { start.0 } else { 0 };
        let to = if y == end.1 {
            end.0
        } else {
            buf.area.right().saturating_sub(1)
        };
        let line: String = (from..=to.min(buf.area.right().saturating_sub(1)))
            .map(|x| buf[(x, y)].symbol())
            .collect();
        lines.push(line.trim_end().to_string());
    }
    lines.join("\n")
}
