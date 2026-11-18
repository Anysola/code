//! The list and plain dialogs built from plain data, so tests render them without a core.

use chrono::{FixedOffset, TimeZone};

use crate::dialog::{Action, Body, BodyKind, Kind, Line, Opt, Select, Size, Tone, line};
use crate::text::width;

/// `5:02 PM` (`toLocaleTimeString` short style, en-US).
pub fn clock(ms: u64, zone: FixedOffset) -> String {
    zone.timestamp_millis_opt(ms as i64)
        .single()
        .map(|at| at.format("%-I:%M %p").to_string())
        .unwrap_or_default()
}

/// `Tue Nov 03 2026`, or `Today`.
fn day(ms: u64, now: u64, zone: FixedOffset) -> String {
    let date = |ms: u64| {
        zone.timestamp_millis_opt(ms as i64)
            .single()
            .map(|at| at.date_naive())
    };
    if date(ms) == date(now) {
        return "Today".into();
    }
    zone.timestamp_millis_opt(ms as i64)
        .single()
        .map(|at| at.format("%a %b %d %Y").to_string())
        .unwrap_or_default()
}

pub struct ModelRow {
    pub provider: String,
    pub id: String,
    pub name: String,
    pub note: Option<String>,
}

/// `Select model`: favourites, recent, then the Anysola models.
pub fn models(
    rows: &[ModelRow],
    current: Option<&str>,
    favorites: &[String],
    recent: &[String],
) -> Select {
    let mut options = Vec::new();
    let opt = |m: &ModelRow, category: &str, description: String| {
        Opt::new(&m.name, format!("{}/{}", m.provider, m.id))
            .category(category)
            .description(description)
            .current(Some(m.id.as_str()) == current)
    };
    for (category, ids) in [("Favorites", favorites), ("Recent", recent)] {
        for id in ids {
            if let Some(m) = rows.iter().find(|m| &m.id == id) {
                options.push(opt(
                    m,
                    category,
                    m.note.clone().unwrap_or_else(|| "Anysola".into()),
                ));
            }
        }
    }
    for m in rows {
        let description = match (&m.note, favorites.contains(&m.id)) {
            (Some(note), _) => note.clone(),
            (None, true) => "(Favorite)".into(),
            (None, false) => String::new(),
        };
        options.push(opt(m, "Anysola", description));
    }
    let mut select = Select::new(Kind::Models, "Select model", options);
    select.actions = vec![
        Action::left("Connect an integration", "ctrl+a"),
        Action::left("Favorite", "ctrl+f"),
    ];
    select
}

pub fn variants(variants: &[String], current: Option<&str>) -> Select {
    let mut options = vec![Opt::new("Default", "default").current(current.is_none())];
    options.extend(
        variants
            .iter()
            .map(|v| Opt::new(v, v).current(Some(v.as_str()) == current)),
    );
    Select::new(Kind::Variants, "Select variant", options)
}

pub fn agents(agents: &[(String, String)], current: &str) -> Select {
    let options = agents
        .iter()
        .map(|(id, description)| {
            Opt::new(id, id)
                .description(description)
                .current(id == current)
        })
        .collect();
    Select::new(Kind::Agents, "Select agent", options)
}

pub fn themes(names: &[String], current: &str) -> Select {
    let mut sorted = names.to_vec();
    sorted.sort_by_key(|n| n.to_lowercase());
    let options = sorted
        .iter()
        .map(|n| Opt::new(n, n).current(n == current))
        .collect();
    Select::new(
        Kind::Themes {
            original: current.into(),
        },
        "Themes",
        options,
    )
}

pub struct SessionItem {
    pub id: String,
    pub title: String,
    pub updated: u64,
    /// Basename of the session's directory when it is outside this project.
    pub elsewhere: Option<String>,
    /// `!` permission or `?` question waiting.
    pub marker: Option<char>,
}

/// A session row of `Open`: footer `<project> · <ago>`.
pub struct OpenSession {
    pub id: String,
    pub title: String,
    pub project: String,
    pub ago: String,
}

