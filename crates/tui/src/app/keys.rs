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
        let Some(session) = self.state.session_mut() else {
            return;
        };
        let Some(permission) = session.permission.as_mut() else {
            return;
        };
        let count = permission.options().len();
        match stroke.key.as_str() {
            "left" | "h" => permission.selected = (permission.selected + count - 1) % count,
            "right" | "l" | "tab" => permission.selected = (permission.selected + 1) % count,
            "f" if stroke.ctrl => permission.fullscreen = !permission.fullscreen,
            "return" => {
                let reply = match permission.options()[permission.selected] {
                    "Allow once" => Reply::Once,
                    "Always allow" => Reply::Always,
                    _ => Reply::Reject(None),
                };
                let id = permission.request_id.clone();
                self.core.reply_permission(&id, reply);
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
        let total = q.questions.len();
        let Some(current) = q.questions.get(q.tab).cloned() else {
            match stroke.key.as_str() {
                "return" => {
                    let id = q.request_id.clone();
                    let answers = q.answers.clone();
                    answer(
                        &self.core,
                        q.form.as_deref(),
                        &id,
                        QuestionReply::Answered(answers),
                    );
                    session.question = None;
                }
                "left" | "h" => q.tab = total.saturating_sub(1),
                "escape" => {
                    let id = q.request_id.clone();
                    answer(
                        &self.core,
                        q.form.as_deref(),
                        &id,
                        QuestionReply::Dismissed(None),
                    );
                    session.question = None;
                }
                _ => {}
            }
            return;
        };
        let rows = current.options.len() + 1;
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
                "escape" => {
                    let id = q.request_id.clone();
                    answer(
                        &self.core,
                        q.form.as_deref(),
                        &id,
                        QuestionReply::Dismissed(None),
                    );
                    session.question = None;
                    return;
                }
                key => {
                    if let Some(digit) =
                        key.parse::<usize>().ok().filter(|d| (1..=rows).contains(d))
                    {
                        q.selected = digit - 1;
                        pick(q, digit - 1);
                    }
                }
            }
        }
        if q.tab != 0 && q.selected >= rows {
            q.selected = 0;
        }
        // A single question submits as soon as it is answered.
        if total == 1 && q.tab >= 1 {
            let id = q.request_id.clone();
            let answers = q.answers.clone();
            answer(
                &self.core,
                q.form.as_deref(),
                &id,
                QuestionReply::Answered(answers),
            );
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
        let path = trimmed
            .strip_prefix("file://")
            .unwrap_or(trimmed)
            .replace("\\ ", " ");
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
        if bytes.len() > 20 * 1024 * 1024 {
            self.toast(
                None,
                "Attachments are limited to 20 MiB".into(),
                Variant::Warning,
            );
            return true;
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
        let name = std::path::Path::new(&path)
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        self.state.composer.insert_atom(
            &format!("[{kind} {n}]"),
            AtomKind::Attachment(mime.into(), data, name),
        );
        true
    }

    fn mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        let at = (mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::ScrollUp => self.scroll(3),
            MouseEventKind::ScrollDown => self.scroll(-3),
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
                    return self.run_command("anysola.credits", None);
                }
                if self.state.route == Route::Home {
                    self.click_recent(mouse.row);
                }
            }
            MouseEventKind::Moved => {
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
                    } else {
                        let text = selected_text(&self.last, anchor, end);
                        term::copy(&text);
                        self.toast(None, "Copied to clipboard".into(), Variant::Info);
                    }
                }
            }
            _ => {}
        }
    }

    /// The terminal pane's left edge, where a drag resizes it.
    fn on_pane_edge(&self, x: u16) -> bool {
        self.state.pane.is_some()
            && matches!(self.state.route, Route::Session(_))
            && x == crate::view::pane_width(&self.state)
    }

    /// Whether `at` falls on the credit status line in the last frame.
    fn over_status(&self, at: (u16, u16)) -> bool {
        self.state
            .status_text()
            .is_some_and(|text| over_text(&self.last, at, &text))
    }

    /// The status line or the home plan line: both open `/credits` on click.
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
            self.state.popup = (!rows.is_empty()).then_some(crate::state::Popup {
                trigger: '/',
                start,
                rows,
                selected: 0,
                offset: 0,
                pad: 0,
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
            let directory = self.core.directory().to_path_buf();
            let files = self
                .files
                .get_or_insert_with(|| crate::complete::walk(&directory, 20_000));
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
        buffer.set_string(10, 1, "731 credits", ratatui::style::Style::new());
        assert!(super::over_text(&buffer, (10, 1), "731 credits"));
        assert!(super::over_text(&buffer, (20, 1), "731 credits"));
        assert!(!super::over_text(&buffer, (21, 1), "731 credits"));
        assert!(!super::over_text(&buffer, (12, 0), "731 credits"));
    }
}
