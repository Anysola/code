//! Every control the screen draws answers a click: the drawn label's cells map to its hit.

use anysola_tui::dialog::{Confirm, ConfirmKind, Dialog, Export};
use anysola_tui::menus::palette;
use anysola_tui::state::{Permission, Route, Session, State, Tab, Toast, Variant};
use anysola_tui::theme::{Catalog, Mode as ThemeMode};
use anysola_tui::transcript::model::Item;
use anysola_tui::view::{Drawn, Hit, hit_at};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use serde_json::json;

fn draw(state: &State) -> (Buffer, Drawn) {
    let theme = Catalog::bundled().load("anysola", ThemeMode::Dark).unwrap();
    let mut buffer = Buffer::empty(Rect::new(0, 0, state.width, state.height));
    let drawn = anysola_tui::view::draw(state, &theme, &mut buffer);
    (buffer, drawn)
}

/// The cell where `text` first appears on screen (searching from the bottom when `last`).
fn find(buffer: &Buffer, text: &str, last: bool) -> (u16, u16) {
    let area = buffer.area;
    let rows: Vec<u16> = if last {
        (0..area.height).rev().collect()
    } else {
        (0..area.height).collect()
    };
    for y in rows {
        let cells: Vec<&str> = (0..area.width).map(|x| buffer[(x, y)].symbol()).collect();
        let line: String = cells.concat();
        if let Some(byte) = line.find(text) {
            // Back from the byte offset to a column: count the cells before it.
            let mut seen = 0;
            for (x, cell) in cells.iter().enumerate() {
                if seen >= byte {
                    return (x as u16, y);
                }
                seen += cell.len();
            }
        }
    }
    panic!("{text:?} is not on screen");
}

/// Asserts a click anywhere on `text` hits `want`.
#[track_caller]
fn assert_click(state: &State, text: &str, want: Hit) {
    assert_click_at(state, text, false, want);
}

#[track_caller]
fn assert_click_at(state: &State, text: &str, last: bool, want: Hit) {
    let (buffer, drawn) = draw(state);
    let (x, y) = find(&buffer, text, last);
    let w = anysola_tui::text::width(text) as u16;
    for at in [x, x + w - 1] {
        assert_eq!(
            hit_at(&drawn.hits, (at, y)),
            Some(&want),
            "click on {text:?} at ({at}, {y})"
        );
    }
}

fn base(width: u16, height: u16) -> State {
    let mut state = State::new("0.5.1");
    state.width = width;
    state.height = height;
    state.location = "~/my-app:master".into();
    for (id, title) in [("ses_a", "First tab"), ("ses_b", "Second tab")] {
        state.tabs.push(Tab {
            session_id: id.into(),
            title: title.into(),
            ..Tab::default()
        });
    }
    state
}

fn session(width: u16, height: u16, items: Vec<Item>) -> State {
    let mut state = base(width, height);
    state.route = Route::Session("ses_b".into());
    state.sessions.insert(
        "ses_b".into(),
        Session {
            id: "ses_b".into(),
            title: "Second tab".into(),
            directory: "/home/user/my-app".into(),
            items,
            ..Session::default()
        },
    );
    state
}

#[test]
fn horizontal_tabs_and_new_session() {
    let state = session(120, 30, Vec::new());
    assert_click(&state, "First tab", Hit::Tab(0));
    assert_click(&state, "Second tab", Hit::Tab(1));
    let (buffer, drawn) = draw(&state);
    let (x, y) = find(&buffer, "+", false);
    assert_eq!((y, hit_at(&drawn.hits, (x, y))), (0, Some(&Hit::NewTab)));

    let home = base(120, 30);
    assert_click(&home, "First tab", Hit::Tab(0));
    assert_click(&home, "+ New session", Hit::NewTab);
}

#[test]
fn list_dialog_options_close_and_backdrop() {
    let mut state = base(120, 36);
    let keymap = anysola_tui::keymap::Keymap::defaults();
    let list = palette::dialog(&keymap, &palette::Context::default());
    let first = list.options[list.matches()[0]].title.clone();
    let second = list.options[list.matches()[1]].title.clone();
    state.dialogs.push(Dialog::Select(list.clone()));
    assert_click(&state, &first, Hit::Option(list.matches()[0]));
    assert_click(&state, &second, Hit::Option(list.matches()[1]));
    assert_click_at(&state, "esc", false, Hit::Key("esc".into()));
    // The tab strip behind the backdrop is inert; outside the panel nothing is hit.
    let (_, drawn) = draw(&state);
    assert_eq!(hit_at(&drawn.hits, (2, 0)), None);
    assert_eq!(hit_at(&drawn.hits, (0, 20)), None);
}

#[test]
fn confirm_buttons() {
    let mut state = base(120, 36);
    state.dialogs.push(Dialog::Confirm(Confirm {
        kind: ConfirmKind::Revoke { id: "tok_1".into() },
        title: "Revoke device".into(),
        message: "Sign this device out?".into(),
        button: "Revoke".into(),
        yes: false,
    }));
    assert_click(&state, "Cancel", Hit::Button(0));
    assert_click_at(&state, "Revoke", true, Hit::Button(1));
}

#[test]
fn export_options() {
    let mut state = base(120, 36);
    state.dialogs.push(Dialog::Export(Export {
        json: false,
        thinking: true,
        tools: true,
        sanitize: false,
        focus: 0,
    }));
    assert_click(&state, "◉ Markdown", Hit::Format(false));
    assert_click(&state, "○ JSON", Hit::Format(true));
    assert_click(&state, "[x] Include thinking", Hit::Field(1));
    assert_click(&state, "[x] Include tools", Hit::Field(2));
    assert_click(&state, "Copy", Hit::Button(0));
    assert_click_at(&state, "Export", true, Hit::Button(1));
}

#[test]
fn permission_options_and_confirm() {
    let mut state = session(120, 40, Vec::new());
    let s = state.sessions.get_mut("ses_b").unwrap();
    s.permission = Some(Permission {
        request_id: "per_1".into(),
        action: "shell".into(),
        resources: vec!["ls".into()],
        save: vec!["ls *".into()],
        metadata: json!(null),
        subject: Some("ls".into()),
        selected: 0,
        fullscreen: false,
        reject: None,
    });
    assert_click(&state, "Allow once", Hit::Choice(0));
    assert_click(&state, "Always allow", Hit::Choice(1));
    assert_click(&state, "Reject", Hit::Choice(2));
    assert_click_at(&state, "enter confirm", true, Hit::Key("enter".into()));
}

#[test]
fn toast_without_action_closes() {
    let mut state = session(120, 30, Vec::new());
    state.toast = Some(Toast {
        title: None,
        message: "Copied to clipboard".into(),
        variant: Variant::Info,
        until: u64::MAX,
        action: None,
    });
    assert_click(&state, "Copied to clipboard", Hit::CloseToast);
}

#[test]
fn jump_to_latest() {
    let items = (0..80)
        .map(|i| Item::Notice(format!("notice {i}")))
        .collect();
    let mut state = session(120, 30, items);
    state.sessions.get_mut("ses_b").unwrap().scroll = 10;
    assert_click(
        &state,
        "Jump to latest ↓",
        Hit::Command("session.last".into()),
    );
}