/// A project row of `Open`.
pub struct OpenProject {
    pub name: String,
    pub directory: String,
    /// The directory with home shown as `~`.
    pub shown: String,
    pub git: bool,
    pub current: bool,
}

/// `Open`: recent sessions (not already in a tab), then every known project; `●` marks the
/// current one, ` →` a git project. Paths are cut to the large dialog's content width.
pub fn open(sessions: &[OpenSession], projects: &[OpenProject], screen: usize) -> Select {
    let content = (Size::Large.width() as usize)
        .min(screen.saturating_sub(2))
        .saturating_sub(12);
    let mut options: Vec<Opt> = sessions
        .iter()
        .map(|s| {
            Opt::new(&s.title, format!("session:{}", s.id))
                .category("Sessions")
                .footer(format!(
                    "{} · {}",
                    crate::text::truncate(&s.project, 30),
                    s.ago
                ))
        })
        .collect();
    options.extend(projects.iter().map(|p| {
        let room = content.saturating_sub(width(&p.name) + if p.git { 2 } else { 0 });
        let arrow = if p.git { " →" } else { "" };
        Opt::new(&p.name, format!("project:{}", p.directory))
            .category("Projects")
            .footer(format!(
                "{}{arrow}",
                crate::text::truncate_file_path(&p.shown, room)
            ))
            .current(p.current)
    }));
    let mut select = Select::new(Kind::Open, "Open", options);
    select.selected = 0;
    select.size = Size::Large;
    select.placeholder = "Search sessions and projects…".into();
    select.empty = "No recent sessions or projects";
    select
}

/// `Sessions[ for <project>]`, grouped by day.
pub fn sessions(
    items: &[SessionItem],
    project: Option<&str>,
    now: u64,
    zone: FixedOffset,
) -> Select {
    let options = items
        .iter()
        .map(|s| {
            let mut opt = Opt::new(&s.title, &s.id)
                .category(day(s.updated, now, zone))
                .footer(
                    s.elsewhere
                        .as_deref()
                        .map(|d| crate::text::truncate(d, 25))
                        .unwrap_or_default(),
                );
            opt.gutter = s.marker.map(|m| (m.to_string(), Tone::Warning));
            opt
        })
        .collect();
    let mut select = Select::new(
        Kind::Sessions {
            all: project.is_none(),
        },
        "Sessions",
        options,
    );
    select.selected = 0;
    select.size = Size::Large;
    select.grouped_search = true;
    if let Some(project) = project {
        select.suffix = format!(" for {project}");
    }
    select.empty = "No sessions available";
    let scope = if project.is_some() {
        "all projects"
    } else {
        "current directory"
    };
    select.actions = vec![
        Action::left("delete", "ctrl+d"),
        Action::left("rename", "ctrl+r"),
        Action::right(scope, "ctrl+a"),
    ];
    select
}

/// User messages newest first: `(message id, text, created)`.
pub fn timeline(messages: &[(String, String, u64)], zone: FixedOffset) -> Select {
    let options = messages
        .iter()
        .rev()
        .map(|(id, text, at)| Opt::new(text.replace('\n', " "), id).footer(clock(*at, zone)))
        .collect();
    let mut select = Select::new(Kind::Timeline, "Timeline", options);
    select.size = Size::Large;
    select
}

pub fn fork(messages: &[(String, String, u64)], zone: FixedOffset) -> Select {
    let mut select = timeline(messages, zone);
    select.options.insert(0, Opt::new("Full session", ""));
    select.kind = Kind::Fork;
    select.title = "Fork session".into();
    select
}

pub fn message_actions(message: &str) -> Select {
    let options = [
        ("Jump to", "view message in session", "jump"),
        ("Revert", "undo messages and file changes", "revert"),
        ("Copy", "message text to clipboard", "copy"),
        ("Fork", "create a new session", "fork"),
    ]
    .iter()
    .map(|(t, d, v)| Opt::new(*t, *v).description(*d))
    .collect();
    Select::new(
        Kind::MessageActions {
            message: message.into(),
        },
        "Message Actions",
        options,
    )
}

