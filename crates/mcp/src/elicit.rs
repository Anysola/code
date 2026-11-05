//! `elicitation/create` as a form the front ends show (`<server> is requesting input`).

use anysola_core::api::Core;
use anysola_core::mcp::{Form, FormReply};
use serde_json::{Map, Value, json};

/// The form field a URL elicitation carries.
const URL_FIELD: &str = "elicitation";

/// One requested property as a form field.
fn field(key: &str, property: &Value, required: bool) -> Value {
    // Some servers emit machine titles like "string with format email".
    let machine = |title: &str| {
        let title = title.trim().to_lowercase();
        let first = title.split_whitespace().next().unwrap_or_default();
        let rest = title[first.len()..].trim_start();
        ["boolean", "string", "number", "integer", "array", "object"].contains(&first)
            && (rest.is_empty() || rest.starts_with("with ") || rest.starts_with("in "))
    };
    let description = property["description"].as_str();
    let title = property["title"]
        .as_str()
        .filter(|t| !machine(t))
        .or(description)
        .unwrap_or(key);
    let mut out = json!({"key": key, "title": title});
    if let Some(description) = description.filter(|d| *d != title) {
        out["description"] = json!(description);
    }
    if required {
        out["required"] = json!(true);
    }
    let copy = |out: &mut Value, keys: &[&str]| {
        for k in keys {
            if let Some(v) = property.get(*k) {
                out[*k] = v.clone();
            }
        }
    };
    let option = |value: &Value, label: &Value| json!({"value": value, "label": label});
    match property["type"].as_str().unwrap_or_default() {
        "boolean" => {
            out["type"] = json!("boolean");
            copy(&mut out, &["default"]);
        }
        kind @ ("number" | "integer") => {
            out["type"] = json!(kind);
            copy(&mut out, &["minimum", "maximum", "default"]);
        }
        "array" => {
            out["type"] = json!("multiselect");
            let items = &property["items"];
            let options: Vec<Value> = match items["anyOf"].as_array() {
                Some(any) => any
                    .iter()
                    .map(|o| option(&o["const"], &o["title"]))
                    .collect(),
                None => items["enum"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|v| option(v, v))
                    .collect(),
            };
            out["options"] = json!(options);
            out["custom"] = json!(false);
            copy(&mut out, &["minItems", "maxItems", "default"]);
        }
        _ => {
            out["type"] = json!("string");
            copy(&mut out, &["format", "minLength", "maxLength", "default"]);
            let options: Option<Vec<Value>> =
                match (property["oneOf"].as_array(), property["enum"].as_array()) {
                    (Some(one), _) => Some(
                        one.iter()
                            .map(|o| option(&o["const"], &o["title"]))
                            .collect(),
                    ),
                    (None, Some(values)) => Some(
                        values
                            .iter()
                            .enumerate()
                            .map(|(i, v)| {
                                let label = property["enumNames"]
                                    .get(i)
                                    .filter(|l| l.is_string())
                                    .unwrap_or(v);
                                option(v, label)
                            })
                            .collect(),
                    ),
                    (None, None) => None,
                };
            if let Some(options) = options {
                out["options"] = json!(options);
                out["custom"] = json!(false);
            }
        }
    }
    out
}

/// Asks the front ends and returns the `elicitation/create` result.
pub async fn ask(core: &Core, server: &str, params: &Value, id: String) -> Value {
    let mut metadata = json!({"kind": "mcp-elicitation", "server": server});
    if let Some(elicitation) = params["elicitationId"].as_str() {
        metadata["elicitationID"] = json!(elicitation);
    }
    metadata["message"] = params["message"].clone();
    let title = format!("{server} is requesting input");
    if params["mode"] == "url" {
        let form = Form {
            id,
            title,
            metadata,
            fields: vec![json!({"key": URL_FIELD, "type": "external", "url": params["url"]})],
        };
        return match core.ask_form(form).await {
            FormReply::Answered(_) => json!({"action": "accept"}),
            FormReply::Cancelled => json!({"action": "cancel"}),
        };
    }
    let schema = &params["requestedSchema"];
    let required: Vec<&str> = schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let fields: Vec<Value> = schema["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, property)| field(key, property, required.contains(&key.as_str())))
        .collect();
    if fields.is_empty() {
        return json!({"action": "accept", "content": {}});
    }
    match core
        .ask_form(Form {
            id,
            title,
            metadata,
            fields,
        })
        .await
    {
        FormReply::Answered(answer) => {
            json!({"action": "accept", "content": Value::Object(answer.into_iter().collect::<Map<_, _>>())})
        }
        FormReply::Cancelled => json!({"action": "cancel"}),
    }
}

/// A URL elicitation the server reports finished settles its form.
pub fn complete(core: &Core, form: &str) {
    core.reply_form(
        form,
        FormReply::Answered(Map::from_iter([(URL_FIELD.to_string(), json!(true))])),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields() {
        assert_eq!(
            field(
                "email",
                &json!({"type": "string", "title": "string with format email", "description": "Your email", "format": "email"}),
                true
            ),
            json!({"key": "email", "title": "Your email", "required": true, "type": "string", "format": "email"})
        );
        assert_eq!(
            field(
                "c",
                &json!({"type": "string", "enum": ["r", "g"], "enumNames": ["Red"]}),
                false
            ),
            json!({"key": "c", "title": "c", "type": "string",
                   "options": [{"value": "r", "label": "Red"}, {"value": "g", "label": "g"}], "custom": false})
        );
    }
}
