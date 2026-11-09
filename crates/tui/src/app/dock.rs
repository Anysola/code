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
            (key, false) if !stroke.alt && key.chars().count() == 1 => {
                // Typing goes back to the prompt; a child session has none, so it is ignored.
                if self.in_child() {
                    return true;
                }
                self.state.dock = None;
                return false;
            }
            _ => return false,
        }
        true
    }

    pub(super) fn in_child(&self) -> bool {
        self.state
            .session()
            .and_then(|s| self.core.session(&s.id).ok().flatten())
            .is_some_and(|row| row.parent_id.is_some())
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

    /// The shell output dialog (`component/dialog-shell-output.tsx`).
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
        let mut body = Body::new(
            BodyKind::ShellOutput {
                id: shell.id,
                command: shell.command,
                file: shell.file,
                polled: 0,
                ended: false,
            },
            "Shell output",
            Vec::new(),
        );
        body.size = crate::dialog::Size::XLarge;
        body.centered = true;
        body.actions = vec![
            crate::dialog::Action::right("scroll", "↑/↓"),
            crate::dialog::Action::right("follow", "end"),
        ];
        self.push(Dialog::Body(body));
        self.poll_shell_output();
    }

    /// Once a second: which of the session's background shells still run, and the output of
    /// those whose rows are expanded (the last 1 MiB, `routes/session/index.tsx:3016-3076`).
    pub(super) fn poll_shells(&mut self) {
        use crate::transcript::model::{Item, Part};
        let now = super::now();
        if now < self.shells_polled + 1000 {
            return;
        }
        self.shells_polled = now;
        let Some(session) = self.state.session() else {
            return;
        };
        let live = self.core.background_shells(&session.id);
        let calls: Vec<(String, String)> = session
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Assistant(a) => Some(&a.parts),
                _ => None,
            })
            .flatten()
            .filter_map(|p| match p {
                Part::Tool(t) => {
                    crate::transcript::shell_id(t).map(|id| (id.to_string(), t.call_id.clone()))
                }
                _ => None,
            })
            .collect();
        let Some(session) = self.state.session_mut() else {
            return;
        };
        // Rows whose call left the transcript stop counting as running too.
        for view in session.shells.values_mut() {
            view.running = false;
        }
        for (id, call) in calls {
            let shell = live.iter().find(|s| s.id == id);
            let view = session.shells.entry(id).or_default();
            view.running = shell.is_some();
            if let Some(shell) = shell {
                view.file = Some(shell.file.clone());
            }
            if !session.toggled.contains(&format!("shell:{call}")) {
                continue;
            }
            const LIMIT: u64 = 1024 * 1024;
            if let Some((bytes, truncated)) = view.file.as_ref().and_then(|f| read_tail(f, LIMIT)) {
                view.truncated = truncated;
                view.output = String::from_utf8_lossy(&bytes).into_owned();
            }
        }
    }

    /// Re-reads the open Shell output dialog's file once a second, keeping the view at the end
    /// while it was there.
    pub(super) fn poll_shell_output(&mut self) {
        let height = self.state.height;
        let now = super::now();
        let Some(Dialog::Body(b)) = self.state.dialogs.last_mut() else {
            return;
        };
        let BodyKind::ShellOutput {
            id,
            command,
            file,
            polled,
            ended,
        } = &mut b.kind
        else {
            return;
        };
        if now < *polled + 1000 {
            return;
        }
        *polled = now;
        let end = self.core.shell_end(id);
        *ended = end.is_some();
        // Only the last part is shown; a multi-GB log must not be read whole every second.
        let bytes = read_tail(file.as_path(), 1024 * 1024)
            .map(|(bytes, _)| bytes)
            .ok_or_else(|| std::io::Error::other("unreadable"));
        let room = crate::dialog::Size::XLarge.width().saturating_sub(6) as usize;
        let (status, lines) = shell_output_lines(command, end, bytes, room);
        let rows = (height.saturating_sub(10) as usize).min(lines.len());
        let following = b.scroll + rows >= b.lines.len();
        b.title = vec![
            ("Shell output".into(), Tone::Bold),
            (format!("  {status}"), Tone::Muted),
        ];
        b.lines = lines;
        if following {
            b.scroll = b.lines.len().saturating_sub(rows);
        }
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

    /// `N subagent(s) · N shell(s)`: running sessions of the whole family but this one, and
    /// this session's background shells.
    pub(super) fn live(&self) -> Option<String> {
        let current = self.state.session()?.id.clone();
        let mut root = current.clone();
        while let Some(parent) = self
            .core
            .session(&root)
            .ok()
            .flatten()
            .and_then(|row| row.parent_id)
        {
            root = parent;
        }
        let mut running = 0;
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            for (child, ..) in self.core.children(&id).unwrap_or_default() {
                if child != current && self.core.child_running(&child) {
                    running += 1;
                }
                stack.push(child);
            }
        }
        let shells = self.core.background_shells(&current).len();
        let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
        let parts: Vec<String> = [(running, "subagent"), (shells, "shell")]
            .into_iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, word)| plural(n, word))
            .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