/// `(name, description)` of the discovered skills.
pub fn skills(skills: &[(String, String)]) -> Select {
    let longest = skills.iter().map(|(n, _)| width(n)).max().unwrap_or(0);
    let options = skills
        .iter()
        .map(|(name, description)| {
            let collapsed = description.split_whitespace().collect::<Vec<_>>().join(" ");
            Opt::new(crate::text::pad_right(name, longest), name).description(collapsed)
        })
        .collect();
    let mut select = Select::new(Kind::Skills, "Skills", options);
    select.size = Size::Large;
    select.empty = "No skills available";
    select
}

/// How Anysola is connected: `$ANYSOLA_TOKEN`, a stored login, or not at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connection {
    Env,
    Login,
    None,
}

pub fn connect(connection: Connection) -> Select {
    let mut opt = Opt::new("Anysola", "anysola").category("Services");
    match connection {
        Connection::Env => opt.footer = "$ANYSOLA_TOKEN".into(),
        Connection::Login => opt.footer = "Signed in".into(),
        Connection::None => {
            opt.footer = "Sign in required →".into();
            opt.footer_tone = Some(Tone::Warning);
        }
    }
    if connection != Connection::None {
        opt.gutter = Some(("✓".into(), Tone::Success));
    }
    let mut select = Select::new(Kind::Connect, "Connect an integration", vec![opt]);
    select.empty = "No integrations available";
    select
}

pub struct McpRow {
    pub name: String,
    /// `Connecting …`, `Connected ✓`, `Failed !`, `Sign in required →`, `Disabled ○`.
    pub status: anysola_core::mcp::Status,
}

fn mcp_footer(status: &anysola_core::mcp::Status) -> (&'static str, Tone, &'static str) {
    use anysola_core::mcp::Status;
    match status {
        Status::Pending => ("Connecting …", Tone::Muted, "disconnect"),
        Status::Connected => ("Connected ✓", Tone::Success, "disconnect"),
        Status::Failed { .. } => ("Failed !", Tone::Error, "retry"),
        Status::NeedsAuth { .. } => ("Sign in required →", Tone::Warning, "sign in"),
        Status::Disabled => ("Disabled ○", Tone::Muted, "connect"),
    }
}

pub fn mcps(servers: &[McpRow]) -> Select {
    let options = servers
        .iter()
        .map(|s| {
            let (footer, tone, _) = mcp_footer(&s.status);
            let mut opt = Opt::new(&s.name, &s.name).footer(footer);
            opt.footer_tone = Some(tone);
            opt
        })
        .collect();
    let mut select = Select::new(Kind::Mcps, "MCP servers", options);
    mcp_actions(&mut select, servers);
    select
}

/// The `space` action follows the focused server's status; `view error enter` when it failed.
pub fn mcp_actions(select: &mut Select, servers: &[McpRow]) {
    use anysola_core::mcp::Status;
    let focused = select
        .current()
        .and_then(|o| servers.iter().find(|s| s.name == o.value));
    let action = focused.map_or("connect", |s| mcp_footer(&s.status).2);
    select.actions = vec![Action::left(action, "space")];
    if focused.is_some_and(|s| matches!(s.status, Status::Failed { .. } | Status::NeedsAuth { .. }))
    {
        select.actions.push(Action::left("view error", "enter"));
    }
}

/// `Status`: the MCP servers and how they are.
pub fn status(servers: &[McpRow]) -> Body {
    use anysola_core::mcp::Status;
    let mut lines: Vec<Line> = Vec::new();
    if servers.is_empty() {
        lines.push(line("No MCP servers", Tone::Base));
    } else {
        lines.push(line(format!("{} MCP server(s)", servers.len()), Tone::Base));
        for s in servers {
            let (dot, text) = match &s.status {
                Status::Connected => (Tone::Success, "Connected".to_string()),
                Status::Failed { error } => (Tone::Error, error.clone()),
                Status::Disabled => (Tone::Muted, "Disabled in configuration".into()),
                Status::NeedsAuth { error } => {
                    (Tone::Warning, format!("Needs authentication: {error}"))
                }
                Status::Pending => (Tone::Muted, "pending".into()),
            };
            lines.push(vec![
                ("• ".into(), dot),
                (s.name.clone(), Tone::Bold),
                (format!(" {text}"), Tone::Muted),
            ]);
        }
    }
    Body::new(BodyKind::Status, "Status", lines)
}

