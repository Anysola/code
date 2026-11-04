//! Managed policy statements (`policies` in config): allow/deny decisions that rules cannot
//! override. The statement deciding an action on a resource is the last match in [`Policies`],
//! which lists earlier-loaded documents last so they win, managed documents last of all.

use serde_json::Value;

use super::matches;

pub const BLOCKED: &str = "Blocked by configuration policy";

/// `provider.use`, `tool.use` (`<tool>:<resource>`), `integration.connect`, `integration.use`,
/// `mcp.use`, `skill.use`, `plugin.use`.
const ACTIONS: &[&str] = &[
    "provider.use",
    "tool.use",
    "integration.connect",
    "integration.use",
    "mcp.use",
    "skill.use",
    "plugin.use",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub action: String,
    pub resource: String,
    pub allow: bool,
    /// Shown when this statement denies.
    pub message: String,
}

impl Policy {
    /// One statement as written; `integration.use` with an `mcp:`/`skill:`/`plugin:` resource is the
    /// legacy spelling of the newer actions.
    pub fn parse(value: &Value) -> Option<Policy> {
        let action = value.get("action")?.as_str()?;
        let resource = value.get("resource")?.as_str()?;
        let allow = match value.get("effect")?.as_str()? {
            "allow" => true,
            "deny" => false,
            _ => return None,
        };
        if !ACTIONS.contains(&action) {
            return None;
        }
        let legacy = (action == "integration.use")
            .then(|| resource.split_once(':'))
            .flatten()
            .and_then(|(prefix, rest)| {
                let action = match prefix {
                    "mcp" => "mcp.use",
                    "skill" => "skill.use",
                    "plugin" => "plugin.use",
                    _ => return None,
                };
                Some((action, rest))
            });
        let (action, resource) = legacy.unwrap_or((action, resource));
        Some(Policy {
            action: action.into(),
            resource: resource.into(),
            allow,
            message: BLOCKED.into(),
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policies(pub Vec<Policy>);

impl Policies {
    fn find(&self, action: &str, resource: &str) -> Option<&Policy> {
        self.0
            .iter()
            .rev()
            .find(|p| p.action == action && matches(resource, &p.resource))
    }

    /// Whether `action` (`mcp.use`, `skill.use`, …) is allowed on `resource`; allowed by default.
    pub fn allows(&self, action: &str, resource: &str) -> bool {
        self.find(action, resource).is_none_or(|p| p.allow)
    }

    /// The message of the first `tool.use` statement denying the tool call on one of `resources`.
    pub fn tool_denial(&self, tool: &str, resources: &[String]) -> Option<String> {
        resources
            .iter()
            .filter_map(|resource| self.find("tool.use", &format!("{tool}:{resource}")))
            .find(|p| !p.allow)
            .map(|p| p.message.clone())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn policies(list: Value) -> Policies {
        Policies(
            list.as_array()
                .unwrap()
                .iter()
                .filter_map(Policy::parse)
                .collect(),
        )
    }

    #[test]
    fn last_match_decides() {
        let p = policies(json!([
            {"action": "tool.use", "resource": "shell:*", "effect": "deny"},
            {"action": "tool.use", "resource": "shell:git *", "effect": "allow"},
        ]));
        assert_eq!(p.tool_denial("shell", &["git status".into()]), None);
        assert_eq!(
            p.tool_denial("shell", &["git status".into(), "rm -rf x".into()])
                .as_deref(),
            Some(BLOCKED)
        );
        assert_eq!(p.tool_denial("read", &["x".into()]), None);
    }

    #[test]
    fn legacy_integration_prefixes() {
        let p = policies(json!([
            {"action": "integration.use", "resource": "mcp:github", "effect": "deny"},
            {"action": "bogus", "resource": "*", "effect": "deny"},
            {"action": "skill.use", "resource": "*", "effect": "maybe"},
        ]));
        assert_eq!(p.0.len(), 1);
        assert!(!p.allows("mcp.use", "github"));
        assert!(p.allows("mcp.use", "linear"));
    }
}
