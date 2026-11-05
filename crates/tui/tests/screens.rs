//! Text snapshots of the TUI against the 0.5.1 captures in `spec/verbatim/tui/screens/`.

use std::path::PathBuf;

use anysola_core::events::CreditsSnapshot;
use anysola_core::gateway::account::Reply;
use anysola_tui::composer::{AtomKind, Mode};
use anysola_tui::dialog::{Dialog, Select};
use anysola_tui::menus::{anysola, lists, palette, settings};
use anysola_tui::state::{
    Account, Hints, ModelChoice, Permission, Popup, PopupAction, PopupRow, Recent, Route, Session,
    State, Tab, Toast, Variant,
};
use anysola_tui::text::parse_iso;
use anysola_tui::theme::{Catalog, Mode as ThemeMode};
use anysola_tui::transcript::model::{Assistant, Item, Part, Status, Tool};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use serde_json::json;

fn capture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../spec/verbatim/tui/screens")
        .join(format!("{name}.txt"));
    std::fs::read_to_string(path).unwrap()
}

fn render(state: &State) -> String {
    let theme = Catalog::bundled().load("anysola", ThemeMode::Dark).unwrap();
    let mut buffer = Buffer::empty(Rect::new(0, 0, state.width, state.height));
    anysola_tui::view::draw(state, &theme, &mut buffer);
    let mut out = Vec::new();
    for y in 0..state.height {
        let mut line = String::new();
        for x in 0..state.width {
            line.push_str(buffer[(x, y)].symbol());
        }
        out.push(line);
    }
    out.join("\n")
}

/// Compares row by row, ignoring trailing spaces; prints both screens on a mismatch.
fn assert_screen(state: &State, name: &str) {
    let actual = render(state);
    let expected = capture(name);
    let norm =
        |s: &str| -> Vec<String> { s.split('\n').map(|l| l.trim_end().to_string()).collect() };
    let (a, e) = (norm(&actual), norm(&expected));
    let rows = a.len().max(e.len());
    let diff: Vec<String> = (0..rows)
        .filter(|i| a.get(*i).map_or("", String::as_str) != e.get(*i).map_or("", String::as_str))
        .map(|i| {
            format!(
                "row {i}\n  want |{}|\n  got  |{}|",
                e.get(i).map_or("", String::as_str),
                a.get(i).map_or("", String::as_str)
            )
        })
        .collect();
    assert!(diff.is_empty(), "{name} differs:\n{}", diff.join("\n"));
}

fn now() -> u64 {
    parse_iso("2026-11-05T12:00:00Z").unwrap()
}

fn base(width: u16, height: u16) -> State {
    let mut state = State::new("0.5.1");
    state.width = width;
    state.height = height;
    state.now = now();
    state.location = "~/my-app:master".into();
    state.home = "/home/user".into();
    state.directory = "/home/user/my-app".into();
    state.account = Account {
        plan: Some("team".into()),
        renews: parse_iso("2026-11-23T00:00:00Z"),
        available: Some(731.0),
        ..Account::default()
    };
    state.model = Some(ModelChoice {
        provider: "anysola".into(),
        id: "claude-sonnet-5-5".into(),
        name: "Claude Sonnet 5.5".into(),
        variants: Vec::new(),
        context: Some(1_000_000),
    });
    state.hints = Hints {
        agents: Some("shift+tab".into()),
        commands: Some("ctrl+p".into()),
        sessions: Some("ctrl+x l".into()),
        background: Some("ctrl+b".into()),
        fullscreen: Some("ctrl+f".into()),
    };
    state.example = 1;
    state.zone = chrono::FixedOffset::east_opt(3 * 3600).unwrap();
    state
}

fn tabs(state: &mut State) {
    for id in ["ses_a", "ses_b"] {
        state.tabs.push(Tab {
            session_id: id.into(),
            title: "Fix typo in hello.txt".into(),
            attention: false,
        });
    }
}

fn home(width: u16, height: u16) -> State {
    let mut state = base(width, height);
    tabs(&mut state);
    state.recent = vec![
        Recent {
            id: "ses_b".into(),
            title: "Fix typo in hello.txt".into(),
            updated: now() - 10_000,
        },
        Recent {
            id: "ses_a".into(),
            title: "Fix typo in hello.txt".into(),
            updated: now() - 70_000,
        },
    ];
    state
}

#[test]
fn home_120x36_recent() {
    assert_screen(&home(120, 36), "home-120x36-recent");
}

#[test]
fn home_120x36_empty() {
    let state = base(120, 36);
    assert_screen(&state, "home-120x36-empty");
}

