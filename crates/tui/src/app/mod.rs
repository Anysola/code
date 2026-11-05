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
use tokio::sync::{broadcast, mpsc};

use crate::composer::History;
use crate::dialog::{BodyKind, Dialog};
use crate::keymap::{Keymap, Stroke};
use crate::settings::Tui;
use crate::state::{Account, ModelChoice, Recent, Route, Session, State, Toast, Variant};
use crate::term::{self, Backend};
use crate::theme::{Catalog, Theme};
use crate::transcript::model;

mod anysola;
mod commands;
mod dialogs;
mod diff;
mod dock;
mod goal;
pub use goal::status as goal_status;
mod events;
mod keys;
mod models;
mod pane;
mod relocate;
mod tabs;
mod worktrees;

pub use relocate::{Restart, resolve as resolve_directory};

/// How bare `anysola` was started.
#[derive(Default, Clone)]
pub struct Options {
    pub session: Option<String>,
    pub continue_last: bool,
    pub prompt: Option<String>,
    pub auto: bool,
    /// Starts a core for another directory; without it `/cd` is refused.
    pub restart: Option<Restart>,
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
    /// Work finished in the background that updates the app itself.
    Apply(Box<dyn FnOnce(&mut App) + Send>),
}

/// The OSC 11 reply must arrive within this long; first frame waits for it.
const BACKGROUND_QUERY_MS: i32 = 100;
const TOAST_MS: u64 = 5000;

pub struct App {
    core: Core,
    pub state: State,
    theme: Theme,
    catalog: Catalog,
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
    /// Stashed prompts, oldest first.
    stash: crate::stash::Stash,
    models: models::Models,
    db_path: PathBuf,
    /// The budget gate let the next prompt through once.
    gate_passed: bool,
    /// `/update` installed this version; `restart` re-executes after leaving.
    installed: Option<String>,
    restart: bool,
    /// Debounces `/me` after credits events.
    account_generation: u64,
    /// Terminals of the pane; `shown` is the one on screen.
    terminals: Vec<crate::terminal::Terminal>,
    shown: Option<usize>,
    /// Whether the terminal window has focus, once it reported it.
    focused: Option<bool>,
    /// The editor's file and selection, and whether it went out with a prompt already.
    editor: Option<crate::editor::Selection>,
    editor_sent: bool,
    /// Tabs closed this run, for reopening (latest last).
    closed: Vec<String>,
    /// The last status seen of each MCP server.
    mcp_seen: std::collections::HashMap<String, anysola_core::mcp::Status>,
    restarter: Option<Restart>,
    /// Where the built-in alert sounds are written for the player.
    sounds: PathBuf,
    /// The terminal pane's edge is being dragged.
    pane_drag: bool,
    recents: relocate::Recents,
    tab_store: tabs::TabStore,
    /// OSC 7501 records, when the terminal answered the support query.
    records: Option<crate::status::Records>,
    /// A directory move waiting for running turns.
    relocating: Option<relocate::Pending>,
    /// The prompt waiting for a deferred worktree is submitted once the move lands.
    submit_after_move: bool,
    /// The new core's events, taken by the loop after a swap.
    resubscribe: Option<broadcast::Receiver<anysola_core::events::Event>>,
}

fn now() -> u64 {
    anysola_core::time::now_ms()
}

use crate::text::abbreviate;

