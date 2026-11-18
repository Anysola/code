//! What a folder's project files want to run (hooks, MCP servers, formatters) and the user's
//! answer for each, as it is now. Nothing of a project file is applied before it is allowed.

use anysola_core::api::{Core, TrustItem, TrustKind};
use anysola_core::hooks::display;
use serde::Deserialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, State};

use crate::{Answer, App, core_for, key, text};

/// One item the window answers for, as it was shown.
#[derive(Debug, Clone, Deserialize)]
pub struct Asked {
    path: String,
    kind: String,
    digest: String,
}

/// An item as the window gets it (`TrustItem`); repository text goes through `display`.
fn item_json(item: &TrustItem) -> Value {
    json!({
        "path": item.path,
        "kind": item.kind.as_str(),
        "digest": item.digest,
        "decision": item.decision.map(|allow| if allow { "allowed" } else { "denied" }),
        "standing": item.standing,
        "entries": item.entries.iter().map(|e| json!({
            "label": display(&e.label),
            "command": display(&e.command),
            "detail": e.detail.as_deref().map(display),
        })).collect::<Vec<_>>(),
    })
}

fn list(core: &Core) -> Value {
    Value::Array(core.trust_items().iter().map(item_json).collect())
}

/// The asked items, each still what the folder has now; one that changed (or is gone) fails
/// them all, so an answer never lands on content the user did not see.
fn current(
    items: &[TrustItem],
    asked: &[Asked],
) -> Result<Vec<(String, TrustKind, String)>, String> {
    asked
        .iter()
        .map(|a| {
            let kind = TrustKind::parse(&a.kind)
                .ok_or_else(|| format!("“{}” isn’t something a project file can run.", a.kind))?;
            let now = items
                .iter()
                .any(|i| i.path == a.path && i.kind == kind && i.digest == a.digest);
            if !now {
                return Err(format!(
                    "{} changed since it was shown. Look at it again, then answer.",
                    display(&a.path)
                ));
            }
            Ok((a.path.clone(), kind, a.digest.clone()))
        })
        .collect()
}

#[tauri::command]
pub async fn trust(handle: AppHandle, app: State<'_, App>, directory: String) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    Ok(list(&core))
}

/// Records one answer for each item; the folder's core (and any other open folder that reads
/// the same files) has taken it before this returns.
#[tauri::command]
pub async fn trust_answer(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    items: Vec<Asked>,
    allow: bool,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let answers = current(&core.trust_items(), &items)?;
    for (path, kind, digest) in &answers {
        core.trust(path, *kind, digest, allow).map_err(text)?;
    }
    // `Core::trust` reloads in the background: wait for it, so approved servers are starting
    // and the list below is the new one.
    core.reload().await;
    let paths: Vec<&str> = answers.iter().map(|(p, _, _)| p.as_str()).collect();
    let hooks = answers.iter().any(|(_, kind, _)| *kind == TrustKind::Hooks);
    for (other, running) in app.cores_running().await {
        let here = key(&other) == key(&directory);
        let reads = here
            || running
                .trust_items()
                .iter()
                .any(|i| paths.contains(&i.path.as_str()));
        if !reads {
            continue;
        }
        if !here {
            running.reload().await;
        }
        let _ = handle.emit("core", json!({"type": "trust", "directory": other}));
        // Servers that were refused never ran: no status change says the list changed.
        let _ = handle.emit("core", json!({"type": "mcp", "directory": other}));
        if hooks {
            let _ = handle.emit("core", json!({"type": "hooks", "directory": other}));
        }
    }
    Ok(list(&core))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anysola_core::api::TrustEntry;

    fn item(path: &str, kind: TrustKind, digest: &str, decision: Option<bool>) -> TrustItem {
        TrustItem {
            path: path.into(),
            kind,
            digest: digest.into(),
            decision,
            standing: false,
            entries: vec![TrustEntry {
                label: "PreToolUse (Bash)".into(),
                command: "./guard.sh\u{202e}\n".into(),
                detail: Some("timeout 5 s".into()),
            }],
        }
    }

    fn asked(path: &str, kind: &str, digest: &str) -> Asked {
        Asked {
            path: path.into(),
            kind: kind.into(),
            digest: digest.into(),
        }
    }

    #[test]
    fn items_as_the_window_reads_them() {
        let v = item_json(&item("/p/anysola.json", TrustKind::Hooks, "abc", None));
        assert_eq!(v["kind"], "hooks");
        assert_eq!(v["decision"], Value::Null);
        assert_eq!(v["standing"], false);
        assert_eq!(v["entries"][0]["label"], "PreToolUse (Bash)");
        assert_eq!(v["entries"][0]["detail"], "timeout 5 s");
        let command = v["entries"][0]["command"].as_str().unwrap();
        assert!(command.starts_with("./guard.sh"));
        assert!(!command.contains('\u{202e}') && !command.contains('\n'));
        let refused = item_json(&item("/p/a.json", TrustKind::Mcp, "d", Some(false)));
        assert_eq!(
            (refused["kind"].clone(), refused["decision"].clone()),
            (json!("mcp"), json!("denied"))
        );
        let allowed = item_json(&item("/p/a.json", TrustKind::Formatter, "d", Some(true)));
        assert_eq!(allowed["decision"], "allowed");
    }

    #[test]
    fn a_stale_item_fails_the_whole_answer() {
        let items = [
            item("/p/anysola.json", TrustKind::Hooks, "h1", None),
            item("/p/anysola.json", TrustKind::Mcp, "m1", None),
        ];
        assert_eq!(
            current(
                &items,
                &[
                    asked("/p/anysola.json", "hooks", "h1"),
                    asked("/p/anysola.json", "mcp", "m1")
                ]
            )
            .unwrap(),
            [
                (
                    "/p/anysola.json".to_string(),
                    TrustKind::Hooks,
                    "h1".to_string()
                ),
                (
                    "/p/anysola.json".to_string(),
                    TrustKind::Mcp,
                    "m1".to_string()
                ),
            ]
        );
        for stale in [
            asked("/p/anysola.json", "mcp", "old"),
            asked("/p/anysola.json", "formatter", "m1"),
            asked("/q/anysola.json", "mcp", "m1"),
        ] {
            let error =
                current(&items, &[asked("/p/anysola.json", "hooks", "h1"), stale]).unwrap_err();
            assert!(error.contains("changed since it was shown"), "{error}");
        }
        assert!(current(&items, &[asked("/p/anysola.json", "plugins", "x")]).is_err());
        assert!(current(&items, &[]).unwrap().is_empty());
    }
}
