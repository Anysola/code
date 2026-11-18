//! The app's own preferences (`~/.anysola/desktop.json`): what the window shows and how, and
//! what the Rust side reads itself (notifications, Discord, Claude Code's skill folders). A file
//! that can't be read or parsed gives the defaults and is left where it is; unknown keys and
//! values are dropped, missing ones take their defaults.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;

use crate::{Answer, App, text};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    pub general: General,
    pub appearance: Appearance,
    pub conversation: Conversation,
    pub notifications: Notifications,
    pub discord: Discord,
    pub sources: Sources,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct General {
    pub default_folder: Option<String>,
    pub start_mode: String,
    pub check_updates: bool,
    /// An editor's id (see `editor.rs`); none picks the first one found.
    pub editor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Appearance {
    pub theme: String,
    pub text_size: String,
    pub width: String,
    pub wrap_code: bool,
    pub reduce_motion: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Conversation {
    pub changes: String,
    pub commands: String,
    pub lookups: String,
    pub shorten: bool,
    pub send: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Notifications {
    pub enabled: bool,
    pub approval: bool,
    pub question: bool,
    pub finished: bool,
    pub failed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Discord {
    pub enabled: bool,
    pub project: bool,
    pub session: bool,
    pub model: bool,
    pub elapsed: bool,
    pub application_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Sources {
    pub claude_skills: bool,
}

impl Default for General {
    fn default() -> Self {
        General {
            default_folder: None,
            start_mode: "last".into(),
            check_updates: true,
            editor: None,
        }
    }
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance {
            theme: "dark".into(),
            text_size: "default".into(),
            width: "default".into(),
            wrap_code: false,
            reduce_motion: false,
        }
    }
}

impl Default for Conversation {
    fn default() -> Self {
        Conversation {
            changes: "closed".into(),
            commands: "closed".into(),
            lookups: "grouped".into(),
            shorten: true,
            send: "enter".into(),
        }
    }
}

impl Default for Notifications {
    fn default() -> Self {
        Notifications {
            enabled: true,
            approval: true,
            question: true,
            finished: true,
            failed: true,
        }
    }
}

impl Default for Discord {
    fn default() -> Self {
        Discord {
            enabled: false,
            project: true,
            session: false,
            model: true,
            elapsed: true,
            application_id: None,
        }
    }
}

impl Default for Sources {
    fn default() -> Self {
        Sources {
            claude_skills: true,
        }
    }
}

/// `value` when it is one of `allowed`, else `fallback`: a value from a newer or hand-edited
/// file must not reach a window that can't show it.
fn one_of(value: &mut String, allowed: &[&str], fallback: &str) {
    if !allowed.contains(&value.as_str()) {
        *value = fallback.into();
    }
}

/// Empty text counts as unset.
fn optional(value: &mut Option<String>) {
    if value.as_deref().is_some_and(|v| v.trim().is_empty()) {
        *value = None;
    } else if let Some(v) = value {
        *v = v.trim().to_string();
    }
}

impl Prefs {
    fn normalized(mut self) -> Prefs {
        let defaults = Prefs::default();
        one_of(
            &mut self.general.start_mode,
            &["last", "ask", "edits", "auto"],
            &defaults.general.start_mode,
        );
        optional(&mut self.general.default_folder);
        optional(&mut self.general.editor);
        let a = &mut self.appearance;
        one_of(&mut a.theme, &["dark", "light", "system"], "dark");
        one_of(&mut a.text_size, &["small", "default", "large"], "default");
        one_of(&mut a.width, &["narrow", "default", "wide"], "default");
        let c = &mut self.conversation;
        one_of(&mut c.changes, &["closed", "open"], "closed");
        one_of(&mut c.commands, &["closed", "open"], "closed");
        one_of(&mut c.lookups, &["grouped", "listed"], "grouped");
        one_of(&mut c.send, &["enter", "ctrl-enter"], "enter");
        optional(&mut self.discord.application_id);
        self
    }

    /// The file's preferences; the defaults when it is missing, unreadable or not valid JSON
    /// (the file is kept: a newer version may have written it).
    #[cfg(test)]
    pub fn load(file: &Path) -> Prefs {
        Prefs::read(file).0
    }

    /// [`Prefs::load`], and whether the file is there but couldn't be taken (to be moved aside
    /// before the first save replaces it).
    pub fn read(file: &Path) -> (Prefs, bool) {
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Prefs::default(), false),
            Err(_) => return (Prefs::default(), true),
        };
        let prefs = match serde_json::from_str::<Value>(&text) {
            // Field by field: one value of the wrong type costs that section, not the file.
            Ok(Value::Object(map)) => {
                let mut prefs = Prefs::default();
                macro_rules! section {
                    ($key:literal, $field:ident) => {
                        if let Some(Ok(v)) =
                            map.get($key).map(|v| serde_json::from_value(v.clone()))
                        {
                            prefs.$field = v;
                        }
                    };
                }
                section!("general", general);
                section!("appearance", appearance);
                section!("conversation", conversation);
                section!("notifications", notifications);
                section!("discord", discord);
                section!("sources", sources);
                prefs.normalized()
            }
            _ => {
                anysola_core::log::warn(
                    "ignoring unreadable desktop preferences",
                    &[("file", &file.display().to_string())],
                );
                return (Prefs::default(), true);
            }
        };
        (prefs, false)
    }

    /// Moves a file that couldn't be read to `desktop.json.bak`, so saving doesn't lose it.
    pub fn set_aside(file: &Path) -> Result<(), String> {
        let backup = file.with_extension("json.bak");
        match std::fs::rename(file, &backup) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!(
                "Can’t move {} aside to {} before saving: {e}",
                file.display(),
                backup.display()
            )),
        }
    }

    /// Writes the file whole: a temporary file renamed over it, so a crash never leaves half.
    pub fn save(&self, file: &Path) -> Result<(), String> {
        let fail = |e: std::io::Error| format!("Can’t save {}: {e}", file.display());
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(fail)?;
        }
        let temp = file.with_extension(format!("json.{}.tmp", std::process::id()));
        let text = serde_json::to_string_pretty(self).map_err(text)? + "\n";
        std::fs::write(&temp, text).map_err(fail)?;
        std::fs::rename(&temp, file).map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            fail(e)
        })
    }
}

