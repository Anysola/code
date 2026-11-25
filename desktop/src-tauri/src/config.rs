//! The agent's settings in the global config file (`~/.anysola/config.json`), which the terminal
//! app reads too. Written in place so the user's comments and layout stay; a setting put back to
//! its default leaves the file rather than being written out.

use std::path::Path;

use anysola_core::config::jsonc;
use anysola_core::paths::Paths;
use serde_json::{Map, Value, json};
use tauri::{AppHandle, State};

use crate::{Answer, App, core_for, info, text};

/// Older names the core still reads for a setting: written or reset, the old name goes too, so
/// it can't keep a setting looking stuck.
const ALIASES: [(&[&str], &[&str]); 1] = [(&["snapshots"], &["snapshot"])];

/// The settings this screen writes: the key in `overridden`, its path in the file.
const KEYS: [(&str, &[&str]); 9] = [
    ("model", &["model"]),
    ("websearch", &["websearch"]),
    ("experimental.todo", &["experimental", "todo"]),
    (
        "experimental.subagent_depth",
        &["experimental", "subagent_depth"],
    ),
    ("compaction.auto", &["compaction", "auto"]),
    ("snapshots", &["snapshots"]),
    ("shell", &["shell"]),
    ("lsp", &["lsp"]),
    ("sandbox", &["sandbox"]),
];

const DEPTH: (u64, u64) = (1, 3);

/// What the screen says about a global file it can't parse, and refuses to change it with.
pub(crate) fn invalid(file: &Path) -> String {
    format!(
        "{} isn’t valid JSON. Fix it by hand, then try again.",
        file.display()
    )
}

/// The global file's text and its values as the core reads them (0.5.x names moved to the
/// current ones, variables substituted); `{}` when it does not exist.
pub(crate) fn read(paths: &Paths) -> Result<(String, Map<String, Value>), String> {
    let file = paths.config_file();
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(("{}\n".into(), Map::new()));
        }
        Err(e) => return Err(format!("Can’t read {}: {e}", file.display())),
    };
    if text.trim().is_empty() {
        return Ok(("{}\n".into(), Map::new()));
    }
    let dir = file.parent().unwrap_or(Path::new("/"));
    match anysola_core::config::document(&text, dir, paths) {
        Some(data) => Ok((text, data)),
        None => Err(invalid(&file)),
    }
}

/// Writes the file in place: a symlinked file stays a link and keeps
/// its permissions (it can hold MCP headers with secrets). Text that doesn't parse back as an
/// object is never written.
pub(crate) fn write(file: &Path, text: &str) -> Result<(), String> {
    if !matches!(jsonc::parse(text), Ok(Value::Object(_))) {
        return Err(format!(
            "The change to {} came out invalid, so nothing was written.",
            file.display()
        ));
    }
    let fail = |e: std::io::Error| format!("Can’t save {}: {e}", file.display());
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    std::fs::write(file, text).map_err(fail)
}

fn bool_at(data: &Map<String, Value>, path: &[&str], default: bool) -> bool {
    let mut value = data.get(path[0]);
    for key in &path[1..] {
        value = value.and_then(|v| v.get(*key));
    }
    value.and_then(Value::as_bool).unwrap_or(default)
}

/// `anysola/<id>[#variant]` as the id the model list uses.
fn model_id(reference: &str) -> &str {
    let id = reference.strip_prefix("anysola/").unwrap_or(reference);
    id.split('#').next().unwrap_or(id)
}

/// The file's values, the built-in defaults where it has none.
fn values(data: &Map<String, Value>) -> Value {
    let depth = data
        .get("experimental")
        .and_then(|e| e.get("subagent_depth"))
        .and_then(Value::as_u64)
        .unwrap_or(DEPTH.0)
        .clamp(DEPTH.0, DEPTH.1);
    json!({
        "model": data.get("model").and_then(Value::as_str).map(model_id),
        "websearch": bool_at(data, &["websearch"], true),
        "todo": bool_at(data, &["experimental", "todo"], false),
        "subagentDepth": depth,
        "autoCompact": bool_at(data, &["compaction", "auto"], true),
        "snapshots": bool_at(data, &["snapshots"], true),
        "shell": data.get("shell").and_then(Value::as_str).filter(|s| !s.is_empty()),
        // Read as the core reads them: only `true` turns LSP on; the sandbox's value may be an object.
        "lsp": data.get("lsp") == Some(&Value::Bool(true)),
        "sandbox": anysola_core::tool::shell::sandbox::SandboxConfig::from_value(data.get("sandbox")).enabled,
    })
}