/// `~/my-app:master`: the directory with home abbreviated and the git branch.
fn location(directory: &Path, home: &Path) -> String {
    let shown = abbreviate(directory, home);
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

#[cfg(unix)]
async fn recv_signal(signal: &mut Option<tokio::signal::unix::Signal>) -> Option<()> {
    match signal {
        Some(signal) => signal.recv().await,
        None => std::future::pending().await,
    }
}

#[cfg(not(unix))]
async fn recv_signal(_: &mut Option<()>) -> Option<()> {
    std::future::pending().await
}

/// Runs the UI; a panic shows the crash screen, whose Restart starts the UI again.
pub async fn run(core: Core, mut options: Options) -> Result<(), String> {
    use futures_util::FutureExt;
    loop {
        let attempt = std::panic::AssertUnwindSafe(run_once(core.clone(), options.clone()))
            .catch_unwind()
            .await;
        let Err(_) = attempt else {
            return attempt.unwrap_or(Ok(()));
        };
        let (message, stack) = crate::crash::take();
        let paths = Paths::from_env();
        let light =
            Tui::load(paths.data.join("tui.json")).theme_mode() == Some(crate::theme::Mode::Light);
        let mut terminal = term::enter(false).map_err(|e| e.to_string())?;
        let choice = crate::crash::show(&mut terminal, message.clone(), stack, light);
        term::restore();
        match choice {
            crate::crash::Choice::Restart => options.prompt = None,
            crate::crash::Choice::Quit => return Err(message),
        }
    }
}

async fn run_once(core: Core, options: Options) -> Result<(), String> {
    let paths = Paths::from_env();
    let tui = Tui::load(paths.data.join("tui.json"));
    let keymap = Keymap::from_config(&tui.value, cfg!(windows))?;
    let locked = tui.theme_mode();
    let probe = term::probe(BACKGROUND_QUERY_MS, locked.is_none());
    let mode = locked.or(probe.mode).unwrap_or(crate::theme::Mode::Dark);
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
    let tab_store = tabs::TabStore::new(&paths.data, core.directory(), tui.global_tabs());
    let mut app = App {
        state: State::new(anysola_core::version::VERSION),
        core,
        theme,
        catalog,
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
        stash: crate::stash::Stash::load(&paths.data.join("state").join("prompt-stash.jsonl")),
        models: models::Models::load(paths.data.join("state").join("model.json")),
        db_path: paths.database(),
        gate_passed: false,
        installed: None,
        restart: false,
        account_generation: 0,
        terminals: Vec::new(),
        shown: None,
        focused: None,
        editor: None,
        editor_sent: false,
        mcp_seen: std::collections::HashMap::new(),
        closed: Vec::new(),
        restarter: options.restart.clone(),
        sounds: paths.data.join("cache").join("sounds"),
        pane_drag: false,
        recents: relocate::Recents::new(&paths.data),
        tab_store,
        records: probe.program_status.then(crate::status::Records::default),
        relocating: None,
        submit_after_move: false,
        resubscribe: None,
    };
    app.init(&paths, &options);
    if let Some((title, message)) = failure {
        app.toast(Some(title), message, Variant::Error);
    }
    let mut input = EventStream::new();
    #[cfg(unix)]
    let mut reload_signal =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined2()).ok();
    #[cfg(not(unix))]
    let mut reload_signal: Option<()> = None;
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
            Some(()) = recv_signal(&mut reload_signal) => app.reload_themes(&paths),
            _ = tokio::time::sleep(Duration::from_millis(80)), if tick => {}
        }
        if let Some(receiver) = app.resubscribe.take() {
            events = receiver;
        }
    }
    if let Some(records) = app.records.as_mut() {
        term::write_raw(&records.clear());
    }
    term::restore();
    app.core.shutdown();
    if let Some(summary) = app.summary() {
        print!("{}", crate::menus::anysola::exit_summary(&summary));
    }
    if app.restart
        && let Ok(exe) = std::env::current_exe()
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let _ = std::process::Command::new(exe).args(args).status();
    }
    Ok(())
}