pub fn help(palette_key: &str) -> Body {
    let text =
        format!("Press {palette_key} to see all available actions and commands in any context.");
    let lines = crate::text::wrap(&text, 54)
        .into_iter()
        .map(|l| line(l, Tone::Muted))
        .collect();
    let mut body = Body::new(BodyKind::Help, "Help", lines);
    body.close = "esc/enter".into();
    body.buttons = vec!["ok".into()];
    body.button_margin = 2;
    body
}

pub struct DebugInfo {
    pub version: String,
    pub date: String,
    pub os: String,
    pub terminal: String,
    pub session: Option<String>,
    pub model: Option<String>,
}

pub fn debug(info: &DebugInfo) -> Body {
    let rows = [
        ("Version", format!("{} (latest)", info.version)),
        ("Date", info.date.clone()),
        ("OS", info.os.clone()),
        ("Terminal", info.terminal.clone()),
        (
            "Session ID",
            info.session.clone().unwrap_or_else(|| "n/a".into()),
        ),
        ("Model", info.model.clone().unwrap_or_else(|| "n/a".into())),
    ];
    let report: Vec<String> = rows.iter().map(|(k, v)| format!("{k:<10} {v}")).collect();
    let lines = rows
        .iter()
        .map(|(k, v)| vec![(format!("{k:<10} "), Tone::Muted), (v.clone(), Tone::Base)])
        .collect();
    let mut body = Body::new(
        BodyKind::Debug {
            report: report.join("\n"),
        },
        "Debug",
        lines,
    );
    body.size = Size::Large;
    body.footer = line("Share this when reporting an issue.", Tone::Muted);
    body.actions = vec![Action::right("copy", "enter")];
    body
}

/// What the update dialog is showing.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    Checking,
    Current,
    Available(String),
    Installing(String),
    Installed,
    Failed(String),
}

pub fn update(state: &Update) -> Body {
    let (title, text, tone, buttons): (&str, String, Tone, &[&str]) = match state {
        Update::Checking => ("Update", "Checking for updates…".into(), Tone::Muted, &[]),
        Update::Current => (
            "Update",
            "Anysola Code is already up to date.".into(),
            Tone::Base,
            &["Skip"],
        ),
        Update::Available(_) => (
            "Update available",
            "An update is available. After installing, you'll be prompted to restart Anysola Code.".into(),
            Tone::Base,
            &["Skip", "Update"],
        ),
        Update::Installing(v) => (
            "Updating Anysola Code",
            format!("Installing Anysola Code {v}…"),
            Tone::Muted,
            &[],
        ),
        Update::Installed => (
            "Update",
            "Update successful! Restart closes Anysola Code, then run anysola to use the new version.".into(),
            Tone::Base,
            &["Skip", "Restart"],
        ),
        Update::Failed(error) => ("Update", error.clone(), Tone::Error, &["Skip"]),
    };
    let lines = crate::text::wrap(&text, 54)
        .into_iter()
        .map(|l| line(l, tone))
        .collect();
    let mut body = Body::new(BodyKind::Update, title, lines);
    body.centered = true;
    body.buttons = buttons.iter().map(|b| b.to_string()).collect();
    body.active = body.buttons.len().saturating_sub(1);
    body
}

/// `Worktrees`: the project's checkouts, the current one first.
pub fn worktrees(current: &str, others: &[String], in_session: bool) -> Select {
    let mut options = vec![Opt::new(current, current).category("Current").current(true)];
    options.extend(others.iter().map(|d| Opt::new(d, d).category("Other")));
    let mut select = Select::new(Kind::Worktrees, "Worktrees", options);
    select.size = Size::XLarge;
    select.tall = true;
    select.empty = "No worktrees available";
    if in_session {
        select.actions.push(Action::left("move", "ctrl+m"));
    }
    select.actions.extend([
        Action::left("new", "ctrl+a"),
        Action::left("delete", "ctrl+d"),
        Action::left("refresh", "ctrl+r"),
    ]);
    select
}

