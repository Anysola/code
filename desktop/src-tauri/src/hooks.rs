//! Hooks in Settings: every hook the folder's core has, the project files that wait for the
//! user's answer, and adding, changing and removing the hooks of the global config file (as
//! `servers.rs` does for MCP servers there).
//!
//! `jsonc` edits object keys, and hooks sit in arrays: a change rewrites the one event's list it
//! touches (comments inside that list go), and leaves the rest of the file as it was.

use anysola_core::api::{Core, HookInfo, HookScope};
use anysola_core::config::jsonc;
use anysola_core::hooks::{self, Event};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tauri::{AppHandle, Emitter, State};

use crate::{Answer, App, core_for, key, text};

/// What the window's hook form sends (`HookDraft`).
#[derive(Debug, Clone, Deserialize)]
pub struct Draft {
    event: String,
    #[serde(default)]
    matcher: Option<String>,
    command: String,
    #[serde(default)]
    timeout: Option<f64>,
    /// The hook's own shell (a name or a path); none: where hooks without one run.
    #[serde(default)]
    shell: Option<String>,
}

/// A draft checked: a known event, a command, a matcher only where the event uses one.
#[derive(Debug, Clone, PartialEq)]
struct Checked {
    event: Event,
    matcher: Option<String>,
    command: String,
    timeout: Option<u64>,
    shell: Option<String>,
}

fn check(draft: &Draft) -> Result<Checked, String> {
    let event = Event::parse(&draft.event)
        .ok_or_else(|| format!("“{}” isn’t a moment a hook can run at.", draft.event))?;
    let command = draft.command.trim();
    if command.is_empty() {
        return Err("A hook needs a command.".into());
    }
    let matcher = draft
        .matcher
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty() && event.uses_matcher())
        .map(str::to_string);
    let timeout = match draft.timeout {
        None => None,
        Some(t) if t.is_finite() && t >= 1.0 => Some(t.round() as u64),
        Some(_) => return Err("A hook’s timeout is a number of seconds, 1 or more.".into()),
    };
    let shell = draft
        .shell
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(Checked {
        event,
        matcher,
        command: command.to_string(),
        timeout,
        shell,
    })
}

/// What an id that no longer names the same hook answers.
const STALE: &str = "This hook was changed or removed elsewhere. Look at the list again and retry.";

/// A handler of the global file the core runs, as the file has it.
#[derive(Debug, Clone, PartialEq)]
struct Listed {
    id: String,
    event: String,
    matcher: Option<String>,
    command: String,
}

/// Names one handler with its group's matcher: an id made from a position alone could name
/// another hook once the file changed.
fn fingerprint(group: &Value, handler: &Value) -> String {
    let digest =
        hooks::config::digest(&json!({"matcher": group.get("matcher"), "handler": handler}));
    digest[..12].to_string()
}

/// The group with only `handler` in it, read as the core reads it: the handler it runs, if any.
fn runs(event: &str, group: &Value, handler: &Value) -> Option<hooks::Handler> {
    let mut single = group.as_object()?.clone();
    single.insert("hooks".into(), json!([handler]));
    hooks::config::parse(&json!({ event: [single] }), "", &mut Vec::new())
        .into_iter()
        .next()
}

/// The handlers of the file's `hooks` (the values as the core reads them) the core runs, in its
/// order, each with its id: `<event>:<group>:<handler>:<fingerprint>`.
fn listed(data: &Map<String, Value>) -> Vec<Listed> {
    let mut out = Vec::new();
    let Some(events) = data.get("hooks").and_then(Value::as_object) else {
        return out;
    };
    for (event, groups) in events {
        for (g, group) in groups.as_array().into_iter().flatten().enumerate() {
            let handlers = group.get("hooks").and_then(Value::as_array);
            for (h, handler) in handlers.into_iter().flatten().enumerate() {
                if let Some(ran) = runs(event, group, handler) {
                    out.push(Listed {
                        id: format!("{event}:{g}:{h}:{}", fingerprint(group, handler)),
                        event: event.clone(),
                        matcher: ran.matcher,
                        command: ran.command,
                    });
                }
            }
        }
    }
    out
}

/// Where an id points: the event, group and handler, checked against the file as it is now.
#[derive(Debug, Clone, PartialEq)]
struct Spot {
    event: String,
    group: usize,
    handler: usize,
}

fn locate(data: &Map<String, Value>, id: &str) -> Result<Spot, String> {
    let mut parts = id.rsplitn(4, ':');
    let (Some(print), Some(h), Some(g), Some(event)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(STALE.into());
    };
    let (Ok(g), Ok(h)) = (g.parse::<usize>(), h.parse::<usize>()) else {
        return Err(STALE.into());
    };
    let group = data
        .get("hooks")
        .and_then(|e| e.get(event))
        .and_then(|groups| groups.get(g))
        .ok_or(STALE)?;
    let handler = group
        .get("hooks")
        .and_then(|list| list.get(h))
        .ok_or(STALE)?;
    if fingerprint(group, handler) != print || runs(event, group, handler).is_none() {
        return Err(STALE.into());
    }
    Ok(Spot {
        event: event.to_string(),
        group: g,
        handler: h,
    })
}

