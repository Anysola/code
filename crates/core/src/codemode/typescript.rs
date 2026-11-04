//! JSON Schema rendered as the TypeScript-like signatures of the Code Mode catalog and `search`
//! results, as 0.5.1's `tool-schema.ts` renders them (pretty form: one property per line with
//! JSDoc for descriptions and constraints).

use std::collections::HashSet;

use serde_json::{Map, Value};

const MAX_RENDER_DEPTH: usize = 8;
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
const ANNOTATIONS: [&str; 6] = [
    "title",
    "description",
    "default",
    "examples",
    "readOnly",
    "writeOnly",
];

/// `tools.a.b` for identifier segments, `tools.a["b-c"]` otherwise.
pub fn tool_expression(path: &str) -> String {
    let mut out = String::from("tools");
    for segment in path.split('.') {
        if is_identifier(segment) {
            out.push('.');
            out.push_str(segment);
        } else {
            out.push('[');
            out.push_str(&crate::json::compact(&Value::String(segment.into())));
            out.push(']');
        }
    }
    out
}

pub fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// `tools.ns.tool(<input>): Promise<<output>>`; a tool without input properties takes `()`, and
/// one without an output schema returns its text (`string | null`).
pub fn signature(path: &str, input: &Value, output: Option<&Value>) -> String {
    let input = render(input);
    let arguments = if input == "{}" {
        "()".to_string()
    } else {
        format!("({input})")
    };
    let output = output.map_or_else(|| "string | null".to_string(), render);
    format!("{}{arguments}: Promise<{output}>", tool_expression(path))
}

/// One schema as a pretty TypeScript type.
pub fn render(schema: &Value) -> String {
    Renderer::default().schema(schema, 0, &HashSet::new())
}

#[derive(Default, Clone)]
struct Renderer {
    definitions: Map<String, Value>,
}

fn definition_name(reference: &str) -> Option<&str> {
    let rest = reference.strip_prefix("#/")?;
    let (group, name) = rest.split_once('/')?;
    (matches!(group, "$defs" | "definitions") && !name.contains('/')).then_some(name)
}

fn literal(value: &Value) -> String {
    crate::json::compact(value)
}

fn render_key(name: &str) -> String {
    if is_identifier(name) {
        name.to_string()
    } else {
        literal(&Value::String(name.into()))
    }
}

fn type_is(schema: &Value, kind: &str) -> bool {
    schema.get("type").and_then(Value::as_str) == Some(kind)
}

/// Effect's encoding of an unchecked number's non-finite values.
fn number_sentinel(schema: &Value) -> bool {
    type_is(schema, "string")
        && schema
            .get("enum")
            .and_then(Value::as_array)
            .is_some_and(|values| {
                !values.is_empty()
                    && values
                        .iter()
                        .all(|v| matches!(v.as_str(), Some("NaN" | "Infinity" | "-Infinity")))
            })
}

/// Effect's marker for a struct with no properties: `{ not: { type: "null" } }`.
fn empty_struct(schema: &Value) -> bool {
    let Some(map) = schema.as_object() else {
        return false;
    };
    map.get("not")
        .and_then(Value::as_object)
        .is_some_and(|not| not.len() == 1 && not.get("type") == Some(&Value::from("null")))
        && map
            .keys()
            .all(|k| k == "not" || ANNOTATIONS.contains(&k.as_str()))
}

