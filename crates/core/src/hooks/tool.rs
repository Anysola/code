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
        "notebook" => "NotebookEdit",
        other => other,
    }
    .to_string()
}

/// `mcp__<server>__<tool>`, Claude Code's name for an MCP server's tool.
pub fn mcp_name(server: &str, tool: &str) -> String {
    format!("mcp__{server}__{tool}")
}

/// How a call's tool resolves its `path`, then through symlinks as far as the path exists, so
/// `file_path` names the file the tool actually changes (a project's `notes.md` may link outside).
pub(crate) fn resolver(ctx: &crate::tool::ToolContext) -> impl Fn(&str) -> String + '_ {
    |path| {
        crate::config::real_path(&crate::tool::access::resolve_path(ctx, path))
            .display()
            .to_string()
    }
}

/// Our tools that take a file as `path`: hooks get it as `file_path`. Every file-writing tool
/// ([`crate::tool::writes_files`]) is one (a test holds the built-ins to it).
pub fn takes_file(anysola: &str) -> bool {
    matches!(anysola, "edit" | "write" | "read" | "notebook")
}

/// The input hooks see: ours with Claude Code's keys added (`file_path` is `path` made absolute
/// by `resolve`).
pub fn input(anysola: &str, input: &Value, resolve: impl Fn(&str) -> String) -> Value {
    let Value::Object(map) = input else {
        return input.clone();
    };
    let mut out = map.clone();
    let aliases: &[(&str, &str)] = match anysola {
        "shell" => &[("background", "run_in_background")],
        "edit" => &[
            ("oldString", "old_string"),
            ("newString", "new_string"),
            ("replaceAll", "replace_all"),
        ],
        "grep" => &[("include", "glob")],
        "subagent" => &[("agent", "subagent_type")],
        "notebook" => &[
            ("cellId", "cell_id"),
            ("source", "new_source"),
            ("cellType", "cell_type"),
            ("mode", "edit_mode"),
        ],
        _ => &[],
    };
    // Claude Code's keys always mirror ours, whatever the model put under those names: hooks
    // read what the tool runs with.
    for (from, to) in aliases {
        match map.get(*from) {
            Some(value) => out.insert(to.to_string(), value.clone()),
            None => out.remove(*to),
        };
    }
    if takes_file(anysola) {
        let resolved = map.get("path").and_then(Value::as_str).map(&resolve);
        // NotebookEdit's own key too, so Claude Code's notebook hooks read it.
        let keys: &[&str] = if anysola == "notebook" {
            &["file_path", "notebook_path"]
        } else {
            &["file_path"]
        };
        for key in keys {
            match &resolved {
                Some(path) => out.insert((*key).into(), json!(path)),
                None => out.remove(*key),
            };
        }
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
    fn every_writing_tool_gives_hooks_its_file() {
        let registry = crate::tool::registry::Registry::builtin();
        let tools = registry.active(&[]);
        // Every built-in tool with the `edit` permission writes files (namespace tools such as
        // `opencode_worktree_remove` share the action but name no file).
        for tool in tools.iter().filter(|t| t.hook_name().is_none()) {
            if tool.permission() == "edit" {
                assert!(crate::tool::writes_files(tool.as_ref()), "{}", tool.name());
            }
        }
        let writers: Vec<String> = tools
            .iter()
            .filter(|t| crate::tool::writes_files(t.as_ref()))
            .map(|t| t.name().to_string())
            .collect();
        assert_eq!(writers, ["edit", "notebook", "write"]);
        for name in writers {
            assert!(takes_file(&name), "{name}");
        }
        let cwd = Path::new("/work");
        let notebook = input(
            "notebook",
            &json!({"path": "n.ipynb", "cellId": "a", "source": "x", "notebook_path": "planted"}),
            cwd,
        );
        let expected = cwd.join("n.ipynb").display().to_string();
        assert_eq!(notebook["file_path"], expected);
        assert_eq!(notebook["notebook_path"], expected);
        assert_eq!(notebook["cell_id"], "a");
        assert_eq!(notebook["new_source"], "x");
        assert_eq!(claude_name("notebook"), "NotebookEdit");
    }

    #[test]
    fn claude_code_keys_overwrite_what_the_model_planted() {
        let cwd = Path::new("/work");
        let write = input(
            "write",
            &json!({"path": "protected/secret.txt", "file_path": "ok.txt", "content": "x"}),
            cwd,
        );
        assert_eq!(
            write["file_path"],
            cwd.join("protected/secret.txt").display().to_string()
        );
        let shell = input(
            "shell",
            &json!({"command": "rm -rf x", "run_in_background": false, "background": true}),
            cwd,
        );
        assert_eq!(shell["run_in_background"], true);
        let edit = input("edit", &json!({"path": "a", "old_string": "planted"}), cwd);
        assert!(edit.get("old_string").is_none());
        // Other tools' own keys of those names stay.
        let mcp = input("github_get", &json!({"file_path": "theirs"}), cwd);
        assert_eq!(mcp["file_path"], "theirs");
    }

    #[test]
    fn file_path_is_the_path_the_tool_resolves() {
        let dir = tempfile::tempdir().unwrap();
        let root = crate::paths::canonicalize(dir.path()).unwrap();
        let test = crate::tool::test_support::context(&root);
        let resolve = resolver(&test.ctx);
        assert_eq!(
            resolve("src/../a.rs"),
            root.join("a.rs").display().to_string()
        );
        #[cfg(unix)]
        {
            std::fs::create_dir(root.join("outside")).unwrap();
            std::fs::write(root.join("outside/real.md"), "x").unwrap();
            std::os::unix::fs::symlink(root.join("outside/real.md"), root.join("notes.md"))
                .unwrap();
            assert_eq!(
                resolve("notes.md"),
                root.join("outside/real.md").display().to_string(),
                "through the symlink"
            );
        }
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