#[tauri::command]
pub async fn prefs_get(app: State<'_, App>) -> Answer {
    serde_json::to_value(app.prefs()).map_err(text)
}

/// Saves the whole object, then applies what the Rust side reads itself.
#[tauri::command]
pub async fn prefs_set(app: State<'_, App>, prefs: Prefs) -> Answer {
    let prefs = prefs.normalized();
    let file = app.paths.desktop_prefs();
    use std::sync::atomic::Ordering;
    if app.prefs_invalid.swap(false, Ordering::SeqCst)
        && let Err(error) = Prefs::set_aside(&file)
    {
        app.prefs_invalid.store(true, Ordering::SeqCst);
        return Err(error);
    }
    prefs.save(&file)?;
    let before = std::mem::replace(
        &mut *app.prefs.write().unwrap_or_else(|e| e.into_inner()),
        prefs.clone(),
    );
    if before.discord != prefs.discord
        && let Some(discord) = app.discord.get()
    {
        discord.set_prefs(prefs.discord.clone());
    }
    if before.sources.claude_skills != prefs.sources.claude_skills {
        anysola_core::skill::set_claude_skills(prefs.sources.claude_skills);
        // Running cores read their skill folders again now, before this returns, so the
        // window's next `sources` already shows the change.
        for (_, core) in app.cores_running().await {
            core.reload().await;
        }
    }
    serde_json::to_value(prefs).map_err(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "anysola-prefs-{name}-{}-{}",
            std::process::id(),
            anysola_core::ids::part()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_have_the_contract_shape() {
        assert_eq!(
            serde_json::to_value(Prefs::default()).unwrap(),
            json!({
                "general": {"defaultFolder": null, "startMode": "last", "checkUpdates": true,
                            "editor": null},
                "appearance": {"theme": "dark", "textSize": "default", "width": "default",
                               "wrapCode": false, "reduceMotion": false},
                "conversation": {"changes": "closed", "commands": "closed", "lookups": "grouped",
                                 "shorten": true, "send": "enter"},
                "notifications": {"enabled": true, "approval": true, "question": true,
                                  "finished": true, "failed": true},
                "discord": {"enabled": false, "project": true, "session": false, "model": true,
                            "elapsed": true, "applicationId": null},
                "sources": {"claudeSkills": true},
            })
        );
    }

    #[test]
    fn a_partial_or_odd_file_keeps_what_it_can() {
        let dir = temp("partial");
        let file = dir.join("desktop.json");
        std::fs::write(
            &file,
            r#"{"appearance": {"theme": "light", "textSize": "huge", "extra": 1},
                "notifications": {"finished": false},
                "discord": "not an object",
                "general": {"checkUpdates": false},
                "unknown": true}"#,
        )
        .unwrap();
        let prefs = Prefs::load(&file);
        assert_eq!(prefs.appearance.theme, "light");
        assert_eq!(prefs.appearance.text_size, "default");
        assert!(!prefs.notifications.finished && prefs.notifications.approval);
        assert_eq!(prefs.discord, Discord::default());
        assert!(!prefs.general.check_updates);
        assert_eq!(
            prefs.general.editor, None,
            "a file from before editors had no key"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_invalid_file_gives_defaults_and_stays() {
        let dir = temp("invalid");
        let file = dir.join("desktop.json");
        std::fs::write(&file, "{ not json").unwrap();
        assert_eq!(Prefs::load(&file), Prefs::default());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ not json");
        assert_eq!(Prefs::load(&dir.join("missing.json")), Prefs::default());
        assert!(Prefs::read(&file).1, "to be moved aside before a save");
        assert!(!Prefs::read(&dir.join("missing.json")).1);
        Prefs::set_aside(&file).unwrap();
        assert!(!file.exists());
        assert_eq!(
            std::fs::read_to_string(dir.join("desktop.json.bak")).unwrap(),
            "{ not json"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saves_and_loads_back() {
        let dir = temp("save");
        let file = dir.join("nested/desktop.json");
        let mut prefs = Prefs::default();
        prefs.discord.application_id = Some(" 123 ".into());
        prefs.general.start_mode = "auto".into();
        prefs.general.editor = Some("cursor".into());
        let prefs = prefs.normalized();
        prefs.save(&file).unwrap();
        assert_eq!(Prefs::load(&file), prefs);
        assert_eq!(prefs.discord.application_id.as_deref(), Some("123"));
        let left: Vec<_> = std::fs::read_dir(file.parent().unwrap()).unwrap().collect();
        assert_eq!(left.len(), 1, "no temporary file left behind");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
