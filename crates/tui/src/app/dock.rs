//! The composer dock: Subagents (the session family), Shell (background commands) and
//! Terminals tabs in place of the prompt, plus parent/child session navigation.

use super::App;
use crate::dialog::{Body, BodyKind, Dialog, Tone, line};
use crate::keymap::Stroke;
use crate::state::{Dock, DockRow, DockTab, Route, Variant};
use crate::text::title_case;

impl App {
    /// Opens the dock on the Subagents tab (`down` on an empty prompt, `session.child.first`).
    pub(super) fn open_dock(&mut self) {
        if !matches!(self.state.route, Route::Session(_)) {
            return;
        }
        let mut tabs = vec![DockTab::Subagents, DockTab::Shell];
        if cfg!(any(unix, windows)) {
            tabs.push(DockTab::Terminals);
        }
        self.state.dock = Some(Dock {
            tabs,
            tab: 0,
            rows: Vec::new(),
            selected: 0,
            inactive: false,
        });
        self.refresh_dock();
    }

    pub(super) fn refresh_dock(&mut self) {
        let Some(session) = self.state.session().map(|s| s.id.clone()) else {
            self.state.dock = None;
            return;
        };
        let Some(dock) = self.state.dock.as_ref() else {
            return;
        };
        let (tab, inactive) = (dock.current(), dock.inactive);
        let rows = match tab {
            DockTab::Subagents => self.family(&session, inactive),
            DockTab::Shell => self
                .core
                .background_shells(&session)
                .into_iter()
                .map(|shell| DockRow {
                    label: shell.command.lines().next().unwrap_or_default().to_string(),
                    right: String::new(),
                    value: shell.id,
                })
                .collect(),
            DockTab::Terminals => {
                let mut rows: Vec<DockRow> = self
                    .terminals
                    .iter()
                    .enumerate()
                    .map(|(i, _)| DockRow {
                        label: format!("Terminal {}", i + 1),
                        right: if self.shown == Some(i) {
                            "shown".into()
                        } else {
                            String::new()
                        },
                        value: i.to_string(),
                    })
                    .collect();
                rows.push(DockRow {
                    label: "+ New terminal".into(),
                    right: String::new(),
                    value: "new".into(),
                });
                rows
            }
        };
        if let Some(dock) = self.state.dock.as_mut() {
            dock.selected = dock.selected.min(rows.len().saturating_sub(1));
            dock.rows = rows;
        }
    }

    /// The session's descendants as a tree: `├─ Explore: title` with `Running` at right.
    fn family(&self, session: &str, inactive: bool) -> Vec<DockRow> {
        let mut rows = Vec::new();
        let mut stack: Vec<(String, String)> = vec![(session.to_string(), String::new())];
        while let Some((parent, indent)) = stack.pop() {
            let children = self.core.children(&parent).unwrap_or_default();
            let count = children.len();
            let mut next = Vec::new();
            for (i, (id, title, agent)) in children.into_iter().enumerate() {
                let last = i + 1 == count;
                let running = self.core.child_running(&id);
                let branch = if last { "└─ " } else { "├─ " };
                if running != inactive {
                    let agent = title_case(agent.as_deref().unwrap_or("subagent"));
                    rows.push(DockRow {
                        label: format!("{indent}{branch}{agent}: {title}"),
                        right: if running {
                            "Running".into()
                        } else {
                            String::new()
                        },
                        value: id.clone(),
                    });
                }
                next.push((id, format!("{indent}{}", if last { "   " } else { "│  " })));
            }
            stack.extend(next.into_iter().rev());
        }
        rows
    }

