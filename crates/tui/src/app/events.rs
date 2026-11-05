//! Core events folded into the state.

use anysola_core::events::Event;
use anysola_core::session::gate::Reply;

use super::{App, now};
use crate::sound::Sound;
use crate::state::{Permission, Question, Variant};
use crate::transcript::model;

impl App {
    /// The open session an event belongs to: its own, or, for a subagent's ask, the busy
    /// session on screen.
    fn target(&self, session_id: &str) -> Option<String> {
        if self.state.sessions.contains_key(session_id) {
            return Some(session_id.to_string());
        }
        self.state
            .session()
            .filter(|s| s.busy)
            .map(|s| s.id.clone())
    }

    fn attention(&mut self, session_id: &str, on: bool) {
        if let Some(tab) = self
            .state
            .tabs
            .iter_mut()
            .find(|t| t.session_id == session_id)
        {
            tab.attention = on;
            tab.question = false;
        }
    }

    fn question_waiting(&mut self, session_id: &str) {
        self.attention(session_id, true);
        if let Some(tab) = self
            .state
            .tabs
            .iter_mut()
            .find(|t| t.session_id == session_id)
        {
            tab.question = true;
        }
    }

    /// A desktop notification for a session that needs attention, when enabled and the
    /// terminal reported losing focus.
    fn alert(&self, session_id: &str, message: &str, sound: Sound) {
        if self.tui.sound() {
            let custom = self.tui.sound_file(sound.name());
            crate::sound::play(sound, custom.as_deref(), &self.sounds, self.tui.volume());
        }
        let child = self
            .core
            .session(session_id)
            .ok()
            .flatten()
            .is_some_and(|row| row.parent_id.is_some());
        if child || !self.tui.notifications() || self.focused != Some(false) {
            return;
        }
        let title = self
            .state
            .sessions
            .get(session_id)
            .map(|s| s.title.as_str())
            .unwrap_or_default();
        crate::attention::notify(title, message);
    }

    /// An MCP server's form, asked as questions in the open session.
    fn form(&mut self, form: &anysola_core::mcp::Form) {
        let Some(session) = self.state.session_mut() else {
            self.core
                .reply_form(&form.id, anysola_core::mcp::FormReply::Cancelled);
            return;
        };
        let (questions, fields) = crate::form::questions(form);
        session.question = Some(Question {
            request_id: form.id.clone(),
            answers: vec![Vec::new(); questions.len()],
            questions,
            tab: 0,
            selected: 0,
            custom: None,
            form: Some(fields),
        });
        let id = session.id.clone();
        self.question_waiting(&id);
        self.alert(&id, "Input needs response", Sound::Question);
    }

    /// Toasts when a server starts needing sign-in or fails (once per change).
    fn mcp_status(&mut self, server: &anysola_core::mcp::Server) {
        use anysola_core::mcp::Status;
        let previous = self
            .mcp_seen
            .insert(server.name.clone(), server.status.clone());
        if previous.as_ref() == Some(&server.status) {
            return;
        }
        let action = Some(("Open MCP servers".to_string(), "mcp.list".to_string()));
        match &server.status {
            Status::NeedsAuth { .. } => self.toast_with(
                Some("MCP server needs authentication".into()),
                format!("Connect \"{}\" to use its tools.", server.name),
                Variant::Warning,
                action,
            ),
            Status::Failed { .. } => self.toast_with(
                Some(format!("MCP server failed: {}", server.name)),
                "Run /mcps to view details.".into(),
                Variant::Error,
                action,
            ),
            _ => {}
        }
        let rows = self.mcp_rows();
        self.count_mcp(&rows);
        if let Some(crate::dialog::Dialog::Select(s)) = self.state.dialogs.last_mut()
            && s.kind == crate::dialog::Kind::Mcps
        {
            let query = s.query.clone();
            let mut fresh = crate::menus::lists::mcps(&rows);
            fresh.query = query;
            *s = fresh;
        }
    }

    pub(super) fn count_mcp(&mut self, rows: &[crate::menus::lists::McpRow]) {
        use anysola_core::mcp::Status;
        let count = |f: fn(&Status) -> bool| rows.iter().filter(|r| f(&r.status)).count();
        self.state.mcp = (
            rows.len(),
            count(|s| matches!(s, Status::Connected)),
            count(|s| matches!(s, Status::Failed { .. })),
        );
        self.state.mcp_list = rows
            .iter()
            .map(|r| {
                let (dot, word) = match &r.status {
                    Status::Connected => ("text.feedback.success.base", "Connected"),
                    Status::Pending => ("text.muted", "Connecting"),
                    Status::Failed { .. } => ("text.feedback.error.base", "Error"),
                    Status::NeedsAuth { .. } => ("text.feedback.warning.base", "Sign in"),
                    _ => ("text.muted", "Disabled"),
                };
                (r.name.clone(), dot, word)
            })
            .collect();
    }

