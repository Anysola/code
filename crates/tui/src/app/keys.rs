//! Terminal input: keys through the keymap layers (leader, prompts, popup, global commands,
//! composer editing), pastes and the mouse.

use anysola_core::session::gate::Reply;
use anysola_core::tool::question::QuestionReply;
use crossterm::event::{Event, KeyEventKind, MouseButton, MouseEventKind};

use super::{App, now, selected_text};
use crate::composer::{AtomKind, Mode};
use crate::keymap::Stroke;
use crate::state::{PopupAction, Route, Variant};
use crate::term;
use crate::view::Hit;

/// Commands reachable from anywhere, checked in this order.
const GLOBAL: &[&str] = &[
    "command.palette.show",
    "session.list",
    "session.new",
    "open.menu",
    "model.list",
    "agent.list",
    "agent.cycle",
    "agent.cycle.reverse",
    "variant.cycle",
    "model.cycle_recent",
    "model.cycle_recent_reverse",
    "model.cycle_favorite",
    "model.cycle_favorite_reverse",
    "session.background",
    "session.compact",
    "session.undo",
    "session.redo",
    "session.timeline",
    "session.export",
    "session.rename",
    "session.sidebar.toggle",
    "messages.copy",
    "opencode.status",
    "prompt.editor",
    "prompt.queue",
    "prompt.images.view",
    "session.page.up",
    "session.page.down",
    "session.half.page.up",
    "session.half.page.down",
    "session.line.up",
    "session.line.down",
    "session.first",
    "session.last",
    "terminal.suspend",
    "terminal.toggle",
    "terminal.select",
    "terminal.close",
    "pane.focus.left",
    "pane.focus.right",
    "session.tab.close",
    "session.tab.reopen",
    "session.message.next",
    "session.message.previous",
    "session.message.user.next",
    "session.message.user.previous",
    "session.messages_last_user",
    "session.tab.next",
    "session.tab.previous",
    "session.tab.next_unread",
    "session.tab.previous_unread",
    "session.tab.select.1",
    "session.tab.select.2",
    "session.tab.select.3",
    "session.tab.select.4",
    "session.tab.select.5",
    "session.tab.select.6",
    "session.tab.select.7",
    "session.tab.select.8",
    "session.tab.select.9",
    "session.tab.select.10",
];

/// Composer editing commands (the textarea layer).
const EDITING: &[&str] = &[
    "input.newline",
    "input.move.left",
    "input.move.right",
    "input.line.home",
    "input.line.end",
    "input.buffer.home",
    "input.buffer.end",
    "input.delete.line",
    "input.delete.to.line.end",
    "input.delete.to.line.start",
    "input.backspace",
    "input.delete",
    "input.undo",
    "input.redo",
    "input.word.forward",
    "input.word.backward",
    "input.delete.word.forward",
    "input.delete.word.backward",
    "prompt.history.previous",
    "prompt.history.next",
    "input.move.up",
    "input.move.down",
    "input.submit",
];