/// A hook's timeout when it is not the default (600 s; 30 s for `UserPromptSubmit`).
pub fn hook_timeout(hook: &anysola_core::api::HookInfo) -> Option<String> {
    let default = if hook.event == "UserPromptSubmit" {
        30
    } else {
        600
    };
    (hook.timeout != default).then(|| format!("timeout {} s", hook.timeout))
}

/// A hook's matcher as shown (repository text, made visible), none when it matches everything.
fn hook_matcher(hook: &anysola_core::api::HookInfo) -> String {
    hook.matcher
        .as_deref()
        .filter(|m| !m.is_empty() && *m != "*")
        .map(crate::text::visible)
        .unwrap_or_default()
}

/// Where a configured hook's file sits for `/hooks`, and the user's answer for a project file.
fn hook_state(
    hook: &anysola_core::api::HookInfo,
    decision: Option<Option<bool>>,
) -> (&'static str, Tone) {
    use anysola_core::api::HookScope;
    match (hook.active, hook.scope, decision) {
        (true, _, _) => ("runs", Tone::Success),
        (false, HookScope::Project, Some(None)) => ("not approved", Tone::Warning),
        (false, HookScope::Project, Some(Some(false))) => ("refused", Tone::Muted),
        _ => ("off", Tone::Muted),
    }
}

/// `/hooks`: every configured hook in run order, grouped by the file it comes from (`show` turns
/// a path into what the user reads, made visible). A project file's rows open its approval; a
/// project file that no longer defines hooks but waits for an answer gets a row of its own.
/// `off`: why hooks are turned off, if they are.
pub fn hooks(
    hooks: &[anysola_core::api::HookInfo],
    documents: &[anysola_core::api::HookDocument],
    show: &dyn Fn(&str) -> String,
    config: &str,
    off: Option<&str>,
) -> Select {
    use anysola_core::api::HookScope;
    let longest = hooks.iter().map(|h| width(&h.event)).max().unwrap_or(0);
    let matchers = hooks
        .iter()
        .map(|h| width(&hook_matcher(h)))
        .max()
        .unwrap_or(0);
    let mut options: Vec<Opt> = hooks
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let document = documents.iter().find(|d| d.path == h.source);
            let decision = document.map(|d| d.decision);
            let scope = match (h.scope, decision) {
                (HookScope::Project, Some(Some(true))) => "project, allowed",
                (HookScope::Project, Some(Some(false))) => "project, refused",
                (HookScope::Project, _) => "project, not approved",
                (scope, _) => scope.as_str(),
            };
            let command = crate::text::visible(&h.command);
            let title = format!(
                "{} {}{command}",
                crate::text::pad_right(&h.event, longest),
                if matchers > 0 {
                    format!("{} ", crate::text::pad_right(&hook_matcher(h), matchers))
                } else {
                    String::new()
                }
            );
            let (state, tone) = hook_state(h, decision);
            let footer = match hook_timeout(h) {
                Some(timeout) => format!("{timeout} · {state}"),
                None => state.to_string(),
            };
            let value = match document {
                Some(d) => format!("doc:{}", d.path),
                None => format!("hook:{i}"),
            };
            let mut opt = Opt::new(title, value)
                .category(format!("{} · {scope}", show(&h.source)))
                .footer(footer);
            opt.footer_tone = Some(tone);
            opt.search = format!("{} {} {}", hook_matcher(h), h.command, h.source);
            opt
        })
        .collect();
    for d in documents
        .iter()
        .filter(|d| d.hooks.is_empty() && !hooks.iter().any(|h| h.source == d.path))
    {
        let mut opt = Opt::new("No hooks defined now", format!("doc:{}", d.path))
            .category(format!("{} · project", show(&d.path)))
            .footer(if d.decision.is_none() {
                "waiting for your answer"
            } else {
                "nothing runs"
            });
        opt.footer_tone = Some(if d.decision.is_none() {
            Tone::Warning
        } else {
            Tone::Muted
        });
        opt.search = d.path.clone();
        options.push(opt);
    }
    let mut select = Select::new(Kind::Hooks, "Hooks", options);
    select.selected = 0;
    select.size = Size::Large;
    select.empty = "No hooks configured";
    if let Some(off) = off {
        select.suffix = format!(" · {off}");
    }
    select.note = if select.options.is_empty() {
        format!("Add hooks in {config}")
    } else {
        "Edit hooks in the files above".into()
    };
    hook_actions(&mut select);
    select
}