#[test]
fn home_170x40() {
    let mut state = home(170, 40);
    state.example = 2;
    assert_screen(&state, "home-170x40");
}

#[test]
fn home_60x20_narrow() {
    let mut state = home(60, 20);
    state.example = 2;
    assert_screen(&state, "home-60x20-narrow");
}

#[test]
fn shell_mode() {
    let mut state = base(120, 36);
    state.composer.mode = Mode::Shell;
    state.shell_example = 2;
    assert_screen(&state, "shell-mode");
}

fn slash_rows() -> Vec<PopupRow> {
    let rows = [
        ("/account", "Open your Anysola account"),
        ("/agents", "Switch agent"),
        (
            "/btw",
            "One-shot answer from the session's context without adding to the conversation",
        ),
        ("/budget", "Session budget"),
        ("/cd", "Change working directory"),
        ("/clear", "Clear session"),
        ("/connect", "Connect an integration"),
        ("/continue", "Switch session"),
        ("/credits", "Credits and plan"),
        ("/debug", "View debug info"),
        ("/worktrees", "Manage workspaces"),
    ];
    rows.iter()
        .map(|(name, description)| PopupRow {
            display: name.to_string(),
            description: description.to_string(),
            label: String::new(),
            action: PopupAction::Command(name.to_string()),
        })
        .collect()
}

#[test]
fn slash_popup() {
    let mut state = base(120, 36);
    state.composer.insert("/");
    state.popup = Some(Popup {
        trigger: '/',
        start: 0,
        rows: slash_rows(),
        selected: 0,
        offset: 0,
        pad: 12,
    });
    assert_screen(&state, "slash-popup");
}

#[test]
fn at_mention_popup() {
    let mut state = base(120, 36);
    state.composer.insert("@");
    let skills = [
        (
            "browser",
            "Use the in-app browser: open and read pages, click and type, run scripts",
        ),
        (
            "built-in-browser",
            "Read this skill before the first step that uses the browser",
        ),
        (
            "chrome-browser",
            "Read this skill before the first step that uses Chrome",
        ),
        (
            "computer-use",
            "Read this skill before the first step of any request",
        ),
        (
            "deep-research",
            "Use this skill when the user's prompt requires (1) research",
        ),
        (
            "docs",
            "docs (editable docs people share and comment on; the default for any document",
        ),
        (
            "docx",
            "Use this skill whenever the user wants to create, read, edit documents",
        ),
        (
            "google-workspace",
            "Read this before the first Google Drive, Docs, Sheets call",
        ),
        (
            "import-memory",
            "Import a memory export from another AI assistant into memory",
        ),
        (
            "merakli-stil",
            "Meraklı YouTube kanalı için banner, kapak, karakter, animasyon",
        ),
    ];
    state.popup = Some(Popup {
        trigger: '@',
        start: 0,
        rows: skills
            .iter()
            .map(|(name, description)| PopupRow {
                display: format!("@{name}"),
                description: description.to_string(),
                label: "skill".into(),
                action: PopupAction::Mention(AtomKind::Skill, format!("@{name}")),
            })
            .collect(),
        selected: 0,
        offset: 0,
        pad: 0,
    });
    assert_screen(&state, "at-mention-popup");
}

const PATCH: &str = "Index: hello.txt\n===================================================================\n--- hello.txt\n+++ hello.txt\n@@ -1,1 +1,1 @@\n-Helo wrold\n+Hello world\n";

const ANSWER: &str = "# Fixed\n\nThe typo in hello.txt is fixed:\n\n- before: Helo wrold\n- after: Hello world\n\n```ts\nexport const greet = (name: string) => `Hello ${name}`\n```\n\n| file | change |\n|---|---|\n| hello.txt | 1 line |\n\n> Run `bun test` to check.";

fn tool(name: &str, input: serde_json::Value, status: Status) -> Tool {
    Tool {
        call_id: format!("call_{name}"),
        name: name.into(),
        input,
        status,
        output: String::new(),
        metadata: Default::default(),
        error: None,
        started: now(),
        awaiting: false,
    }
}

