//! Permission rules and the gate tools ask through.
//!
//! A ruleset is an ordered list; the last rule matching both action and resource wins and no match
//! means `ask`, except that a rule from a project can never weaken a denial: when the rules from
//! everywhere else ([`Rule::project`] false) deny on their own, the answer is deny. The interactive side (asking, replies, saved "always" rules) lives behind
//! [`PermissionGate`], implemented by the session layer. Managed [`policy`] statements can deny
//! what rules allow.

pub mod policy;
mod wildcard;

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use wildcard::matches;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub action: String,
    pub resource: String,
    pub effect: Effect,
    /// Written in a project's config (its documents, their `agents`, its agent markdown files).
    /// Never read from or written to JSON: only the loader that found the rule sets it.
    #[serde(skip)]
    pub project: bool,
}

impl Rule {
    pub fn new(action: &str, resource: &str, effect: Effect) -> Self {
        Rule {
            action: action.into(),
            resource: resource.into(),
            effect,
            project: false,
        }
    }

    /// The same rule, marked as coming from a project.
    pub fn from_project(self) -> Self {
        Rule {
            project: true,
            ..self
        }
    }
}

/// The last rule matching both action and resource decides (no match: `ask`); but when the
/// last match is a project's rule and the rules without the project's would deny, it denies.
pub fn evaluate(action: &str, resource: &str, rules: &[Rule]) -> Effect {
    let matching =
        |rule: &&Rule| matches(action, &rule.action) && matches(resource, &rule.resource);
    let Some(last) = rules.iter().rev().find(matching) else {
        return Effect::Ask;
    };
    let own_denial = || {
        rules
            .iter()
            .rev()
            .filter(|rule| !rule.project)
            .find(matching)
            .is_some_and(|rule| rule.effect == Effect::Deny)
    };
    if last.project && last.effect != Effect::Deny && own_denial() {
        return Effect::Deny;
    }
    last.effect
}

/// A tool is left out of the request when the last rule matching its action denies `*`, or the
/// last such rule outside the project's does.
pub fn wholly_disabled(action: &str, rules: &[Rule]) -> bool {
    let denies_all = |rule: Option<&Rule>| {
        rule.is_some_and(|rule| rule.resource == "*" && rule.effect == Effect::Deny)
    };
    denies_all(
        rules
            .iter()
            .rev()
            .find(|rule| matches(action, &rule.action)),
    ) || denies_all(
        rules
            .iter()
            .rev()
            .find(|rule| !rule.project && matches(action, &rule.action)),
    )
}

#[derive(Debug, Clone, Default)]
pub struct PermissionRequest {
    pub action: String,
    pub resources: Vec<String>,
    /// Patterns stored when the user answers "always".
    pub save: Vec<String>,
    pub metadata: Value,
}

/// The metadata key that marks a request for an MCP server's tool (`{"server", "tool"}`): its
/// action is the tool's own name, so only this tells it apart from the built-in tools'.
pub const MCP_TOOL_KEY: &str = "mcpTool";

