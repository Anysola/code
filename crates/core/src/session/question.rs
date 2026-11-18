//! Pending question forms of every session in this process: the question tool waits here until a
//! front end answers through [`crate::api::Core::reply_question`].

use std::sync::Mutex;

use tokio::sync::{broadcast, oneshot};

use crate::events::Event;
use crate::tool::question::{Question, QuestionReply};

struct Waiting {
    session_id: String,
    questions: Vec<Question>,
    answer: oneshot::Sender<QuestionReply>,
}

/// Pending questions, in the order they were asked.
#[derive(Default)]
pub struct Questions {
    pending: Mutex<Vec<(String, Waiting)>>,
}

/// Takes the question out of the list however its wait ends (answered, or the turn that asked
/// it went away) and says so once.
struct Settle<'a> {
    questions: &'a Questions,
    events: &'a broadcast::Sender<Event>,
    session_id: String,
    id: String,
}

impl Drop for Settle<'_> {
    fn drop(&mut self) {
        self.questions
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(id, _)| *id != self.id);
        let _ = self.events.send(Event::QuestionReplied {
            session_id: std::mem::take(&mut self.session_id),
            request_id: std::mem::take(&mut self.id),
        });
    }
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
            .push((
                id.clone(),
                Waiting {
                    session_id: session_id.to_string(),
                    questions: questions.clone(),
                    answer: sender,
                },
            ));
        let _settle = Settle {
            questions: self,
            events,
            session_id: session_id.to_string(),
            id: id.clone(),
        };
        let _ = events.send(Event::QuestionAsked {
            session_id: session_id.to_string(),
            request_id: id,
            questions,
        });
        receiver.await.unwrap_or(QuestionReply::Dismissed(None))
    }

    /// Answers a pending form; false when it is unknown or already answered.
    pub fn reply(&self, request_id: &str, reply: QuestionReply) -> bool {
        let waiting = {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            pending
                .iter()
                .position(|(id, _)| id == request_id)
                .map(|at| pending.remove(at).1)
        };
        waiting.is_some_and(|w| w.answer.send(reply).is_ok())
    }

    /// Every question still waiting, as the events that asked them, oldest first.
    pub fn pending(&self) -> Vec<Event> {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(id, w)| Event::QuestionAsked {
                session_id: w.session_id.clone(),
                request_id: id.clone(),
                questions: w.questions.clone(),
            })
            .collect()
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
        assert!(matches!(
            receiver.recv().await,
            Ok(Event::QuestionReplied { request_id: r, .. }) if r == request_id
        ));
    }

    #[tokio::test]
    async fn a_dropped_question_leaves_the_list_and_says_so() {
        let questions = std::sync::Arc::new(Questions::default());
        let (events, mut receiver) = broadcast::channel(4);
        let asker = {
            let questions = questions.clone();
            tokio::spawn(async move { questions.ask(&events, "ses_a", Vec::new()).await })
        };
        let Ok(Event::QuestionAsked { request_id, .. }) = receiver.recv().await else {
            panic!("no question event");
        };
        assert!(matches!(
            questions.pending().as_slice(),
            [Event::QuestionAsked { request_id: r, session_id, .. }] if *r == request_id && session_id == "ses_a"
        ));
        // The turn that asked is interrupted: its tool call is dropped.
        asker.abort();
        assert!(matches!(
            receiver.recv().await,
            Ok(Event::QuestionReplied { request_id: r, .. }) if r == request_id
        ));
        assert!(questions.pending().is_empty());
        assert!(!questions.reply(&request_id, QuestionReply::Dismissed(None)));
    }
}
