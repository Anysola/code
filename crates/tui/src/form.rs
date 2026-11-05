//! MCP elicitation forms asked through the question prompt: one question per field, answers
//! turned back into typed values.

use anysola_core::mcp::Form;
use anysola_core::tool::question::{Question, QuestionOption};
use serde_json::{Map, Value};

fn text(field: &Value, key: &str) -> String {
    field
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The questions and each field's `(key, type)`.
pub fn questions(form: &Form) -> (Vec<Question>, Vec<(String, String)>) {
    let message = form
        .metadata
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or(&form.title);
    let mut questions = Vec::new();
    let mut fields = Vec::new();
    for field in &form.fields {
        let key = text(field, "key");
        let kind = text(field, "type");
        let title = Some(text(field, "title"))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| key.clone());
        let options = match (kind.as_str(), field.get("enum").and_then(Value::as_array)) {
            ("boolean", _) => ["Yes", "No"]
                .map(|l| QuestionOption {
                    label: l.into(),
                    description: String::new(),
                })
                .to_vec(),
            (_, Some(values)) => values
                .iter()
                .filter_map(Value::as_str)
                .map(|v| QuestionOption {
                    label: v.into(),
                    description: String::new(),
                })
                .collect(),
            _ => Vec::new(),
        };
        let description = text(field, "description");
        let question = if description.is_empty() {
            message.to_string()
        } else {
            format!("{message}\n{description}")
        };
        questions.push(Question {
            question,
            header: title,
            options,
            multiple: None,
        });
        fields.push((key, kind));
    }
    (questions, fields)
}

/// The answers as the form's values; unanswered fields are left out.
pub fn values(fields: &[(String, String)], answers: &[Vec<String>]) -> Map<String, Value> {
    let mut out = Map::new();
    for ((key, kind), answer) in fields.iter().zip(answers) {
        let Some(first) = answer.first() else {
            continue;
        };
        let value = match kind.as_str() {
            "boolean" => Value::Bool(first == "Yes"),
            "integer" => first
                .parse::<i64>()
                .map_or_else(|_| Value::String(first.clone()), Value::from),
            "number" => first
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map_or_else(|| Value::String(first.clone()), Value::Number),
            _ => Value::String(first.clone()),
        };
        out.insert(key.clone(), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fields_become_questions_and_answers_values() {
        let form = Form {
            id: "frm_1".into(),
            title: "Deploy".into(),
            metadata: json!({"message": "Confirm the deploy"}),
            fields: vec![
                json!({"key": "env", "type": "string", "enum": ["staging", "prod"]}),
                json!({"key": "force", "type": "boolean", "title": "Force"}),
                json!({"key": "count", "type": "integer"}),
            ],
        };
        let (questions, fields) = questions(&form);
        assert_eq!(questions[0].options.len(), 2);
        assert_eq!(questions[1].header, "Force");
        let answers = vec![vec!["prod".into()], vec!["Yes".into()], vec!["3".into()]];
        assert_eq!(
            Value::Object(values(&fields, &answers)),
            json!({"env": "prod", "force": true, "count": 3})
        );
    }
}
