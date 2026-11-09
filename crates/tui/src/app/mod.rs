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
    /// Unsent prompts of the tabs that are not shown, by route key, and the key shown now.
    drafts: std::collections::HashMap<String, crate::composer::Composer>,
    draft_route: String,
    /// The labels' fade-in has been started once, as 0.5.1 plays it once per app instance.
    faded: bool,
    keymap: Keymap,
    tui: Tui,
    history: History,
    tx: mpsc::UnboundedSender<Msg>,
    /// Strokes of a pending leader sequence and when it expires.
    pending: Vec<Stroke>,
    pending_until: u64,
    /// When background shell rows were last polled.
    shells_polled: u64,
    /// The `@` file list, walked on first use (off the loop; empty until the walk lands).
    files: Option<Vec<String>>,
    files_walked: bool,
    /// A drag selection in screen cells: anchor and current end.
    selection: Option<((u16, u16), (u16, u16))>,
    /// The last frame, for copying selections.
    last: Buffer,
    /// The last frame's clickable controls, in the routes' coordinates.
    hits: Vec<(ratatui::layout::Rect, crate::view::Hit)>,
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
    /// Debounces `/me` after usage events.
    account_generation: u64,
    /// Windows already warned about near their limit: `(window, its reset time)`.
    limit_warned: Vec<(anysola_core::gateway::limits::WindowId, Option<u64>)>,
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
    /// When the typed session search runs.
    session_search: Option<u64>,
    /// The vertical tab column's edge is being dragged.
    tabs_drag: bool,
    /// `layout.json`: the vertical tab column's width.
    layout_path: std::path::PathBuf,
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

