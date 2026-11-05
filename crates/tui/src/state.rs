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

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub session_id: String,
    pub title: String,
    /// A permission or question waits in this session (`!` marker).
    pub attention: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Account {
    pub plan: Option<String>,
    pub renews: Option<u64>,
    pub available: Option<f64>,
    /// `(used, daily)` turns of a free account.
    pub free: Option<(u64, u64)>,
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
}

impl Session {
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
        }
    }
}

/// Footer and hint keys as the keymap displays them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hints {
    pub agents: Option<String>,
    pub commands: Option<String>,
    pub sessions: Option<String>,
    pub background: Option<String>,
    pub fullscreen: Option<String>,
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
    /// A dialog this build does not have yet: its title.
    pub dialog: Option<String>,
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
    pub settings: Settings,
    pub hints: Hints,
    /// `/update to install v<version>` on the home screen.
    pub update: Option<String>,
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
            dialog: None,
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
            settings: Settings::default(),
            hints: Hints::default(),
            update: None,
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
        Some(parts.join(" · "))
    }

    /// Low balance or a spent budget colours the status line.
    pub fn status_warning(&self) -> bool {
        let over = self
            .session()
            .and_then(|s| s.credits.as_ref())
            .is_some_and(|c| c.budget.is_some_and(|b| c.total >= b));
        let low = self.account.free.is_none() && self.remaining().is_some_and(|r| r < 100.0);
        over || low
    }

    pub fn agent_index(&self, agent: &str) -> usize {
        self.agents.iter().position(|a| a == agent).unwrap_or(0)
    }
}
