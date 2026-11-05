//! `mcp`: one document's servers in the native shape (legacy V1 entries migrated), merged across
//! documents, then the defaults for known servers and the `mcp.use` policy.

use serde_json::{Map, Value, json};

use crate::permission::policy::Policies;

/// The `mcp` key of one document, normalized: `(timeout, servers)`.
fn normalize(document: &Map<String, Value>) -> (Map<String, Value>, Map<String, Value>) {
    let mut timeout = Map::new();
    if let Some(value) = document
        .get("experimental")
        .and_then(|e| e.get("mcp_timeout"))
        .filter(|v| positive(v))
    {
        // V1 never applied this to connecting, unlike a per-server timeout.
        timeout.insert("catalog".into(), value.clone());
        timeout.insert("execution".into(), value.clone());
    }
    let mut legacy = Map::new();
    let mut native = Map::new();
    let Some(Value::Object(mcp)) = document.get("mcp") else {
        return (timeout, native);
    };
    for (name, value) in mcp {
        let typed = matches!(
            value.get("type").and_then(Value::as_str),
            Some("local" | "remote")
        );
        if value.get("type").is_none() && value.get("enabled").is_some_and(Value::is_boolean) {
            crate::log::warn(
                "omitted enabled-only legacy MCP entry",
                &[("path", &format!("mcp.{name}"))],
            );
            continue;
        }
        match (name.as_str(), value) {
            ("servers", Value::Object(servers)) if !typed => {
                for (key, server) in servers {
                    let mut server = server.clone();
                    if let Some(n) = server.get("timeout").filter(|t| t.is_number()).cloned() {
                        server["timeout"] = all_timeouts(&n);
                    }
                    if valid(&server) {
                        native.insert(key.clone(), server);
                    }
                }
            }
            ("timeout", _) if !typed => {
                let value = if value.is_number() {
                    all_timeouts(value)
                } else {
                    value.clone()
                };
                if let Value::Object(fields) = value {
                    for key in ["startup", "catalog", "execution"] {
                        if let Some(v) = fields.get(key).filter(|v| positive(v)) {
                            timeout.insert(key.into(), v.clone());
                        }
                    }
                }
            }
            _ => {
                if let Some(server) = migrate_v1(value).filter(valid) {
                    legacy.insert(name.clone(), server);
                }
            }
        }
    }
    // Native entries win over legacy ones of the same name.
    legacy.extend(native);
    (timeout, legacy)
}

fn positive(value: &Value) -> bool {
    value.as_u64().is_some_and(|n| n > 0)
}

fn all_timeouts(ms: &Value) -> Value {
    json!({"startup": ms, "catalog": ms, "execution": ms})
}

fn valid(server: &Value) -> bool {
    match server.get("type").and_then(Value::as_str) {
        Some("local") => server
            .get("command")
            .and_then(Value::as_array)
            .is_some_and(|c| !c.is_empty() && c.iter().all(Value::is_string)),
        Some("remote") => server.get("url").is_some_and(Value::is_string),
        _ => false,
    }
}

fn migrate_v1(v1: &Value) -> Option<Value> {
    let v1 = v1.as_object()?;
    let mut server = Map::new();
    let kind = v1.get("type")?.as_str()?;
    server.insert("type".into(), json!(kind));
    let copy: &[&str] = if kind == "local" {
        &["command", "cwd", "environment"]
    } else {
        &["url", "headers"]
    };
    for key in copy {
        if let Some(v) = v1.get(*key) {
            server.insert((*key).into(), v.clone());
        }
    }
    if kind == "remote" {
        match v1.get("oauth") {
            Some(Value::Object(oauth)) => {
                let renamed: Map<String, Value> = [
                    ("clientId", "client_id"),
                    ("clientSecret", "client_secret"),
                    ("scope", "scope"),
                    ("callbackPort", "callback_port"),
                    ("redirectUri", "redirect_uri"),
                ]
                .iter()
                .filter_map(|(from, to)| oauth.get(*from).map(|v| ((*to).to_string(), v.clone())))
                .collect();
                server.insert("oauth".into(), Value::Object(renamed));
            }
            Some(Value::Bool(false)) => {
                server.insert("oauth".into(), json!(false));
            }
            _ => {}
        }
    }
    if let Some(enabled) = v1.get("enabled").and_then(Value::as_bool) {
        server.insert("disabled".into(), json!(!enabled));
    }
    if let Some(codemode) = v1.get("codemode") {
        server.insert("codemode".into(), codemode.clone());
    }
    if let Some(timeout) = v1.get("timeout").filter(|t| positive(t)) {
        server.insert("timeout".into(), all_timeouts(timeout));
    }
    Some(Value::Object(server))
}

/// Every document's `mcp`, merged in load order.
#[derive(Debug, Clone, Default)]
pub struct Merged {
    /// Global timeouts, later documents overriding single fields.
    timeout: Map<String, Value>,
    /// A later document's server replaces an earlier one of the same name.
    servers: Map<String, Value>,
}

impl Merged {
    pub(super) fn apply(&mut self, document: &Map<String, Value>) {
        let (timeout, servers) = normalize(document);
        self.timeout.extend(timeout);
        self.servers.extend(servers);
    }

