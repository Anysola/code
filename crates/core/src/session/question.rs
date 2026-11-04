//! Pending question forms of every session in this process: the question tool waits here until a
//! front end answers through [`crate::api::Core::reply_question`].

use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::{broadcast, oneshot};

use crate::events::Event;
use crate::tool::question::{Question, QuestionReply};

#[derive(Default)]
pub struct Questions {
    pending: Mutex<HashMap<String, oneshot::Sender<QuestionReply>>>,
}

impl Questions {
    pub async fn ask(
        &self,
        events: &broadcast::Sender<Event>,
        session_id: &str,
        questions: Vec<Question>,
    ) -> QuestionReply {
        let id = crate::ids::form();
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.clone(), sender);
        let _ = events.send(Event::QuestionAsked {
            session_id: session_id.to_string(),
            request_id: id,
            questions,
        });
        receiver.await.unwrap_or(QuestionReply::Dismissed(None))
    }

    /// Answers a pending form; false when it is unknown or already answered.
    pub fn reply(&self, request_id: &str, reply: QuestionReply) -> bool {
        let sender = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(request_id);
        sender.is_some_and(|s| s.send(reply).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reply_reaches_the_asker() {
        let questions = std::sync::Arc::new(Questions::default());
        let (events, mut receiver) = broadcast::channel(4);
        let asker = {
            let questions = questions.clone();
            tokio::spawn(async move { questions.ask(&events, "ses_a", Vec::new()).await })
        };
        let Ok(Event::QuestionAsked { request_id, .. }) = receiver.recv().await else {
            panic!("no question event");
        };
        assert!(questions.reply(&request_id, QuestionReply::Answered(vec![])));
        assert!(!questions.reply(&request_id, QuestionReply::Dismissed(None)));
        assert_eq!(asker.await.unwrap(), QuestionReply::Answered(vec![]));
    }
}
