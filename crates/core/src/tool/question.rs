//! The `question` tool: one `Questions` form with a field per question; the answers come back as
//! one line of text.

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    DefinitionEnv, Tool, ToolContext, ToolDefinition, ToolError, ToolErrorKind, ToolOutput,
    ToolResult, parse_input,
};
use crate::embedded_def;
use crate::permission::PermissionRequest;

embedded_def!(def, "question");

/// What the step sees when the user closes the form without a message: the step ends.
pub const DISMISSED: &str = "The user dismissed this question";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub question: String,
    pub header: String,
    pub options: Vec<QuestionOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiple: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum QuestionReply {
    /// One answer list per question; empty when unanswered.
    Answered(Vec<Vec<String>>),
    /// Closed without answers. With a message the model sees it and continues.
    Dismissed(Option<String>),
}

#[derive(Deserialize)]
struct Input {
    questions: Vec<Question>,
}

pub fn answer_text(questions: &[Question], answers: &[Vec<String>]) -> String {
    let formatted = questions
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let answer = match answers.get(index) {
                Some(answer) if !answer.is_empty() => answer.join(", "),
                _ => "Unanswered".to_string(),
            };
            format!("\"{}\"=\"{answer}\"", question.question)
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "User has answered your questions: {formatted}. You can now continue with the user's answers in mind."
    )
}

pub struct QuestionTool;

impl Tool for QuestionTool {
    fn name(&self) -> &'static str {
        "question"
    }

    fn definition(&self, _: &DefinitionEnv) -> ToolDefinition {
        let def = def();
        ToolDefinition {
            name: self.name().into(),
            description: def.description.clone(),
            input_schema: def.input_schema.clone(),
        }
    }

    fn execute<'a>(&'a self, input: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolResult> {
        async move {
            let input: Input = parse_input(self.name(), &def().input_schema, input)?;
            ctx.ask(PermissionRequest {
                action: self.name().into(),
                resources: vec!["*".into()],
                ..Default::default()
            })
            .await?;
            match ctx
                .host
                .ask_questions(&ctx.session_id, input.questions.clone())
                .await
            {
                QuestionReply::Answered(answers) => {
                    Ok(ToolOutput::text(answer_text(&input.questions, &answers))
                        .with_metadata("answers", serde_json::json!(answers)))
                }
                QuestionReply::Dismissed(Some(message)) => Err(ToolError::execution(message)),
                QuestionReply::Dismissed(None) => Err(ToolError {
                    kind: ToolErrorKind::Declined,
                    message: DISMISSED.into(),
                }),
            }
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::test_support::context;
    use serde_json::json;

    fn input() -> Value {
        json!({"questions": [
            {"question": "Pick?", "header": "Pick", "options": [{"label": "A", "description": "a"}]},
            {"question": "More?", "header": "More", "options": [], "multiple": true}
        ]})
    }

    #[tokio::test]
    async fn answers_and_unanswered() {
        let dir = tempfile::tempdir().unwrap();
        let test = context(dir.path());
        *test.host.question_reply.lock().unwrap() =
            Some(QuestionReply::Answered(vec![vec!["A".into()], vec![]]));
        let output = QuestionTool.execute(input(), &test.ctx).await.unwrap();
        assert_eq!(
            output.content,
            vec![super::super::ToolContent::text(
                "User has answered your questions: \"Pick?\"=\"A\", \"More?\"=\"Unanswered\". You can now continue with the user's answers in mind."
            )]
        );
        assert_eq!(output.metadata["answers"], json!([["A"], []]));
        let asked = test.gate.asked.lock().unwrap();
        assert_eq!(
            (asked[0].action.as_str(), &asked[0].resources),
            ("question", &vec!["*".to_string()])
        );
        assert!(asked[0].save.is_empty());
        assert_eq!(test.host.questions.lock().unwrap()[0].len(), 2);
    }

    #[tokio::test]
    async fn dismissed() {
        let dir = tempfile::tempdir().unwrap();
        let test = context(dir.path());
        *test.host.question_reply.lock().unwrap() =
            Some(QuestionReply::Dismissed(Some("no answers here".into())));
        let error = QuestionTool.execute(input(), &test.ctx).await.unwrap_err();
        assert_eq!(error, ToolError::execution("no answers here"));
        *test.host.question_reply.lock().unwrap() = Some(QuestionReply::Dismissed(None));
        let error = QuestionTool.execute(input(), &test.ctx).await.unwrap_err();
        assert_eq!(
            (error.kind, error.message.as_str()),
            (ToolErrorKind::Declined, DISMISSED)
        );
    }

    #[test]
    fn several_answers_joined() {
        let questions: Vec<Question> =
            serde_json::from_value(input()["questions"].clone()).unwrap();
        assert!(
            answer_text(&questions, &[vec!["A".into(), "B".into()]])
                .contains("\"Pick?\"=\"A, B\", \"More?\"=\"Unanswered\"")
        );
    }
}