/// The file's own `hooks` (variables not substituted: that is what gets written back).
fn raw_hooks(text: &str) -> Result<Map<String, Value>, String> {
    let raw = jsonc::parse(text).map_err(|_| "The settings file isn’t valid JSON.".to_string())?;
    match raw.get("hooks") {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(events)) => Ok(events.clone()),
        Some(_) => Err(
            "“hooks” in the settings file isn’t an object. Fix it by hand, then try again.".into(),
        ),
    }
}

/// The event's list of groups in the raw file (empty when it has none).
fn raw_groups(events: &Map<String, Value>, event: &str) -> Result<Vec<Value>, String> {
    match events.get(event) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(groups)) => Ok(groups.clone()),
        Some(_) => Err(format!(
            "“hooks.{event}” in the settings file isn’t a list. Fix it by hand, then try again."
        )),
    }
}

/// Writes the event's list back; an empty one goes, and `hooks` with it when nothing is left.
fn put(text: &str, event: &str, groups: Vec<Value>) -> Result<String, String> {
    if groups.is_empty() {
        Ok(jsonc::remove(text, &["hooks", event]))
    } else {
        jsonc::set(text, &["hooks", event], Value::Array(groups))
    }
}

fn group_of(checked: &Checked, handler: Value) -> Value {
    let mut group = Map::new();
    if let Some(matcher) = &checked.matcher {
        group.insert("matcher".into(), json!(matcher));
    }
    group.insert("hooks".into(), json!([handler]));
    Value::Object(group)
}

/// The handler as written: the old one's other keys kept, its command kept as written when the
/// draft's is the one the list showed (the file may say `{env:…}` where the list showed the value).
fn handler_of(checked: &Checked, old: Option<(&Value, &str)>) -> Value {
    let mut handler = old
        .and_then(|(h, _)| h.as_object().cloned())
        .unwrap_or_default();
    handler.insert("type".into(), json!("command"));
    let same = old.is_some_and(|(_, shown)| shown == checked.command);
    if !same {
        handler.insert("command".into(), json!(checked.command));
    }
    // The list shows every hook's timeout: the event's default sent back is no setting.
    let default = if checked.event == Event::UserPromptSubmit {
        30
    } else {
        600
    };
    match checked.timeout.filter(|t| *t != default) {
        Some(t) => handler.insert("timeout".into(), json!(t)),
        None => handler.remove("timeout"),
    };
    // None: the hook runs where hooks without a shell of their own do.
    match &checked.shell {
        Some(shell) => handler.insert("shell".into(), json!(shell)),
        None => handler.remove("shell"),
    };
    Value::Object(handler)
}

/// Adds a hook as a group of its own at the end of its event's list.
fn add(text: &str, data: &Map<String, Value>, checked: &Checked) -> Result<String, String> {
    let event = checked.event.as_str();
    let duplicate = listed(data)
        .into_iter()
        .any(|l| l.event == event && l.command == checked.command && l.matcher == checked.matcher);
    if duplicate {
        return Err("That hook is already there.".into());
    }
    let events = raw_hooks(text)?;
    let mut groups = raw_groups(&events, event)?;
    groups.push(group_of(checked, handler_of(checked, None)));
    put(text, event, groups)
}

/// The raw event's groups with the spot's handler, checked to be there.
fn raw_at(text: &str, spot: &Spot) -> Result<(Vec<Value>, Value), String> {
    let groups = raw_groups(&raw_hooks(text)?, &spot.event)?;
    let handler = groups
        .get(spot.group)
        .and_then(|g| g.get("hooks"))
        .and_then(|list| list.get(spot.handler))
        .cloned()
        .ok_or(STALE)?;
    Ok((groups, handler))
}

/// `groups` without the spot's handler: a group left empty goes.
fn without(mut groups: Vec<Value>, spot: &Spot) -> Vec<Value> {
    if let Some(Value::Array(list)) = groups[spot.group].get_mut("hooks") {
        list.remove(spot.handler);
        if list.is_empty() {
            groups.remove(spot.group);
        }
    }
    groups
}

fn remove(text: &str, data: &Map<String, Value>, id: &str) -> Result<String, String> {
    let spot = locate(data, id)?;
    let (groups, _) = raw_at(text, &spot)?;
    put(text, &spot.event, without(groups, &spot))
}