impl App {
    /// `SIGUSR2`: rescans the theme directories and reloads the current theme, keeping the
    /// current one when the scan or the load fails.
    fn reload_themes(&mut self, paths: &Paths) {
        let mut catalog = Catalog::bundled();
        let result = catalog
            .discover(&theme_dirs(paths, self.core.directory()))
            .and_then(|()| catalog.load(&self.theme.name, self.theme.mode));
        match result {
            Ok(theme) => {
                self.catalog = catalog;
                self.theme = theme;
            }
            Err(error) => self.toast(
                Some("Failed to reload themes".into()),
                error,
                Variant::Error,
            ),
        }
    }

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
            child: self.keymap.first("session.child.first"),
        };
        self.refresh_loaded();
        self.refresh_recent();
        self.fetch_account();
        self.check_update(false);
        self.watch_editor(&paths.home);
        let rows = self.mcp_rows();
        self.count_mcp(&rows);
        self.tick_account();
        self.restore_tabs();
        self.open_target(options);
    }

    /// Agents and the default model from the loaded config.
    fn refresh_loaded(&mut self) {
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
    }

    fn tick_account(&self) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let mut ticks = 0u64;
            loop {
                tokio::time::sleep(Duration::from_secs(300)).await;
                ticks += 1;
                let update = ticks.is_multiple_of(2);
                let apply = move |app: &mut App| {
                    app.fetch_account();
                    if update {
                        app.check_update(false);
                    }
                };
                if tx.send(Msg::Apply(Box::new(apply))).is_err() {
                    break;
                }
            }
        });
    }

    fn open_target(&mut self, options: &Options) {
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
        let expired = self
            .state
            .toast
            .as_ref()
            .is_some_and(|t| t.until <= self.state.now);
        if expired && self.state.toast_hover.is_none() {
            self.state.toast = (!self.state.toast_queue.is_empty()).then(|| {
                let mut next = self.state.toast_queue.remove(0);
                next.until = self.state.now + TOAST_MS;
                next
            });
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
        self.sync_pane();
        let docked = crate::view::diff_docked(&self.state);
        if let Some(view) = self.state.diff.as_mut() {
            view.docked = docked;
        }
        self.state.live = self.live();
        self.state.editor_label = self
            .editor
            .as_ref()
            .filter(|_| self.tui.editor_context())
            .map(|s| (crate::editor::file_label(s), !self.editor_sent));
        if let Some(session) = self.state.session_mut() {
            session.missing =
                !session.directory.is_empty() && !std::path::Path::new(&session.directory).is_dir();
        }
        self.tab_store.save(&self.state.tabs);
        self.report_status();
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

    /// What the exit summary reports: the open session, or only the installed update.
    fn summary(&self) -> Option<crate::menus::anysola::Summary> {
        let session = self.state.session();
        if session.is_none() && self.installed.is_none() {
            return None;
        }
        let credits = session.map(|s| self.core.credits(&s.id));
        Some(crate::menus::anysola::Summary {
            title: session.map(|s| s.title.clone()).unwrap_or_default(),
            id: session.map(|s| s.id.clone()).unwrap_or_default(),
            turns: credits.as_ref().map_or(0, |c| c.turns),
            spent: credits.as_ref().map_or(0, |c| c.total),
            updated: self.installed.clone(),
        })
    }

    fn toast(&mut self, title: Option<String>, message: String, variant: Variant) {
        self.toast_with(title, message, variant, None);
    }

    /// Shows a toast, or queues it while the visible one is hovered or others wait.
    fn toast_with(
        &mut self,
        title: Option<String>,
        message: String,
        variant: Variant,
        action: Option<(String, String)>,
    ) {
        let toast = Toast {
            title,
            message,
            variant,
            until: now() + TOAST_MS,
            action,
        };
        let busy = self.state.toast_hover.is_some() || !self.state.toast_queue.is_empty();
        if self.state.toast.is_some() && busy {
            self.state.toast_queue.push(toast);
        } else {
            self.state.toast = Some(toast);
        }
    }

    /// Hovering the toast pauses its clock; leaving resumes it with the time it had left.
    fn hover_toast(&mut self, over: bool) {
        let now = now();
        match (over, self.state.toast_hover, self.state.toast.as_mut()) {
            (true, None, Some(toast)) => {
                self.state.toast_hover = Some(toast.until.saturating_sub(now));
            }
            (false, Some(left), Some(toast)) => {
                toast.until = now + left;
                self.state.toast_hover = None;
            }
            (false, Some(_), None) => self.state.toast_hover = None,
            _ => {}
        }
    }

    fn message(&mut self, message: Msg) {
        match message {
            Msg::Account(account) => {
                if account.me.is_some() || self.state.account.me.is_none() {
                    self.state.account = account;
                }
                if let Some(Dialog::Body(b)) = self.state.dialogs.last()
                    && b.kind == BodyKind::Credits
                {
                    let body = self.credits_body();
                    self.state.dialogs.pop();
                    self.state.dialogs.push(Dialog::Body(body));
                }
            }
            Msg::Done { session_id, error } => {
                if let Some(session) = self.state.sessions.get_mut(&session_id) {
                    session.busy = false;
                }
                if let Some(error) = error {
                    self.toast(Some("Failed to send prompt".into()), error, Variant::Error);
                }
                self.reload(&session_id);
                self.fetch_account();
                self.resume_relocate();
            }
            Msg::Reload(session_id) => self.reload(&session_id),
            Msg::Toast(title, message, variant) => self.toast(title, message, variant),
            Msg::Apply(apply) => apply(self),
        }
    }

    pub(super) fn fetch_account(&self) {
        let core = self.core.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let reply = core.account(Method::GET, "/me").await;
            let account = match &reply {
                Ok(r) if r.ok() => Account::from_me(r.body.clone()),
                other => Account {
                    error: Some(crate::menus::anysola::error_text(other)),
                    ..Account::default()
                },
            };
            let _ = tx.send(Msg::Account(account));
        });
    }

    /// Follows the IDE (lock file or port) or Zed for editor context.
    fn watch_editor(&self, home: &Path) {
        let directory = self.core.directory().to_path_buf();
        let tx = self.tx.clone();
        let updates = move |update: crate::editor::Update| {
            let _ = tx.send(Msg::Apply(Box::new(move |app: &mut App| {
                app.editor_update(update)
            })));
        };
        if let Some((port, token)) = crate::editor::connection(home, &directory) {
            tokio::spawn(crate::editor::watch_ide(port, token, updates));
        } else if crate::editor::in_zed()
            && let Some(db) = crate::editor::zed_db(home)
        {
            tokio::spawn(async move {
                let mut last = None;
                loop {
                    let (db2, dir2) = (db.clone(), directory.clone());
                    let current = tokio::task::spawn_blocking(move || {
                        crate::editor::zed_selection(&db2, &dir2)
                    })
                    .await
                    .ok()
                    .flatten();
                    if current != last {
                        if let Some(selection) = current.clone() {
                            updates(crate::editor::Update::Selection(selection));
                        }
                        last = current;
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            });
        }
    }

    fn editor_update(&mut self, update: crate::editor::Update) {
        match update {
            crate::editor::Update::Selection(selection) => {
                if self.editor.as_ref() != Some(&selection) {
                    self.editor = Some(selection);
                    self.editor_sent = false;
                }
            }
            crate::editor::Update::Mention(mention) => {
                self.state
                    .composer
                    .insert_atom(&mention, crate::composer::AtomKind::File);
                self.state.composer.insert(" ");
            }
        }
    }

    /// `/me` again 1.5 s after the last credits event.
    fn account_soon(&mut self) {
        let generation = self.account_generation.wrapping_add(1);
        self.account_generation = generation;
        let tx = self.tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let _ = tx.send(Msg::Apply(Box::new(move |app: &mut App| {
                if app.account_generation == generation {
                    app.fetch_account();
                }
            })));
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
                goal: self.core.goal(id),
                ..Session::default()
            };
            self.state.sessions.insert(id.to_string(), session);
            self.reload(id);
        }
        if let Some(tab) = self.state.tabs.iter_mut().find(|t| t.session_id == id) {
            tab.unread = false;
        }
        if !self.state.tabs.iter().any(|t| t.session_id == id) {
            self.state.tabs.push(crate::state::Tab {
                session_id: id.to_string(),
                title: row.title,
                ..crate::state::Tab::default()
            });
        }
        let child = row.parent_id.is_some();
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
        // A child session has no prompt of its own: the dock stands in for it.
        self.state.dock = None;
        if child {
            self.open_dock();
        }
    }

    /// Rebuilds a session's transcript from storage (turn end, lag, revert).
    fn reload(&mut self, id: &str) {
        let boundary = self.core.revert_boundary(id).ok().flatten();
        let reverted = boundary.is_some();
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
            session.reverted = reverted;
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