/// Whether the shell sandbox can start on this computer: None when it can, else why not.
fn sandbox_unavailable() -> Option<&'static str> {
    sandbox_unavailable_on(cfg!(target_os = "linux"), bwrap_found(), bwrap_starts)
}

fn sandbox_unavailable_on(
    linux: bool,
    installed: bool,
    starts: impl FnOnce() -> bool,
) -> Option<&'static str> {
    if !linux {
        Some("It only works on Linux.")
    } else if !installed {
        Some("bubblewrap (bwrap) isn’t installed on this computer.")
    } else if !starts() {
        Some("bubblewrap can’t start here: this system restricts unprivileged user namespaces.")
    } else {
        None
    }
}

fn bwrap_found() -> bool {
    anysola_core::which::which("bwrap").is_some()
}

#[cfg(target_os = "linux")]
fn bwrap_starts() -> bool {
    anysola_core::tool::shell::sandbox::bwrap_path().is_some()
}

#[cfg(not(target_os = "linux"))]
fn bwrap_starts() -> bool {
    false
}

/// `sandbox` written as the user's value allows: an object keeps its other keys (`network`),
/// with `"enabled": false` to turn it off; otherwise `true`, or no key (off is the default).
fn sandbox_value(current: Option<&Value>, on: bool) -> Option<Value> {
    match current {
        Some(Value::Object(map)) => {
            let mut map = map.clone();
            if on {
                map.remove("enabled");
            } else {
                map.insert("enabled".into(), json!(false));
            }
            Some(Value::Object(map))
        }
        _ => on.then_some(json!(true)),
    }
}

/// The file's text with the patch applied: only the keys present, a default value removing its
/// key (a null `model` or `shell` too).
fn apply(text: &str, patch: &Map<String, Value>) -> Result<String, String> {
    let sandbox_now = jsonc::parse(text)
        .ok()
        .and_then(|v| v.get("sandbox").cloned());
    let mut text = text.to_string();
    let flag = |key: &str| -> Result<Option<bool>, String> {
        match patch.get(key) {
            None => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(format!("“{key}” takes true or false.")),
        }
    };
    let name = |key: &str| -> Result<Option<Option<String>>, String> {
        match patch.get(key) {
            None => Ok(None),
            Some(Value::Null) => Ok(Some(None)),
            Some(Value::String(s)) if s.trim().is_empty() => Ok(Some(None)),
            Some(Value::String(s)) => Ok(Some(Some(s.trim().to_string()))),
            Some(_) => Err(format!("“{key}” takes text or null.")),
        }
    };
    let mut put = |path: &[&str], value: Option<Value>| -> Result<(), String> {
        for (current, old) in ALIASES {
            if current == path {
                text = jsonc::remove(&text, old);
            }
        }
        text = match value {
            Some(value) => jsonc::set(&text, path, value)?,
            None => jsonc::remove(&text, path),
        };
        Ok(())
    };
    if let Some(model) = name("model")? {
        let value = model.map(|id| {
            let id = id.strip_prefix("anysola/").unwrap_or(&id).to_string();
            json!(format!("anysola/{id}"))
        });
        put(&["model"], value)?;
    }
    for (key, path, default) in [
        ("websearch", &["websearch"][..], true),
        ("todo", &["experimental", "todo"][..], false),
        ("autoCompact", &["compaction", "auto"][..], true),
        ("snapshots", &["snapshots"][..], true),
    ] {
        if let Some(on) = flag(key)? {
            put(path, (on != default).then_some(json!(on)))?;
        }
    }
    if let Some(depth) = patch.get("subagentDepth") {
        let depth = depth
            .as_f64()
            .ok_or("“subagentDepth” takes a number from 1 to 3.")?
            .round()
            .clamp(DEPTH.0 as f64, DEPTH.1 as f64) as u64;
        put(
            &["experimental", "subagent_depth"],
            (depth != DEPTH.0).then_some(json!(depth)),
        )?;
    }
    if let Some(shell) = name("shell")? {
        put(&["shell"], shell.map(Value::from))?;
    }
    if let Some(on) = flag("lsp")? {
        put(&["lsp"], on.then_some(json!(true)))?;
    }
    if let Some(on) = flag("sandbox")? {
        put(&["sandbox"], sandbox_value(sandbox_now.as_ref(), on))?;
    }
    Ok(text)
}