    pub(super) fn event(&mut self, event: Event) {
        let boundary = matches!(
            event,
            Event::Succeeded { .. } | Event::Failed { .. } | Event::Interrupted { .. }
        );
        self.fold(event);
        if boundary {
            self.refresh_dock();
            self.resume_relocate();
        }
    }

    fn fold(&mut self, event: Event) {
        let id = event.session_id().to_string();
        match &event {
            Event::PermissionAsked {
                request_id,
                action,
                resources,
                save,
                metadata,
                ..
            } => {
                if self.state.auto {
                    self.core.reply_permission(request_id, Reply::Once);
                    return;
                }
                let Some(target) = self.target(&id) else {
                    return;
                };
                let session = self.state.sessions.get_mut(&target).expect("open session");
                let tool = model::mark_awaiting(&mut session.items, action);
                let directory = session.directory.clone();
                let subject = tool.and_then(|t| match action.as_str() {
                    "shell" => t.str("command").map(str::to_string),
                    _ => t.str("filePath").or_else(|| t.str("path")).map(|p| {
                        p.strip_prefix(&format!("{directory}/"))
                            .unwrap_or(p)
                            .to_string()
                    }),
                });
                session.permission = Some(Permission {
                    request_id: request_id.clone(),
                    action: action.clone(),
                    resources: resources.clone(),
                    save: save.clone(),
                    metadata: metadata.clone(),
                    subject,
                    selected: 0,
                    fullscreen: false,
                    reject: None,
                });
                self.attention(&target, true);
                self.alert(&target, "Permission needs input", Sound::Permission);
            }
            Event::PermissionReplied { request_id, .. } => {
                for session in self.state.sessions.values_mut() {
                    if session
                        .permission
                        .as_ref()
                        .is_some_and(|p| &p.request_id == request_id)
                    {
                        session.permission = None;
                        model::clear_awaiting(&mut session.items);
                    }
                }
                let quiet: Vec<String> = self
                    .state
                    .sessions
                    .values()
                    .filter(|s| s.permission.is_none() && s.question.is_none())
                    .map(|s| s.id.clone())
                    .collect();
                for session in quiet {
                    self.attention(&session, false);
                }
            }
            Event::QuestionAsked {
                request_id,
                questions,
                ..
            } => {
                let Some(target) = self.target(&id) else {
                    return;
                };
                let session = self.state.sessions.get_mut(&target).expect("open session");
                session.question = Some(Question {
                    request_id: request_id.clone(),
                    answers: vec![Vec::new(); questions.len()],
                    questions: questions.clone(),
                    tab: 0,
                    selected: 0,
                    custom: None,
                    form: None,
                });
                self.question_waiting(&target);
                self.alert(&target, "Input needs response", Sound::Question);
            }
            Event::Credits(snapshot) => {
                if let Some(session) = self.state.sessions.get_mut(&id) {
                    session.credits = Some(snapshot.clone());
                }
                self.account_soon();
            }
            Event::GoalChanged { goal, .. } => {
                if let Some(session) = self.state.sessions.get_mut(&id) {
                    session.goal = goal.clone();
                }
                self.refresh_goal_dialog();
            }
            Event::TitleChanged { title, .. } => {
                if let Some(session) = self.state.sessions.get_mut(&id) {
                    session.title = title.clone();
                }
                if let Some(tab) = self.state.tabs.iter_mut().find(|t| t.session_id == id) {
                    tab.title = title.clone();
                }
            }
            Event::FormAsked { form } => self.form(form),
            Event::McpStatusChanged { server } => self.mcp_status(server),
            Event::Succeeded { .. } | Event::Failed { .. } | Event::Interrupted { .. } => {
                if let Some(session) = self.state.sessions.get_mut(&id) {
                    session.busy = false;
                    session.permission = None;
                    session.question = None;
                }
                self.attention(&id, false);
                let shown = self.state.session().is_some_and(|s| s.id == id);
                if let Some(tab) = self.state.tabs.iter_mut().find(|t| t.session_id == id) {
                    tab.unread = !shown;
                }
                match &event {
                    Event::Failed { error, .. } => {
                        self.toast(
                            Some("Session failed".into()),
                            error.message.clone(),
                            Variant::Error,
                        );
                        self.alert(&id, &error.message, Sound::Error);
                    }
                    Event::Succeeded { .. } => {
                        let child = self
                            .core
                            .session(&id)
                            .ok()
                            .flatten()
                            .is_some_and(|row| row.parent_id.is_some());
                        let sound = if child {
                            Sound::SubagentDone
                        } else {
                            Sound::Done
                        };
                        self.alert(&id, "Session done", sound);
                    }
                    _ => {}
                }
                self.reload(&id);
                self.refresh_recent();
            }
            _ => {
                let loaded = self.core.loaded();
                let names = |model: &str| super::display_name(&loaded.models, model);
                if let Some(session) = self.state.sessions.get_mut(&id) {
                    model::apply(&mut session.items, &event, now(), &names);
                    session.busy = true;
                }
            }
        }
    }
}