impl App {
    pub(super) fn input(&mut self, event: Event) {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if let Some(stroke) = Stroke::from_event(&key) {
                    self.key(stroke);
                }
            }
            Event::Paste(text) if text.is_empty() => self.paste_clipboard(),
            Event::Paste(text) => {
                if !self.pane_paste(&text) {
                    self.paste(&text)
                }
            }
            Event::Mouse(mouse) => self.mouse(mouse),
            Event::FocusGained => self.focused = Some(true),
            Event::FocusLost => self.focused = Some(false),
            _ => {}
        }
    }

    fn is(&self, id: &str, stroke: &Stroke) -> bool {
        self.keymap
            .find(std::slice::from_ref(stroke), &[id])
            .is_some()
    }

    fn key(&mut self, stroke: Stroke) {
        if self.selection.take().is_some() && stroke.key == "escape" {
            return;
        }
        if !self.state.dialogs.is_empty() {
            return self.dialog_key(&stroke);
        }
        if self.state.diff.is_some() {
            return self.diff_key(&stroke);
        }
        let blocked = self
            .state
            .session()
            .is_some_and(|s| s.permission.is_some() || s.question.is_some());
        if crate::view::sidebar_overlay(&self.state)
            && !blocked
            && (stroke.key == "escape" || (stroke.ctrl && stroke.key == "c"))
        {
            return self.run_command("session.sidebar.toggle", None);
        }
        let mut strokes = std::mem::take(&mut self.pending);
        if now() > self.pending_until {
            strokes.clear();
        }
        strokes.push(stroke.clone());
        if self.keymap.is_leader(&strokes) {
            self.pending = strokes;
            self.pending_until = now() + self.keymap.leader_timeout_ms;
            return;
        }
        if strokes.len() > 1 {
            let mut ids: Vec<&str> = GLOBAL.to_vec();
            ids.push("app.exit");
            if let Some(id) = self.keymap.find(&strokes, &ids) {
                self.run_command(id, None);
            }
            return;
        }
        if self.pane_key(&stroke) {
            return;
        }
        if self.state.session().is_some_and(|s| s.permission.is_some()) {
            return self.permission_key(&stroke);
        }
        if self.state.session().is_some_and(|s| s.question.is_some()) {
            return self.question_key(&stroke);
        }
        if self.state.session().is_some_and(|s| s.location_missing()) {
            if stroke.key == "return" {
                self.run_command("session.move", None);
            } else if self.is("app.exit", &stroke) {
                self.quit = true;
            } else if let Some(id) = self.keymap.find(std::slice::from_ref(&stroke), GLOBAL) {
                self.run_command(id, None);
            }
            return;
        }
        if self.state.dock.is_some() && self.dock_key(&stroke) {
            return;
        }
        if self.state.popup.is_some() && self.popup_key(&stroke) {
            return;
        }
        let empty = self.state.composer.is_empty();
        if stroke.key == "escape" && !stroke.ctrl {
            return self.escape();
        }
        if !empty && self.is("prompt.clear", &stroke) {
            let text = self.state.composer.expanded();
            if text.trim().chars().count() >= 20 {
                self.history.push(crate::composer::Entry {
                    text,
                    shell: self.state.composer.mode == Mode::Shell,
                });
            }
            self.state.composer.clear();
            return self.refresh_popup();
        }
        if empty && self.state.composer.mode == Mode::Shell && stroke.ctrl && stroke.key == "c" {
            self.state.composer.mode = Mode::Normal;
            return;
        }
        if empty && self.is("app.exit", &stroke) {
            self.quit = true;
            return;
        }
        if let Some(id) = self.keymap.find(std::slice::from_ref(&stroke), GLOBAL) {
            return self.run_command(id, None);
        }
        if let Some(id) = self.keymap.find(std::slice::from_ref(&stroke), EDITING) {
            return self.edit(id);
        }
        let plain = !stroke.ctrl && !stroke.alt && !stroke.super_;
        if plain && (stroke.key.chars().count() == 1 || stroke.key == "space") {
            let mut text = if stroke.key == "space" {
                " ".to_string()
            } else {
                stroke.key.clone()
            };
            if stroke.shift {
                text = text.to_uppercase();
            }
            let composer = &mut self.state.composer;
            if text == "!"
                && composer.cursor == 0
                && composer.mode == Mode::Normal
                && self.state.popup.is_none()
            {
                composer.mode = Mode::Shell;
                self.state.shell_example = (now() / 13) as usize % 3;
                return;
            }
            composer.insert(&text);
            self.refresh_popup();
        }
    }

    fn escape(&mut self) {
        if self.state.composer.mode == Mode::Shell {
            self.state.composer.mode = Mode::Normal;
            return;
        }
        let Some(session) = self.state.session() else {
            return;
        };
        if !session.busy {
            return;
        }
        let id = session.id.clone();
        match self.state.interrupt_armed {
            Some(_) => {
                self.state.interrupt_armed = None;
                self.core.interrupt(&id);
            }
            None => self.state.interrupt_armed = Some(now()),
        }
    }

    fn edit(&mut self, id: &str) {
        let composer = &mut self.state.composer;
        match id {
            "input.newline" => composer.insert("\n"),
            "input.move.left" => composer.left(),
            "input.move.right" => composer.right(),
            "input.line.home" => composer.home(),
            "input.line.end" => composer.end(),
            "input.buffer.home" => composer.buffer_home(),
            "input.buffer.end" => composer.buffer_end(),
            "input.delete.line" => composer.delete_line(),
            "input.delete.to.line.end" => composer.delete_to_line_end(),
            "input.delete.to.line.start" => composer.delete_to_line_start(),
            "input.backspace" => {
                if composer.cursor == 0 && composer.mode == Mode::Shell {
                    composer.mode = Mode::Normal;
                } else {
                    composer.backspace();
                }
            }
            "input.delete" => composer.delete(),
            "input.undo" => composer.undo(),
            "input.redo" => composer.redo(),
            "input.word.forward" => composer.word_right(),
            "input.word.backward" => composer.word_left(),
            "input.delete.word.forward" => composer.delete_word_forward(),
            "input.delete.word.backward" => composer.delete_word_back(),
            "prompt.history.previous" | "input.move.up" => {
                if composer.cursor > 0 && composer.up() {
                } else if composer.cursor > 0 && !composer.text[..composer.cursor].contains('\n') {
                    composer.buffer_home();
                } else if let Some(entry) = self.history.previous(&composer.expanded()) {
                    composer.set(&entry.text);
                    composer.mode = if entry.shell {
                        Mode::Shell
                    } else {
                        Mode::Normal
                    };
                    composer.buffer_home();
                }
            }
            "prompt.history.next" | "input.move.down" => {
                if composer.down() {
                } else if composer.cursor < composer.text.len() {
                    composer.buffer_end();
                } else if let Some(entry) = self.history.next(&composer.expanded()) {
                    composer.set(&entry.text);
                    composer.mode = if entry.shell {
                        Mode::Shell
                    } else {
                        Mode::Normal
                    };
                } else if composer.is_empty() {
                    return self.open_dock();
                }
            }
            "input.submit" => return self.submit(false),
            _ => {}
        }
        self.refresh_popup();
    }

    fn permission_key(&mut self, stroke: &Stroke) {
        let exit = self.is("app.exit", stroke);
        // A child session asks why before rejecting.
        let child = self.in_child();
        let Some(session) = self.state.session_mut() else {
            return;
        };
        let Some(permission) = session.permission.as_mut() else {
            return;
        };
        if let Some(reason) = permission.reject.as_mut() {
            match stroke.key.as_str() {
                "return" => {
                    let text = reason.trim().to_string();
                    let id = permission.request_id.clone();
                    let reply = Reply::Reject((!text.is_empty()).then_some(text));
                    self.core.reply_permission(&id, reply);
                }
                "escape" => permission.reject = None,
                "backspace" => {
                    reason.pop();
                }
                "space" => reason.push(' '),
                key if key.chars().count() == 1 && !stroke.ctrl => {
                    reason.push_str(&if stroke.shift {
                        key.to_uppercase()
                    } else {
                        key.to_string()
                    });
                }
                _ => {}
            }
            return;
        }
        let count = permission.options().len();
        match stroke.key.as_str() {
            "left" | "h" => permission.selected = (permission.selected + count - 1) % count,
            "right" | "l" | "tab" => permission.selected = (permission.selected + 1) % count,
            "f" if stroke.ctrl => permission.fullscreen = !permission.fullscreen,
            "return" => {
                let reply = match permission.options()[permission.selected] {
                    "Allow once" => Reply::Once,
                    "Always allow" => Reply::Always,
                    _ if child => {
                        permission.reject = Some(String::new());
                        return;
                    }
                    _ => Reply::Reject(None),
                };
                let id = permission.request_id.clone();
                self.core.reply_permission(&id, reply);
            }
            _ if child && (stroke.key == "escape" || exit) => {
                permission.reject = Some(String::new());
            }
            _ if stroke.key == "escape" || exit => {
                let id = permission.request_id.clone();
                self.core.reply_permission(&id, Reply::Reject(None));
            }
            _ => {}
        }
    }

    fn question_key(&mut self, stroke: &Stroke) {
        let Some(session) = self.state.session_mut() else {
            return;
        };
        let Some(q) = session.question.as_mut() else {
            return;
        };
        if let Some(reply) = question_step(q, stroke) {
            let id = q.request_id.clone();
            answer(&self.core, q.form.as_deref(), &id, reply);
            session.question = None;
        }
    }

    /// Autocomplete keys; false when the key belongs to the composer.
    fn popup_key(&mut self, stroke: &Stroke) -> bool {
        let one = std::slice::from_ref(stroke);
        let ids = [
            "prompt.autocomplete.prev",
            "prompt.autocomplete.next",
            "prompt.autocomplete.hide",
            "prompt.autocomplete.select",
            "prompt.autocomplete.complete",
        ];
        let action = self.keymap.find(one, &ids);
        // Enter on a `/cd` row that is already typed submits instead of inserting it again.
        let typed = self.state.popup.as_ref().is_some_and(|p| {
            p.rows.get(p.selected).is_some_and(|r| {
                matches!(&r.action, PopupAction::Insert(v)
                    if self.state.composer.text.get(p.start..) == Some(v.as_str()))
            })
        });
        if typed && action == Some("prompt.autocomplete.select") {
            self.state.popup = None;
            return false;
        }
        let recent = self.state.popup.as_ref().is_some_and(|p| {
            p.rows
                .get(p.selected)
                .is_some_and(|r| r.description == "recent")
        });
        if recent && stroke.ctrl && stroke.key == "d" && action.is_none() {
            self.remove_recent();
            return true;
        }
        self.state.popup_confirm = false;
        let Some(popup) = self.state.popup.as_mut() else {
            return false;
        };
        let count = popup.rows.len().max(1);
        match action {
            Some("prompt.autocomplete.prev") => {
                popup.selected = (popup.selected + count - 1) % count
            }
            Some("prompt.autocomplete.next") => popup.selected = (popup.selected + 1) % count,
            Some("prompt.autocomplete.hide") => {
                self.popup_hidden = Some(self.state.composer.text.clone());
                self.state.popup = None;
                return true;
            }
            Some("prompt.autocomplete.select") => {
                self.choose(false);
                return true;
            }
            Some(_) => {
                self.choose(true);
                return true;
            }
            None if stroke.ctrl && stroke.key == "c" => {
                let start = popup.start;
                self.state.composer.replace_token(start, "");
                self.state.popup = None;
                return true;
            }
            None => return false,
        }
        if popup.selected < popup.offset {
            popup.offset = popup.selected;
        } else if popup.selected >= popup.offset + 10 {
            popup.offset = popup.selected + 1 - 10;
        }
        true
    }

    /// The popup row under the pointer.
    fn popup_row(&self, at: (u16, u16)) -> Option<usize> {
        let popup = self.state.popup.as_ref()?;
        let area = popup.area.get();
        let inside = area.contains(ratatui::layout::Position::new(at.0, at.1));
        let row = popup.offset + usize::from(at.1.wrapping_sub(area.y));
        (inside && row < popup.rows.len()).then_some(row)
    }

    fn hover_popup(&mut self, at: (u16, u16)) {
        if let Some(row) = self.popup_row(at)
            && let Some(popup) = self.state.popup.as_mut()
        {
            popup.selected = row;
        }
    }

    fn choose(&mut self, complete: bool) {
        let Some(popup) = self.state.popup.take() else {
            return;
        };
        let Some(row) = popup.rows.get(popup.selected).cloned() else {
            return;
        };
        let composer = &mut self.state.composer;
        match row.action {
            PopupAction::Command(id) => {
                composer.replace_token(popup.start, "");
                self.run_command(&id, None);
                return;
            }
            PopupAction::Insert(text) => composer.replace_token(popup.start, &text),
            PopupAction::Directory(text) if complete => composer.replace_token(popup.start, &text),
            PopupAction::Directory(text) => {
                composer.replace_token(popup.start, "");
                composer.insert_atom(&text, AtomKind::File);
            }
            PopupAction::Mention(kind, text) => {
                composer.replace_token(popup.start, "");
                composer.insert_atom(&text, kind);
            }
        }
        self.refresh_popup();
    }

    fn paste(&mut self, text: &str) {
        if let Some(session) = self.state.session_mut()
            && let Some(q) = session.question.as_mut()
            && let Some(custom) = q.custom.as_mut()
        {
            custom.push_str(text);
            return;
        }
        if self.attach(text) {
            return;
        }
        let compact = self.state.settings.paste_compact;
        self.state.composer.paste(text, compact);
        self.refresh_popup();
    }

    /// A pasted local image or PDF path becomes an `[Image N]`/`[PDF N]` chip.
    fn attach(&mut self, text: &str) -> bool {
        let trimmed = text.trim().trim_matches(['\'', '"']);
        // `file:///C:/My%20Docs/a.png` (Windows) and `file:///home/a%20b.png` decode to a path.
        let from_url = trimmed
            .starts_with("file://")
            .then(|| url::Url::parse(trimmed).ok()?.to_file_path().ok())
            .flatten();
        let path = match from_url {
            Some(path) => path.to_string_lossy().into_owned(),
            None => trimmed
                .strip_prefix("file://")
                .unwrap_or(trimmed)
                .replace("\\ ", " "),
        };
        let lower = path.to_lowercase();
        let mime = [
            (".png", "image/png"),
            (".jpg", "image/jpeg"),
            (".jpeg", "image/jpeg"),
            (".gif", "image/gif"),
            (".webp", "image/webp"),
            (".pdf", "application/pdf"),
        ]
        .iter()
        .find(|(ext, _)| lower.ends_with(ext))
        .map(|(_, mime)| *mime);
        let Some(mime) = mime else { return false };
        let Ok(bytes) = std::fs::read(&path) else {
            return false;
        };
        let name = std::path::Path::new(&path)
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        self.attach_bytes(mime, bytes, name);
        true
    }

    /// Adds an `[Image N]`/`[PDF N]` chip for the bytes (20 MiB limit, toast past it).
    fn attach_bytes(&mut self, mime: &str, bytes: Vec<u8>, name: String) {
        if bytes.len() > 20 * 1024 * 1024 {
            self.toast(
                None,
                "Attachments are limited to 20 MiB".into(),
                Variant::Warning,
            );
            return;
        }
        let kind = if mime == "application/pdf" {
            "PDF"
        } else {
            "Image"
        };
        let n = self
            .state
            .composer
            .atoms
            .iter()
            .filter(|a| matches!(&a.kind, AtomKind::Attachment(m, ..) if (m == "application/pdf") == (kind == "PDF")))
            .count()
            + 1;
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        self.state.composer.insert_atom(
            &format!("[{kind} {n}]"),
            AtomKind::Attachment(mime.into(), data, name),
        );
    }

    /// `ctrl+v` and an empty bracketed paste: the clipboard's image, else its text.
    pub(super) fn paste_clipboard(&mut self) {
        match crate::clipboard::read() {
            Ok(Some(crate::clipboard::Content::Image(bytes))) => {
                self.attach_bytes("image/png", bytes, "clipboard".into());
            }
            Ok(Some(crate::clipboard::Content::Text(text))) => {
                if !self.pane_paste(&text) {
                    self.paste(&text);
                }
            }
            Ok(None) => {}
            Err(message) => self.toast(None, message, Variant::Error),
        }
    }

    fn mouse(&mut self, mut mouse: crossterm::event::MouseEvent) {
        if self.tabs_mouse(mouse) {
            return;
        }
        mouse.column -= self.state.left;
        let at = (mouse.column, mouse.row);
        if self.hit_mouse(mouse.kind, at) {
            return;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp => self.scroll(3),
            MouseEventKind::ScrollDown => self.scroll(-3),
            MouseEventKind::Down(button @ (MouseButton::Left | MouseButton::Right))
                if self.state.diff.is_some()
                    && self.state.dialogs.is_empty()
                    && self.diff_click(button == MouseButton::Right, at) => {}
            MouseEventKind::Down(MouseButton::Left) if self.popup_row(at).is_some() => {
                self.hover_popup(at);
            }
            MouseEventKind::Up(MouseButton::Left) if self.popup_row(at).is_some() => {
                self.hover_popup(at);
                self.choose(false);
            }
            MouseEventKind::Down(MouseButton::Left)
                if crate::view::sidebar_overlay(&self.state)
                    && self.state.dialogs.is_empty()
                    && at.0 < self.state.width.saturating_sub(crate::view::SIDEBAR_WIDTH) => {}
            MouseEventKind::Down(MouseButton::Left) => {
                // A click while text is selected only clears the selection.
                if self.selection.is_some_and(|(a, b)| a != b) {
                    self.selection = None;
                    return;
                }
                if self.on_pane_edge(at.0) {
                    self.pane_drag = true;
                    return;
                }
                self.selection = Some((at, at));
                if self.over_account(at) {
                    self.selection = None;
                    return self.run_command("anysola.usage", None);
                }
                let busy = self.state.session().is_some_and(|s| s.busy);
                if !busy
                    && self.state.new_worktree.is_none()
                    && at.1 + 3 >= self.state.height
                    && over_text(&self.last, at, &self.state.location)
                {
                    self.selection = None;
                    return self.working_directory_menu();
                }
                if let Some(goal) = self.state.session().and_then(|s| s.goal.as_ref())
                    && over_text(&self.last, at, super::goal_status(goal).0)
                {
                    self.selection = None;
                    return self.open_goal();
                }
                if let Some(live) = &self.state.live
                    && over_text(&self.last, at, live)
                {
                    self.selection = None;
                    return self.run_command("session.child.first", None);
                }
                if let Some((label, command)) =
                    self.state.toast.as_ref().and_then(|t| t.action.clone())
                    && over_text(&self.last, at, &format!("› {label}"))
                {
                    self.selection = None;
                    self.state.toast = None;
                    return self.run_command(&command, None);
                }
                if self.state.route == Route::Home
                    && self.state.mcp.0 > 0
                    && at.1 + 3 >= self.state.height
                    && (over_text(&self.last, at, "MCP failed")
                        || over_text(&self.last, at, " MCP")
                        || over_text(&self.last, at, "⊙"))
                {
                    self.selection = None;
                    return self.run_command("mcp.list", None);
                }
                if self.state.session().is_some()
                    && crate::view::onboarding_shown(&self.state)
                    && at.0 >= crate::view::pane_width(&self.state)
                {
                    if over_text(&self.last, at, "✕") {
                        self.selection = None;
                        self.state.onboarding_dismissed = true;
                        return;
                    }
                    if over_text(&self.last, at, "Sign in") || over_text(&self.last, at, "/connect")
                    {
                        self.selection = None;
                        return self.run_command("provider.connect", None);
                    }
                }
                if self.state.mcp_list.len() > 2
                    && at.0 >= crate::view::pane_width(&self.state)
                    && (over_text(&self.last, at, "▼ MCP") || over_text(&self.last, at, "▶ MCP"))
                {
                    self.selection = None;
                    self.state.mcp_closed = !self.state.mcp_closed;
                    return;
                }
                if self.toggle_group(at.1) {
                    self.selection = None;
                    return;
                }
                if self.state.route == Route::Home {
                    self.click_recent(mouse.row);
                }
            }
            MouseEventKind::Moved => {
                self.hover_popup(at);
                let over = crate::view::toast_rect(&self.state)
                    .is_some_and(|r| r.contains(ratatui::layout::Position::new(at.0, at.1)));
                self.hover_toast(over);
                self.state.status_hover = false;
                self.state.status_hover = self.over_status(at);
            }
            MouseEventKind::Drag(MouseButton::Left) if self.pane_drag => {
                self.state.pane_width = Some(self.state.width.saturating_sub(at.0));
            }
            MouseEventKind::Up(MouseButton::Left) if self.pane_drag => self.pane_drag = false,
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some((anchor, _)) = self.selection {
                    self.selection = Some((anchor, at));
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some((anchor, end)) = self.selection {
                    if anchor == end {
                        self.selection = None;
                        // A plain click on a link opens it; modified clicks are the terminal's.
                        let plain = mouse.modifiers.is_empty();
                        let link = self
                            .last
                            .cell(at)
                            .and_then(crate::hyperlink::url)
                            .filter(|_| plain && self.state.dialogs.is_empty());
                        if let Some(url) = link {
                            term::open_url(&url);
                        } else if let Some(index) = self
                            .hit(at.1)
                            .and_then(|id| id.strip_prefix("user:")?.parse().ok())
                            .filter(|_| self.state.dialogs.is_empty())
                        {
                            self.click_user(index);
                        }
                    } else if self.state.settings.copy_on_select {
                        self.copy_selection(anchor, end);
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Right)
                if !self.state.settings.copy_on_select
                    && self.selection.is_some_and(|(a, b)| a != b) =>
            {
                if let Some((anchor, end)) = self.selection {
                    self.copy_selection(anchor, end);
                }
            }
            _ => {}
        }
    }

    /// Clicks on the controls the last frame drew; true when the event is used up. With a
    /// dialog open the screen behind it is inert: wheel moves the list, a click outside closes.
    fn hit_mouse(&mut self, kind: MouseEventKind, at: (u16, u16)) -> bool {
        let dialog = !self.state.dialogs.is_empty();
        let hit = crate::view::hit_at(&self.hits, at).cloned();
        if kind == MouseEventKind::Moved {
            self.state.tab_hover = match hit {
                Some(Hit::Tab(index) | Hit::CloseTab(index)) => Some(index),
                _ => None,
            };
        }
        match kind {
            MouseEventKind::Down(MouseButton::Left) => match hit {
                Some(hit) => {
                    self.selection = None;
                    self.click(hit);
                    true
                }
                None if dialog => {
                    self.dialog_key(&Stroke::parse("esc").expect("esc"));
                    true
                }
                None => false,
            },
            MouseEventKind::Down(MouseButton::Middle) => {
                if let Some(Hit::Tab(index)) = hit {
                    self.close_tab(index);
                }
                true
            }
            MouseEventKind::Moved if dialog => {
                if let Some(Hit::Option(index)) = hit {
                    self.hover_option(index);
                }
                true
            }
            MouseEventKind::ScrollUp if dialog => {
                self.dialog_key(&Stroke::parse("up").expect("up"));
                true
            }
            MouseEventKind::ScrollDown if dialog => {
                self.dialog_key(&Stroke::parse("down").expect("down"));
                true
            }
            _ => dialog,
        }
    }

    fn click(&mut self, hit: Hit) {
        match hit {
            Hit::Tab(index) => self.select_tab(index + 1),
            Hit::NewTab => self.run_command("session.new", None),
            Hit::CloseTab(index) => {
                self.state.tab_hover = None;
                self.close_tab(index);
            }
            Hit::Key(key) => {
                if let Some(stroke) = Stroke::parse(&key) {
                    self.key(stroke);
                }
            }
            Hit::Command(id) => self.run_command(&id, None),
            Hit::Panel => {}
            Hit::Option(_) | Hit::Button(_) | Hit::Field(_) | Hit::Format(_) => {
                self.dialog_click(hit)
            }
            Hit::Choice(index) => {
                let Some(session) = self.state.session_mut() else {
                    return;
                };
                // Straight to the prompt: a focused terminal pane would take the key otherwise.
                if let Some(p) = session.permission.as_mut() {
                    p.selected = index;
                    self.permission_key(&Stroke::parse("enter").expect("enter"));
                } else if let Some(q) = session.question.as_mut() {
                    q.selected = index;
                    let multiple = q.questions.get(q.tab).and_then(|a| a.multiple) == Some(true);
                    let key = if multiple { "space" } else { "enter" };
                    self.question_key(&Stroke::parse(key).expect("key"));
                }
            }
            Hit::DockTab(index) => {
                if let Some(dock) = self.state.dock.as_mut()
                    && dock.tab != index
                {
                    dock.tab = index;
                    dock.selected = 0;
                    self.refresh_dock();
                }
            }
            Hit::DockRow(index) => {
                if let Some(dock) = self.state.dock.as_mut() {
                    dock.selected = index;
                    self.key(Stroke::parse("enter").expect("enter"));
                }
            }
            Hit::ToggleTodos => self.state.todo_closed = !self.state.todo_closed,
            Hit::Image(set, index) => self.view_images(set, index),
            Hit::CloseToast => {
                if let Some(toast) = self.state.toast.as_mut() {
                    toast.until = 0;
                }
                self.state.toast_hover = None;
            }
        }
    }

    fn copy_selection(&mut self, anchor: (u16, u16), end: (u16, u16)) {
        let text = selected_text(&self.last, anchor, end);
        term::copy(&text);
        self.toast(None, "Copied to clipboard".into(), Variant::Info);
    }

    /// A click on an exploration group's row opens or closes it.
    /// The click target on a transcript row of the last frame.
    fn hit(&self, row: u16) -> Option<String> {
        self.state
            .session()?
            .layout
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .hits
            .iter()
            .find(|(y, _)| *y == row)
            .map(|(_, id)| id.clone())
    }

    fn toggle_group(&mut self, row: u16) -> bool {
        // User messages open their actions on release, so a drag inside one still selects.
        let Some(id) = self.hit(row).filter(|id| !id.starts_with("user:")) else {
            return false;
        };
        let Some(session) = self.state.session_mut() else {
            return false;
        };
        if let Some(call_id) = id.strip_prefix("execute:") {
            let call_id = call_id.to_string();
            self.open_execute(&call_id);
            return true;
        }
        if id == "revert" {
            self.run_command("session.redo", None);
            return true;
        }
        if id == "thinking" {
            self.run_command("session.toggle.thinking", None);
            return true;
        }
        if let Some(child) = id.strip_prefix("child:") {
            let child = child.to_string();
            self.open_session(&child);
            return true;
        }
        if id.starts_with("shell:") {
            self.shells_polled = 0;
        }
        if !session.toggled.remove(&id) {
            session.toggled.insert(id);
        }
        true
    }

    /// The terminal pane's left edge, where a drag resizes it.
    fn on_pane_edge(&self, x: u16) -> bool {
        self.state.pane.is_some()
            && matches!(self.state.route, Route::Session(_))
            && x == crate::view::pane_width(&self.state)
    }

    /// Whether `at` falls on the usage status line in the last frame.
    fn over_status(&self, at: (u16, u16)) -> bool {
        self.state
            .status_text()
            .is_some_and(|text| over_text(&self.last, at, &text))
    }

    /// The status line or the home plan line: both open `/usage` on click.
    fn over_account(&self, at: (u16, u16)) -> bool {
        let plan = (self.state.route == Route::Home)
            .then(|| {
                self.state
                    .account
                    .plan
                    .as_deref()
                    .map(crate::text::title_case)
            })
            .flatten();
        let footer = at.1 + 1 == self.state.height;
        self.over_status(at)
            || (footer && plan.is_some_and(|plan| over_text(&self.last, at, &plan)))
    }

    /// A click on a home `Recent` row opens that session.
    fn click_recent(&mut self, row: u16) {
        let target = (0..self.last.area.width).find_map(|x| {
            let line: String = (x..self.last.area.width)
                .map(|c| self.last[(c, row)].symbol())
                .collect();
            let line = line.trim();
            self.state
                .recent
                .iter()
                .find(|r| {
                    !line.is_empty()
                        && line.starts_with(&crate::text::truncate(&r.title, 8))
                        && line.ends_with(&crate::text::ago(r.updated, self.state.now))
                })
                .map(|r| r.id.clone())
        });
        if let Some(id) = target {
            self.selection = None;
            self.open_session(&id);
        }
    }

    /// Mounts older items once the reader is within a screen of the top, and finishes a pending
    /// `session.first` (the history is laid out the frame after `Loading session history…`).
    pub(super) fn reveal_history(&mut self) {
        let width = crate::view::session_content_width(&self.state);
        let height = self.state.height as usize;
        let Some(session) = self.state.session() else {
            return;
        };
        if session.floor == 0 {
            return;
        }
        if session.history_loading {
            if let Some(session) = self.state.session_mut() {
                session.floor = 0;
                session.history_loading = false;
            }
            return self.scroll(isize::MAX / 4);
        }
        let total = crate::view::transcript_rows(&self.state, session, &self.theme, width);
        if total.saturating_sub(session.scroll) > 2 * height {
            return;
        }
        if let Some(session) = self.state.session_mut() {
            session.floor = crate::transcript::rows::floor_before(
                &session.items,
                session.floor,
                crate::transcript::rows::BACKFILL_CHUNK,
            );
        }
    }

    pub(super) fn scroll(&mut self, rows: isize) {
        let Some(session) = self.state.session() else {
            return;
        };
        let width = crate::view::session_content_width(&self.state);
        let total = crate::view::transcript_rows(&self.state, session, &self.theme, width);
        let limit = total.saturating_sub(self.state.height as usize / 2);
        if let Some(session) = self.state.session_mut() {
            session.scroll =
                ((session.scroll as isize).saturating_add(rows).max(0) as usize).min(limit);
        }
    }

    /// Recomputes the `/` or `@` popup for the token under the cursor.
    pub(super) fn refresh_popup(&mut self) {
        if self.popup_hidden.as_deref() == Some(&self.state.composer.text) {
            return;
        }
        self.popup_hidden = None;
        if let Some((start, query)) = self.state.composer.cd_argument() {
            let home = std::path::PathBuf::from(&self.state.home);
            let recents: Vec<String> = self
                .recents
                .list(self.core.project_id())
                .iter()
                .map(|d| crate::complete::recent_value(d, &home))
                .collect();
            let rows =
                crate::complete::directory_rows(&query, self.core.directory(), &home, &recents);
            self.state.popup = Some(crate::state::Popup {
                trigger: 'd',
                start,
                rows,
                selected: 0,
                offset: 0,
                pad: 0,
                area: Default::default(),
            });
            return;
        }
        let Some((trigger, start, query)) = self.state.composer.token() else {
            self.state.popup = None;
            return;
        };
        let loaded = self.core.loaded();
        let (rows, pad) = if trigger == '/' {
            let server: Vec<(String, String)> = loaded
                .commands
                .list()
                .iter()
                .map(|c| (c.name.clone(), c.description.clone().unwrap_or_default()))
                .collect();
            let in_session = self.state.session().is_some();
            let revert_staged = self
                .state
                .session()
                .is_some_and(|s| self.core.revert_boundary(&s.id).ok().flatten().is_some());
            let scope = crate::complete::Scope {
                in_session,
                revert_staged,
                server: &server,
            };
            let (rows, pad) = crate::complete::slash_rows(&scope);
            (crate::complete::filter(rows, &query, "/", 10), pad)
        } else {
            let files = self.files.get_or_insert_with(Vec::new);
            if !self.files_walked {
                // The walk (up to 20 000 entries, slow on big trees and network disks) runs off
                // the loop; the list fills in when it lands and the popup refreshes then.
                self.files_walked = true;
                let directory = self.core.directory().to_path_buf();
                let tx = self.tx.clone();
                tokio::task::spawn_blocking(move || {
                    let files = crate::complete::walk(&directory, 20_000);
                    let _ = tx.send(super::Msg::Apply(Box::new(move |app: &mut super::App| {
                        app.files = Some(files);
                        if app.state.popup.is_some() {
                            app.refresh_popup();
                        }
                    })));
                });
            }
            let skills: Vec<(String, String)> = loaded
                .skills
                .iter()
                .map(|s| (s.name.clone(), s.description.clone().unwrap_or_default()))
                .collect();
            let references: Vec<(String, String)> = self
                .core
                .references()
                .into_iter()
                .filter(|r| !r.hidden)
                .map(|r| {
                    let place = match &r.source {
                        anysola_core::reference::Source::Git { repository, .. } => {
                            repository.clone()
                        }
                        anysola_core::reference::Source::Local(path) => path.display().to_string(),
                    };
                    let description = r.description.unwrap_or_default();
                    (r.name, format!("{description} {place}"))
                })
                .collect();
            let agents: Vec<(String, String)> = loaded
                .agents
                .list
                .iter()
                .filter(|a| a.mode != anysola_core::agent::Mode::Primary && !a.hidden)
                .map(|a| (a.id.clone(), a.description.clone().unwrap_or_default()))
                .collect();
            let files: Vec<String> = if query.is_empty() {
                files.iter().take(20).cloned().collect()
            } else {
                files.clone()
            };
            (
                crate::complete::mention_rows(&query, &skills, &references, &agents, &files),
                0,
            )
        };
        self.state.popup = Some(crate::state::Popup {
            trigger,
            start,
            rows,
            selected: 0,
            offset: 0,
            pad,
            area: Default::default(),
        });
    }
}

/// Whether `text` is drawn on row `at.1` of `buffer` across column `at.0`.
fn over_text(buffer: &ratatui::buffer::Buffer, (x, y): (u16, u16), text: &str) -> bool {
    if y >= buffer.area.height {
        return false;
    }
    let row: Vec<&str> = (0..buffer.area.width)
        .map(|c| buffer[(c, y)].symbol())
        .collect();
    let line: String = row.concat();
    let Some(byte) = line.find(text) else {
        return false;
    };
    let start = crate::text::width(&line[..byte]) as u16;
    (start..start + crate::text::width(text) as u16).contains(&x)
}

/// One key in a question prompt; the reply once it is answered or dismissed. Moving to another
/// question starts at its first row.
fn question_step(q: &mut crate::state::Question, stroke: &Stroke) -> Option<QuestionReply> {
    let total = q.questions.len();
    let Some(current) = q.questions.get(q.tab).cloned() else {
        match stroke.key.as_str() {
            "return" => return Some(QuestionReply::Answered(q.answers.clone())),
            "left" | "h" => {
                q.tab = total.saturating_sub(1);
                q.selected = 0;
            }
            "escape" => return Some(QuestionReply::Dismissed(None)),
            _ => {}
        }
        return None;
    };
    let rows = current.options.len() + 1;
    if q.selected >= rows {
        q.selected = 0;
    }
    let tab = q.tab;
    if let Some(custom) = q.custom.as_mut() {
        match stroke.key.as_str() {
            "return" => {
                if !custom.trim().is_empty() {
                    q.answers[q.tab] = vec![custom.trim().to_string()];
                }
                q.custom = None;
                q.tab += 1;
            }
            "escape" => q.custom = None,
            "backspace" => {
                custom.pop();
            }
            "space" => custom.push(' '),
            key if key.chars().count() == 1 && !stroke.ctrl => {
                let text = if stroke.shift {
                    key.to_uppercase()
                } else {
                    key.to_string()
                };
                custom.push_str(&text);
            }
            _ => {}
        }
    } else {
        let multiple = current.multiple == Some(true);
        let pick = |q: &mut crate::state::Question, index: usize| {
            if index + 1 == rows {
                q.custom = Some(String::new());
                return;
            }
            let label = current.options[index].label.clone();
            let answers = &mut q.answers[q.tab];
            if multiple {
                match answers.iter().position(|a| *a == label) {
                    Some(i) => {
                        answers.remove(i);
                    }
                    None => answers.push(label),
                }
            } else {
                *answers = vec![label];
                q.tab += 1;
            }
        };
        match stroke.key.as_str() {
            "up" | "k" => q.selected = (q.selected + rows - 1) % rows,
            "down" | "j" => q.selected = (q.selected + 1) % rows,
            "space" if multiple => pick(q, q.selected),
            "return" if multiple && q.selected + 1 < rows => q.tab += 1,
            "return" => pick(q, q.selected),
            "tab" | "right" | "l" => q.tab = (q.tab + 1).min(total),
            "left" | "h" => q.tab = q.tab.saturating_sub(1),
            "escape" => return Some(QuestionReply::Dismissed(None)),
            key => {
                if let Some(digit) = key.parse::<usize>().ok().filter(|d| (1..=rows).contains(d)) {
                    q.selected = digit - 1;
                    pick(q, digit - 1);
                }
            }
        }
    }
    if q.tab != tab {
        q.selected = 0;
    }
    // A single question submits as soon as it is answered.
    (total == 1 && q.tab >= 1).then(|| QuestionReply::Answered(q.answers.clone()))
}

/// Replies to a question, or to the MCP form it stands for.
fn answer(
    core: &anysola_core::api::Core,
    form: Option<&[(String, String)]>,
    id: &str,
    reply: QuestionReply,
) {
    match form {
        Some(fields) => {
            let reply = match reply {
                QuestionReply::Answered(answers) => {
                    anysola_core::mcp::FormReply::Answered(crate::form::values(fields, &answers))
                }
                QuestionReply::Dismissed(_) => anysola_core::mcp::FormReply::Cancelled,
            };
            core.reply_form(id, reply);
        }
        None => {
            core.reply_question(id, reply);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    #[test]
    fn over_text_hits_only_the_drawn_cells() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 30, 2));
        buffer.set_string(10, 1, "5h 42% · we", ratatui::style::Style::new());
        assert!(super::over_text(&buffer, (10, 1), "5h 42% · we"));
        assert!(super::over_text(&buffer, (20, 1), "5h 42% · we"));
        assert!(!super::over_text(&buffer, (21, 1), "5h 42% · we"));
        assert!(!super::over_text(&buffer, (12, 0), "5h 42% · we"));
    }

    fn asked(options: usize) -> anysola_core::tool::question::Question {
        anysola_core::tool::question::Question {
            question: "Which?".into(),
            header: "Pick".into(),
            options: (0..options)
                .map(|i| anysola_core::tool::question::QuestionOption {
                    label: format!("option {i}"),
                    description: String::new(),
                })
                .collect(),
            multiple: None,
        }
    }

    #[test]
    fn the_next_question_starts_at_its_first_row() {
        use crate::keymap::Stroke;
        use anysola_core::tool::question::QuestionReply;
        let mut q = crate::state::Question {
            request_id: "que_1".into(),
            questions: vec![asked(3), asked(1)],
            tab: 0,
            selected: 0,
            answers: vec![Vec::new(); 2],
            custom: None,
            form: None,
        };
        let key = |q: &mut crate::state::Question, key: &str| {
            super::question_step(q, &Stroke::parse(key).expect("key"))
        };
        // The third option of the first question, then Enter on the second (one option).
        assert_eq!(key(&mut q, "down"), None);
        assert_eq!(key(&mut q, "down"), None);
        assert_eq!(key(&mut q, "return"), None);
        assert_eq!(key(&mut q, "return"), None);
        assert_eq!((q.tab, q.selected), (2, 0));
        assert_eq!(q.answers, vec![vec!["option 2"], vec!["option 0"]]);
        assert_eq!(
            key(&mut q, "return"),
            Some(QuestionReply::Answered(q.answers.clone()))
        );
        // A row left over from a longer question is clamped before it is used.
        let mut q = crate::state::Question {
            questions: vec![asked(1)],
            selected: 5,
            answers: vec![Vec::new()],
            ..q
        };
        q.tab = 0;
        assert_eq!(
            key(&mut q, "return"),
            Some(QuestionReply::Answered(vec![vec!["option 0".into()]]))
        );
    }
}