/// `review enter` while a project file's row has focus.
pub fn hook_actions(select: &mut Select) {
    let project = select
        .current()
        .is_some_and(|o| o.value.starts_with("doc:"));
    select.actions = if project {
        vec![Action::left("review", "enter")]
    } else {
        Vec::new()
    };
}

/// Asks whether a project file's hooks may run: names the file, lists every hook with its whole
/// command (repository text, made visible) and any timeout that is not the default, and offers
/// `Not now` (focused) and `Allow`. A file that no longer defines hooks asks whether to accept
/// that (the hooks approved earlier stop). `shown`: the file as the user reads it, already made
/// visible. `review`: opened from `/hooks`.
pub fn hook_trust(
    document: &anysola_core::api::HookDocument,
    shown: &str,
    review: bool,
    now: u64,
) -> Body {
    const ROOM: usize = 80;
    let text = |lines: &mut Vec<Line>, text: &str, tone: Tone| {
        lines.extend(
            crate::text::wrap(text, ROOM)
                .into_iter()
                .map(|l| line(l, tone)),
        );
    };
    let mut lines: Vec<Line> = Vec::new();
    if document.hooks.is_empty() {
        text(
            &mut lines,
            &format!("{shown} no longer defines hooks."),
            Tone::Base,
        );
        lines.push(Vec::new());
        let state = match (review, document.decision) {
            (true, Some(_)) => "Nothing from it runs.",
            _ => {
                "The hooks you approved earlier still run until you answer. Allow accepts the change: they stop running."
            }
        };
        text(&mut lines, state, Tone::Muted);
    } else {
        text(
            &mut lines,
            &format!(
                "The hooks in {shown} are waiting for your answer. They run these commands on this computer:"
            ),
            Tone::Base,
        );
        if review && document.decision.is_some() {
            lines.clear();
            text(
                &mut lines,
                &format!("The hooks in {shown} run these commands on this computer:"),
                Tone::Base,
            );
        }
        for hook in &document.hooks {
            lines.push(Vec::new());
            let mut head = vec![(hook.event.clone(), Tone::Bold)];
            let matcher = hook_matcher(hook);
            if !matcher.is_empty() {
                head.push((format!(" {matcher}"), Tone::Muted));
            }
            if let Some(timeout) = hook_timeout(hook) {
                head.push((format!(" · {timeout}"), Tone::Muted));
            }
            lines.push(head);
            for part in crate::text::wrap(&crate::text::visible(&hook.command), ROOM - 2) {
                lines.push(line(format!("  {part}"), Tone::Base));
            }
        }
        lines.push(Vec::new());
        let state = match (review, document.decision) {
            (true, Some(true)) => "They run now.",
            (true, Some(false)) => "They do not run: you refused them.",
            _ => "They run only if you allow them. You can change this later in /hooks.",
        };
        text(&mut lines, state, Tone::Muted);
    }
    let mut body = Body::new(
        BodyKind::HookTrust {
            path: document.path.clone(),
            digest: document.digest.clone(),
            review,
            shown: now,
        },
        "Project hooks",
        lines,
    );
    body.size = Size::Large;
    let refuse = if review { "Refuse" } else { "Not now" };
    body.buttons = vec![refuse.into(), "Allow".into()];
    body.active = 0;
    if body.lines.len() > 12 {
        body.actions = vec![Action::left("scroll", "↑/↓")];
    }
    body
}
