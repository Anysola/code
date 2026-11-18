//! Claude Code's tool vocabulary: existing hook scripts match `Bash` and read
//! `.tool_input.file_path`, ours are `shell` and `path`. The tool's own keys stay; the Claude Code
//! keys are added as copies.

use serde_json::{Value, json};

/// The name hooks see for one of our tools (MCP tools are named by [`mcp_name`]).
pub fn claude_name(anysola: &str) -> String {
    match anysola {
        "shell" => "Bash",
        "edit" => "Edit",
        "write" => "Write",
        "read" => "Read",
        "glob" => "Glob",
        "grep" => "Grep",
        "webfetch" => "WebFetch",
        "websearch" => "WebSearch",
        "subagent" => "Agent",
        "question" => "AskUserQuestion",
        other => other,
    }
    .to_string()
}

/// `mcp__<server>__<tool>`, Claude Code's name for an MCP server's tool.
pub fn mcp_name(server: &str, tool: &str) -> String {
    format!("mcp__{server}__{tool}")
}

/// How a call's tool resolves its `path`, so `file_path` names the file the tool touches.
pub(crate) fn resolver(ctx: &crate::tool::ToolContext) -> impl Fn(&str) -> String + '_ {
    |path| {
        crate::tool::access::resolve_path(ctx, path)
            .display()
            .to_string()
    }
}

/// The input hooks see: ours with Claude Code's keys added (`file_path` is `path` made absolute
/// by `resolve`).
pub fn input(anysola: &str, input: &Value, resolve: impl Fn(&str) -> String) -> Value {
    let Value::Object(map) = input else {
        return input.clone();
    };
    let mut out = map.clone();
    let mut copy = |from: &str, to: &str| {
        if let Some(value) = map.get(from)
            && !out.contains_key(to)
        {
            out.insert(to.to_string(), value.clone());
        }
    };
    match anysola {
        "shell" => copy("background", "run_in_background"),
        "edit" => {
            copy("oldString", "old_string");
            copy("newString", "new_string");
            copy("replaceAll", "replace_all");
        }
        "grep" => copy("include", "glob"),
        "subagent" => copy("agent", "subagent_type"),
        _ => {}
    }
    if matches!(anysola, "edit" | "write" | "read")
        && let Some(path) = map.get("path").and_then(Value::as_str)
        && !out.contains_key("file_path")
    {
        out.insert("file_path".into(), json!(resolve(path)));
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn input(anysola: &str, value: &Value, cwd: &Path) -> Value {
        super::input(anysola, value, |path| {
            if Path::new(path).is_absolute() {
                path.to_string()
            } else {
                cwd.join(path).display().to_string()
            }
        })
    }

    #[test]
    fn file_path_is_the_path_the_tool_resolves() {
        let dir = tempfile::tempdir().unwrap();
        let test = crate::tool::test_support::context(dir.path());
        let resolve = resolver(&test.ctx);
        assert_eq!(
            resolve("src/../a.rs"),
            dir.path().join("a.rs").display().to_string()
        );
        assert_eq!(
            resolve("~/notes.txt"),
            test.ctx.paths.home.join("notes.txt").display().to_string()
        );
        let mapped = super::input("edit", &json!({"path": "~/x"}), &resolve);
        assert_eq!(mapped["file_path"], resolve("~/x"));
    }

    #[test]
    fn names_map_to_claude_code_and_the_rest_keep_theirs() {
        let pairs = [
            ("shell", "Bash"),
            ("edit", "Edit"),
            ("write", "Write"),
            ("read", "Read"),
            ("glob", "Glob"),
            ("grep", "Grep"),
            ("webfetch", "WebFetch"),
            ("websearch", "WebSearch"),
            ("subagent", "Agent"),
            ("question", "AskUserQuestion"),
            ("execute", "execute"),
            ("skill", "skill"),
            ("todowrite", "todowrite"),
        ];
        for (ours, theirs) in pairs {
            assert_eq!(claude_name(ours), theirs);
        }
        assert_eq!(
            mcp_name("github", "list_issues"),
            "mcp__github__list_issues"
        );
    }

    #[test]
    fn inputs_gain_claude_code_keys_as_copies() {
        let cwd = if cfg!(windows) {
            Path::new("C:\\work")
        } else {
            Path::new("/work")
        };
        let edit = input(
            "edit",
            &json!({"path": "src/a.rs", "oldString": "a", "newString": "b", "replaceAll": true}),
            cwd,
        );
        assert_eq!(edit["path"], "src/a.rs", "our keys stay");
        assert_eq!(edit["old_string"], "a");
        assert_eq!(edit["new_string"], "b");
        assert_eq!(edit["replace_all"], true);
        assert_eq!(
            edit["file_path"],
            cwd.join("src/a.rs").display().to_string()
        );
        let absolute = cwd.join("b.txt").display().to_string();
        let write = input("write", &json!({"path": absolute, "content": "x"}), cwd);
        assert_eq!(write["file_path"], json!(absolute));
        assert_eq!(
            input("read", &json!({"path": "r"}), cwd)["file_path"],
            json!(cwd.join("r").display().to_string())
        );
        let shell = input("shell", &json!({"command": "ls", "background": true}), cwd);
        assert_eq!(shell["run_in_background"], true);
        assert_eq!(shell["command"], "ls");
        assert_eq!(
            input("grep", &json!({"pattern": "x", "include": "*.rs"}), cwd)["glob"],
            "*.rs"
        );
        assert_eq!(
            input("subagent", &json!({"agent": "explore"}), cwd)["subagent_type"],
            "explore"
        );
        assert_eq!(
            input("glob", &json!({"pattern": "*"}), cwd),
            json!({"pattern": "*"})
        );
        assert_eq!(input("execute", &json!("x"), cwd), json!("x"));
    }
}