/// Changes a hook where it is. A new matcher for one of several handlers of a group gives it a
/// group of its own just after; a new event moves it to the end of that event's list.
fn update(
    text: &str,
    data: &Map<String, Value>,
    id: &str,
    checked: &Checked,
) -> Result<String, String> {
    let spot = locate(data, id)?;
    let (mut groups, old) = raw_at(text, &spot)?;
    let shown = &data["hooks"][&spot.event][spot.group];
    let shown_command = runs(&spot.event, shown, &shown["hooks"][spot.handler])
        .map(|h| h.command)
        .unwrap_or_default();
    let handler = handler_of(checked, Some((&old, &shown_command)));
    if checked.event.as_str() != spot.event {
        let removed = put(text, &spot.event, without(groups, &spot))?;
        let events = raw_hooks(&removed)?;
        let mut target = raw_groups(&events, checked.event.as_str())?;
        target.push(group_of(checked, handler));
        return put(&removed, checked.event.as_str(), target);
    }
    let shown_matcher = shown
        .get("matcher")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty());
    let same_matcher = shown_matcher == checked.matcher.as_deref() || !checked.event.uses_matcher();
    let alone = groups[spot.group]["hooks"]
        .as_array()
        .is_some_and(|l| l.len() == 1);
    if same_matcher {
        groups[spot.group]["hooks"][spot.handler] = handler;
    } else if alone {
        let group = groups[spot.group].as_object_mut().ok_or(STALE)?;
        match &checked.matcher {
            Some(m) => group.insert("matcher".into(), json!(m)),
            None => group.remove("matcher"),
        };
        groups[spot.group]["hooks"][0] = handler;
    } else {
        groups = without(groups, &spot);
        groups.insert(spot.group + 1, group_of(checked, handler));
    }
    put(text, &spot.event, groups)
}

/// `disable_hooks` written when on, gone when off (the default).
fn disable(text: &str, disabled: bool) -> Result<String, String> {
    if disabled {
        jsonc::set(text, &["disable_hooks"], json!(true))
    } else {
        Ok(jsonc::remove(text, &["disable_hooks"]))
    }
}

/// `runs_in`: the shell it runs in (`Core::hook_shell` of its own `shell`).
/// The commands Settings' starters wrote for PowerShell up to 1.2 (hooks ran in PowerShell on
/// Windows then; since 1.3 they run in Git Bash when it is installed, where these fail and block).
const POWERSHELL_STARTERS: [&str; 3] = [
    "if ([Console]::In.ReadToEnd() -match 'git push.*(-f|--force)') { [Console]::Error.WriteLine('Force-pushing is off for the agent.'); exit 2 }",
    "$out = npm run lint --silent 2>&1 | Out-String; if ($LASTEXITCODE) { [Console]::Error.Write($out); exit 2 }",
    "$out = npm test --silent 2>&1 | Out-String; if ($LASTEXITCODE) { [Console]::Error.Write($out); exit 2 }",
];

/// Marks the one-time move of those starters to PowerShell as done (in the database's KV).
const STARTERS_MARKER: &str = "anysola:desktop:powershell-starters";

/// The global file's text with `"shell": "powershell"` on every hook that has no `shell` and
/// whose command is, character for character, one of [`POWERSHELL_STARTERS`]; None when there is
/// none. Nothing else of a command is looked at.
fn pin_powershell_starters(text: &str) -> Result<Option<String>, String> {
    let events = raw_hooks(text)?;
    let mut out = text.to_string();
    let mut changed = false;
    for (event, groups) in &events {
        let Some(groups) = groups.as_array() else {
            continue;
        };
        let mut groups = groups.clone();
        let mut here = false;
        for group in &mut groups {
            let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                continue;
            };
            for handler in handlers {
                let Some(handler) = handler.as_object_mut() else {
                    continue;
                };
                let starter = handler
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|c| POWERSHELL_STARTERS.contains(&c));
                if starter && !handler.contains_key("shell") {
                    handler.insert("shell".into(), json!("powershell"));
                    here = true;
                }
            }
        }
        if here {
            out = put(&out, event, groups)?;
            changed = true;
        }
    }
    Ok(changed.then_some(out))
}

/// Once, at start on Windows, before any core reads the file: hooks added from the ≤1.2
/// PowerShell starters keep running in PowerShell. A file that can't be read or parsed is left
/// alone and tried again next start; done is marked only once the file is settled.
pub(crate) fn pin_powershell_starters_at_start(
    paths: &anysola_core::paths::Paths,
    db: Option<&anysola_core::storage::Db>,
) {
    let Some(db) = db else {
        return;
    };
    if !matches!(db.kv_get(STARTERS_MARKER), Ok(None)) {
        return;
    }
    let file = paths.config_file();
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return,
    };
    let pinned = if text.trim().is_empty() {
        Ok(None)
    } else {
        pin_powershell_starters(&text)
    };
    let written = match pinned {
        Ok(None) => Ok(0),
        Ok(Some(after)) => crate::config::write(&file, &after).map(|_| 1),
        Err(e) => Err(e),
    };
    match written {
        Ok(n) => {
            let _ = db.kv_set(STARTERS_MARKER, &json!({"pinned": n}));
        }
        Err(error) => anysola_core::log::warn(
            "hooks from the PowerShell starters were left as they are",
            &[("error", &error)],
        ),
    }
}