fn answer(app: &App, core: &anysola_core::api::Core) -> Answer {
    let file = app.paths.config_file();
    let loaded = core.loaded();
    let mut warnings = loaded.config.warnings.clone();
    // An unparsable file shows the defaults and says why, rather than failing the screen.
    let data = match read(&app.paths) {
        Ok((_, data)) => data,
        Err(error) => {
            let ignored = format!("Ignoring malformed config {}", file.display());
            warnings.retain(|w| *w != ignored);
            warnings.insert(0, error);
            Map::new()
        }
    };
    let mut out = values(&data);
    let global = file.display().to_string();
    let legacy = app.paths.legacy_config_dir();
    let overridden: Vec<Value> = KEYS
        .iter()
        .filter_map(|(key, path)| {
            // A later document replaces a whole top-level value, nested keys and all. The
            // legacy folder's files load before the global one and can't override it.
            let source = loaded.config.source_of(path[0])?;
            let after = source != global && !Path::new(source).starts_with(&legacy);
            after.then(|| json!({"key": key, "file": source}))
        })
        .collect();
    out["file"] = json!(global);
    out["models"] = info(core, None)["models"].clone();
    let detected = anysola_core::tool::shell::invocation(None, "").0;
    out["shellDetected"] = if detected.is_empty() {
        Value::Null
    } else {
        json!(detected)
    };
    // Where hooks without a shell of their own run, as the core runs them (its config's `shell`,
    // the legacy and managed files' too).
    let hooks_detected = core.hook_shell(None);
    out["hooksShellDetected"] = if hooks_detected.is_empty() {
        Value::Null
    } else {
        json!(hooks_detected)
    };
    out["sandboxUnavailable"] = json!(sandbox_unavailable());
    out["overridden"] = json!(overridden);
    out["warnings"] = json!(warnings);
    Ok(out)
}

/// The global file's settings, and for `directory` what overrides them and the models.
#[tauri::command]
pub async fn config_get(handle: AppHandle, app: State<'_, App>, directory: String) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    answer(&app, &core)
}

