//! Text snapshots of the TUI against the 0.5.1 captures in `spec/verbatim/tui/screens/`.

use std::path::PathBuf;

use anysola_core::events::UsageSnapshot;
use anysola_core::gateway::account::Reply;
use anysola_core::gateway::limits::{Limits, UsageEvent, Window};
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

/// Terminal sizes every rendered screen is also drawn at: drawing must not panic (debug builds
/// catch the arithmetic that would wrap in release) however small or large the terminal is.
const SIZES: &[(u16, u16)] = &[
    (1, 1),
    (2, 2),
    (8, 3),
    (12, 5),
    (20, 6),
    (24, 8),
    (24, 60),
    (26, 9),
    (30, 9),
    (300, 8),
    (32, 100),
    (40, 12),
    (44, 14),
    (52, 16),
    (64, 20),
    (80, 24),
    (100, 30),
    (140, 40),
    (220, 60),
];

fn sweep(state: &State, theme: &anysola_tui::theme::Theme) {
    for &(width, height) in SIZES {
        for left in [0, 20] {
            let mut sized = state.clone();
            sized.width = width;
            sized.height = height;
            sized.left = if width > 60 { left } else { 0 };
            let total = width + sized.left;
            let mut buffer = Buffer::empty(Rect::new(0, 0, total, height));
            let drawn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                anysola_tui::view::draw(&sized, theme, &mut buffer)
            }));
            assert!(
                drawn.is_ok(),
                "drawing panicked at {width}x{height} (left {})",
                sized.left
            );
            // The sizes the app works out between frames (pty, scrolling, pane drags).
            let sizes = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                use anysola_tui::view;
                let _ = view::terminal_size(&sized);
                let _ = view::pane_width(&sized);
                let _ = view::session_content_width(&sized);
                let _ = view::diff_width(&sized);
                if let Some(session) = sized.session() {
                    let w = view::session_content_width(&sized);
                    let _ = view::transcript_rows(&sized, session, theme, w);
                }
                if let Some(diff) = &sized.diff {
                    let _ = view::diff_tree_width(diff, width);
                }
            }));
            assert!(sizes.is_ok(), "layout sizes panicked at {width}x{height}");
        }
    }
}

fn render(state: &State) -> String {
    let theme = Catalog::bundled().load("anysola", ThemeMode::Dark).unwrap();
    sweep(state, &theme);
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
    if std::env::var_os("ANYSOLA_BLESS_SCREENS").is_some() {
        bless(state, name);
    }
    let diff = screen_diff(state, name);
    assert!(diff.is_empty(), "{name} differs:\n{}", diff.join("\n"));
}

/// `ANYSOLA_BLESS_SCREENS=1`: rewrites the rows of a capture that differ.
fn bless(state: &State, name: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../spec/verbatim/tui/screens")
        .join(format!("{name}.txt"));
    let actual: Vec<String> = render(state)
        .split('\n')
        .map(|l| l.trim_end().to_string())
        .collect();
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = old.split('\n').map(str::to_string).collect();
    for (i, row) in actual.iter().enumerate() {
        match lines.get_mut(i) {
            Some(line) if line.trim_end() != row => *line = row.clone(),
            Some(_) => {}
            None if row.is_empty() => {}
            None => {
                lines.resize(i, String::new());
                lines.push(row.clone());
            }
        }
    }
    std::fs::write(&path, lines.join("\n")).unwrap();
}

fn screen_diff(state: &State, name: &str) -> Vec<String> {
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
    diff
}

fn now() -> u64 {
    parse_iso("2026-11-05T12:00:00Z").unwrap()
}

const HOUR: u64 = 3_600_000;

/// The 5-hour window resets in 2 h, the weekly one in 3 d 4 h.
fn limits(five_hour: u32, weekly: u32) -> Limits {
    Limits {
        plan: Some("team".into()),
        allowed: five_hour < 100,
        five_hour: Window {
            percent: five_hour,
            resets_at: Some(now() + 2 * HOUR),
        },
        weekly: Window {
            percent: weekly,
            resets_at: Some(now() + 76 * HOUR),
        },
        ..Limits::default()
    }
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
        limits: Some(limits(42, 18)),
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
        redo: None,
        agents: Some("shift+tab".into()),
        commands: Some("ctrl+p".into()),
        sessions: Some("ctrl+x l".into()),
        background: Some("ctrl+b".into()),
        fullscreen: Some("ctrl+f".into()),
        child: Some("↓".into()),
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
            ..Tab::default()
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
        area: Default::default(),
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
        area: Default::default(),
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
        images: Vec::new(),
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
        images: Vec::new(),
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

fn usage(turn: f64, total: f64, budget: Option<u64>) -> Option<UsageSnapshot> {
    Some(UsageSnapshot {
        session_id: "ses_b".into(),
        latest: Some(UsageEvent {
            five_hour_percent_used: turn,
            five_hour: limits(42, 18).five_hour,
            weekly: limits(42, 18).weekly,
        }),
        turn,
        total,
        turns: 1,
        budget,
    })
}

#[test]
fn session_tools_edit_diff_markdown() {
    let mut state = session(120, 50, turn(true, true));
    state.sessions.get_mut("ses_b").unwrap().usage = usage(12.0, 12.0, None);
    assert_screen(&state, "session-tools-edit-diff-markdown");
}

#[test]
fn session_170x40_sidebar() {
    let mut state = session(170, 40, turn(true, true));
    state.sessions.get_mut("ses_b").unwrap().usage = usage(0.0, 12.0, Some(20));
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
            reject: None,
            shown: 0,
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
            reject: None,
            shown: 0,
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
        "limits": {"plan": "team", "allowed": true, "blockedBy": null,
            "fiveHour": {"percent": 42, "resetsAt": "2026-11-05T14:00:00Z"},
            "weekly": {"percent": 18, "resetsAt": "2026-11-08T16:00:00Z"}, "bank": []},
        "usageToday": {"requests": 3},
    })
}

