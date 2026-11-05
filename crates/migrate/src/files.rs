//! TUI settings and state files. A target that already exists is left alone, so an import that is
//! retried after a failure finds the same files.
//!
//! Destinations: `cli.json` → `~/.anysola/tui.json`; `<state>/prompt-history.jsonl`,
//! `prompt-stash.jsonl`, `model.json` → `~/.anysola/state/`; `<state>/latest/tui/*.json` (tabs,
//! layout, …) → `~/.anysola/state/tui/`.

use std::io;
use std::path::Path;

use anysola_core::paths::Paths;
use serde_json::{Map, Value, json};

use crate::source::Source;

const SCHEMA: &str = "https://opencode.ai/v2/cli.json";
const KEYBINDS: &str = include_str!("keybinds.txt");

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    io::Write::write_all(&mut options.open(&temp)?, bytes)?;
    std::fs::rename(&temp, path)
}

fn copy_new(from: &Path, to: &Path) -> io::Result<()> {
    if to.exists() || !from.is_file() {
        return Ok(());
    }
    write_private(to, &std::fs::read(from)?)
}

/// `cli.json` byte for byte; without one, the v1 `tui.json` and `kv.json` the way 0.5.1 migrated
/// them into `cli.json`.
pub fn settings(source: &Source, paths: &Paths) -> io::Result<()> {
    let target = paths.data.join("tui.json");
    let cli = source.config.join("cli.json");
    if target.exists() || cli.is_file() {
        return copy_new(&cli, &target);
    }
    let read = |path: &Path| {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| anysola_core::config::jsonc::parse(&text).ok())
            .and_then(|value| value.as_object().cloned())
    };
    let legacy = read(&source.config.join("tui.json"));
    let kv = read(&source.state.join("kv.json")).unwrap_or_default();
    let values = migrate_v1(legacy.as_ref(), &kv);
    if values.is_empty() {
        return Ok(());
    }
    let mut out = Map::new();
    out.insert("$schema".into(), SCHEMA.into());
    out.extend(values);
    let text = serde_json::to_string_pretty(&Value::Object(out)).unwrap_or_default() + "\n";
    write_private(&target, text.as_bytes())
}