impl PermissionRequest {
    /// Whether this asks to run an MCP server's tool.
    pub fn is_mcp_tool(&self) -> bool {
        self.metadata
            .get(MCP_TOOL_KEY)
            .is_some_and(|marker| !marker.is_null())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PermissionError {
    /// A rule or a policy denies the request; a policy says why.
    #[error("{}", reason.clone().unwrap_or_else(|| format!("Permission denied: {action}")))]
    Blocked {
        action: String,
        reason: Option<String>,
    },
    /// The user rejected without a message: the step ends.
    #[error("The user declined this tool call")]
    Declined,
    /// The user rejected with feedback for the model.
    #[error("{0}")]
    Corrected(String),
}

pub trait PermissionGate: Send + Sync {
    /// Resolves when the request is allowed (by rule or by the user).
    fn ask(&self, request: PermissionRequest) -> BoxFuture<'_, Result<(), PermissionError>>;
    /// True only when the rules already allow every resource; never asks.
    fn allows(&self, action: &str, resources: &[String]) -> bool;
    /// True when a rule denies any of the resources; never asks. Gates without rules deny nothing.
    fn denies(&self, _action: &str, _resources: &[String]) -> bool {
        false
    }
    /// This gate asking about every request even where the rules allow it (a PreToolUse hook's
    /// `"ask"`); denials stay denials. `None` when nobody can be asked.
    fn asking(&self) -> Option<std::sync::Arc<dyn PermissionGate>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(action: &str, resource: &str, effect: Effect) -> Rule {
        Rule::new(action, resource, effect)
    }

    fn project(action: &str, resource: &str, effect: Effect) -> Rule {
        Rule::new(action, resource, effect).from_project()
    }

    /// The 1.1.1 table: the user's side (built-ins, managed and user documents, the session's
    /// rules) against a project's rule placed after it.
    #[test]
    fn a_project_rule_never_weakens_a_denial() {
        use Effect::{Allow, Ask, Deny};
        let builtin = user("*", "*", Allow);
        let cases: [(&str, Vec<Rule>, Effect); 9] = [
            (
                "user deny, project ask",
                vec![
                    builtin.clone(),
                    user("shell", "*", Deny),
                    project("shell", "*", Ask),
                ],
                Deny,
            ),
            (
                "user deny, project allow (approved)",
                vec![
                    builtin.clone(),
                    user("shell", "*", Deny),
                    project("shell", "git *", Allow),
                ],
                Deny,
            ),
            (
                "user ask, project allow (approved)",
                vec![
                    builtin.clone(),
                    user("shell", "*", Ask),
                    project("shell", "*", Allow),
                ],
                Allow,
            ),
            (
                "no rule (ask by default), project allow",
                vec![project("shell", "*", Allow)],
                Allow,
            ),
            (
                "built-in ask, project allow",
                vec![
                    builtin.clone(),
                    user("shell", "*", Ask),
                    project("shell", "git *", Allow),
                ],
                Allow,
            ),
            (
                "user allow, project ask",
                vec![
                    builtin.clone(),
                    user("shell", "*", Allow),
                    project("shell", "*", Ask),
                ],
                Ask,
            ),
            (
                "user allow, project deny",
                vec![
                    builtin.clone(),
                    user("shell", "*", Allow),
                    project("shell", "*", Deny),
                ],
                Deny,
            ),
            (
                "user ask, no project rule",
                vec![user("shell", "*", Ask)],
                Ask,
            ),
            (
                "user allow, no project rule",
                vec![builtin.clone(), user("shell", "*", Allow)],
                Allow,
            ),
        ];
        for (case, rules, expected) in cases {
            assert_eq!(evaluate("shell", "git push", &rules), expected, "{case}");
        }
    }

    #[test]
    fn only_the_denial_s_own_reach_counts() {
        use Effect::{Allow, Ask, Deny};
        let rules = vec![
            user("*", "*", Allow),
            user("shell", "rm *", Deny),
            // The user's later allow lifts their own denial; the project's ask then applies.
            user("read", "*", Deny),
            user("read", "src/*", Allow),
            project("shell", "*", Ask),
            project("read", "*", Allow),
        ];
        assert_eq!(evaluate("shell", "rm -rf x", &rules), Deny);
        assert_eq!(evaluate("shell", "ls", &rules), Ask);
        assert_eq!(evaluate("read", "src/a.rs", &rules), Allow);
        assert_eq!(evaluate("read", "secret.txt", &rules), Deny);
        // A project may lift its own denial.
        let own = vec![
            user("*", "*", Allow),
            project("edit", "*", Deny),
            project("edit", "docs/*", Allow),
        ];
        assert_eq!(evaluate("edit", "docs/a.md", &own), Allow);
        assert_eq!(evaluate("edit", "a.rs", &own), Deny);
    }

    #[test]
    fn a_tool_the_user_denies_wholly_stays_out_whatever_follows() {
        use Effect::{Allow, Ask, Deny};
        let base = || vec![user("*", "*", Allow), user("webfetch", "*", Deny)];
        assert!(wholly_disabled("webfetch", &base()));
        for later in [
            project("webfetch", "*", Allow),
            project("webfetch", "https://x/*", Allow),
            project("webfetch", "*", Ask),
        ] {
            let mut rules = base();
            rules.push(later.clone());
            assert!(wholly_disabled("webfetch", &rules), "{later:?}");
        }
        // The user's own later rule still counts as before.
        let mut rules = base();
        rules.push(user("webfetch", "https://x/*", Allow));
        assert!(!wholly_disabled("webfetch", &rules));
        // A project's wholesale denial still disables.
        let rules = vec![user("*", "*", Allow), project("shell", "*", Deny)];
        assert!(wholly_disabled("shell", &rules));
        assert!(!wholly_disabled("read", &rules));
    }

    #[test]
    fn where_a_rule_came_from_is_never_read_or_written() {
        let rule: Rule = serde_json::from_value(
            serde_json::json!({"action": "shell", "resource": "*", "effect": "allow", "project": false}),
        )
        .unwrap();
        assert!(!rule.project);
        let written =
            serde_json::to_value(Rule::new("a", "b", Effect::Deny).from_project()).unwrap();
        assert_eq!(
            written,
            serde_json::json!({"action": "a", "resource": "b", "effect": "deny"})
        );
    }
}