fn hook_json(info: &HookInfo, id: Option<String>, runs_in: &str) -> Value {
    // Only a hook the form can edit keeps its text as written. Anything else may be a
    // repository's, and is shown with control characters and bidirectional overrides written
    // out, so a command cannot hide behind what it makes the window draw.
    let shown = |text: &str| match id {
        Some(_) => text.to_string(),
        None => anysola_core::hooks::display(text),
    };
    json!({
        "event": info.event,
        "matcher": info.matcher.as_deref().filter(|m| !m.is_empty()).map(shown),
        "command": shown(&info.command),
        "timeout": info.timeout,
        "shell": info.shell.as_deref().map(shown),
        "runsIn": runs_in,
        "source": info.source,
        "scope": info.scope.as_str(),
        "active": info.active,
        "id": id,
    })
}

/// The core's hooks with the ids of the global file's: matched in order, and none at all past
/// the first that differs (the file changed since the core read it), never a wrong one.
fn with_ids(
    infos: &[HookInfo],
    global: &str,
    listed: Vec<Listed>,
    runs_in: &dyn Fn(Option<&str>) -> String,
) -> Vec<Value> {
    let mut listed = listed.into_iter();
    let mut aligned = true;
    infos
        .iter()
        .map(|info| {
            let mut id = None;
            if aligned && info.scope == HookScope::User && info.source == global {
                match listed.next() {
                    Some(l)
                        if l.event == info.event
                            && l.command == info.command
                            && l.matcher == info.matcher =>
                    {
                        id = Some(l.id)
                    }
                    _ => aligned = false,
                }
            }
            hook_json(info, id, &runs_in(info.shell.as_deref()))
        })
        .collect()
}

fn answer(app: &App, core: &Core) -> Answer {
    let global = app.paths.config_file().display().to_string();
    let listed = crate::config::read(&app.paths)
        .map(|(_, data)| listed(&data))
        .unwrap_or_default();
    let runs_in = |own: Option<&str>| core.hook_shell(own);
    let files: Vec<Value> = core
        .hook_documents()
        .iter()
        .map(|doc| {
            json!({
                "path": doc.path,
                "digest": doc.digest,
                "decision": doc.decision.map(|allow| if allow { "allowed" } else { "denied" }),
                "standing": doc.standing,
                "hooks": doc.hooks.iter().map(|h| hook_json(h, None, &runs_in(h.shell.as_deref()))).collect::<Vec<_>>(),
            })
        })
        .collect();
    let disabled =
        core.loaded().config.hooks.disabled_by_user || anysola_core::env::flag("DISABLE_HOOKS");
    Ok(json!({
        "hooks": with_ids(&core.hooks(), &global, listed, &runs_in),
        "files": files,
        "disabled": disabled,
        "shell": core.hook_shell(None),
    }))
}

/// Tells the window the hooks of every open folder may have changed, but `except`'s (its core
/// says so itself).
async fn changed(handle: &AppHandle, app: &App, except: Option<&str>) {
    for (directory, _) in app.cores_running().await {
        if except.is_some_and(|e| key(e) == key(&directory)) {
            continue;
        }
        let _ = handle.emit("core", json!({"type": "hooks", "directory": directory}));
    }
}