/// The scripted turn: todowrite fails, a shell reads the file, an edit fixes it, then the answer.
fn turn(edit_done: bool, shell_done: bool) -> Vec<Item> {
    let mut todo = tool("todowrite", json!({}), Status::Failed);
    todo.error = Some("Invalid arguments".into());
    let mut shell = tool(
        "shell",
        json!({"command": "cat hello.txt && ls"}),
        Status::Running,
    );
    if shell_done {
        shell.status = Status::Completed;
        shell.output = "Helo wrold\nhello.txt\nopencode.json\n".into();
    }
    let mut items = vec![Item::User {
        id: "msg_u".into(),
        text: "fix the typo in hello.txt".into(),
        agent: "build".into(),
        files: Vec::new(),
        created: parse_iso("2026-11-05T14:02:00Z").unwrap(),
    }];
    let mut parts = vec![Part::Tool(todo), Part::Tool(shell)];
    if shell_done {
        let mut edit = tool(
            "edit",
            json!({"filePath": "/home/user/my-app/hello.txt"}),
            Status::Running,
        );
        if edit_done {
            edit.status = Status::Completed;
            edit.metadata = json!({"files": [{"patch": PATCH}]})
                .as_object()
                .unwrap()
                .clone();
        }
        parts.push(Part::Tool(edit));
    }
    let mut assistant = Assistant {
        id: "msg_a".into(),
        agent: "build".into(),
        model: "Claude Sonnet 5.5".into(),
        created: now(),
        turn_start: now(),
        parts,
        ..Assistant::default()
    };
    if edit_done {
        assistant.parts.push(Part::Reasoning {
            text: "done".into(),
            start: now(),
            end: Some(now() + 4),
        });
        assistant.parts.push(Part::Text {
            text: ANSWER.into(),
            done: true,
        });
        assistant.terminal = true;
        assistant.completed = Some(now() + 7100);
        assistant.output_tokens = 4531;
        assistant.created = now() + 1;
        assistant.usage = 1260;
    }
    items.push(Item::Assistant(assistant));
    items
}

fn session(width: u16, height: u16, items: Vec<Item>) -> State {
    let mut state = base(width, height);
    tabs(&mut state);
    state.route = Route::Session("ses_b".into());
    state.sessions.insert(
        "ses_b".into(),
        Session {
            id: "ses_b".into(),
            title: "Fix typo in hello.txt".into(),
            directory: "/home/user/my-app".into(),
            items,
            ..Session::default()
        },
    );
    state
}

fn credits(turn: u64, total: u64, budget: Option<u64>, remaining: u64) -> Option<CreditsSnapshot> {
    Some(CreditsSnapshot {
        session_id: "ses_b".into(),
        remaining: Some(remaining),
        turn,
        total,
        turns: 1,
        budget,
    })
}

#[test]
fn session_tools_edit_diff_markdown() {
    let mut state = session(120, 50, turn(true, true));
    state.sessions.get_mut("ses_b").unwrap().credits = credits(12, 12, None, 728);
    assert_screen(&state, "session-tools-edit-diff-markdown");
}

#[test]
fn session_170x40_sidebar() {
    let mut state = session(170, 40, turn(true, true));
    state.sessions.get_mut("ses_b").unwrap().credits = credits(0, 12, Some(50), 731);
    assert_screen(&state, "session-170x40-sidebar");
}

fn asking(state: &mut State, permission: Permission) {
    state.tabs[1].attention = true;
    let session = state.sessions.get_mut("ses_b").unwrap();
    session.busy = true;
    session.permission = Some(permission);
}

#[test]
fn permission_shell() {
    let mut state = session(120, 50, turn(false, false));
    // 3.5 s into the shell call, on the last spinner frame.
    state.now = now() + 3500 + 720 - (now() + 3500) % 800;
    asking(
        &mut state,
        Permission {
            request_id: "per_1".into(),
            action: "shell".into(),
            resources: vec!["cat hello.txt".into(), "ls".into()],
            save: vec!["cat *".into(), "ls *".into()],
            metadata: json!(null),
            subject: Some("cat hello.txt && ls".into()),
            selected: 0,
            fullscreen: false,
        },
    );
    assert_screen(&state, "permission-shell");
}

#[test]
fn permission_edit_diff() {
    let mut state = session(120, 50, turn(false, true));
    asking(
        &mut state,
        Permission {
            request_id: "per_2".into(),
            action: "edit".into(),
            resources: vec!["hello.txt".into()],
            save: vec!["*".into()],
            metadata: json!({"files": [{"patch": PATCH}]}),
            subject: Some("hello.txt".into()),
            selected: 0,
            fullscreen: false,
        },
    );
    assert_screen(&state, "permission-edit-diff");
}

fn dialog(mut state: State, dialog: Dialog) -> State {
    state.dialogs.push(dialog);
    state
}

/// The home screen the dialog captures were taken over.
fn under(example: usize) -> State {
    let mut state = base(120, 36);
    state.example = example;
    state
}

fn select(select: Select) -> Dialog {
    Dialog::Select(select)
}

#[test]
fn help() {
    let state = dialog(under(2), Dialog::Body(lists::help("ctrl+p")));
    assert_screen(&state, "help");
}