    /// The servers to run: each with the global timeouts under its own, the defaults for known
    /// servers applied, and those the `mcp.use` policy denies left out.
    pub(super) fn resolve(&self, policies: &Policies) -> Vec<(String, Value)> {
        self.servers
            .iter()
            .filter(|(name, _)| policies.allows("mcp.use", name))
            .map(|(name, server)| {
                let mut server = server.clone();
                let mut timeout = self.timeout.clone();
                if let Some(Value::Object(own)) = server.get("timeout") {
                    timeout.extend(own.clone());
                }
                server["timeout"] = Value::Object(timeout);
                apply_defaults(&mut server);
                (name.clone(), server)
            })
            .collect()
    }
}

/// Servers that are Code Mode themselves get `codemode:false`; PostHog is asked for plain tools.
fn apply_defaults(server: &mut Value) {
    if server.get("codemode").is_some() {
        return;
    }
    if server["type"] == "local" {
        if server["command"][0] == "executor" && server["command"][1] == "mcp" {
            server["codemode"] = json!(false);
        }
        return;
    }
    let Some(url) = server["url"].as_str().and_then(|u| url::Url::parse(u).ok()) else {
        return;
    };
    let host = url.host_str().unwrap_or_default();
    let path = url.path().trim_end_matches('/');
    let segments: Vec<&str> = path.split('/').collect();
    if url.scheme() == "https"
        && host == "executor.sh"
        && url.port().is_none()
        && segments.len() == 3
        && !segments[1].is_empty()
        && segments[2] == "mcp"
    {
        server["codemode"] = json!(false);
    }
    let posthog = matches!(
        host,
        "mcp.posthog.com" | "mcp-eu.posthog.com" | "mcp.us.posthog.com" | "mcp.eu.posthog.com"
    );
    let has_header = server["headers"].as_object().is_some_and(|h| {
        h.keys()
            .any(|k| k.eq_ignore_ascii_case("x-posthog-mcp-mode"))
    });
    if posthog && !url.query_pairs().any(|(k, _)| k == "mode") && !has_header {
        if !server["headers"].is_object() {
            server["headers"] = json!({});
        }
        server["headers"]["x-posthog-mcp-mode"] = json!("tools");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merged(documents: &[Value]) -> Vec<(String, Value)> {
        let mut merged = Merged::default();
        for document in documents {
            merged.apply(document.as_object().unwrap());
        }
        merged.resolve(&Policies::default())
    }

    #[test]
    fn legacy_entries_are_migrated_and_native_wins() {
        let servers = merged(&[json!({
            "experimental": {"mcp_timeout": 900},
            "mcp": {
                "old": {"type": "remote", "url": "https://h/mcp", "enabled": false, "timeout": 50,
                        "oauth": {"clientId": "c", "callbackPort": 9}},
                "both": {"type": "local", "command": ["a"]},
                "flag": {"enabled": true},
                "servers": {"both": {"type": "local", "command": ["b"], "timeout": 7}},
                "timeout": {"startup": 10}
            }
        })]);
        let names: Vec<&str> = servers.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["old", "both"]);
        assert_eq!(
            servers[0].1,
            json!({"type": "remote", "url": "https://h/mcp",
                   "oauth": {"client_id": "c", "callback_port": 9}, "disabled": true,
                   "timeout": {"catalog": 50, "execution": 50, "startup": 50}})
        );
        assert_eq!(servers[1].1["command"], json!(["b"]));
        assert_eq!(
            servers[1].1["timeout"],
            json!({"catalog": 7, "execution": 7, "startup": 7})
        );
    }

    #[test]
    fn documents_merge_timeouts_and_replace_servers() {
        let servers = merged(&[
            json!({"mcp": {"timeout": {"startup": 1, "catalog": 2},
                           "servers": {"a": {"type": "local", "command": ["x"], "environment": {"K": "1"}}}}}),
            json!({"mcp": {"timeout": {"catalog": 3},
                           "servers": {"a": {"type": "local", "command": ["y"], "timeout": {"startup": 9}}}}}),
        ]);
        assert_eq!(
            servers[0].1,
            json!({"type": "local", "command": ["y"], "timeout": {"startup": 9, "catalog": 3}})
        );
    }

    #[test]
    fn defaults_and_policy() {
        let mut merged = Merged::default();
        merged.apply(
            json!({"mcp": {"servers": {
                "ex": {"type": "local", "command": ["executor", "mcp"]},
                "exr": {"type": "remote", "url": "https://executor.sh/team/mcp/"},
                "ph": {"type": "remote", "url": "https://mcp.posthog.com/mcp"},
                "kept": {"type": "remote", "url": "https://executor.sh/team/mcp", "codemode": true},
                "denied": {"type": "remote", "url": "https://d/mcp"}
            }}})
            .as_object()
            .unwrap(),
        );
        let policies = Policies(
            crate::permission::policy::Policy::parse(
                &json!({"action": "mcp.use", "resource": "denied", "effect": "deny"}),
            )
            .into_iter()
            .collect(),
        );
        let servers: Map<String, Value> = merged.resolve(&policies).into_iter().collect();
        assert_eq!(servers["ex"]["codemode"], json!(false));
        assert_eq!(servers["exr"]["codemode"], json!(false));
        assert_eq!(
            servers["ph"]["headers"],
            json!({"x-posthog-mcp-mode": "tools"})
        );
        assert_eq!(servers["kept"]["codemode"], json!(true));
        assert!(!servers.contains_key("denied"));
    }
}