/// Edits the global file; every running core takes it before this returns (it holds every
/// folder's user hooks).
async fn edit(
    handle: &AppHandle,
    app: &App,
    change: impl FnOnce(&str, &Map<String, Value>) -> Result<String, String>,
) -> Result<(), String> {
    let file = app.paths.config_file();
    // One read-change-write of the global file at a time, with `config_set` and MCP servers.
    let _writing = app.config_file.lock().await;
    let (before, data) = crate::config::read(&app.paths)?;
    let after = change(&before, &data)?;
    if after != before {
        crate::config::write(&file, &after)?;
        for (_, running) in app.cores_running().await {
            running.reload().await;
        }
        changed(handle, app, None).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn hooks(handle: AppHandle, app: State<'_, App>, directory: String) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    answer(&app, &core)
}

/// Adds a hook to the global file.
#[tauri::command]
pub async fn hook_add(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    hook: Draft,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let checked = check(&hook)?;
    edit(&handle, &app, |text, data| add(text, data, &checked)).await?;
    answer(&app, &core)
}

#[tauri::command]
pub async fn hook_update(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    id: String,
    hook: Draft,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let checked = check(&hook)?;
    edit(&handle, &app, |text, data| {
        update(text, data, &id, &checked)
    })
    .await?;
    answer(&app, &core)
}

#[tauri::command]
pub async fn hook_remove(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    id: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    edit(&handle, &app, |text, data| remove(text, data, &id)).await?;
    answer(&app, &core)
}

/// The user's answer for a project file's hooks as they are now.
#[tauri::command]
pub async fn hooks_trust(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    path: String,
    digest: String,
    allow: bool,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    if !core
        .hook_documents()
        .iter()
        .any(|d| d.path == path && d.digest == digest)
    {
        return Err(format!(
            "The hooks in {path} changed since they were shown. Look at them again."
        ));
    }
    core.trust_hooks(&path, &digest, allow).map_err(text)?;
    // Another open folder inside this one reads the same file.
    changed(&handle, &app, Some(&directory)).await;
    answer(&app, &core)
}

/// Turns the user's and the project's hooks off or back on, in the global file.
#[tauri::command]
pub async fn hooks_disable(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    disabled: bool,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    edit(&handle, &app, |text, _| disable(text, disabled)).await?;
    answer(&app, &core)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(event: &str, matcher: Option<&str>, command: &str, timeout: Option<f64>) -> Checked {
        check(&Draft {
            event: event.into(),
            matcher: matcher.map(str::to_string),
            command: command.into(),
            timeout,
            shell: None,
        })
        .unwrap()
    }

    fn data(text: &str) -> Map<String, Value> {
        jsonc::parse(text).unwrap().as_object().unwrap().clone()
    }

    fn parsed(text: &str) -> Value {
        jsonc::parse(text).unwrap()
    }

    fn id_of(text: &str, command: &str) -> String {
        listed(&data(text))
            .into_iter()
            .find(|l| l.command == command)
            .unwrap()
            .id
    }

    const FILE: &str = r#"{
  // mine
  "model": "anysola/x",
  "hooks": {
    "PreToolUse": [
      {"matcher": "Bash", "hooks": [
        {"type": "command", "command": "one", "statusMessage": "kept"},
        {"type": "command", "command": "two", "timeout": 5}
      ]},
      {"matcher": "Edit", "hooks": [{"type": "command", "command": "three"}]}
    ],
    "Stop": [{"hooks": [{"type": "command", "command": "four"}]}]
  }
}
"#;

    #[test]
    fn drafts_are_checked() {
        let c = draft("Stop", Some("Bash"), "  x  ", Some(4.6));
        assert_eq!(
            (c.matcher, c.command.as_str(), c.timeout),
            (None, "x", Some(5))
        );
        assert_eq!(draft("PreToolUse", Some(" "), "x", None).matcher, None);
        assert_eq!(
            draft("PreToolUse", Some("Edit|Write"), "x", None)
                .matcher
                .as_deref(),
            Some("Edit|Write")
        );
        let bad = |event: &str, command: &str, timeout| {
            check(&Draft {
                event: event.into(),
                matcher: None,
                command: command.into(),
                timeout,
                shell: None,
            })
            .is_err()
        };
        assert!(bad("TeammateIdle", "x", None));
        for event in ["SubagentStop", "PreCompact", "Notification", "SessionEnd"] {
            assert!(!bad(event, "x", None), "{event}");
        }
        assert!(bad("Stop", " ", None));
        assert!(bad("Stop", "x", Some(0.0)));
    }

    #[test]
    fn add_into_a_missing_or_empty_file() {
        let c = draft("PreToolUse", Some("Bash"), "./guard.sh", Some(30.0));
        for text in ["{}", "{}\n", "{\n  \"hooks\": {}\n}\n"] {
            let out = add(text, &data(text), &c).unwrap();
            assert_eq!(
                parsed(&out),
                json!({"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [
                    {"type": "command", "command": "./guard.sh", "timeout": 30}]}]}})
            );
        }
        let stop = add("{}", &Map::new(), &draft("Stop", None, "x", None)).unwrap();
        assert_eq!(
            parsed(&stop),
            json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "x"}]}]}})
        );
    }

    #[test]
    fn add_to_an_existing_event_keeps_the_rest() {
        let out = add(FILE, &data(FILE), &draft("Stop", None, "five", None)).unwrap();
        assert!(out.contains("// mine"));
        let v = parsed(&out);
        assert_eq!(v["model"], "anysola/x");
        assert_eq!(
            v["hooks"]["PreToolUse"],
            parsed(FILE)["hooks"]["PreToolUse"]
        );
        assert_eq!(v["hooks"]["Stop"][1]["hooks"][0]["command"], "five");
        let again = add(&out, &data(&out), &draft("Stop", None, "five", None));
        assert!(again.is_err(), "the same hook twice");
    }

    #[test]
    fn ids_follow_what_the_core_runs() {
        let text = r#"{"hooks": {
            "TeammateIdle": [{"hooks": [{"type": "command", "command": "skipped"}]}],
            "PreToolUse": [{"hooks": [
                {"type": "prompt", "prompt": "no"},
                {"type": "command", "command": "a", "async": true},
                {"type": "command", "command": "b"}
            ]}]}}"#;
        let l = listed(&data(text));
        assert_eq!(l.len(), 1);
        assert!(l[0].id.starts_with("PreToolUse:0:2:"), "{}", l[0].id);
        assert_eq!(locate(&data(text), &l[0].id).unwrap().handler, 2);
        let ids: Vec<_> = listed(&data(FILE)).into_iter().map(|l| l.command).collect();
        assert_eq!(ids, ["one", "two", "three", "four"]);
    }

    #[test]
    fn update_keeps_position_and_other_keys() {
        let id = id_of(FILE, "one");
        let out = update(
            FILE,
            &data(FILE),
            &id,
            &draft("PreToolUse", Some("Bash"), "uno", Some(9.0)),
        )
        .unwrap();
        let v = parsed(&out);
        assert_eq!(
            v["hooks"]["PreToolUse"][0]["hooks"][0],
            json!({"type": "command", "command": "uno", "statusMessage": "kept", "timeout": 9})
        );
        assert_eq!(v["hooks"]["PreToolUse"][0]["hooks"][1]["command"], "two");
        assert!(out.contains("// mine"));
        // The timeout back to its default leaves the file.
        let id = id_of(FILE, "two");
        let out = update(
            FILE,
            &data(FILE),
            &id,
            &draft("PreToolUse", Some("Bash"), "two", None),
        )
        .unwrap();
        assert_eq!(
            parsed(&out)["hooks"]["PreToolUse"][0]["hooks"][1],
            json!({"type": "command", "command": "two"})
        );
    }

    #[test]
    fn update_with_a_new_matcher_or_event() {
        // One of two in its group: a group of its own, just after.
        let id = id_of(FILE, "two");
        let out = update(
            FILE,
            &data(FILE),
            &id,
            &draft("PreToolUse", Some("Write"), "two", None),
        )
        .unwrap();
        let pre = parsed(&out)["hooks"]["PreToolUse"].clone();
        assert_eq!(pre.as_array().unwrap().len(), 3);
        assert_eq!(pre[0]["hooks"].as_array().unwrap().len(), 1);
        assert_eq!(
            pre[1],
            json!({"matcher": "Write", "hooks": [{"type": "command", "command": "two"}]})
        );
        assert_eq!(pre[2]["matcher"], "Edit");
        // Alone in its group: the group's matcher changes.
        let id = id_of(FILE, "three");
        let out = update(
            FILE,
            &data(FILE),
            &id,
            &draft("PreToolUse", None, "three", None),
        )
        .unwrap();
        assert_eq!(
            parsed(&out)["hooks"]["PreToolUse"][1],
            json!({"hooks": [{"type": "command", "command": "three"}]})
        );
        // Another event: moved to the end of that event's list, its old group gone.
        let id = id_of(FILE, "three");
        let out = update(
            FILE,
            &data(FILE),
            &id,
            &draft("PostToolUse", Some("Edit"), "three", None),
        )
        .unwrap();
        let v = parsed(&out);
        assert_eq!(v["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
        assert_eq!(v["hooks"]["PostToolUse"][0]["matcher"], "Edit");
        let id = id_of(FILE, "four");
        let out = update(
            FILE,
            &data(FILE),
            &id,
            &draft("SessionStart", None, "four", None),
        )
        .unwrap();
        let v = parsed(&out);
        assert!(v["hooks"].get("Stop").is_none());
        assert_eq!(v["hooks"]["SessionStart"][0]["hooks"][0]["command"], "four");
    }

    #[test]
    fn remove_collapses_what_it_empties() {
        let out = remove(FILE, &data(FILE), &id_of(FILE, "one")).unwrap();
        assert_eq!(
            parsed(&out)["hooks"]["PreToolUse"][0]["hooks"],
            json!([{"type": "command", "command": "two", "timeout": 5}])
        );
        let out = remove(&out, &data(&out), &id_of(&out, "two")).unwrap();
        assert_eq!(
            parsed(&out)["hooks"]["PreToolUse"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let out = remove(&out, &data(&out), &id_of(&out, "three")).unwrap();
        assert!(parsed(&out)["hooks"].get("PreToolUse").is_none());
        let out = remove(&out, &data(&out), &id_of(&out, "four")).unwrap();
        assert_eq!(parsed(&out), json!({"model": "anysola/x"}));
        assert!(out.contains("// mine"));
    }

    #[test]
    fn stale_ids_fail_and_change_nothing() {
        let id = id_of(FILE, "one");
        // The first handler removed by hand: the same position now holds another hook.
        let edited = FILE.replace(
            r#"{"type": "command", "command": "one", "statusMessage": "kept"},"#,
            "",
        );
        assert_eq!(
            id_of(&edited, "two").split(':').take(3).collect::<Vec<_>>(),
            id.split(':').take(3).collect::<Vec<_>>(),
            "the same position"
        );
        assert_eq!(remove(&edited, &data(&edited), &id).unwrap_err(), STALE);
        let c = draft("PreToolUse", Some("Bash"), "x", None);
        assert_eq!(update(&edited, &data(&edited), &id, &c).unwrap_err(), STALE);
        for bad in ["", "Stop", "Stop:0:9:abc", "Stop:x:0:abc", "Nope:0:0:abc"] {
            assert_eq!(remove(FILE, &data(FILE), bad).unwrap_err(), STALE, "{bad}");
        }
    }

    #[test]
    fn a_substituted_command_stays_as_written() {
        let text =
            r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "{env:GUARD}"}]}]}}"#;
        let raw = data(text);
        // What the core reads: the variable's value.
        let mut read = raw.clone();
        read["hooks"]["Stop"][0]["hooks"][0]["command"] = json!("/bin/guard");
        let id = id_of(&serde_json::to_string(&read).unwrap(), "/bin/guard");
        let out = update(
            text,
            &read,
            &id,
            &draft("Stop", None, "/bin/guard", Some(3.0)),
        )
        .unwrap();
        assert_eq!(
            parsed(&out)["hooks"]["Stop"][0]["hooks"][0],
            json!({"type": "command", "command": "{env:GUARD}", "timeout": 3})
        );
    }

    #[test]
    fn disable_writes_one_key() {
        let on = disable(FILE, true).unwrap();
        assert_eq!(parsed(&on)["disable_hooks"], true);
        assert!(on.contains("// mine"));
        let off = disable(&on, false).unwrap();
        assert_eq!(parsed(&off), parsed(FILE));
    }

    fn info(event: &str, command: &str, source: &str, scope: HookScope) -> HookInfo {
        HookInfo {
            event: event.into(),
            matcher: None,
            command: command.into(),
            timeout: 600,
            shell: None,
            source: source.into(),
            scope,
            active: true,
        }
    }

    #[test]
    fn a_repositorys_command_is_shown_with_what_it_hides_written_out() {
        let tricky = info(
            "Stop",
            "curl x | sh \u{202e}\u{1b}[2K",
            "/p/anysola.json",
            HookScope::Project,
        );
        let shown = hook_json(&tricky, None, "/bin/bash");
        assert_eq!(shown["command"], "curl x | sh \\u{202e}\\x1b[2K");
        // A hook the form edits keeps its text.
        let own = hook_json(&tricky, Some("Stop:0:0:abc".into()), "/bin/bash");
        assert_eq!(own["command"], "curl x | sh \u{202e}\u{1b}[2K");
    }

    #[test]
    fn ids_go_only_to_the_global_files_hooks_while_they_line_up() {
        let text = r#"{"hooks": {"Stop": [{"hooks": [
            {"type": "command", "command": "a"}, {"type": "command", "command": "b"}]}]}}"#;
        let infos = [
            info("Stop", "m", "/etc/anysola/managed.json", HookScope::Managed),
            info("Stop", "a", "/g/config.json", HookScope::User),
            info("Stop", "p", "/p/anysola.json", HookScope::Project),
            info("Stop", "b", "/g/config.json", HookScope::User),
        ];
        let out = with_ids(&infos, "/g/config.json", listed(&data(text)), &|_| {
            "sh".into()
        });
        let ids: Vec<_> = out.iter().map(|h| h["id"].as_str().is_some()).collect();
        assert_eq!(ids, [false, true, false, true]);
        assert_eq!(out[1]["scope"], "user");
        assert_eq!(out[1]["matcher"], Value::Null);
        // The core read the file before it changed: no ids from the first difference on.
        let infos = [
            info("Stop", "old", "/g/config.json", HookScope::User),
            info("Stop", "b", "/g/config.json", HookScope::User),
        ];
        let out = with_ids(&infos, "/g/config.json", listed(&data(text)), &|_| {
            "sh".into()
        });
        assert!(out.iter().all(|h| h["id"].is_null()));
    }

    /// A hook's own shell is written with it, and choosing the default again takes it out (the
    /// handler's other keys stay).
    #[test]
    fn a_hooks_shell_is_written_and_taken_out() {
        let with = |shell: Option<&str>| {
            check(&Draft {
                event: "Stop".into(),
                matcher: None,
                command: "x".into(),
                timeout: None,
                shell: shell.map(str::to_string),
            })
            .unwrap()
        };
        let out = add("{}", &Map::new(), &with(Some(" powershell "))).unwrap();
        assert_eq!(
            parsed(&out)["hooks"]["Stop"][0]["hooks"][0],
            json!({"type": "command", "command": "x", "shell": "powershell"})
        );
        let text = r#"{"hooks": {"Stop": [{"hooks": [
            {"type": "command", "command": "x", "shell": "cmd", "statusMessage": "kept"}]}]}}"#;
        let out = update(text, &data(text), &id_of(text, "x"), &with(None)).unwrap();
        assert_eq!(
            parsed(&out)["hooks"]["Stop"][0]["hooks"][0],
            json!({"type": "command", "command": "x", "statusMessage": "kept"})
        );
        assert_eq!(with(Some("  ")).shell, None);
        // What it runs in goes to the window with it.
        let mut own = info("Stop", "x", "/g/config.json", HookScope::User);
        own.shell = Some("powershell".into());
        let shown = hook_json(&own, None, "C:/pwsh.exe");
        assert_eq!(
            (shown["shell"].as_str(), shown["runsIn"].as_str()),
            (Some("powershell"), Some("C:/pwsh.exe"))
        );
    }

    /// Only hooks without a shell whose command is exactly one of the ≤1.2 PowerShell starters
    /// get `"shell": "powershell"`; any other text, one changed by a character included, stays.
    #[test]
    fn the_powershell_starters_keep_running_in_powershell() {
        let [block, lint, test] = POWERSHELL_STARTERS;
        let file = json!({"model": "anysola/x", "hooks": {
            "PreToolUse": [{"matcher": "Bash", "hooks": [
                {"type": "command", "command": block},
                {"type": "command", "command": format!("{block} ")},
            ]}],
            "PostToolUse": [{"matcher": "Edit|Write", "hooks": [
                {"type": "command", "command": lint, "shell": "bash"}]}],
            "Stop": [{"hooks": [
                {"type": "command", "command": test, "timeout": 30},
                {"type": "command", "command": "npm test --silent >&2 || exit 2"}]}],
        }});
        let text = format!(
            "// mine\n{}\n",
            serde_json::to_string_pretty(&file).unwrap()
        );
        let out = pin_powershell_starters(&text).unwrap().unwrap();
        let hooks = &parsed(&out)["hooks"];
        assert_eq!(hooks["PreToolUse"][0]["hooks"][0]["shell"], "powershell");
        assert!(hooks["PreToolUse"][0]["hooks"][1].get("shell").is_none());
        assert_eq!(hooks["PostToolUse"][0]["hooks"][0]["shell"], "bash");
        assert_eq!(
            hooks["Stop"][0]["hooks"][0],
            json!({"type": "command", "command": test, "timeout": 30, "shell": "powershell"})
        );
        assert!(hooks["Stop"][0]["hooks"][1].get("shell").is_none());
        assert!(out.starts_with("// mine"), "the rest of the file stays");
        // Done once: nothing more to change.
        assert_eq!(pin_powershell_starters(&out).unwrap(), None);
        assert_eq!(pin_powershell_starters("{}").unwrap(), None);
    }

    #[test]
    fn the_starters_move_once_and_an_unreadable_file_waits() {
        let dir = std::env::temp_dir().join(format!(
            "anysola-starters-{}-{}",
            std::process::id(),
            anysola_core::ids::part()
        ));
        std::fs::create_dir_all(dir.join(".anysola")).unwrap();
        let paths = anysola_core::paths::Paths::isolated(dir.clone());
        let db = anysola_core::storage::Db::open(&dir.join("a.db")).unwrap();
        let file = paths.config_file();
        std::fs::write(&file, "{ nope").unwrap();
        pin_powershell_starters_at_start(&paths, Some(&db));
        assert_eq!(
            db.kv_get(STARTERS_MARKER).unwrap(),
            None,
            "tried again next start"
        );
        let text = json!({"hooks": {"Stop": [{"hooks": [
            {"type": "command", "command": POWERSHELL_STARTERS[2]}]}]}})
        .to_string();
        std::fs::write(&file, &text).unwrap();
        pin_powershell_starters_at_start(&paths, Some(&db));
        let after = std::fs::read_to_string(&file).unwrap();
        assert_eq!(
            parsed(&after)["hooks"]["Stop"][0]["hooks"][0]["shell"],
            "powershell"
        );
        assert!(db.kv_get(STARTERS_MARKER).unwrap().is_some());
        // Marked: a hook the user puts back without a shell later stays so.
        std::fs::write(&file, &text).unwrap();
        pin_powershell_starters_at_start(&paths, Some(&db));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_default_timeout_is_not_written() {
        let out = add("{}", &Map::new(), &draft("Stop", None, "x", Some(600.0))).unwrap();
        assert!(
            parsed(&out)["hooks"]["Stop"][0]["hooks"][0]
                .get("timeout")
                .is_none()
        );
        let out = add(
            "{}",
            &Map::new(),
            &draft("UserPromptSubmit", None, "x", Some(600.0)),
        )
        .unwrap();
        assert_eq!(
            parsed(&out)["hooks"]["UserPromptSubmit"][0]["hooks"][0]["timeout"],
            600
        );
    }

    #[test]
    fn the_global_file_as_the_core_reads_it() {
        let dir = std::env::temp_dir().join(format!(
            "anysola-hooks-{}-{}",
            std::process::id(),
            anysola_core::ids::part()
        ));
        std::fs::create_dir_all(dir.join(".anysola")).unwrap();
        std::fs::create_dir_all(dir.join("work")).unwrap();
        let paths = anysola_core::paths::Paths::isolated(dir.clone());
        std::fs::write(paths.config_file(), FILE).unwrap();
        let (text, data) = crate::config::read(&paths).unwrap();
        let commands: Vec<_> = listed(&data).into_iter().map(|l| l.command).collect();
        assert_eq!(commands, ["one", "two", "three", "four"]);
        let out = remove(&text, &data, &id_of(&text, "three")).unwrap();
        crate::config::write(&paths.config_file(), &out).unwrap();
        let (_, data) = crate::config::read(&paths).unwrap();
        assert_eq!(listed(&data).len(), 3);
        // The core's own load names the file as `config.rs` and `servers.rs` compare it.
        let config = anysola_core::config::load(&paths, &dir.join("work"), &Default::default());
        let global = paths.config_file().display().to_string();
        let sources: Vec<_> = config
            .hooks
            .documents
            .iter()
            .map(|d| d.path.as_str())
            .collect();
        assert_eq!(sources, [global.as_str()]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
