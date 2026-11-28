//! The [`SessionHost`] tools of a running session use: questions, skills and child sessions.

use std::sync::Arc;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;

use crate::api::Shared;
use crate::skill::Skill;
use crate::tool::question::{Question, QuestionReply};
use crate::tool::subagent::SubagentRequest;
use crate::tool::{SessionHost, ToolContext, ToolResult};

pub struct Host {
    pub shared: Arc<Shared>,
}

impl SessionHost for Host {
    fn ask_questions(
        &self,
        session_id: &str,
        questions: Vec<Question>,
    ) -> BoxFuture<'_, QuestionReply> {
        let session_id = session_id.to_string();
        async move {
            let message = match questions.first() {
                Some(first) if !first.question.trim().is_empty() => format!(
                    "Anysola Code has a question for you: {}",
                    first.question.trim()
                ),
                _ => "Anysola Code has a question for you".to_string(),
            };
            // Spawned before the form is shown and never waited for.
            crate::hooks::fire::notification(
                &self.shared,
                &session_id,
                crate::hooks::fire::Notice::Question,
                message,
            );
            self.shared
                .questions
                .ask(&self.shared.events, &session_id, questions)
                .await
        }
        .boxed()
    }

    fn skill(&self, id: &str) -> Option<Skill> {
        self.shared
            .loaded()
            .skills
            .iter()
            .find(|s| s.id == id)
            .cloned()
    }

    fn subagent<'a>(
        &'a self,
        ctx: &'a ToolContext,
        request: SubagentRequest,
    ) -> BoxFuture<'a, ToolResult> {
        super::subagent::run(self.shared.clone(), ctx, request).boxed()
    }
}