const PAGE_BYTES: usize = 64 * 1024;

/// The Shell output dialog's status word and body: the command (at most 3 rows), the omitted
/// notice, then the last 64 KiB of output or the loading/empty/missing text.
pub fn shell_output_lines(
    command: &str,
    end: Option<anysola_core::tool::shell::ShellEnd>,
    output: std::io::Result<Vec<u8>>,
    room: usize,
) -> (String, Vec<crate::dialog::Line>) {
    use anysola_core::tool::shell::ShellEnd;
    let status = match end {
        None => "Running".to_string(),
        Some(ShellEnd::TimedOut) => "Timed out".into(),
        Some(ShellEnd::Killed) => "Killed".into(),
        Some(ShellEnd::Exited(None)) => "Exited".into(),
        Some(ShellEnd::Exited(Some(code))) => format!("Exited · code {code}"),
    };
    let mut lines: Vec<crate::dialog::Line> = crate::text::wrap(command, room)
        .into_iter()
        .take(3)
        .map(|l| line(l, Tone::Muted))
        .collect();
    lines.push(Vec::new());
    match output {
        Ok(bytes) => {
            let omitted = bytes.len() > PAGE_BYTES;
            let tail = String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(PAGE_BYTES)..]);
            let text = crate::text::strip_ansi(&tail)
                .replace("\r\n", "\n")
                .replace('\r', "\n");
            if omitted {
                lines.push(line(
                    "Earlier output omitted · showing recent output",
                    Tone::Muted,
                ));
                lines.push(Vec::new());
            }
            if text.is_empty() {
                lines.push(line(
                    "No captured output. Output redirected to files is not shown here.",
                    Tone::Base,
                ));
                return (status, lines);
            }
            for row in text.trim_end_matches('\n').split('\n') {
                lines.extend(
                    crate::text::wrap(row, room)
                        .into_iter()
                        .map(|l| line(l, Tone::Base)),
                );
            }
        }
        Err(_) => lines.push(line("Shell output is no longer available.", Tone::Error)),
    }
    (status, lines)
}

/// The last `limit` bytes of a file and whether anything was left out.
fn read_tail(path: &std::path::Path, limit: u64) -> Option<(Vec<u8>, bool)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let truncated = len > limit;
    if truncated {
        file.seek(SeekFrom::Start(len - limit)).ok()?;
    }
    let mut bytes = Vec::with_capacity(len.min(limit) as usize);
    file.read_to_end(&mut bytes).ok()?;
    Some((bytes, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anysola_core::tool::shell::ShellEnd;

    fn text(lines: &[crate::dialog::Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.iter().map(|(t, _)| t.as_str()).collect())
            .collect()
    }

    #[test]
    fn shell_output_texts() {
        let (status, lines) = shell_output_lines("make", None, Ok(b"a\r\nb\n".to_vec()), 40);
        assert_eq!(status, "Running");
        assert_eq!(text(&lines), ["make", "", "a", "b"]);
        let (status, lines) =
            shell_output_lines("x", Some(ShellEnd::Exited(Some(2))), Ok(Vec::new()), 40);
        assert_eq!(status, "Exited · code 2");
        assert_eq!(
            text(&lines)[2],
            "No captured output. Output redirected to files is not shown here."
        );
        let big = vec![b'z'; PAGE_BYTES + 10];
        let (status, lines) = shell_output_lines("x", Some(ShellEnd::Killed), Ok(big), 200);
        assert_eq!(status, "Killed");
        assert_eq!(
            text(&lines)[2],
            "Earlier output omitted · showing recent output"
        );
        let missing = Err(std::io::Error::other("gone"));
        let (status, lines) = shell_output_lines("x", Some(ShellEnd::TimedOut), missing, 40);
        assert_eq!(
            (status.as_str(), text(&lines)[2].as_str()),
            ("Timed out", "Shell output is no longer available.")
        );
    }
}