    /// Keys while the dock is open; true when the dock took the key.
    pub(super) fn dock_key(&mut self, stroke: &Stroke) -> bool {
        let Some(dock) = self.state.dock.as_mut() else {
            return false;
        };
        let count = dock.rows.len();
        match (stroke.key.as_str(), stroke.ctrl) {
            ("escape", false) | ("c", true) => self.close_dock(),
            ("left", false) if dock.tabs.len() > 1 => {
                dock.tab = (dock.tab + dock.tabs.len() - 1) % dock.tabs.len();
                dock.selected = 0;
                self.refresh_dock();
            }
            ("right", false) if dock.tabs.len() > 1 => {
                dock.tab = (dock.tab + 1) % dock.tabs.len();
                dock.selected = 0;
                self.refresh_dock();
            }
            ("up", false) | ("k", false) if dock.selected == 0 => self.close_dock(),
            ("up", false) | ("k", false) => dock.selected -= 1,
            ("down", false) | ("j", false) => {
                dock.selected = (dock.selected + 1).min(count.saturating_sub(1))
            }
            ("a", true) if dock.current() == DockTab::Subagents => {
                dock.inactive = !dock.inactive;
                dock.selected = 0;
                self.refresh_dock();
            }
            ("d", true) => self.dock_stop(),
            ("return", false) => self.dock_choose(),
            _ => return true,
        }
        true
    }

    /// `esc`: closes the dock; in a child session it returns to the parent.
    fn close_dock(&mut self) {
        self.state.dock = None;
        self.go_parent();
    }

    fn dock_stop(&mut self) {
        let Some(dock) = self.state.dock.as_ref() else {
            return;
        };
        let Some(row) = dock.rows.get(dock.selected).cloned() else {
            return;
        };
        match dock.current() {
            DockTab::Subagents => self.core.interrupt(&row.value),
            DockTab::Shell => {
                self.core.kill_shell(&row.value);
            }
            DockTab::Terminals => return,
        }
        self.refresh_dock();
    }

    fn dock_choose(&mut self) {
        let Some(dock) = self.state.dock.as_ref() else {
            return;
        };
        let Some(row) = dock.rows.get(dock.selected).cloned() else {
            return;
        };
        match dock.current() {
            DockTab::Subagents => {
                self.state.dock = None;
                self.open_session(&row.value);
            }
            DockTab::Shell => self.shell_output(&row),
            DockTab::Terminals => {
                self.state.dock = None;
                match row.value.parse::<usize>() {
                    Ok(index) => self.show_terminal(index),
                    Err(_) => {
                        self.pane_command("session.terminal");
                    }
                }
            }
        }
    }

    /// The shell output dialog: the command and the tail of what it wrote.
    fn shell_output(&mut self, row: &DockRow) {
        let Some(session) = self.state.session().map(|s| s.id.clone()) else {
            return;
        };
        let Some(shell) = self
            .core
            .background_shells(&session)
            .into_iter()
            .find(|s| s.id == row.value)
        else {
            return self.toast(None, "The command has ended".into(), Variant::Info);
        };
        let text = std::fs::read_to_string(&shell.file).unwrap_or_default();
        let mut lines = vec![
            line(format!("$ {}", shell.command), Tone::Muted),
            Vec::new(),
        ];
        let tail: Vec<&str> = text.lines().collect();
        let skip = tail.len().saturating_sub(200);
        lines.extend(tail[skip..].iter().map(|l| line(*l, Tone::Base)));
        let mut body = Body::new(BodyKind::Status, "Shell output", lines);
        body.size = crate::dialog::Size::Large;
        self.push(Dialog::Body(body));
    }

    /// `session.parent`: from a child session to its parent.
    pub(super) fn go_parent(&mut self) {
        let parent = self
            .state
            .session()
            .and_then(|s| self.core.session(&s.id).ok().flatten())
            .and_then(|row| row.parent_id);
        if let Some(parent) = parent {
            self.open_session(&parent);
        }
    }

    /// `session.child.next` / `previous`: the sibling after or before this child session.
    pub(super) fn go_sibling(&mut self, step: isize) {
        let Some(current) = self.state.session().map(|s| s.id.clone()) else {
            return;
        };
        let Some(parent) = self
            .core
            .session(&current)
            .ok()
            .flatten()
            .and_then(|row| row.parent_id)
        else {
            return;
        };
        let siblings = self.core.children(&parent).unwrap_or_default();
        let Some(at) = siblings.iter().position(|(id, ..)| *id == current) else {
            return;
        };
        let count = siblings.len() as isize;
        let next = (at as isize + step).rem_euclid(count) as usize;
        let id = siblings[next].0.clone();
        self.open_session(&id);
    }
}
