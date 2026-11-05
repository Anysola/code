//! The command palette (`ctrl+p`): every reachable palette command grouped by category in
//! registration order, `Suggested` first, and the Settings rows once something is typed.

use crate::dialog::{Kind, Opt, Select};
use crate::keymap::Keymap;
use crate::keymap::slash::SLASH;

/// What decides which commands are reachable and suggested.
#[derive(Debug, Clone, Default)]
pub struct Context {
    pub session: bool,
    pub sessions_exist: bool,
    pub connected: bool,
    pub variants: bool,
    pub revert: bool,
    pub stash: bool,
    pub prompt_text: bool,
    pub tabs: bool,
    pub terminal_open: bool,
    pub queued: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum When {
    Always,
    Session,
    Revert,
    Variants,
    Stash,
    PromptText,
    Tabs,
    Terminal,
    Queued,
}

/// (id, title, group, when) for commands without a `/name`; `/name` commands take their title
/// and group from the slash table.
const ENTRIES: &[(&str, &str, &str, When)] = &[
    ("session.list", "", "", When::Always),
    ("session.new", "", "", When::Always),
    ("session.clear", "", "", When::Always),
    ("open.menu", "", "", When::Always),
    ("session.tab.close", "Close tab", "Session", When::Tabs),
    (
        "session.tab.reopen",
        "Reopen closed tab",
        "Session",
        When::Tabs,
    ),
    ("model.list", "", "", When::Always),
    ("agent.list", "", "", When::Always),
    ("mcp.list", "", "", When::Always),
    ("variant.cycle", "Variant cycle", "Agent", When::Always),
    ("variant.list", "", "", When::Variants),
    ("provider.connect", "", "", When::Always),
    ("opencode.settings", "", "", When::Always),
    ("opencode.status", "", "", When::Always),
    ("opencode.update", "", "", When::Always),
    ("location.reload", "", "", When::Always),
    ("opencode.debug", "", "", When::Always),
    ("theme.switch", "", "", When::Always),
    ("help.show", "", "", When::Always),
    ("docs.open", "Open docs", "System", When::Always),
    ("app.exit", "", "", When::Always),
    ("session.rename", "", "", When::Session),
    ("session.timeline", "", "", When::Session),
    ("session.fork", "", "", When::Session),
    ("session.compact", "", "", When::Session),
    ("session.undo", "", "", When::Session),
    ("session.redo", "", "", When::Revert),
    ("messages.copy", "", "", When::Session),
    ("session.export", "", "", When::Session),
    ("session.terminal", "", "", When::Session),
    (
        "session.copy.id",
        "Copy session ID",
        "Session",
        When::Session,
    ),
    (
        "session.copy",
        "Copy session transcript",
        "Session",
        When::Session,
    ),
    (
        "session.verbosity.cycle",
        "Verbosity: cycle",
        "Session",
        When::Session,
    ),
    (
        "session.queued_prompts",
        "View queued prompts",
        "Prompt",
        When::Queued,
    ),
    (
        "session.sidebar.toggle",
        "Toggle sidebar",
        "Session",
        When::Session,
    ),
    (
        "terminal.toggle",
        "Toggle terminal pane",
        "Session",
        When::Session,
    ),
    (
        "terminal.select",
        "Select terminal",
        "Session",
        When::Session,
    ),
    (
        "terminal.close",
        "Close terminal pane",
        "Session",
        When::Terminal,
    ),
    ("prompt.editor", "", "", When::Always),
    ("session.move", "", "", When::Always),
    ("prompt.queue", "Queue prompt", "Prompt", When::Always),
    ("prompt.skills", "", "", When::Always),
    ("prompt.stash", "Stash prompt", "Prompt", When::PromptText),
    ("prompt.stash.pop", "Stash pop", "Prompt", When::Stash),
    ("prompt.stash.list", "Stash list", "Prompt", When::Stash),
    ("session.aside", "", "", When::Always),
    ("diff.open", "", "", When::Always),
    ("anysola.credits", "", "", When::Always),
    ("anysola.usage", "", "", When::Always),
    ("anysola.budget", "", "", When::Always),
    ("anysola.devices", "", "", When::Always),
    ("anysola.account", "", "", When::Always),
    ("anysola.feedback", "", "", When::Always),
];

fn reachable(when: When, cx: &Context) -> bool {
    match when {
        When::Always => true,
        When::Session => cx.session,
        When::Revert => cx.session && cx.revert,
        When::Variants => cx.variants,
        When::Stash => cx.stash,
        When::PromptText => cx.prompt_text,
        When::Tabs => cx.tabs,
        When::Terminal => cx.terminal_open,
        When::Queued => cx.queued,
    }
}

fn option(keymap: &Keymap, id: &str, title: &str, group: &str) -> Opt {
    let slash = SLASH.iter().find(|s| s.id == id);
    let title = slash.map_or(title, |s| s.title);
    let group = slash.map_or(group, |s| s.group);
    let description = slash.and_then(|s| s.description).unwrap_or_default();
    let mut opt = Opt::new(title, id)
        .category(group)
        .description(description)
        .footer(keymap.all(id));
    opt.search = format!("{id} {description}");
    opt
}

pub fn dialog(keymap: &Keymap, cx: &Context) -> Select {
    let suggested = [
        ("session.list", cx.sessions_exist),
        ("session.new", cx.session),
        ("model.list", true),
        ("provider.connect", !cx.connected),
        ("opencode.settings", true),
    ];
    let mut options: Vec<Opt> = suggested
        .iter()
        .filter(|(_, on)| *on)
        .map(|(id, _)| {
            let mut opt = option(keymap, id, "", "").category("Suggested");
            opt.searching = Some(false);
            opt
        })
        .collect();
    let commands = ENTRIES
        .iter()
        .filter(|(_, _, _, when)| reachable(*when, cx))
        .map(|(id, title, group, _)| option(keymap, id, title, group));
    options.extend(commands);
    for (i, s) in super::settings::all().iter().enumerate() {
        let mut opt = Opt::new(s.title, format!("setting:{i}")).category(s.category);
        opt.footer = format!("Settings · {}", s.category);
        opt.searching = Some(true);
        options.push(opt);
    }
    let mut select = Select::new(Kind::Palette, "Commands", options);
    select.selected = 0;
    select
}