#[test]
fn debug() {
    let info = lists::DebugInfo {
        // [fix] §B9: the real version, not upstream 2.2.5.
        version: "2.2.5".into(),
        date: "2026-11-04T14:00:36.759Z".into(),
        os: "Linux 7.0.0-1013-oracle (arm64)".into(),
        terminal: "xterm-256color in screen".into(),
        session: None,
        model: Some("anysola/claude-sonnet-5-5".into()),
    };
    let state = dialog(under(1), Dialog::Body(lists::debug(&info)));
    assert_screen(&state, "debug");
}

fn me() -> serde_json::Value {
    json!({
        "plan": "team",
        "access": "paid",
        "credits": {"available": 731, "renewsAt": "2026-11-23T00:00:00Z"},
        "usageToday": {"requests": 3, "credits": 9},
    })
}

#[test]
fn credits_dialog() {
    let me = me();
    let view = anysola::CreditsView {
        me: Ok(&me),
        balance: Some(731.0),
        session: None,
        low: false,
        zone: under(1).zone,
        now: now(),
    };
    let state = dialog(under(1), Dialog::Body(anysola::credits_dialog(&view)));
    assert_screen(&state, "credits");
}

fn usage_reply() -> Result<Reply, String> {
    Ok(Reply {
        status: 200,
        body: json!({
            "totals": {"requests": 42, "credits": 318, "cacheHitRate": 0.64},
            "byDay": [
                {"date": "2026-11-02", "requests": 20, "credits": 150},
                {"date": "2026-11-03", "requests": 22, "credits": 168},
            ],
            "byModel": [
                {"model": "claude-sonnet-5-5", "requests": 30, "credits": 290},
                {"model": "claude-haiku-5-5", "requests": 12, "credits": 28},
            ],
            "creditsAvailable": 731,
            "renewsAt": "2026-11-23T00:00:00Z",
        }),
    })
}

#[test]
fn usage_7_and_30_days() {
    for (days, name) in [(7, "usage-7d"), (30, "usage-30d")] {
        let body = anysola::usage_dialog(days, Some(&usage_reply()), under(1).zone);
        let state = dialog(under(1), Dialog::Body(body));
        assert_screen(&state, name);
    }
}

#[test]
fn devices() {
    let body = json!({"tokens": [
        {"id": "tok_1", "name": "laptop", "client": "Anysola Code 0.5.1 · linux", "current": true, "lastUsedAt": "2026-11-04T09:12:00Z"},
        {"id": "tok_2", "name": "ci-runner", "client": "Anysola Code 0.5.0 · linux"},
    ]});
    let state = dialog(
        under(1),
        select(anysola::devices_dialog(&body, under(1).zone)),
    );
    assert_screen(&state, "devices");
}

#[test]
fn status_in_session() {
    let mut state = session(120, 36, turn(true, true));
    state.sessions.get_mut("ses_b").unwrap().credits = credits(0, 12, None, 731);
    let state = dialog(state, Dialog::Body(lists::status(&[])));
    assert_screen(&state, "status-in-session");
}

#[test]
fn mcps_empty() {
    let state = dialog(under(1), select(lists::mcps(&[])));
    assert_screen(&state, "mcps");
}

#[test]
fn connect() {
    let state = dialog(under(1), select(lists::connect(lists::Connection::Env)));
    assert_screen(&state, "connect");
}

#[test]
fn update_current() {
    let state = dialog(
        under(1),
        Dialog::Body(lists::update(&lists::Update::Current)),
    );
    assert_screen(&state, "update");
}

#[test]
fn theme_picker() {
    let names = Catalog::bundled().names();
    let state = dialog(under(1), select(lists::themes(&names, "anysola")));
    assert_screen(&state, "theme-picker");
}

#[test]
fn agent_picker() {
    let agents = vec![
        (
            "build".into(),
            "The default agent. Executes tools based on configured permissions.".into(),
        ),
        (
            "plan".into(),
            "Read-only agent for exploring the codebase and planning changes.".into(),
        ),
    ];
    let state = dialog(under(2), select(lists::agents(&agents, "build")));
    assert_screen(&state, "agent-picker");
}

#[test]
fn model_picker() {
    let row = |id: &str, name: &str, note: &str, estimate: Option<f64>| lists::ModelRow {
        provider: "anysola".into(),
        id: id.into(),
        name: name.into(),
        note: Some(note.into()),
        estimate,
    };
    let rows = vec![
        row("auto", "Auto", "Picks Haiku or Sonnet per turn", None),
        row(
            "claude-sonnet-5-5",
            "Claude Sonnet 5.5",
            "Default",
            Some(6.0),
        ),
        row("claude-haiku-5-5", "Claude Haiku 5.5", "Fastest", Some(1.0)),
    ];
    let mut state = under(1);
    state.composer.set("Ask");
    let models = lists::models(&rows, Some("claude-sonnet-5-5"), &[], &[]);
    assert_screen(&dialog(state, select(models)), "model-picker");
}

