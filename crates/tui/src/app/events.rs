//! Core events folded into the state.

use anysola_core::events::Event;
use anysola_core::session::gate::Reply;

use super::{App, now};
use crate::sound::Sound;
use crate::state::{Permission, Question, Variant};
use crate::transcript::model;
use anysola_core::gateway::error::USAGE_LIMIT_KIND;

/// Walks a session up its parents to the nearest one that is open (`Ok`), or else the root
/// it reached (`Err`). Stops after a few dozen steps should the parents form a loop.
fn ask_owner(
    session_id: &str,
    open: impl Fn(&str) -> bool,
    parent: impl Fn(&str) -> Option<String>,
) -> Result<String, String> {
    let mut current = session_id.to_string();
    for _ in 0..32 {
        if open(&current) {
            return Ok(current);
        }
        match parent(&current) {
            Some(up) => current = up,
            None => break,
        }
    }
    Err(current)
}

impl App {
    /// The open session an ask is shown in: its own, the nearest open parent of a subagent's,
    /// the busy session on screen, or else its root session, given a tab in the background.
    /// `None` only when the session is unknown.
    fn target(&mut self, session_id: &str) -> Option<String> {
        let parent = |id: &str| self.core.session(id).ok().flatten()?.parent_id;
        let root = match ask_owner(
            session_id,
            |id| self.state.sessions.contains_key(id),
            parent,
        ) {
            Ok(open) => return Some(open),
            Err(root) => root,
        };
        if let Some(busy) = self.state.session().filter(|s| s.busy) {
            return Some(busy.id.clone());
        }
        self.attach_session(&root).map(|_| root)
    }

    pub(super) fn attention(&mut self, session_id: &str, on: bool) {
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
    pub(super) fn alert(&self, session_id: &str, message: &str, sound: Sound) {
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
            shown: now(),
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
                    // Nowhere to ask: refuse rather than leave the turn waiting.
                    self.core.reply_permission(request_id, Reply::Reject(None));
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
                    shown: now(),
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
                    self.core.reply_question(
                        request_id,
                        anysola_core::tool::question::QuestionReply::Dismissed(None),
                    );
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
                    shown: now(),
                });
                self.question_waiting(&target);
                self.alert(&target, "Input needs response", Sound::Question);
            }
            Event::Usage(snapshot) => {
                if let Some(session) = self.state.sessions.get_mut(&id) {
                    session.usage = Some(snapshot.clone());
                }
                if let Some(latest) = &snapshot.latest {
                    self.state.account.apply(latest);
                    self.warn_limits();
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
                    Event::Failed { error, .. } if error.kind == USAGE_LIMIT_KIND => {
                        self.usage_limit(&id, error.message.clone());
                    }
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

#[cfg(test)]
mod tests {
    use super::ask_owner;

    #[test]
    fn asks_go_to_the_nearest_open_parent() {
        // ses_c → ses_b → ses_a (root).
        let parent = |id: &str| match id {
            "ses_c" => Some("ses_b".to_string()),
            "ses_b" => Some("ses_a".to_string()),
            _ => None,
        };
        let open = |ids: &'static [&'static str]| move |id: &str| ids.contains(&id);
        assert_eq!(
            ask_owner("ses_c", open(&["ses_c", "ses_a"]), parent),
            Ok("ses_c".into())
        );
        assert_eq!(
            ask_owner("ses_c", open(&["ses_b", "ses_a"]), parent),
            Ok("ses_b".into())
        );
        assert_eq!(
            ask_owner("ses_c", open(&["ses_a"]), parent),
            Ok("ses_a".into())
        );
        assert_eq!(ask_owner("ses_c", open(&[]), parent), Err("ses_a".into()));
        assert_eq!(ask_owner("ses_x", open(&[]), parent), Err("ses_x".into()));
        let looped = |id: &str| Some(if id == "ses_1" { "ses_2" } else { "ses_1" }.to_string());
        assert!(ask_owner("ses_1", open(&[]), looped).is_err());
    }
}
