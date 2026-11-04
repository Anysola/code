//! JSON helpers matching JavaScript's `JSON.stringify` output.

use serde_json::Value;

/// `JSON.stringify(value)`.
pub fn compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// `JSON.stringify(value, null, 2)`.
pub fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}