/// Writes the keys present in `patch`; running cores take the file again before this returns.
#[tauri::command]
pub async fn config_set(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    patch: Map<String, Value>,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    if let Some(Value::String(model)) = patch.get("model")
        && !model.trim().is_empty()
    {
        let id = model_id(model.trim());
        if !core.loaded().models.iter().any(|m| m.id == id) {
            return Err(format!("“{id}” isn’t one of the models you can use."));
        }
    }
    let file = app.paths.config_file();
    // One read-change-write of the global file at a time: two quick toggles keep both.
    let _writing = app.config_file.lock().await;
    let (before, _) = read(&app.paths)?;
    let after = apply(&before, &patch)?;
    if after != before {
        write(&file, &after)?;
        for (_, running) in app.cores_running().await {
            running.reload().await;
        }
    }
    answer(&app, &core).map_err(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn parsed(text: &str) -> Value {
        jsonc::parse(text).unwrap()
    }

    #[test]
    fn writes_only_the_keys_given_and_drops_defaults() {
        let text = "{\n  // mine\n  \"shell\": \"zsh\",\n  \"experimental\": {\"todo\": true}\n}\n";
        let out = apply(text, &patch(json!({"websearch": false}))).unwrap();
        assert!(out.contains("// mine"));
        assert_eq!(
            parsed(&out),
            json!({"shell": "zsh", "experimental": {"todo": true}, "websearch": false})
        );
        let out = apply(&out, &patch(json!({"todo": false, "websearch": true}))).unwrap();
        assert_eq!(parsed(&out), json!({"shell": "zsh"}));
        assert!(out.contains("// mine"));
        let out = apply(&out, &patch(json!({"shell": null}))).unwrap();
        assert_eq!(parsed(&out), json!({}));
    }

    #[test]
    fn model_depth_and_shell() {
        let out = apply(
            "{}",
            &patch(
                json!({"model": "claude-haiku-5-5", "subagentDepth": 7, "shell": " /bin/fish ",
                          "autoCompact": false, "snapshots": false}),
            ),
        )
        .unwrap();
        assert_eq!(
            parsed(&out),
            json!({"model": "anysola/claude-haiku-5-5", "experimental": {"subagent_depth": 3},
                   "shell": "/bin/fish", "compaction": {"auto": false}, "snapshots": false})
        );
        let back = apply(
            &out,
            &patch(json!({"model": null, "subagentDepth": 1, "autoCompact": true, "snapshots": true, "shell": ""})),
        )
        .unwrap();
        assert_eq!(parsed(&back), json!({}));
        assert!(apply("{}", &patch(json!({"todo": "yes"}))).is_err());
    }

    #[test]
    fn values_with_defaults() {
        assert_eq!(
            values(&Map::new()),
            json!({"model": null, "websearch": true, "todo": false, "subagentDepth": 1,
                   "autoCompact": true, "snapshots": true, "shell": null, "lsp": false,
                   "sandbox": false})
        );
        let data = patch(
            json!({"model": "anysola/claude-sonnet-5-5#high", "websearch": false,
                                "experimental": {"subagent_depth": 9}}),
        );
        let v = values(&data);
        assert_eq!(v["model"], "claude-sonnet-5-5");
        assert_eq!(v["websearch"], false);
        assert_eq!(v["subagentDepth"], 3);
    }

    /// Both stay off unless the user turns them on; turning the sandbox off keeps the rest of
    /// an object the user wrote.
    #[test]
    fn lsp_and_the_sandbox_are_the_users_to_turn_on() {
        let on = apply("{}", &patch(json!({"lsp": true, "sandbox": true}))).unwrap();
        assert_eq!(parsed(&on), json!({"lsp": true, "sandbox": true}));
        let v = values(parsed(&on).as_object().unwrap());
        assert_eq!(
            (v["lsp"].clone(), v["sandbox"].clone()),
            (json!(true), json!(true))
        );
        let off = apply(&on, &patch(json!({"lsp": false, "sandbox": false}))).unwrap();
        assert_eq!(parsed(&off), json!({}));
        // Only `true` counts for LSP, as in the core.
        assert_eq!(values(&patch(json!({"lsp": "yes"})))["lsp"], false);
        let object = r#"{"sandbox": {"network": false}}"#;
        assert_eq!(values(&patch(parsed(object)))["sandbox"], true);
        let off = apply(object, &patch(json!({"sandbox": false}))).unwrap();
        assert_eq!(
            parsed(&off),
            json!({"sandbox": {"network": false, "enabled": false}})
        );
        assert_eq!(values(parsed(&off).as_object().unwrap())["sandbox"], false);
        let back = apply(&off, &patch(json!({"sandbox": true}))).unwrap();
        assert_eq!(parsed(&back), json!({"sandbox": {"network": false}}));
        assert!(apply("{}", &patch(json!({"lsp": 1}))).is_err());
    }

    #[test]
    fn why_the_sandbox_cant_start() {
        assert!(
            sandbox_unavailable_on(false, true, || true)
                .unwrap()
                .contains("Linux")
        );
        assert!(
            sandbox_unavailable_on(true, false, || true)
                .unwrap()
                .contains("installed")
        );
        assert!(
            sandbox_unavailable_on(true, true, || false)
                .unwrap()
                .contains("namespaces")
        );
        assert_eq!(sandbox_unavailable_on(true, true, || true), None);
    }

    fn home(name: &str) -> (std::path::PathBuf, Paths) {
        let dir = std::env::temp_dir().join(format!(
            "anysola-config-{name}-{}-{}",
            std::process::id(),
            anysola_core::ids::part()
        ));
        std::fs::create_dir_all(dir.join(".anysola")).unwrap();
        let paths = Paths::isolated(dir.clone());
        (dir, paths)
    }

    #[test]
    fn values_as_the_core_reads_them_and_old_names_go_with_a_change() {
        let (dir, paths) = home("legacy");
        let text =
            r#"{"snapshot": false, "model": {"providerID": "anysola", "id": "claude-haiku-5-5"}}"#;
        std::fs::write(paths.config_file(), text).unwrap();
        let (text, data) = read(&paths).unwrap();
        let v = values(&data);
        assert_eq!(v["snapshots"], false, "the 0.5.x name counts");
        assert_eq!(v["model"], "claude-haiku-5-5", "the object form counts");
        let on = apply(&text, &patch(json!({"snapshots": true}))).unwrap();
        assert_eq!(parsed(&on).get("snapshot"), None);
        assert_eq!(parsed(&on).get("snapshots"), None);
        let off = apply(&text, &patch(json!({"snapshots": false}))).unwrap();
        assert_eq!(parsed(&off).get("snapshot"), None);
        assert_eq!(parsed(&off)["snapshots"], false);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_invalid_file_is_named() {
        let (dir, paths) = home("invalid");
        std::fs::write(paths.config_file(), "{ nope").unwrap();
        assert_eq!(read(&paths).unwrap_err(), invalid(&paths.config_file()));
        assert!(invalid(&paths.config_file()).contains("isn’t valid JSON"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