#[test]
fn settings_dialog() {
    let mut state = under(1);
    state.composer.set("Ask");
    // [fix] §B11: Animations reads `on` when unset; the 0.5.1 capture shows `off`.
    let tui = json!({"animations": false});
    let state = dialog(state, select(settings::dialog(&tui, 0)));
    assert_screen(&state, "settings");
}

fn session_items() -> Vec<lists::SessionItem> {
    ["ses_b", "ses_a"]
        .iter()
        .map(|id| lists::SessionItem {
            id: id.to_string(),
            title: "Fix typo in hello.txt".into(),
            updated: now() - 60_000,
            elsewhere: None,
            marker: None,
        })
        .collect()
}

#[test]
fn session_list() {
    let mut state = home(120, 36);
    state.example = 1;
    let list = lists::sessions(&session_items(), Some("my-app"), now(), state.zone);
    assert_screen(&dialog(state, select(list)), "session-list");
}

#[test]
fn sessions_empty() {
    let list = lists::sessions(&[], Some("my-app"), now(), under(1).zone);
    assert_screen(&dialog(under(1), select(list)), "sessions-empty");
}

#[test]
fn worktrees() {
    let mut state = under(1);
    state.example = 0;
    let list = lists::worktrees("~/my-app", &[], false);
    assert_screen(&dialog(state, select(list)), "worktrees");
}

#[test]
fn timeline() {
    let mut state = session(120, 36, turn(true, true));
    state.sessions.get_mut("ses_b").unwrap().credits = credits(0, 12, None, 731);
    let messages = vec![(
        "msg_u".to_string(),
        "fix the typo in hello.txt".to_string(),
        parse_iso("2026-11-05T14:02:00Z").unwrap(),
    )];
    let list = lists::timeline(&messages, state.zone);
    assert_screen(&dialog(state, select(list)), "timeline");
}

#[test]
fn command_palette() {
    let mut state = under(1);
    state.composer.set("Ask");
    let keymap = anysola_tui::keymap::Keymap::defaults();
    let cx = palette::Context {
        connected: true,
        tabs: true,
        variants: true,
        ..palette::Context::default()
    };
    let list = palette::dialog(&keymap, &cx);
    assert_screen(&dialog(state, select(list)), "command-palette");
}

#[test]
fn exit_summary() {
    let summary = anysola::Summary {
        title: "Fix typo in hello.txt".into(),
        id: "ses_e59221e89ffevUnouuY8RLvFMr".into(),
        turns: 1,
        spent: 12,
        updated: None,
    };
    let text = anysola::exit_summary(&summary);
    let plain: String = {
        let mut out = String::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c in chars.by_ref() {
                    if c == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    };
    let want: Vec<String> = capture("exit-summary")
        .lines()
        .map(|l| l.trim_end().to_string())
        .collect();
    let got: Vec<String> = plain.lines().map(|l| l.trim_end().to_string()).collect();
    assert_eq!(got, want[..got.len()]);
    assert!(want[got.len()..].iter().all(String::is_empty));
}

#[test]
fn budget_toasts() {
    let mut state = session(120, 36, turn(true, true));
    state.sessions.get_mut("ses_b").unwrap().credits = credits(0, 12, Some(50), 731);
    state.toast = Some(Toast {
        title: None,
        message: anysola::budget_text(12, Some(50)),
        variant: Variant::Success,
        until: u64::MAX,
    });
    assert_screen(&state, "budget-set-toast");
    let mut home = under(1);
    home.toast = Some(Toast {
        title: None,
        message: "Open a session first, or set one for new sessions: /budget default <credits>"
            .into(),
        variant: Variant::Warning,
        until: u64::MAX,
    });
    assert_screen(&home, "budget-toast-home");
}

#[test]
fn home_low_balance_event_banner() {
    let mut state = home(120, 36);
    state.example = 1;
    let mut me = me();
    me["credits"]["available"] = json!(64);
    me["event"] = json!({"label": "Double credits weekend", "until": "2026-11-09T21:00:00Z"});
    state.account = Account::from_me(me);
    state.recent[0].updated = now() - 70_000;
    state.recent[1].updated = now() - 130_000;
    assert_screen(&state, "home-low-balance-event-banner");
}