fn usage_reply() -> Result<Reply, String> {
    Ok(Reply {
        status: 200,
        body: json!({
            "totals": {"requests": 42, "cacheHitRate": 0.64},
            "byDay": [
                {"date": "2026-11-02", "requests": 20},
                {"date": "2026-11-03", "requests": 22},
            ],
            "byModel": [
                {"model": "claude-sonnet-5-5", "requests": 30},
                {"model": "claude-haiku-5-5", "requests": 12},
            ],
            "plan": "team",
            "limits": {"plan": "team", "allowed": true, "blockedBy": null,
                "fiveHour": {"percent": 42, "resetsAt": "2026-11-05T14:00:00Z"},
                "weekly": {"percent": 18, "resetsAt": "2026-11-08T16:00:00Z"},
                "bank": [{"id": "b1", "expiresAt": "2026-11-20T00:00:00Z", "scope": "five_hour"},
                         {"id": "b2", "expiresAt": "2026-12-01T00:00:00Z", "scope": "both"}]},
        }),
    })
}

#[test]
fn usage_7_and_30_days() {
    for (days, name) in [(7, "usage-7d"), (30, "usage-30d")] {
        let view = anysola::UsageView {
            days,
            session: None,
            zone: under(1).zone,
            now: now(),
        };
        let body = anysola::usage_dialog(&view, Some(&usage_reply()));
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
    state.sessions.get_mut("ses_b").unwrap().usage = usage(0.0, 12.0, None);
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
    let row = |id: &str, name: &str, note: &str| lists::ModelRow {
        provider: "anysola".into(),
        id: id.into(),
        name: name.into(),
        note: Some(note.into()),
    };
    let rows = vec![
        row("auto", "Auto", "Picks Haiku or Sonnet per turn"),
        row("claude-sonnet-5-5", "Claude Sonnet 5.5", "Default"),
        row("claude-haiku-5-5", "Claude Haiku 5.5", "Fastest"),
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

/// `V/tui/dialogs.md §5`: while the debounced search runs the list gives way to its status.
#[test]
fn session_search_status() {
    let mut list = lists::sessions(&session_items(), Some("my-app"), now(), under(1).zone);
    list.query = "fix".into();
    list.status = Some((
        "Searching sessions…".into(),
        anysola_tui::dialog::Tone::Muted,
    ));
    let screen = render(&dialog(under(1), select(list)));
    assert!(screen.contains("Searching sessions…"), "{screen}");
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
    state.sessions.get_mut("ses_b").unwrap().usage = usage(0.0, 12.0, None);
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
        spent: 12.0,
        limits: Some("5h 42%".into()),
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
    state.sessions.get_mut("ses_b").unwrap().usage = usage(0.0, 12.0, Some(20));
    state.toast = Some(Toast {
        title: None,
        message: anysola::budget_text(12.0, Some(20)),
        variant: Variant::Success,
        until: u64::MAX,
        action: None,
    });
    assert_screen(&state, "budget-set-toast");
    let mut home = under(1);
    home.toast = Some(Toast {
        title: None,
        message: "Open a session first, or set one for new sessions: /budget default <percent>"
            .into(),
        variant: Variant::Warning,
        until: u64::MAX,
        action: None,
    });
    assert_screen(&home, "budget-toast-home");
}

#[test]
fn home_limit_event_banner() {
    let mut state = home(120, 36);
    state.example = 1;
    let mut me = me();
    me["limits"]["fiveHour"]["percent"] = json!(84);
    me["event"] = json!({"label": "Launch week", "until": "2026-11-09T21:00:00Z"});
    state.account = Account::from_me(me);
    state.recent[0].updated = now() - 70_000;
    state.recent[1].updated = now() - 130_000;
    assert_screen(&state, "home-limit-event-banner");
}

#[test]
fn skills() {
    // The captured list continues below the fold; its longest name (34 cells) sets the padding.
    let mut skills: Vec<(String, String)> = [
        ("Browser", "Use the in-app browser: open and read pages,"),
        (
            "built-in-browser",
            "Read this skill before the first step that us",
        ),
        (
            "chrome-browser",
            "Read this skill before the first step that us",
        ),
        (
            "computer-use",
            "Read this skill before the first step of any",
        ),
        (
            "deep-research",
            "Use this skill when the user's prompt require",
        ),
        ("docs", "docs (editable docs people share and comment"),
        ("docx", "Use this skill whenever the user wants to cre"),
        (
            "google-workspace",
            "Read this before the first Google Drive, Docs",
        ),
        (
            "import-memory",
            "Import a memory export from another AI assist",
        ),
        (
            "merakli-stil",
            "Meraklı YouTube kanalı için banner, kapak, ka",
        ),
        ("morning", "Render the user's morning brief as a styled H"),
        ("pdf", "Use this skill whenever the user wants to do"),
    ]
    .iter()
    .map(|(n, d)| (n.to_string(), d.to_string()))
    .collect();
    skills.push(("x".repeat(34), String::new()));
    let state = dialog(under(1), select(lists::skills(&skills)));
    assert_screen(&state, "skills");
}

#[test]
fn session_120x36_after_turn() {
    let mut items = turn(true, true);
    if let Some(Item::Assistant(a)) = items.last_mut() {
        a.created = now() + 69;
        a.completed = Some(now() + 345);
        a.output_tokens = 183;
        for part in &mut a.parts {
            if let Part::Reasoning { end, .. } = part {
                *end = Some(now() + 19);
            }
        }
    }
    let mut state = session(120, 36, items);
    state.tabs.remove(0);
    state.sessions.get_mut("ses_b").unwrap().usage = usage(12.0, 12.0, None);
    assert_screen(&state, "session-120x36-after-turn");
}

/// The first `/name` drawn in a capture, for finding the scroll offset it was taken at.
fn first_slash(name: &str) -> String {
    capture(name)
        .lines()
        .find_map(|l| {
            l.split('┃')
                .nth(1)?
                .split_whitespace()
                .next()
                .map(str::to_string)
        })
        .unwrap()
}

fn home_scope_server() -> Vec<(String, String)> {
    vec![
        (
            "goal".into(),
            "keep working until an objective is met [pause|resume|clear]".into(),
        ),
        ("init".into(), "guided AGENTS.md setup".into()),
        (
            "review".into(),
            "review changes [commit|branch|pr], defaults to uncommitted".into(),
        ),
    ]
}

#[test]
fn slash_list_home_context() {
    let server = home_scope_server();
    let scope = anysola_tui::complete::Scope {
        in_session: false,
        revert_staged: false,
        server: &server,
    };
    let (rows, pad) = anysola_tui::complete::slash_rows(&scope);
    // `/reset` is new in 1.0.1.
    let ours: Vec<String> = rows
        .iter()
        .filter(|r| r.display != "/reset")
        .map(|r| {
            format!("┃ {:width$}{}", r.display, r.description, width = pad + 1)
                .trim_end()
                .to_string()
        })
        .collect();
    // `/plugins` is not carried over (decision 5); the capture cuts descriptions at the popup.
    let want: Vec<String> = capture("slash-list-home-context")
        .lines()
        .map(|l| l.trim_end().to_string())
        .filter(|l| l.starts_with("┃ /") && !l.starts_with("┃ /plugins"))
        .collect();
    assert_eq!(ours.len(), want.len());
    for (ours, want) in ours.iter().zip(&want) {
        assert!(ours.starts_with(want.as_str()), "{ours} vs {want}");
    }
}

#[test]
fn slash_popup_scroll_series() {
    let server = home_scope_server();
    let scope = anysola_tui::complete::Scope {
        in_session: false,
        revert_staged: false,
        server: &server,
    };
    let (mut rows, pad) = anysola_tui::complete::slash_rows(&scope);
    // `/reset` is new in 1.0.1, so not in the 0.5.1 captures.
    rows.retain(|r| r.display != "/reset");
    // `/plugins` is not carried over (decision 5) but sits in the 0.5.1 captures.
    let at = rows.iter().position(|r| r.display == "/project").unwrap();
    rows.insert(
        at,
        PopupRow {
            display: "/plugins".into(),
            description: "Plugins".into(),
            label: String::new(),
            action: PopupAction::Command("plugins.list".into()),
        },
    );
    for i in 0..=6 {
        let name = format!("slash-popup-scroll-{i}");
        let first = first_slash(&name);
        let offset = rows.iter().position(|r| r.display == first).unwrap();
        let mut state = base(120, 60);
        state.composer.insert("/");
        state.popup = Some(Popup {
            trigger: '/',
            start: 0,
            rows: rows.clone(),
            selected: offset,
            offset,
            pad,
            area: Default::default(),
        });
        assert_screen(&state, &name);
    }
}

#[test]
fn command_palette_scroll_series() {
    let keymap = anysola_tui::keymap::Keymap::defaults();
    let cx = palette::Context {
        connected: true,
        tabs: true,
        variants: true,
        ..palette::Context::default()
    };
    // `Toggle console` and `Plugins` are not carried over but sit in the 0.5.1 captures.
    let list = || {
        let mut list = palette::dialog(&keymap, &cx);
        // New in 1.0.1, so not in the 0.5.1 captures.
        list.options.retain(|o| o.title != "Use a banked reset");
        let at = list
            .options
            .iter()
            .position(|o| o.title == "Exit the app")
            .unwrap()
            + 1;
        for title in ["Plugins", "Toggle console"] {
            let mut opt = list.options[at - 1].clone();
            opt.title = title.into();
            opt.footer = String::new();
            list.options.insert(at, opt);
        }
        list
    };
    for i in 0..=7 {
        let name = format!("command-palette-60rows-scroll-{i}");
        let mut state = under(1);
        state.height = 60;
        state.composer.set("Ask");
        if std::env::var_os("ANYSOLA_BLESS_SCREENS").is_some() {
            let best = (0..200)
                .min_by_key(|&offset| {
                    let mut list = list();
                    list.offset = offset;
                    screen_diff(&dialog(state.clone(), select(list)), &name).len()
                })
                .unwrap();
            let mut list = list();
            list.offset = best;
            bless(&dialog(state.clone(), select(list)), &name);
        }
        let found = (0..200).any(|offset| {
            let mut list = list();
            list.offset = offset;
            screen_diff(&dialog(state.clone(), select(list)), &name).is_empty()
        });
        let list = list();
        assert!(
            found,
            "{name}: no offset matches; offset 0:\n{}",
            screen_diff(&dialog(state, select(list)), &name).join("\n")
        );
    }
}

#[test]
fn open() {
    let titles = [
        "Latest Bun version",
        "Latest Bun version",
        "Explain this project",
        "Add a --json flag",
        "Fix date parser tests",
    ];
    let sessions: Vec<lists::OpenSession> = titles
        .iter()
        .enumerate()
        .map(|(i, title)| lists::OpenSession {
            id: format!("ses_{i}"),
            title: title.to_string(),
            project: "my-app".into(),
            ago: "5h".into(),
        })
        .collect();
    let elsewhere = "/tmp/claude-1001/-home-ubuntu/baa4cc54-bc0a-55b0-9ae5-1c58ec01d2f3/scratchpad/shots2/home-0.5.1/my-app";
    let projects = vec![
        lists::OpenProject {
            name: "my-app".into(),
            directory: "/home/user/my-app".into(),
            shown: "~/my-app".into(),
            git: true,
            current: true,
        },
        lists::OpenProject {
            name: "my-app".into(),
            directory: elsewhere.into(),
            shown: elsewhere.into(),
            git: false,
            current: false,
        },
    ];
    let state = under(1);
    let list = lists::open(&sessions, &projects, state.width as usize);
    assert_screen(&dialog(state, select(list)), "open");
}

/// No capture exists; 0.5.1 puts the banners in the composer's top slot.
#[test]
fn session_banners_sit_above_the_composer() {
    let mut state = session(120, 36, turn(true, true));
    let mut me = me();
    me["limits"]["fiveHour"]["percent"] = json!(84);
    me["event"] = json!({"label": "Launch week", "until": "2026-11-09T21:00:00Z"});
    state.account = Account::from_me(me);
    let screen = render(&state);
    let lines: Vec<&str> = screen.lines().collect();
    let low = lines
        .iter()
        .position(|l| l.trim() == "5-hour limit 84% used · resets in 2 h · /usage")
        .expect("limit banner");
    assert!(
        lines[low - 1]
            .trim()
            .starts_with("Launch week · until Nov 10")
    );
    assert!(lines[low + 1].trim_start().starts_with('┃'));
}

#[test]
fn status_line_hover_shows_the_reset() {
    let mut state = session(120, 36, turn(true, true));
    state.sessions.get_mut("ses_b").unwrap().usage = usage(1.5, 12.0, Some(20));
    let screen = render(&state);
    assert!(screen.contains("5h 42% · session 12%/20%  "));
    state.status_hover = true;
    let screen = render(&state);
    assert!(screen.contains("5h 42% · session 12%/20% · resets in 2 h"));
}

#[test]
fn status_line_names_the_weekly_window_when_it_matters() {
    let mut state = session(120, 36, turn(true, true));
    state.account.limits = Some(limits(30, 85));
    let screen = render(&state);
    assert!(screen.contains("5h 30% · week 85%"));
    state.account.limits = Some(limits(42, 18));
    assert!(!render(&state).contains("week"));
}

#[test]
fn banked_reset_offer() {
    let mut limits = limits(100, 18);
    limits.bank = vec![
        anysola_core::gateway::limits::Banked {
            id: "b1".into(),
            expires_at: parse_iso("2026-11-20T00:00:00Z"),
            both: false,
        },
        anysola_core::gateway::limits::Banked {
            id: "b2".into(),
            expires_at: parse_iso("2026-12-01T00:00:00Z"),
            both: true,
        },
    ];
    let zone = under(1).zone;
    let message = "5-hour limit reached. Resets in 2 h. You have 2 banked resets.";
    let text = anysola::banked_reset_message(message, &limits, now(), zone).unwrap();
    assert_eq!(
        text,
        "5-hour limit reached. Resets in 2 h. You have 2 banked resets. The next one expires Nov 20, 03:00 AM. Using one clears the window and sends your prompt again."
    );
    let state = dialog(
        session(120, 36, turn(true, true)),
        Dialog::Confirm(anysola_tui::dialog::Confirm {
            kind: anysola_tui::dialog::ConfirmKind::BankedReset {
                session: "ses_b".into(),
            },
            title: "Use a banked reset?".into(),
            message: text,
            button: "Use banked reset".into(),
            yes: true,
        }),
    );
    let screen = render(&state);
    assert!(screen.contains("Use a banked reset?"), "{screen}");
    assert!(screen.contains("Wait"), "{screen}");
    assert!(screen.contains("Use banked reset"), "{screen}");
    limits.bank.clear();
    assert_eq!(
        anysola::banked_reset_message(message, &limits, now(), zone),
        None
    );
}

#[test]
fn use_banked_reset_confirm() {
    let banked = |id: &str, at: &str, both: bool| anysola_core::gateway::limits::Banked {
        id: id.into(),
        expires_at: parse_iso(at),
        both,
    };
    let zone = under(1).zone;
    let mut limits = limits(100, 18);
    limits.bank = vec![
        banked("b1", "2026-11-20T00:00:00Z", false),
        banked("b2", "2026-12-01T00:00:00Z", true),
    ];
    let text = anysola::use_reset_message(&limits, now(), zone).unwrap();
    assert_eq!(
        text,
        "You have 2 banked resets. The next one expires Nov 20, 03:00 AM. Using one clears your 5-hour window."
    );
    limits.bank = vec![banked("b2", "2026-12-01T00:00:00Z", true)];
    let text = anysola::use_reset_message(&limits, now(), zone).unwrap();
    assert_eq!(
        text,
        "You have 1 banked reset. The next one expires Dec 1, 03:00 AM. Using one clears your 5-hour and weekly windows."
    );
    let state = dialog(
        under(1),
        Dialog::Confirm(anysola_tui::dialog::Confirm {
            kind: anysola_tui::dialog::ConfirmKind::UseBankedReset { usage: None },
            title: "Use a banked reset?".into(),
            message: text,
            button: "Use banked reset".into(),
            yes: true,
        }),
    );
    let screen = render(&state);
    assert!(screen.contains("Use a banked reset?"), "{screen}");
    assert!(screen.contains("Cancel"), "{screen}");
    assert!(screen.contains("5-hour and weekly"), "{screen}");
    // Expired entries do not count; an empty bank has nothing to confirm.
    limits.bank = vec![banked("b0", "2026-11-01T00:00:00Z", true)];
    assert_eq!(anysola::use_reset_message(&limits, now(), zone), None);
    limits.bank.clear();
    assert_eq!(anysola::use_reset_message(&limits, now(), zone), None);
}

#[test]
fn usage_offers_the_bank_only_when_it_holds_one() {
    let view = anysola::UsageView {
        days: 7,
        session: None,
        zone: under(1).zone,
        now: now(),
    };
    let keys = |body: &anysola_tui::dialog::Body| {
        body.actions
            .iter()
            .map(|a| a.key.clone())
            .collect::<Vec<_>>()
    };
    let body = anysola::usage_dialog(&view, Some(&usage_reply()));
    assert_eq!(keys(&body), ["t", "r"]);
    let screen = render(&dialog(under(1), Dialog::Body(body)));
    assert!(screen.contains("use banked reset r"), "{screen}");
    let mut empty = usage_reply().unwrap();
    empty.body["limits"]["bank"] = json!([]);
    assert_eq!(keys(&anysola::usage_dialog(&view, Some(&Ok(empty)))), ["t"]);
    assert_eq!(keys(&anysola::usage_dialog(&view, None)), ["t"]);
}

#[test]
fn terminal_pane_width_follows_the_drag_within_bounds() {
    let mut state = base(120, 36);
    assert_eq!(anysola_tui::view::terminal_width(&state), 60);
    state.pane_width = Some(30);
    assert_eq!(anysola_tui::view::terminal_width(&state), 30);
    state.pane_width = Some(5);
    assert_eq!(anysola_tui::view::terminal_width(&state), 24);
    state.pane_width = Some(110);
    assert_eq!(anysola_tui::view::terminal_width(&state), 80);
}

/// No capture exists; the dock replaces the prompt with tabs, rows and hints.
#[test]
fn composer_dock_replaces_the_prompt() {
    use anysola_tui::state::{Dock, DockRow, DockTab};
    let mut state = session(120, 36, turn(true, true));
    state.dock = Some(Dock {
        tabs: vec![DockTab::Subagents, DockTab::Shell, DockTab::Terminals],
        tab: 0,
        rows: vec![DockRow {
            label: "└─ Explore: Find the parser".into(),
            right: "Running".into(),
            value: "ses_c".into(),
        }],
        selected: 0,
        inactive: false,
    });
    let screen = render(&state);
    let lines: Vec<&str> = screen.lines().map(str::trim_end).collect();
    let header = lines
        .iter()
        .position(|l| l.contains("Subagents  Shell  Terminals"))
        .unwrap();
    assert!(lines[header].ends_with("esc"));
    assert!(lines[header + 1].contains("└─ Explore: Find the parser"));
    assert!(lines[header + 1].ends_with("Running"));
    assert!(lines[header + 3].contains("show inactive ctrl+a  interrupt ctrl+d  tabs ←/→"));
    assert!(!screen.contains("Build · Claude Sonnet 5.5\n"));
    state.dock.as_mut().unwrap().rows.clear();
    state.dock.as_mut().unwrap().tab = 1;
    assert!(render(&state).contains("No shell commands"));
}

/// `feature-plugins/prompt/footer.tsx`: the live item leads the footer's right side.
#[test]
fn footer_live_subagents_and_shells() {
    let mut state = session(120, 36, turn(true, true));
    state.live = Some("2 subagents · 1 shell".into());
    let screen = render(&state);
    let footer = screen
        .lines()
        .find(|l| l.contains("subagents"))
        .expect("footer row");
    assert!(footer.contains("↓ 2 subagents · 1 shell"), "{footer}");
}

/// `component/prompt/autocomplete.tsx`: the first `ctrl+d` on a `recent` row asks first.
#[test]
fn cd_recent_asks_before_removal() {
    let mut state = session(120, 36, Vec::new());
    state.popup = Some(Popup {
        trigger: '/',
        start: 4,
        rows: vec![PopupRow {
            display: "~/old".into(),
            description: "recent".into(),
            label: String::new(),
            action: PopupAction::Insert("~/old".into()),
        }],
        selected: 0,
        offset: 0,
        pad: 0,
        area: Default::default(),
    });
    state.popup_confirm = true;
    let screen = render(&state);
    assert!(screen.contains("Press ctrl+d to confirm"));
    assert!(!screen.contains("~/old"));
}

/// `component/session-tabs.tsx` `TabIndicator`: `?` for a question, `•` for unread.
#[test]
fn tab_markers() {
    let mut state = session(120, 36, Vec::new());
    state.tabs[0].attention = true;
    state.tabs[0].question = true;
    state.tabs[1].unread = true;
    let strip = render(&state).lines().next().unwrap().to_string();
    assert!(strip.starts_with(" ? "), "{strip}");
    assert!(strip.contains(" • "), "{strip}");
}

/// `routes/session/location-missing.tsx`: the question replaces the prompt.
#[test]
fn session_location_unavailable() {
    let mut state = session(120, 36, Vec::new());
    state.sessions.get_mut("ses_b").unwrap().missing = true;
    let screen = render(&state);
    let shown = format!("~{}my-app", std::path::MAIN_SEPARATOR);
    for text in [
        "△ Session location unavailable",
        shown.as_str(),
        "Choose another directory to continue this session.",
        " Choose directory ",
        "enter confirm",
    ] {
        assert!(screen.contains(text), "{text}\n{screen}");
    }
}

/// `component/prompt/move.tsx`: a worktree chosen on home waits for the first submit.
#[test]
fn home_footer_new_worktree() {
    let mut state = home(120, 36);
    state.new_worktree = Some("brave-fox".into());
    assert!(render(&state).contains("(new worktree)"));
    state.worktree_progress = Some("Creating worktree".into());
    assert!(render(&state).contains("Creating worktree..."));
}

/// `feature-plugins/system/diff-viewer.tsx`: from 108 columns `/diff` docks beside the session.
#[test]
fn diff_docks_beside_a_wide_session() {
    let mut state = session(120, 36, turn(true, true));
    state.diff = Some(anysola_tui::diffview::Diff::new(
        anysola_tui::diffview::Source::All,
    ));
    let screen = render(&state);
    let header = screen.lines().nth(1).unwrap();
    assert!(
        header[header.char_indices().nth(60).unwrap().0..].contains("All"),
        "{header}"
    );
    assert!(screen.contains("Loading diff…"));
    state.width = 100;
    let narrow = render(&state);
    assert!(
        narrow.lines().next().unwrap().contains("All"),
        "full screen below 108"
    );
}

/// `ui/toast.tsx`: an action replaces the close mark with `› <label>`.
#[test]
fn toast_action_affordance() {
    let mut state = home(120, 36);
    state.toast = Some(Toast {
        title: Some("MCP server failed: docs".into()),
        message: "Run /mcps to view details.".into(),
        variant: Variant::Error,
        until: u64::MAX,
        action: Some(("Open MCP servers".into(), "mcp.list".into())),
    });
    state.toast_queue.push(state.toast.clone().unwrap());
    let screen = render(&state);
    let rows: Vec<&str> = screen.lines().collect();
    assert!(
        rows[2].contains("MCP server failed: docs") && rows[2].contains("› Open MCP servers"),
        "{}",
        rows[2]
    );
    // `toast.tsx`: a blank row under the title, `+N more` under the message.
    assert!(!rows[3].contains("Run /mcps") && rows[4].contains("Run /mcps to view details."));
    assert!(rows[6].contains("+1 more"), "{}", rows[6]);
}

/// `feature-plugins/sidebar/mcp.tsx`: one row per server; more than two can collapse.
#[test]
fn sidebar_mcp_section() {
    let mut state = session(170, 40, turn(true, true));
    state.mcp = (3, 1, 1);
    state.mcp_list = vec![
        ("docs".into(), "text.feedback.success.base", "Connected"),
        ("jira".into(), "text.feedback.error.base", "Error"),
        ("drive".into(), "text.feedback.warning.base", "Sign in"),
    ];
    let screen = render(&state);
    assert!(screen.contains("▼ MCP"));
    let row = screen.lines().find(|l| l.contains("• jira")).unwrap();
    assert!(row.trim_end().ends_with("Error"), "{row}");
    state.mcp_closed = true;
    assert!(render(&state).contains("▶ MCP (1 active, 1 error)"));
}

/// `feature-plugins/prompt/footer.tsx`: the goal follows the live counts after ` · `.
#[test]
fn footer_goal_status() {
    use anysola_core::session::goal::{Goal, Status as GoalStatus};
    let mut state = session(120, 36, turn(true, true));
    state.live = Some("2 subagents".into());
    state.sessions.get_mut("ses_b").unwrap().goal = Some(Goal {
        id: "g".into(),
        objective: "Ship it".into(),
        status: GoalStatus::Paused,
        reason: None,
        rounds: 0,
        max_rounds: 50,
        stalls: None,
        boot: String::new(),
        updated_at: 0,
    });
    assert!(render(&state).contains("↓ 2 subagents · ◎ goal paused"));
}

/// `component/prompt/autocomplete.tsx`: an empty `/cd` argument popup says so.
#[test]
fn no_matching_directories() {
    let mut state = session(120, 36, Vec::new());
    state.popup = Some(Popup {
        trigger: 'd',
        start: 4,
        rows: Vec::new(),
        selected: 0,
        offset: 0,
        pad: 0,
        area: Default::default(),
    });
    assert!(render(&state).contains("No matching directories"));
    let area = state.popup.as_ref().unwrap().area.get();
    assert_eq!(area.height, 1);
}

/// `feature-plugins/sidebar/footer.tsx`: the `Getting started` card shows only when signed out
/// and the terminal is at least 22 rows tall.
#[test]
fn getting_started_card() {
    let mut state = session(170, 40, Vec::new());
    state.account.signed_out = true;
    let screen = render(&state);
    for text in ["Getting started", "Sign in with your Anysola", "/connect"] {
        assert!(screen.contains(text), "{text}");
    }
    state.onboarding_dismissed = true;
    assert!(!render(&state).contains("Getting started"));
    state.onboarding_dismissed = false;
    state.account.signed_out = false;
    assert!(!render(&state).contains("Getting started"));
}

/// `component/title-shimmer.tsx`: a pending title keeps its text while it shimmers.
#[test]
fn pending_title_shimmers() {
    let mut state = session(170, 40, Vec::new());
    let session = state.sessions.get_mut("ses_b").unwrap();
    session.title.clear();
    session.busy = true;
    assert!(render(&state).contains("New session"));
}

/// `util/signal.ts` `createFadeIn`: the labels start at the background colour and reach full
/// strength after 160 ms.
#[test]
fn metadata_fade_in() {
    let mut state = session(170, 40, Vec::new());
    state.meta_fade = Some(1000);
    state.now = 1000;
    assert_eq!(state.meta_alpha(), 0.0);
    state.now = 1080;
    assert_eq!(state.meta_alpha(), 0.5);
    state.now = 1200;
    assert_eq!(state.meta_alpha(), 1.0);
    state.meta_fade = None;
    assert_eq!(state.meta_alpha(), 1.0);
}

/// `routes/session/index.tsx` `AssistantRetry`: counts down each second, then says it is due.
#[test]
fn retry_countdown() {
    use anysola_tui::transcript::model::Retry;
    let assistant = Assistant {
        id: "msg_a".into(),
        agent: "build".into(),
        model: "Claude Sonnet 5.5".into(),
        created: now(),
        turn_start: now(),
        retry: Some(Retry {
            at: now() + 4500,
            attempt: 2,
            message: "Overloaded".into(),
        }),
        ..Assistant::default()
    };
    let mut state = session(120, 36, vec![Item::Assistant(assistant)]);
    state.now = now();
    assert!(render(&state).contains("⚠ Retrying in 5s · attempt 2 · Overloaded"));
    state.now = now() + 4600;
    assert!(render(&state).contains("⚠ Retry due · attempt 2 · Overloaded"));
}

/// `routes/session/permission.tsx` `PermissionReject`: the reason dialog of a child session.
#[test]
fn permission_reject_reason() {
    let mut state = session(120, 50, turn(false, false));
    asking(
        &mut state,
        Permission {
            request_id: "per_3".into(),
            action: "shell".into(),
            resources: vec!["ls".into()],
            save: Vec::new(),
            metadata: json!(null),
            subject: Some("ls".into()),
            selected: 1,
            fullscreen: false,
            reject: Some("use cat".into()),
            shown: 0,
        },
    );
    let screen = render(&state);
    for text in [
        "△ Reject permission",
        "Tell Anysola Code what to do differently",
        "use cat",
        "enter confirm",
        "esc cancel",
    ] {
        assert!(screen.contains(text), "{text}");
    }
    assert!(!screen.contains("Permission required"));
}

/// `routes/session/index.tsx` `RevertMessage` and `CompactionQueued`.
#[test]
fn revert_banner_and_queued_compaction() {
    use anysola_tui::state::Revert;
    let mut state = session(120, 40, Vec::new());
    state.hints.redo = Some("ctrl+x r".into());
    let s = state.sessions.get_mut("ses_b").unwrap();
    s.compact_queued = true;
    s.revert = Some(Revert {
        count: 2,
        files: vec![
            ('M', "src/main.rs".into(), 3, 1),
            ('A', "notes.md".into(), 5, 0),
        ],
    });
    let screen = render(&state);
    for text in [
        "◇ Compaction queued",
        "2 messages reverted",
        "M src/main.rs +3 -1",
        "A notes.md +5",
        "ctrl+x r or /redo to restore",
    ] {
        assert!(screen.contains(text), "{text}");
    }
}

/// `routes/session/group-view.tsx`: expanded thoughts are markdown, not source text.
#[test]
fn expanded_thought_is_markdown() {
    let assistant = Assistant {
        id: "msg_a".into(),
        agent: "build".into(),
        model: "Claude Sonnet 5.5".into(),
        created: now(),
        turn_start: now(),
        terminal: true,
        parts: vec![Part::Reasoning {
            text: "**Plan**\n\nUse `cat` then **stop**".into(),
            start: now(),
            end: Some(now() + 2),
        }],
        ..Assistant::default()
    };
    let mut state = session(120, 36, vec![Item::Assistant(assistant)]);
    state.settings.thinking = true;
    let screen = render(&state);
    assert!(screen.contains("Use cat then stop"), "{screen}");
    assert!(!screen.contains("**"));
}

/// `dialogs.md` §5 and §19: a search with no hit names what was searched.
#[test]
fn empty_search_texts() {
    let mut state = base(120, 30);
    let mut skills = lists::skills(&[("a".into(), "b".into())]);
    skills.query = "zzz".into();
    state.dialogs.push(Dialog::Select(skills));
    assert!(render(&state).contains("No skills found"));
}

/// `routes/session/index.tsx` `Execute`: nested calls and the runtime error preview.
#[test]
fn execute_rows() {
    let mut call = tool("execute", json!({"code": "x"}), Status::Completed);
    call.metadata = json!({
        "error": true,
        "toolCalls": [
            {"tool": "opencode.models", "status": "completed", "input": {"query": "x"}},
            {"tool": "opencode.worktree_list", "status": "error"},
        ],
    })
    .as_object()
    .unwrap()
    .clone();
    call.output = "\x1b[31mTypeError: boom\x1b[0m\nat line 1".into();
    let assistant = Assistant {
        id: "msg_a".into(),
        agent: "build".into(),
        model: "Claude Sonnet 5.5".into(),
        created: now(),
        turn_start: now(),
        parts: vec![Part::Tool(call)],
        ..Assistant::default()
    };
    let screen = render(&session(120, 36, vec![Item::Assistant(assistant.clone())]));
    for text in [
        "✗ execute",
        "› opencode.models [query=x]",
        "✗ opencode.worktree_list",
        "↳ TypeError: boom",
        "  at line 1",
    ] {
        assert!(screen.contains(text), "{text}\n{screen}");
    }
    // `ExecuteCallView`: a click on a call with input lists its arguments under a `│` border.
    let mut state = session(120, 36, vec![Item::Assistant(assistant)]);
    state
        .sessions
        .get_mut("ses_b")
        .unwrap()
        .toggled
        .insert("call:call_execute:0".into());
    let screen = render(&state);
    assert!(screen.contains("› opencode.models  "), "{screen}");
    assert!(screen.contains("│ query: x"), "{screen}");
}

/// `dialog-execute.tsx`: title, status and the copy actions of the Execute dialog.
#[test]
fn execute_dialog_frame() {
    use anysola_tui::dialog::{Action, Body, BodyKind, Size, Tone};
    let mut state = base(120, 30);
    let lines = vec![
        vec![("Code".into(), Tone::Bold)],
        vec![(" 1 ".into(), Tone::Muted), ("return 1".into(), Tone::Base)],
        vec![("Output".into(), Tone::Bold)],
        vec![("No output".into(), Tone::Muted)],
    ];
    let mut body = Body::new(
        BodyKind::Execute {
            code: "return 1".into(),
            output: String::new(),
        },
        "execute",
        lines,
    );
    body.title.push(("  Completed".into(), Tone::Muted));
    body.size = Size::Large;
    body.actions = vec![
        Action::left("copy code", "c"),
        Action::left("copy output", "o"),
    ];
    state.dialogs.push(Dialog::Body(body));
    let screen = render(&state);
    for text in [
        "execute",
        "Completed",
        "return 1",
        "copy code c",
        "copy output o",
    ] {
        assert!(screen.contains(text), "{text}");
    }
}

/// `session-frame.tsx:387-398`: at ≤ 120 columns the toggled sidebar is an overlay, right-aligned
/// over the dimmed session, with the docked sidebar's contents.
#[test]
fn sidebar_overlay_below_121_columns() {
    let mut state = session(100, 40, turn(true, true));
    state.sessions.get_mut("ses_b").unwrap().usage = usage(0.0, 12.0, Some(20));
    let closed = render(&state);
    assert!(!anysola_tui::view::sidebar_overlay(&state));
    state.sidebar_open = true;
    assert!(anysola_tui::view::sidebar_overlay(&state));
    let wide = {
        let mut wide = session(170, 40, turn(true, true));
        wide.sessions.get_mut("ses_b").unwrap().usage = usage(0.0, 12.0, Some(20));
        render(&wide)
    };
    let open = render(&state);
    for (row, (narrow, docked)) in open.split('\n').zip(wide.split('\n')).enumerate().skip(1) {
        let tail: String = narrow.chars().skip(58).collect();
        let want: String = docked.chars().skip(128).collect();
        assert_eq!(tail, want, "row {row}");
    }
    assert_ne!(open, closed);
    let theme = Catalog::bundled().load("anysola", ThemeMode::Dark).unwrap();
    let mut buffer = Buffer::empty(Rect::new(0, 0, 100, 40));
    anysola_tui::view::draw(&state, &theme, &mut buffer);
    assert_ne!(buffer[(5, 10)].bg, theme.fg("background.base"));
    let mut tiny = session(30, 20, turn(true, true));
    tiny.sidebar_open = true;
    assert!(render(&tiny).contains("Fix typo"));
}

/// `routes/session/index.tsx:2967-3137`: a backgrounded shell shows its command with a spinner
/// while it runs and the ` Background ` badge; its output appears only once the row is expanded.
#[test]
fn background_shell_row() {
    let mut shell = tool(
        "shell",
        json!({"command": "npm run dev"}),
        Status::Completed,
    );
    shell.metadata = json!({"status": "running", "shellID": "sh_1"})
        .as_object()
        .unwrap()
        .clone();
    let assistant = Assistant {
        parts: vec![Part::Tool(shell)],
        ..Assistant::default()
    };
    let mut state = session(100, 30, vec![Item::Assistant(assistant)]);
    state.settings.animations = false;
    let s = state.sessions.get_mut("ses_b").unwrap();
    s.shells.insert(
        "sh_1".into(),
        anysola_tui::transcript::rows::BackgroundView {
            running: true,
            output: "ready on :3000\n".into(),
            ..Default::default()
        },
    );
    let collapsed = render(&state);
    assert!(collapsed.contains("⋯ npm run dev"));
    assert!(collapsed.contains(" Background "));
    assert!(!collapsed.contains("ready on :3000"));
    let s = state.sessions.get_mut("ses_b").unwrap();
    s.toggled.insert("shell:call_shell".into());
    assert!(render(&state).contains("ready on :3000"));
    let s = state.sessions.get_mut("ses_b").unwrap();
    s.shells.get_mut("sh_1").unwrap().running = false;
    s.shells.get_mut("sh_1").unwrap().truncated = true;
    let done = render(&state);
    assert!(done.contains("$ npm run dev") && done.contains("[earlier output omitted]"));
    let mut task = tool(
        "subagent",
        json!({"description": "scan", "subagent_type": "explore"}),
        Status::Completed,
    );
    task.metadata = json!({"status": "running"}).as_object().unwrap().clone();
    let assistant = Assistant {
        parts: vec![Part::Tool(task)],
        ..Assistant::default()
    };
    let screen = render(&session(100, 20, vec![Item::Assistant(assistant)]));
    assert!(
        screen.contains("✓ Explore Subagent — scan  Background "),
        "{screen}"
    );
}

/// `routes/session/index.tsx:147-148`, `:1560-1563`: only the newest entries are laid out; the
/// status row says `Loading session history…` while `session.first` lays out the rest.
#[test]
fn tail_first_transcript() {
    let items: Vec<Item> = (0..100)
        .map(|n| Item::User {
            id: format!("m{n}"),
            text: format!("message {n}"),
            agent: "build".into(),
            files: Vec::new(),
            images: Vec::new(),
            created: now(),
        })
        .collect();
    let floor = anysola_tui::transcript::rows::floor_before(&items, items.len(), 40);
    assert_eq!(floor, 60);
    assert_eq!(
        anysola_tui::transcript::rows::floor_before(&items, floor, 60),
        0
    );
    let mut state = session(100, 30, items);
    let s = state.sessions.get_mut("ses_b").unwrap();
    s.floor = floor;
    let full = {
        let s = state.sessions.get("ses_b").unwrap().clone();
        let mut whole = s.clone();
        whole.floor = 0;
        let theme = Catalog::bundled().load("anysola", ThemeMode::Dark).unwrap();
        (
            anysola_tui::view::transcript_rows(&state, &s, &theme, 96),
            anysola_tui::view::transcript_rows(&state, &whole, &theme, 96),
        )
    };
    assert!(full.0 * 2 < full.1, "{full:?}");
    state.sessions.get_mut("ses_b").unwrap().history_loading = true;
    assert!(render(&state).contains("Loading session history…"));
}

/// `routes/session/index.tsx:1724-1863`: with `debug.turn_tokens` each finished turn gets a
/// token summary; a click opens the per-step table with cache-bust warnings.
#[test]
fn turn_token_table() {
    use anysola_core::session::message::{Cache, Tokens};
    use anysola_tui::transcript::model::StepUsage;
    let step = |id: &str, read: u64, finish: &str, terminal: bool| {
        Item::Assistant(Assistant {
            id: id.into(),
            agent: "build".into(),
            model: "Claude Sonnet 5.5".into(),
            created: now(),
            turn_start: now(),
            completed: Some(now() + 1000),
            terminal,
            step: Some(Box::new(StepUsage {
                tokens: Tokens {
                    input: 1000,
                    output: 200,
                    reasoning: 0,
                    cache: Cache { read, write: 0 },
                },
                finish: finish.into(),
                model: "Claude Sonnet 5.5".into(),
            })),
            parts: vec![Part::Text {
                text: "ok".into(),
                done: true,
            }],
            ..Assistant::default()
        })
    };
    let items = vec![
        step("a1", 12000, "tool-calls", false),
        step("a2", 9000, "stop", true),
    ];
    let mut state = session(110, 40, items);
    let off = render(&state);
    assert!(!off.contains("Tokens:"));
    state.settings.turn_tokens = anysola_tui::state::TurnTokens::On;
    let screen = render(&state);
    assert!(
        screen.contains("+ Tokens: 2 steps · latest: 1,200 new · 9,000 cached · 10,200 total · ! 1 likely cache bust"),
        "{screen}"
    );
    state
        .sessions
        .get_mut("ses_b")
        .unwrap()
        .toggled
        .insert("tokens:a2".into());
    let screen = render(&state);
    for text in [
        "- Tokens: 2 steps",
        "Step         New  Cached   Total",
        "tool-call  1,200  12,000  13,200",
        "stop       1,200   9,000  10,200",
        "! Likely cache bust: 3,000 fewer cached tokens than the previous step",
    ] {
        assert!(screen.contains(text), "{text}\n{screen}");
    }
}

/// `component/session-tabs.tsx:542-1273`, `ui/layout.ts`: `tabs.layout = vertical` puts the tabs
/// in a 42-column rail (number or status, title, project) with the routes beside it.
#[test]
fn vertical_tabs() {
    use anysola_tui::view::{vertical_tab_at, vertical_tabs_width};
    assert_eq!(vertical_tabs_width(170, 42), Some(42));
    assert_eq!(vertical_tabs_width(105, 42), None);
    assert_eq!(vertical_tabs_width(150, 90), Some(72));
    let mut state = session(170, 30, turn(true, true));
    state.settings.tabs_vertical = true;
    state.settings.tab_numbers = true;
    state.left = 42;
    state.width = 128;
    let screen = render_full(&state, 170);
    let rows: Vec<&str> = screen.split('\n').collect();
    let column = |y: usize| rows[y].chars().take(42).collect::<String>();
    assert!(column(1).starts_with(" 1 "), "{}", column(1));
    assert!(
        column(4).starts_with(" 2 Fix typo in hello.txt"),
        "{}",
        column(4)
    );
    assert!(column(5).contains("my-app"), "{}", column(5));
    assert!(column(7).starts_with(" + New session"), "{}", column(7));
    assert_eq!(vertical_tab_at(&state, 4), Some(Some(1)));
    assert_eq!(vertical_tab_at(&state, 6), None);
    assert_eq!(vertical_tab_at(&state, 7), Some(None));
}

fn render_full(state: &State, width: u16) -> String {
    let theme = Catalog::bundled().load("anysola", ThemeMode::Dark).unwrap();
    let mut buffer = Buffer::empty(Rect::new(0, 0, width, state.height));
    anysola_tui::view::draw(state, &theme, &mut buffer);
    (0..state.height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The `/` popup in a session sits on top of the transcript: the conversation behind it must not
/// be drawn over the popup's rows (the transcript was drawn after the popup).
#[test]
fn slash_popup_over_transcript() {
    let mut state = session(100, 30, turn(true, true));
    state.composer.insert("/");
    state.popup = Some(Popup {
        trigger: '/',
        start: 0,
        rows: slash_rows(),
        selected: 0,
        offset: 0,
        pad: 12,
        area: Default::default(),
    });
    let screen = render(&state);
    for row in slash_rows() {
        let line = screen
            .lines()
            .find(|l| l.contains(&format!("{:<12}", row.display)))
            .unwrap_or_else(|| panic!("{} row missing:\n{screen}", row.display));
        let start = row.description.chars().take(20).collect::<String>();
        assert!(
            line.contains(&start),
            "transcript drawn over popup row {}:\n{screen}",
            row.display
        );
        assert_eq!(line.trim_start().chars().next(), Some('┃'), "{line}");
    }
}
