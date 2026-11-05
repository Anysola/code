//! Text snapshots of the TUI against the 0.5.1 captures in `spec/verbatim/tui/screens/`.

use std::path::PathBuf;

use anysola_core::events::CreditsSnapshot;
use anysola_tui::composer::{AtomKind, Mode};
use anysola_tui::state::{
    Account, Hints, ModelChoice, Permission, Popup, PopupAction, PopupRow, Recent, Route, Session,
    State, Tab,
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
        free: None,
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