/// Custom theme directories: the old global config dir, `~/.anysola`, then `.opencode/` and
/// `.anysola/` of each ancestor up to home (outermost first, the new name after the old).
fn theme_dirs(paths: &Paths, directory: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![paths.legacy_config.clone(), paths.data.clone()];
    let mut ancestors: Vec<PathBuf> = Vec::new();
    for dir in directory.ancestors() {
        // `~/.anysola` is already listed as the global dir.
        if dir.join(".anysola") != paths.data {
            ancestors.push(dir.join(".anysola"));
        }
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

/// Stashes the prompt of the tab `left` and brings back the draft of the tab `shown`.
fn swap_draft(
    drafts: &mut std::collections::HashMap<String, crate::composer::Composer>,
    left: String,
    shown: &str,
    composer: &mut crate::composer::Composer,
) {
    let current = std::mem::take(composer);
    if !current.is_empty() {
        drafts.insert(left, current);
    }
    if let Some(draft) = drafts.remove(shown) {
        *composer = draft;
    }
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
            crate::crash::Choice::Quit => {
                core.shutdown();
                return Err(message);
            }
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
        drafts: Default::default(),
        draft_route: String::new(),
        faded: false,
        keymap,
        tui,
        history,
        tx,
        pending: Vec::new(),
        pending_until: 0,
        shells_polled: 0,
        files: None,
        files_walked: false,
        selection: None,
        last: Buffer::empty(ratatui::layout::Rect::default()),
        hits: Vec::new(),
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
        limit_warned: Vec::new(),
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
        session_search: None,
        tabs_drag: false,
        layout_path: paths.data.join("state").join("tui").join("layout.json"),
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
    // A closed terminal (SIGHUP) or a kill (SIGTERM) ends the UI the normal way, so the core
    // shuts down and background shells are killed instead of outliving the window.
    #[cfg(unix)]
    let mut hangup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()).ok();
    #[cfg(unix)]
    let mut terminate =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
    #[cfg(not(unix))]
    let mut hangup: Option<()> = None;
    #[cfg(not(unix))]
    let mut terminate: Option<()> = None;
    loop {
        app.draw(&mut terminal);
        if app.quit {
            break;
        }
        let tick = app.ticking();
        tokio::select! {
            Some(Ok(event)) = input.next() => {
                // Keys that come in one burst may be a paste typed out (see `paste`).
                let mut burst = vec![event];
                while matches!(burst.last(), Some(crossterm::event::Event::Key(_)))
                    && burst.len() < 100_000
                {
                    let gap = Duration::from_millis(crate::paste::BURST_GAP_MS);
                    match tokio::time::timeout(gap, input.next()).await {
                        Ok(Some(Ok(event))) => burst.push(event),
                        _ => break,
                    }
                }
                for event in crate::paste::coalesce(burst) {
                    app.input(event);
                }
                app.run_deferred(&mut terminal);
            }
            event = events.recv() => match event {
                Ok(event) => app.event(event),
                Err(broadcast::error::RecvError::Lagged(_)) => app.reload_all(),
                Err(broadcast::error::RecvError::Closed) => {}
            },
            Some(message) = rx.recv() => app.message(message),
            Some(()) = recv_signal(&mut reload_signal) => app.reload_themes(&paths),
            Some(()) = recv_signal(&mut hangup) => app.quit = true,
            Some(()) = recv_signal(&mut terminate) => app.quit = true,
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
            redo: self.keymap.first("session.redo"),
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
        self.load_layout();
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
        busy || self.fading()
            || self.state.toast.is_some()
            || !self.pending.is_empty()
            || self.state.interrupt_armed.is_some()
            || self.session_search.is_some()
            || self.polling_shells()
    }

    /// `draw` still has shell output to read: the open Shell output dialog until it saw the
    /// shell end, and the shown session's background shells until a poll saw the last one end.
    fn polling_shells(&self) -> bool {
        use crate::dialog::{BodyKind, Dialog};
        let dialog = matches!(
            self.state.dialogs.last(),
            Some(Dialog::Body(b)) if matches!(b.kind, BodyKind::ShellOutput { ended: false, .. })
        );
        dialog
            || self.state.session().is_some_and(|s| {
                s.shells.values().any(|v| v.running)
                    || !self.core.background_shells(&s.id).is_empty()
            })
    }

    /// The metadata labels are still fading in.
    fn fading(&self) -> bool {
        self.state.meta_fade.is_some() && self.state.meta_alpha() < 1.0
    }

    /// Leaving a tab keeps its unsent prompt, restored when it is shown again (not persisted).
    fn sync_draft(&mut self) {
        let key = match &self.state.route {
            Route::Home => String::new(),
            Route::Session(id) => id.clone(),
        };
        if key == self.draft_route {
            return;
        }
        let left = std::mem::replace(&mut self.draft_route, key.clone());
        swap_draft(&mut self.drafts, left, &key, &mut self.state.composer);
        self.popup_hidden = None;
        self.state.popup = None;
    }

    fn draw(&mut self, terminal: &mut Terminal<Backend>) {
        self.sync_draft();
        self.state.now = now();
        if self.state.meta_fade.is_none() && self.state.settings.animations && !self.faded {
            self.faded = true;
            self.state.meta_fade = Some(self.state.now);
        }
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
            self.state.left = if self.state.settings.tabs_vertical && !self.state.tabs.is_empty() {
                crate::view::vertical_tabs_width(size.width, self.state.tabs_width).unwrap_or(0)
            } else {
                0
            };
            self.state.width = size.width - self.state.left;
            self.state.height = size.height;
        }
        self.sync_pane();
        self.poll_shell_output();
        self.poll_shells();
        self.poll_session_search();
        self.reveal_history();
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
        let left = self.state.left;
        let mut hits = Vec::new();
        let frame = terminal.draw(|frame| {
            let drawn = crate::view::draw(&self.state, &self.theme, frame.buffer_mut());
            hits = drawn.hits;
            if let Some((a, b)) = selection {
                highlight(
                    frame.buffer_mut(),
                    (a.0 + left, a.1),
                    (b.0 + left, b.1),
                    left,
                );
            }
            if let Some((x, y)) = drawn.cursor {
                frame.set_cursor_position((x, y));
            }
        });
        if let Ok(frame) = frame {
            self.hits = hits;
            // Hit tests read the routes' own coordinates, right of a vertical tab column.
            self.last = if left == 0 {
                frame.buffer.clone()
            } else {
                let area = frame.buffer.area;
                let mut body = Buffer::empty(ratatui::layout::Rect::new(
                    0,
                    0,
                    area.width - left,
                    area.height,
                ));
                for y in 0..area.height {
                    for x in 0..area.width - left {
                        body[(x, y)] = frame.buffer[(x + left, y)].clone();
                    }
                }
                body
            };
        }
    }

    /// What the exit summary reports: the open session, or only the installed update.
    fn summary(&self) -> Option<crate::menus::anysola::Summary> {
        let session = self.state.session();
        if session.is_none() && self.installed.is_none() {
            return None;
        }
        let usage = session.map(|s| self.core.usage(&s.id));
        let limits = self
            .state
            .windows()
            .map(|(five, week)| anysola_core::usage::windows_text(five, week));
        Some(crate::menus::anysola::Summary {
            title: session.map(|s| s.title.clone()).unwrap_or_default(),
            id: session.map(|s| s.id.clone()).unwrap_or_default(),
            turns: usage.as_ref().map_or(0, |c| c.turns),
            spent: usage.as_ref().map_or(0.0, |c| c.total),
            limits,
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
                    self.warn_limits();
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
                self.run_queued_compaction(&session_id);
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
                    signed_out: matches!(other, Ok(r) if r.status == 401),
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

    /// `/me` again 1.5 s after the last usage event.
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
    pub(super) fn open_session(&mut self, id: &str) {
        let Some(row) = self.attach_session(id) else {
            self.toast(None, format!("Session not found: {id}"), Variant::Error);
            return;
        };
        if let Some(tab) = self.state.tabs.iter_mut().find(|t| t.session_id == id) {
            tab.unread = false;
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

    /// Loads a session into the state and adds its tab, leaving the shown route as it is.
    pub(super) fn attach_session(&mut self, id: &str) -> Option<anysola_core::storage::SessionRow> {
        let Ok(Some(row)) = self.core.session(id) else {
            return None;
        };
        if !self.state.sessions.contains_key(id) {
            let session = Session {
                id: row.id.clone(),
                title: row.title.clone(),
                directory: row.directory.clone(),
                usage: Some(self.core.usage(id)),
                goal: self.core.goal(id),
                ..Session::default()
            };
            self.state.sessions.insert(id.to_string(), session);
            self.reload(id);
        }
        if !self.state.tabs.iter().any(|t| t.session_id == id) {
            self.state.tabs.push(crate::state::Tab {
                session_id: id.to_string(),
                title: row.title.clone(),
                ..crate::state::Tab::default()
            });
        }
        Some(row)
    }

    /// Rebuilds a session's transcript from storage (turn end, lag, revert).
    fn reload(&mut self, id: &str) {
        let boundary = self.core.revert_boundary(id).ok().flatten();
        let reverted = boundary.is_some();
        let Ok(messages) = self.core.messages(id) else {
            return;
        };
        let revert = boundary.as_deref().map(|boundary| {
            let hidden = messages.iter().skip_while(|(m, _)| m != boundary);
            let files = self.core.revert_files(id).unwrap_or_default();
            crate::state::Revert {
                count: hidden
                    .filter(|(_, m)| matches!(m, anysola_core::session::message::Message::User(_)))
                    .count(),
                files: files
                    .iter()
                    .map(|f| {
                        let text = |k: &str| f.get(k).and_then(|v| v.as_str()).unwrap_or_default();
                        let number = |k: &str| f.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
                        let mark = match text("status") {
                            "added" => 'A',
                            "deleted" => 'D',
                            _ => 'M',
                        };
                        (
                            mark,
                            text("file").to_string(),
                            number("additions"),
                            number("deletions"),
                        )
                    })
                    .collect(),
            }
        });
        let loaded = self.core.loaded();
        let names = |model: &str| display_name(&loaded.models, model);
        let items = model::from_messages(messages, boundary.as_deref(), &names);
        let title = self.core.session(id).ok().flatten().map(|r| r.title);
        if let Some(session) = self.state.sessions.get_mut(id) {
            let awaiting = session.permission.is_some();
            session.floor = if session.items.is_empty() {
                crate::transcript::rows::floor_before(
                    &items,
                    items.len(),
                    crate::transcript::rows::TAIL_ROWS,
                )
            } else {
                session.floor.min(items.len())
            };
            session.items = items;
            session.reverted = reverted;
            session.revert = revert;
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
fn highlight(buf: &mut Buffer, a: (u16, u16), b: (u16, u16), left: u16) {
    let (start, end) = if (a.1, a.0) <= (b.1, b.0) {
        (a, b)
    } else {
        (b, a)
    };
    let area = buf.area;
    for y in start.1..=end.1.min(area.bottom().saturating_sub(1)) {
        let from = if y == start.1 { start.0 } else { left };
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

#[cfg(test)]
mod draft_tests {
    use super::*;

    #[test]
    fn tabs_keep_their_own_drafts() {
        let mut drafts = Default::default();
        let mut composer = crate::composer::Composer::default();
        composer.insert("home text");
        swap_draft(&mut drafts, String::new(), "ses_a", &mut composer);
        assert!(composer.is_empty());
        composer.insert("in a");
        swap_draft(&mut drafts, "ses_a".into(), "", &mut composer);
        assert_eq!(composer.text, "home text");
        swap_draft(&mut drafts, String::new(), "ses_a", &mut composer);
        assert_eq!(composer.text, "in a");
    }
}
