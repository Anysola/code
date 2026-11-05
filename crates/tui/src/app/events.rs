//! Core events folded into the state.

use anysola_core::events::Event;
use anysola_core::session::gate::Reply;

use super::{App, now};
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
        }
    }

    pub(super) fn event(&mut self, event: Event) {
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
                });
                self.attention(&target, true);
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
                });
                self.attention(&target, true);
            }
            Event::Credits(snapshot) => {
                if let Some(session) = self.state.sessions.get_mut(&id) {
                    session.credits = Some(snapshot.clone());
                }
            }
            Event::TitleChanged { title, .. } => {
                if let Some(session) = self.state.sessions.get_mut(&id) {
                    session.title = title.clone();
                }
                if let Some(tab) = self.state.tabs.iter_mut().find(|t| t.session_id == id) {
                    tab.title = title.clone();
                }
            }
            Event::Succeeded { .. } | Event::Failed { .. } | Event::Interrupted { .. } => {
                if let Some(session) = self.state.sessions.get_mut(&id) {
                    session.busy = false;
                    session.permission = None;
                    session.question = None;
                }
                self.attention(&id, false);
                if matches!(event, Event::Failed { .. }) {
                    self.toast(None, "Session failed".into(), Variant::Error);
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