/// Prompt history and stash, model recents/favorites/variants and the TUI's stored keys.
pub fn state(source: &Source, paths: &Paths) -> io::Result<()> {
    let to = paths.state_dir();
    for name in ["prompt-history.jsonl", "prompt-stash.jsonl", "model.json"] {
        copy_new(&source.state.join(name), &to.join(name))?;
    }
    let Ok(entries) = std::fs::read_dir(source.state.join("latest").join("tui")) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            copy_new(&path, &to.join("tui").join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn keybind_target(name: &str) -> Option<&'static str> {
    let mut target = name;
    for line in KEYBINDS.lines() {
        if let Some((from, to)) = line.split_once(' ')
            && from == name
        {
            target = to;
        }
    }
    KEYBINDS
        .lines()
        .find(|line| !line.contains(' ') && *line == target)
}

/// Adds `name` with the present entries, unless none is present.
fn section(out: &mut Map<String, Value>, name: &str, entries: Vec<(&str, Option<Value>)>) {
    let map: Map<String, Value> = entries
        .into_iter()
        .filter_map(|(k, v)| Some((k.to_string(), v?)))
        .collect();
    if !map.is_empty() {
        out.insert(name.into(), Value::Object(map));
    }
}

/// `ConfigMigration.migrateV1`: v1 TUI options and KV toggles as `cli.json` keys.
fn migrate_v1(legacy: Option<&Map<String, Value>>, kv: &Map<String, Value>) -> Map<String, Value> {
    let legacy_get = |key: &str| legacy.and_then(|l| l.get(key)).filter(|v| !v.is_null());
    let kv_get = |key: &str| kv.get(key).filter(|v| !v.is_null());
    let mut out = Map::new();

    section(
        &mut out,
        "theme",
        vec![
            (
                "name",
                legacy_get("theme").or_else(|| kv_get("theme")).cloned(),
            ),
            ("mode", kv_get("theme_mode_lock").cloned()),
        ],
    );
    if let Some(Value::Object(keybinds)) = legacy_get("keybinds") {
        let renamed: Map<String, Value> = keybinds
            .iter()
            .filter_map(|(name, value)| Some((keybind_target(name)?.to_string(), value.clone())))
            .collect();
        out.insert("keybinds".into(), Value::Object(renamed));
    }
    let mut plugins: Vec<Value> = legacy_get("plugin")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|plugin| match plugin.as_array() {
            Some(pair) => json!({"package": pair.first(), "options": pair.get(1)}),
            None => plugin.clone(),
        })
        .collect();
    if let Some(Value::Object(enabled)) = legacy_get("plugin_enabled") {
        for (id, on) in enabled {
            let truthy = !matches!(on, Value::Bool(false) | Value::Null);
            plugins.push(Value::String(if truthy {
                id.clone()
            } else {
                format!("-{id}")
            }));
        }
    }
    if !plugins.is_empty() {
        out.insert("plugins".into(), Value::Array(plugins));
    }
    if let Some(timeout) = legacy_get("leader_timeout") {
        out.insert("leader".into(), json!({ "timeout": timeout }));
    }
    section(
        &mut out,
        "scroll",
        vec![
            ("speed", legacy_get("scroll_speed").cloned()),
            (
                "acceleration",
                legacy_get("scroll_acceleration")
                    .and_then(|a| a.get("enabled"))
                    .cloned(),
            ),
        ],
    );
    let mut attention: Vec<(String, Option<Value>)> = legacy_get("attention")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(k, v)| (k.clone(), Some(v.clone())))
        .collect();
    attention.push(("sound_pack".into(), kv_get("attention_sound_pack").cloned()));
    section(
        &mut out,
        "attention",
        attention
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect(),
    );
    let view = kv_get("diff_viewer_view").cloned().or_else(|| {
        (legacy_get("diff_style").and_then(Value::as_str) == Some("stacked"))
            .then(|| "unified".into())
    });
    section(
        &mut out,
        "diffs",
        vec![
            ("wrap", kv_get("diff_wrap_mode").cloned()),
            ("tree", kv_get("diff_viewer_show_file_tree").cloned()),
            ("single", kv_get("diff_viewer_single_patch").cloned()),
            ("view", view),
        ],
    );
    section(
        &mut out,
        "terminal",
        vec![("title", kv_get("terminal_title_enabled").cloned())],
    );
    let flag = |key: &str, on: &str, off: &str| {
        kv_get(key).map(|v| Value::String(if v.as_bool() == Some(true) { on } else { off }.into()))
    };
    section(
        &mut out,
        "prompt",
        vec![
            ("editor", kv_get("file_context_enabled").cloned()),
            ("paste", flag("paste_summary_enabled", "compact", "full")),
        ],
    );
    section(
        &mut out,
        "session",
        vec![
            ("sidebar", kv_get("sidebar").cloned()),
            ("scrollbar", kv_get("scrollbar_visible").cloned()),
            (
                "thinking",
                kv_get("thinking_mode")
                    .cloned()
                    .or_else(|| flag("thinking_visibility", "show", "hide")),
            ),
            ("grouping", flag("exploration_grouping", "auto", "none")),
        ],
    );
    for (key, from) in [
        ("animations", kv_get("animations_enabled")),
        ("mouse", legacy_get("mouse")),
        ("cursor", legacy_get("cursor")),
    ] {
        if let Some(value) = from {
            out.insert(key.into(), value.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_settings_become_cli_keys() {
        let legacy = json!({
            "theme": "nord",
            "keybinds": {"sidebar_toggle": "<leader>s", "input_undo": "ctrl+z", "nope": "x"},
            "plugin": ["a", ["b", {"x": 1}]],
            "plugin_enabled": {"c": false},
            "leader_timeout": 900,
            "diff_style": "stacked",
            "mouse": false
        });
        let kv = json!({"theme_mode_lock": "dark", "thinking_visibility": false, "paste_summary_enabled": true});
        let out = migrate_v1(legacy.as_object(), kv.as_object().unwrap());
        assert_eq!(
            Value::Object(out),
            json!({
                "theme": {"name": "nord", "mode": "dark"},
                "keybinds": {"session.sidebar.toggle": "<leader>s", "input.undo": "ctrl+z"},
                "plugins": ["a", {"package": "b", "options": {"x": 1}}, "-c"],
                "leader": {"timeout": 900},
                "diffs": {"view": "unified"},
                "prompt": {"paste": "compact"},
                "session": {"thinking": "hide"},
                "mouse": false
            })
        );
    }

    #[test]
    fn nothing_to_migrate_is_empty() {
        assert!(migrate_v1(None, &Map::new()).is_empty());
    }
}
