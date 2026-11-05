//! Everything a frame is drawn from. The event loop owns one `State`; views only read it, so
//! tests build a `State` directly and render it.

use std::collections::HashMap;

use anysola_core::events::CreditsSnapshot;
use anysola_core::tool::question::Question as Asked;
use serde_json::Value;

use crate::composer::Composer;
use crate::transcript::model::Item;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Home,
    Session(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tab {
    pub session_id: String,
    pub title: String,
    /// A permission or question waits in this session (`!` marker).
    pub attention: bool,
    /// The waiting input is a question (`?` marker).
    pub question: bool,
    /// The session finished a turn while another one was shown (`•` marker).
    pub unread: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Account {
    pub plan: Option<String>,
    pub renews: Option<u64>,
    pub available: Option<f64>,
    /// `(used, daily)` turns of a free account.
    pub free: Option<(u64, u64)>,
    /// The server asks to warn about the balance.
    pub low: bool,
    /// The whole `/me` reply.
    pub me: Option<Value>,
    /// Why `/me` failed.
    pub error: Option<String>,
}

impl Account {
    pub fn from_me(me: Value) -> Account {
        let credits = me.get("credits");
        let quota = me.get("freeQuota").filter(|q| q.is_object());
        let free = (me.get("access").and_then(Value::as_str) == Some("free"))
            .then_some(quota)
            .flatten()
            .map(|q| {
                // [fix] §B4: `/me` sends `turns`.
                let used = q
                    .get("turns")
                    .or_else(|| q.get("usedTurns"))
                    .and_then(Value::as_u64);
                (
                    used.unwrap_or(0),
                    q.get("dailyTurns").and_then(Value::as_u64).unwrap_or(0),
                )
            });
        Account {
            plan: me.get("plan").and_then(Value::as_str).map(str::to_string),
            renews: credits
                .and_then(|c| c.get("renewsAt"))
                .and_then(Value::as_str)
                .and_then(crate::text::parse_iso),
            available: credits
                .and_then(|c| c.get("available"))
                .and_then(Value::as_f64),
            free,
            low: me.get("lowBalance") == Some(&Value::Bool(true)),
            error: None,
            me: Some(me),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelChoice {
    pub provider: String,
    pub id: String,
    pub name: String,
    pub variants: Vec<String>,
    pub context: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Permission {
    pub request_id: String,
    pub action: String,
    pub resources: Vec<String>,
    pub save: Vec<String>,
    pub metadata: Value,
    /// The shell command or path the ask is about, from the tool call it belongs to.
    pub subject: Option<String>,
    pub selected: usize,
    pub fullscreen: bool,
}

impl Permission {
    pub fn options(&self) -> Vec<&'static str> {
        if self.save.is_empty() {
            vec!["Allow once", "Reject"]
        } else {
            vec!["Allow once", "Always allow", "Reject"]
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    pub request_id: String,
    pub questions: Vec<Asked>,
    /// The open field; `questions.len()` is the review tab.
    pub tab: usize,
    pub selected: usize,
    pub answers: Vec<Vec<String>>,
    /// Text typed into the "Type your own answer" row, while editing it.
    pub custom: Option<String>,
    /// An MCP server's form: the `(key, type)` of each field, in question order.
    pub form: Option<Vec<(String, String)>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub directory: String,
    pub items: Vec<Item>,
    pub busy: bool,
    /// Rows scrolled up from the bottom; 0 follows new output.
    pub scroll: usize,
    pub permission: Option<Permission>,
    pub question: Option<Question>,
    pub credits: Option<CreditsSnapshot>,
    /// Ids of prompts admitted while a turn ran, not yet taken.
    pub queued: Vec<String>,
    /// A revert is staged (`/redo` is available).
    pub reverted: bool,
    pub layout: LayoutCache,
    /// The session's directory is gone (deleted worktree, moved checkout); the app checks it.
    pub missing: bool,
    /// Exploration groups whose open state the user flipped, by group ID.
    pub toggled: std::collections::HashSet<String>,
}

/// The session's transcript layout cache, shared by clones of the session.
#[derive(Clone, Default)]
pub struct LayoutCache(pub std::sync::Arc<std::sync::Mutex<crate::transcript::rows::Cache>>);

impl std::fmt::Debug for LayoutCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LayoutCache")
    }
}

/// A cache never makes two sessions differ.
impl PartialEq for LayoutCache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Session {
    pub fn location_missing(&self) -> bool {
        self.missing
    }

    /// Tokens of the latest assistant step with usage.
    pub fn context_tokens(&self) -> Option<u64> {
        self.items.iter().rev().find_map(|item| match item {
            Item::Assistant(a) if a.usage > 0 => Some(a.usage),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PopupAction {
    /// Run this command id (and remove the typed `/token`).
    Command(String),
    /// Replace the token with this text and keep typing (commands that take arguments).
    Insert(String),
    Mention(crate::composer::AtomKind, String),
    /// A directory: Tab descends into it.
    Directory(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PopupRow {
    pub display: String,
    pub description: String,
    pub label: String,
    pub action: PopupAction,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Popup {
    pub trigger: char,
    pub start: usize,
    pub rows: Vec<PopupRow>,
    pub selected: usize,
    pub offset: usize,
    /// Display column the descriptions start at (`/` rows pad names to the longest + 2).
    pub pad: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub title: Option<String>,
    pub message: String,
    pub variant: Variant,
    pub until: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffView {
    Auto,
    Split,
    Unified,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub thinking: bool,
    pub source_markdown: bool,
    pub diff_view: DiffView,
    pub sidebar: bool,
    pub paste_compact: bool,
    pub animations: bool,
    pub title: bool,
    pub grouping: bool,
    pub verbosity: Verbosity,
}

/// `session.verbosity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Verbosity {
    Low,
    #[default]
    Medium,
    High,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            thinking: false,
            source_markdown: false,
            diff_view: DiffView::Auto,
            sidebar: true,
            paste_compact: true,
            animations: true,
            title: true,
            grouping: true,
            verbosity: Verbosity::Medium,
        }
    }
}

/// The composer dock's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockTab {
    Subagents,
    Shell,
    Terminals,
}

impl DockTab {
    pub fn label(self) -> &'static str {
        match self {
            DockTab::Subagents => "Subagents",
            DockTab::Shell => "Shell",
            DockTab::Terminals => "Terminals",
        }
    }
}

/// One dock row: text left, a status right, and what it acts on.
#[derive(Debug, Clone, PartialEq)]
pub struct DockRow {
    pub label: String,
    pub right: String,
    pub value: String,
}

/// The dock that replaces the prompt in a session (Subagents / Shell / Terminals).
#[derive(Debug, Clone, PartialEq)]
pub struct Dock {
    pub tabs: Vec<DockTab>,
    pub tab: usize,
    pub rows: Vec<DockRow>,
    pub selected: usize,
    /// Subagents: inactive children instead of active ones.
    pub inactive: bool,
}

impl Dock {
    pub fn current(&self) -> DockTab {
        self.tabs[self.tab]
    }

    /// The text shown when the tab has no rows.
    pub fn empty(&self) -> &'static str {
        match (self.current(), self.inactive) {
            (DockTab::Subagents, false) => "No active subagents",
            (DockTab::Subagents, true) => "No inactive subagents",
            (DockTab::Shell, _) => "No shell commands",
            (DockTab::Terminals, _) => "",
        }
    }

    /// Footer hints: `(label, key)`.
    pub fn hints(&self) -> Vec<(&'static str, &'static str)> {
        let mut hints = match self.current() {
            DockTab::Subagents => vec![
                (
                    if self.inactive {
                        "show active"
                    } else {
                        "show inactive"
                    },
                    "ctrl+a",
                ),
                ("interrupt", "ctrl+d"),
            ],
            DockTab::Shell => vec![("output", "enter"), ("kill", "ctrl+d")],
            DockTab::Terminals => Vec::new(),
        };
        if self.tabs.len() > 1 {
            hints.push(("tabs", "←/→"));
        }
        hints
    }
}

/// The terminal pane: the screen of the shown terminal and whether keys go to it.
#[derive(Debug, Clone, PartialEq)]
pub struct Pane {
    pub screen: crate::terminal::Screen,
    pub focused: bool,
}

/// Footer and hint keys as the keymap displays them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hints {
    pub agents: Option<String>,
    pub commands: Option<String>,
    pub sessions: Option<String>,
    pub background: Option<String>,
    pub fullscreen: Option<String>,
    pub child: Option<String>,
}

pub const HOME_EXAMPLES: [&str; 3] = [
    "Explain this project",
    "Fix the failing tests",
    "Review my last commit",
];
pub const SHELL_EXAMPLES: [&str; 3] = ["ls -la", "git status", "pwd"];

#[derive(Debug, Clone, PartialEq)]
pub struct Recent {
    pub id: String,
    pub title: String,
    pub updated: u64,
}

#[derive(Debug, Clone)]
pub struct State {
    pub width: u16,
    pub height: u16,
    pub now: u64,
    pub route: Route,
    pub tabs: Vec<Tab>,
    pub sessions: HashMap<String, Session>,
    pub composer: Composer,
    pub popup: Option<Popup>,
    pub toast: Option<Toast>,
    /// Open dialogs; the last one is on top.
    pub dialogs: Vec<crate::dialog::Dialog>,
    pub account: Account,
    /// `~/my-app:master`.
    pub location: String,
    pub directory: String,
    pub home: String,
    pub version: String,
    /// Visible primary agents, in order (their position picks the agent colour).
    pub agents: Vec<String>,
    pub agent: String,
    pub model: Option<ModelChoice>,
    pub variant: Option<String>,
    pub auto: bool,
    pub recent: Vec<Recent>,
    pub example: usize,
    pub shell_example: usize,
    /// When the first Esc of a running session was pressed.
    pub interrupt_armed: Option<u64>,
    /// The pointer rests on the credit status line (it then shows the renewal date).
    pub status_hover: bool,
    pub settings: Settings,
    pub hints: Hints,
    /// `/update to install v<version>` on the home screen.
    pub update: Option<String>,
    /// The `/diff` viewer, while open (full screen).
    pub diff: Option<crate::diffview::Diff>,
    /// MCP servers: `(configured, connected, failed)`.
    pub mcp: (usize, usize, usize),
    /// `/btw` side questions waiting for their answer.
    pub asides: usize,
    /// The footer's `↓ N subagents · N shells` item while the session family has live work.
    pub live: Option<String>,
    /// The selected popup row asks `Press ctrl+d to confirm` before it is removed.
    pub popup_confirm: bool,
    /// A worktree chosen on home, created on the first submit (`(new worktree)` in the footer).
    pub new_worktree: Option<String>,
    /// `Creating worktree` while that happens.
    pub worktree_progress: Option<String>,
    /// The terminal pane beside the session, while shown.
    pub pane: Option<Pane>,
    pub dock: Option<Dock>,
    /// The right pane's width once dragged.
    pub pane_width: Option<u16>,
    /// The local time zone dates are shown in.
    pub zone: chrono::FixedOffset,
}

impl State {
    pub fn new(version: &str) -> State {
        State {
            width: 80,
            height: 24,
            now: 0,
            route: Route::Home,
            tabs: Vec::new(),
            sessions: HashMap::new(),
            composer: Composer::default(),
            popup: None,
            toast: None,
            dialogs: Vec::new(),
            account: Account::default(),
            location: String::new(),
            directory: String::new(),
            home: String::new(),
            version: version.to_string(),
            agents: vec!["build".into(), "plan".into()],
            agent: "build".into(),
            model: None,
            variant: None,
            auto: false,
            recent: Vec::new(),
            example: 0,
            shell_example: 0,
            interrupt_armed: None,
            status_hover: false,
            settings: Settings::default(),
            hints: Hints::default(),
            update: None,
            pane: None,
            dock: None,
            pane_width: None,
            asides: 0,
            live: None,
            popup_confirm: false,
            new_worktree: None,
            worktree_progress: None,
            mcp: (0, 0, 0),
            diff: None,
            zone: *chrono::Local::now().offset(),
        }
    }

    pub fn session(&self) -> Option<&Session> {
        match &self.route {
            Route::Session(id) => self.sessions.get(id),
            Route::Home => None,
        }
    }

    pub fn session_mut(&mut self) -> Option<&mut Session> {
        match &self.route {
            Route::Session(id) => self.sessions.get_mut(id),
            Route::Home => None,
        }
    }

    /// The balance shown in the status line: this process's latest gateway figure, else `/me`.
    pub fn remaining(&self) -> Option<f64> {
        let session = self
            .session()
            .and_then(|s| s.credits.as_ref())
            .and_then(|c| c.remaining);
        let latest = self
            .sessions
            .values()
            .find_map(|s| s.credits.as_ref().and_then(|c| c.remaining));
        session
            .or(latest)
            .map(|r| r as f64)
            .or(self.account.available)
    }

    /// `835 credits · this turn 12 · session 40[/50]`, or `today 3/20` for free accounts.
    pub fn status_text(&self) -> Option<String> {
        if let Some((used, daily)) = self.account.free {
            return Some(format!("today {used}/{daily}"));
        }
        let mut parts = vec![format!(
            "{} credits",
            crate::text::credits(self.remaining()?)
        )];
        if let Some(c) = self.session().and_then(|s| s.credits.as_ref()) {
            if c.turn > 0 {
                parts.push(format!("this turn {}", crate::text::credits(c.turn as f64)));
            }
            if c.total > 0 || c.budget.is_some() {
                let budget = c.budget.map_or(String::new(), |b| {
                    format!("/{}", crate::text::credits(b as f64))
                });
                parts.push(format!(
                    "session {}{budget}",
                    crate::text::credits(c.total as f64)
                ));
            }
        }
        if self.status_hover
            && let Some(renews) = self.account.renews
        {
            parts.push(format!(
                "renews {}",
                crate::text::short_date(renews, self.now)
            ));
        }
        Some(parts.join(" · "))
    }

    /// The balance when it is low: not free, and the server says so or fewer than 100 remain.
    pub fn low_balance(&self) -> Option<f64> {
        let remaining = self.remaining()?;
        let free = self.account.free.is_some()
            || self
                .account
                .me
                .as_ref()
                .and_then(|m| m.get("access"))
                .and_then(Value::as_str)
                == Some("free");
        (!free && (self.account.low || remaining < 100.0)).then_some(remaining)
    }

    /// Low balance or a spent budget colours the status line.
    pub fn status_warning(&self) -> bool {
        let over = self
            .session()
            .and_then(|s| s.credits.as_ref())
            .is_some_and(|c| c.budget.is_some_and(|b| c.total >= b));
        over || self.low_balance().is_some()
    }

    pub fn agent_index(&self, agent: &str) -> usize {
        self.agents.iter().position(|a| a == agent).unwrap_or(0)
    }
}