fn intersection(members: Vec<String>) -> String {
    let concrete: Vec<String> = members.into_iter().filter(|m| m != "unknown").collect();
    match concrete.len() {
        0 => "unknown".into(),
        1 => concrete.into_iter().next().unwrap_or_default(),
        _ => concrete
            .iter()
            .map(|m| {
                if m.contains(" | ") {
                    format!("({m})")
                } else {
                    m.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" & "),
    }
}

fn without(schema: &Value, keys: &[&str]) -> Value {
    let mut map = schema.as_object().cloned().unwrap_or_default();
    for key in keys {
        map.remove(*key);
    }
    Value::Object(map)
}

/// A number as JavaScript prints it in a template string.
fn js_number(value: &Value) -> String {
    match value.as_f64() {
        Some(n) if n.fract() == 0.0 && n.abs() < 1e21 => format!("{n:.0}"),
        Some(n) => n.to_string(),
        None => value.to_string(),
    }
}

fn doc_tags(schema: &Value) -> Vec<String> {
    let mut tags = Vec::new();
    let get = |key: &str| schema.get(key);
    let number = |key: &str| get(key).filter(|v| v.is_number());
    if get("deprecated") == Some(&Value::Bool(true)) {
        tags.push("@deprecated".into());
    }
    if let Some(default) = get("default") {
        tags.push(format!("@default {}", literal(default)));
    }
    if let Some(format) = get("format").and_then(Value::as_str) {
        tags.push(format!("@format {format}"));
    }
    let integer = type_is(schema, "integer");
    if integer {
        tags.push("@integer".into());
    }
    if let Some(min) =
        number("minimum").filter(|m| !(integer && m.as_f64() == Some(-MAX_SAFE_INTEGER)))
    {
        tags.push(format!("@minimum {}", js_number(min)));
    }
    if let Some(max) =
        number("maximum").filter(|m| !(integer && m.as_f64() == Some(MAX_SAFE_INTEGER)))
    {
        tags.push(format!("@maximum {}", js_number(max)));
    }
    for key in [
        "exclusiveMinimum",
        "exclusiveMaximum",
        "multipleOf",
        "minLength",
        "maxLength",
    ] {
        if let Some(value) = number(key) {
            tags.push(format!("@{key} {}", js_number(value)));
        }
    }
    if let Some(pattern) = get("pattern").and_then(Value::as_str) {
        tags.push(format!("@pattern {pattern}"));
    }
    for key in ["minItems", "maxItems"] {
        if let Some(value) = number(key) {
            tags.push(format!("@{key} {}", js_number(value)));
        }
    }
    if get("uniqueItems") == Some(&Value::Bool(true)) {
        tags.push("@uniqueItems true".into());
    }
    tags
}

fn doc_lines(schema: &Value, width: usize) -> Vec<String> {
    let summary = doc_tags(schema).join(" ");
    let description = schema
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut lines: Vec<String> = description
        .split('\n')
        .map(|l| l.trim_end().to_string())
        .collect();
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    if lines.len() == 1 && !summary.is_empty() {
        let first = &lines[0];
        let inline = format!(
            "{first}{} {summary}",
            if first.ends_with('.') { "" } else { "." }
        );
        if width + inline.chars().count() + 7 <= 120 {
            return vec![inline];
        }
    }
    if !summary.is_empty() {
        lines.push(summary);
    }
    lines
}

/// The JSDoc block above a property; `*/` in model-provided text is neutralized.
fn jsdoc(schema: &Value, pad: &str) -> String {
    let mut content = doc_lines(schema, pad.len());
    let types: Vec<&str> = match schema.get("type") {
        Some(Value::String(t)) => vec![t.as_str()],
        Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    let mut append = |label: &str, child: &Value| {
        for (index, line) in doc_lines(child, pad.len() + label.len() + 2)
            .into_iter()
            .enumerate()
        {
            if index == 0 {
                content.push(format!("{label}: {line}"));
            } else if line.is_empty() {
                content.push(String::new());
            } else {
                content.push(format!("  {line}"));
            }
        }
    };
    if types.contains(&"array")
        && let Some(items) = schema.get("items").filter(|i| i.is_object())
    {
        append("Each item", items);
    }
    if (types.contains(&"object") || schema.get("properties").is_some())
        && let Some(additional) = schema.get("additionalProperties").filter(|a| a.is_object())
    {
        let has_properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .is_some_and(|p| !p.is_empty());
        let label = if has_properties {
            "Each additional value"
        } else {
            "Each value"
        };
        append(label, additional);
    }
    if content.is_empty() {
        return String::new();
    }
    let escaped: Vec<String> = content.iter().map(|l| l.replace("*/", "* /")).collect();
    if escaped.len() == 1 && pad.len() + escaped[0].chars().count() + 7 <= 120 {
        return format!("{pad}/** {} */\n", escaped[0]);
    }
    let body: Vec<String> = escaped
        .iter()
        .map(|l| {
            if l.is_empty() {
                format!("{pad} *")
            } else {
                format!("{pad} * {l}")
            }
        })
        .collect();
    format!("{pad}/**\n{}\n{pad} */\n", body.join("\n"))
}

impl Renderer {
    fn has_unresolved_ref(&self, schema: &Value, seen: &HashSet<String>, depth: usize) -> bool {
        if depth > 64 {
            return false;
        }
        if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            let Some(name) = definition_name(reference) else {
                return true;
            };
            let Some(target) = self.definitions.get(name) else {
                return true;
            };
            if seen.contains(name) {
                return true;
            }
            let mut next = seen.clone();
            next.insert(name.to_string());
            if self.has_unresolved_ref(target, &next, depth + 1) {
                return true;
            }
        }
        let mut children: Vec<&Value> = Vec::new();
        for key in ["anyOf", "oneOf", "allOf"] {
            if let Some(list) = schema.get(key).and_then(Value::as_array) {
                children.extend(list);
            }
        }
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            children.extend(properties.values());
        }
        children.extend(schema.get("items"));
        children.extend(schema.get("additionalProperties").filter(|a| a.is_object()));
        children
            .into_iter()
            .any(|child| self.has_unresolved_ref(child, seen, depth + 1))
    }

    fn schema(&self, schema: &Value, depth: usize, seen: &HashSet<String>) -> String {
        if depth > MAX_RENDER_DEPTH {
            return "unknown".into();
        }
        let local = ["definitions", "$defs"]
            .iter()
            .filter_map(|k| schema.get(*k).and_then(Value::as_object))
            .collect::<Vec<_>>();
        let nested;
        let this = if local.is_empty() {
            self
        } else {
            let mut definitions = self.definitions.clone();
            for group in local {
                definitions.extend(group.clone());
            }
            nested = Renderer { definitions };
            &nested
        };
        this.body(schema, depth, seen)
    }

    fn body(&self, schema: &Value, depth: usize, seen: &HashSet<String>) -> String {
        if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            let Some(name) = definition_name(reference) else {
                return "unknown".into();
            };
            let Some(target) = self.definitions.get(name) else {
                return "unknown".into();
            };
            if seen.contains(name) {
                return "unknown".into();
            }
            let mut next = seen.clone();
            next.insert(name.to_string());
            return intersection(vec![
                self.schema(target, depth, &next),
                self.schema(&without(schema, &["$ref"]), depth + 1, seen),
            ]);
        }
        if let Some(constant) = schema.get("const") {
            return literal(constant);
        }
        if let Some(options) = schema.get("enum").and_then(Value::as_array) {
            return options.iter().map(literal).collect::<Vec<_>>().join(" | ");
        }
        let listed = schema
            .get("anyOf")
            .or_else(|| schema.get("oneOf"))
            .and_then(Value::as_array);
        if let Some(listed) = listed {
            let has_number = listed.iter().any(|i| type_is(i, "number"));
            let alternatives: Vec<&Value> = listed
                .iter()
                .filter(|i| !(has_number && number_sentinel(i)))
                .collect();
            if alternatives.iter().any(|i| type_is(i, "number"))
                && alternatives
                    .iter()
                    .all(|i| type_is(i, "number") || number_sentinel(i))
            {
                return "number".into();
            }
            if alternatives.len() == 2
                && type_is(alternatives[0], "object")
                && alternatives[0].get("properties").is_none()
                && type_is(alternatives[1], "array")
                && alternatives[1].get("items").is_none()
            {
                return "{}".into();
            }
            let members: Vec<String> = alternatives
                .iter()
                .map(|i| self.schema(i, depth + 1, seen))
                .collect();
            if members.iter().any(|m| m == "unknown") {
                return "unknown".into();
            }
            return intersection(vec![
                members.join(" | "),
                self.schema(&without(schema, &["anyOf", "oneOf"]), depth + 1, seen),
            ]);
        }
        if let Some(all) = schema.get("allOf").and_then(Value::as_array) {
            if all
                .iter()
                .any(|i| self.has_unresolved_ref(i, &HashSet::new(), 0))
            {
                return "unknown".into();
            }
            let mut members = vec![self.schema(&without(schema, &["allOf"]), depth + 1, seen)];
            members.extend(all.iter().map(|i| self.schema(i, depth + 1, seen)));
            return intersection(members);
        }
        if let Some(Value::Array(types)) = schema.get("type") {
            return types
                .iter()
                .map(|t| {
                    let mut single = schema.clone();
                    single["type"] = t.clone();
                    self.schema(&single, depth + 1, seen)
                })
                .collect::<Vec<_>>()
                .join(" | ");
        }
        match schema.get("type").and_then(Value::as_str) {
            Some("string") => return "string".into(),
            Some("number" | "integer") => return "number".into(),
            Some("boolean") => return "boolean".into(),
            Some("null") => return "null".into(),
            Some("array") => {
                let items = schema
                    .get("items")
                    .cloned()
                    .unwrap_or(Value::Object(Map::new()));
                return format!("Array<{}>", self.schema(&items, depth + 1, seen));
            }
            _ => {}
        }
        if type_is(schema, "object") || schema.get("properties").is_some() {
            return self.object(schema, depth, seen);
        }
        if empty_struct(schema) {
            return "{}".into();
        }
        "unknown".into()
    }

    fn object(&self, schema: &Value, depth: usize, seen: &HashSet<String>) -> String {
        let required: Vec<&str> = schema
            .get("required")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let empty = Map::new();
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let index = schema
            .get("additionalProperties")
            .filter(|a| a.is_object())
            .map(|a| self.schema(a, depth + 1, seen));
        if properties.is_empty() && index.is_none() {
            return "{}".into();
        }
        let pad = "  ".repeat(depth + 1);
        let mut lines: Vec<String> = properties
            .iter()
            .map(|(name, value)| {
                format!(
                    "{}{pad}{}{}: {},",
                    jsdoc(value, &pad),
                    render_key(name),
                    if required.contains(&name.as_str()) {
                        ""
                    } else {
                        "?"
                    },
                    self.schema(value, depth + 1, seen)
                )
            })
            .collect();
        if let Some(index) = index {
            lines.push(format!("{pad}[key: string]: {index},"));
        }
        format!("{{\n{}\n{}}}", lines.join("\n"), "  ".repeat(depth))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn expressions() {
        assert_eq!(tool_expression("opencode.models"), "tools.opencode.models");
        assert_eq!(tool_expression("my-srv.get"), "tools[\"my-srv\"].get");
    }

    #[test]
    fn empty_input_takes_no_arguments() {
        let input = json!({"type": "object", "properties": {}, "additionalProperties": false});
        assert_eq!(
            signature("a.b", &input, None),
            "tools.a.b(): Promise<string | null>"
        );
        assert_eq!(
            signature(
                "a.b",
                &json!({"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}),
                Some(&json!({}))
            ),
            "tools.a.b({\n  text: string,\n}): Promise<unknown>"
        );
    }

    #[test]
    fn index_signatures_and_unions() {
        let schema = json!({
            "type": "object",
            "properties": {
                "files": {"type": "object", "additionalProperties": {"type": "string"}},
                "next": {"anyOf": [{"type": "object", "properties": {"offset": {"type": "integer", "minimum": 0}}, "required": ["offset"]}, {"type": "null"}]},
                "weird-key": {"type": ["string", "null"]}
            },
            "required": ["files", "next"]
        });
        assert_eq!(
            render(&schema),
            "{\n  files: {\n    [key: string]: string,\n  },\n  next: {\n      /** @integer @minimum 0 */\n      offset: number,\n    } | null,\n  \"weird-key\"?: string | null,\n}"
        );
    }
}
