//! The Settings dialog rows (`V/tui/dialogs.md §8`): where each lives in `tui.json`, its values
//! and labels, and how ←/→ cycle or step it.

use serde_json::Value;

use crate::dialog::{Action, Kind, Opt, Select};

pub enum Values {
    /// Fixed choices with their labels.
    Choice(&'static [(&'static str, &'static str)]),
    Flag,
    /// `min`, `max`, `step` and how the value is shown.
    Number(f64, f64, f64, fn(f64) -> String),
    Theme,
}

pub struct Setting {
    pub category: &'static str,
    pub title: &'static str,
    pub path: &'static [&'static str],
    pub values: Values,
    pub default: Value,
}

const OFF_ON: Values = Values::Flag;

fn fixed(n: f64) -> String {
    format!("{n:.2}")
}

fn ms(n: f64) -> String {
    format!("{n} ms")
}

fn percent(n: f64) -> String {
    format!("{}%", (n * 100.0).round())
}

macro_rules! choice {
    ($($v:literal $(=> $l:literal)?),*) => {
        Values::Choice(&[$( ($v, choice!(@l $v $(, $l)?)) ),*])
    };
    (@l $v:literal) => { $v };
    (@l $v:literal, $l:literal) => { $l };
}

fn s(text: &str) -> Value {
    Value::String(text.into())
}

pub fn all() -> Vec<Setting> {
    let copy = if cfg!(windows) { "manual" } else { "select" };
    vec![
        Setting {
            category: "Appearance",
            title: "Theme",
            path: &["theme", "name"],
            values: Values::Theme,
            default: s(crate::theme::DEFAULT),
        },
        Setting {
            category: "Appearance",
            title: "Color mode",
            path: &["theme", "mode"],
            values: choice!("system", "dark", "light"),
            default: s("system"),
        },
        // [fix] §B11: the row shows `on` when unset, as animations then run.
        Setting {
            category: "Appearance",
            title: "Animations",
            path: &["animations"],
            values: OFF_ON,
            default: Value::Bool(true),
        },
        Setting {
            category: "Session",
            title: "Sidebar",
            path: &["session", "sidebar"],
            values: choice!("hide", "auto"),
            default: s("auto"),
        },
        Setting {
            category: "Session",
            title: "Scrollbar",
            path: &["session", "scrollbar"],
            values: OFF_ON,
            default: Value::Bool(false),
        },
        Setting {
            category: "Session",
            title: "Thinking",
            path: &["session", "thinking"],
            values: choice!("hide", "show"),
            default: s("hide"),
        },
        Setting {
            category: "Session",
            title: "Markdown",
            path: &["session", "markdown"],
            values: choice!("source", "rendered"),
            default: s("rendered"),
        },
        Setting {
            category: "Session",
            title: "Tool grouping",
            path: &["session", "grouping"],
            values: choice!("none", "auto"),
            default: s("auto"),
        },
        Setting {
            category: "Session",
            title: "Verbosity",
            path: &["session", "verbosity"],
            values: choice!("low", "medium", "high"),
            default: s("medium"),
        },
        Setting {
            category: "Session",
            title: "Transcript images",
            path: &["session", "image_preview"],
            values: OFF_ON,
            default: Value::Bool(false),
        },
        Setting {
            category: "Session",
            title: "TPS",
            path: &["session", "tps"],
            values: OFF_ON,
            default: Value::Bool(true),
        },
        Setting {
            category: "Session",
            title: "New session location",
            path: &["session", "new_location"],
            values: choice!("launch" => "launch directory", "inherit" => "active session"),
            default: s("launch"),
        },
        Setting {
            category: "Session",
            title: "Permissions",
            path: &["session", "permissions"],
            values: choice!("prompt", "autoaccept" => "auto accept"),
            default: s("prompt"),
        },
        Setting {
            category: "Tabs",
            title: "Mode",
            path: &["tabs", "mode"],
            values: choice!("off", "on", "auto"),
            default: s("auto"),
        },
        Setting {
            category: "Tabs",
            title: "Scope",
            path: &["tabs", "scope"],
            values: choice!("cwd" => "current directory", "global"),
            default: s("cwd"),
        },
        Setting {
            category: "Tabs",
            title: "Layout",
            path: &["tabs", "layout"],
            values: choice!("horizontal", "vertical"),
            default: s("horizontal"),
        },
        Setting {
            category: "Tabs",
            title: "Indicators",
            path: &["tabs", "indicators"],
            values: choice!("status" => "status icons", "numbers" => "always show numbers"),
            default: s("status"),
        },
        Setting {
            category: "Diffs",
            title: "Layout",
            path: &["diffs", "view"],
            values: choice!("auto", "split", "unified"),
            default: s("auto"),
        },
        Setting {
            category: "Diffs",
            title: "Wrapping",
            path: &["diffs", "wrap"],
            values: choice!("none", "word"),
            default: s("word"),
        },
        Setting {
            category: "Diffs",
            title: "File tree",
            path: &["diffs", "tree"],
            values: OFF_ON,
            default: Value::Bool(true),
        },
        Setting {
            category: "Diffs",
            title: "Single patch",
            path: &["diffs", "single"],
            values: OFF_ON,
            default: Value::Bool(false),
        },
        Setting {
            category: "Input",
            title: "Scroll speed",
            path: &["scroll", "speed"],
            values: Values::Number(0.25, 10.0, 0.25, fixed),
            default: Value::from(3),
        },
        Setting {
            category: "Input",
            title: "Acceleration",
            path: &["scroll", "acceleration"],
            values: OFF_ON,
            default: Value::Bool(false),
        },
        Setting {
            category: "Input",
            title: "Mouse",
            path: &["mouse"],
            values: OFF_ON,
            default: Value::Bool(true),
        },
        Setting {
            category: "Input",
            title: "Editor context",
            path: &["prompt", "editor"],
            values: OFF_ON,
            default: Value::Bool(true),
        },
        Setting {
            category: "Input",
            title: "Large pastes",
            path: &["prompt", "paste"],
            values: choice!("compact", "full"),
            default: s("compact"),
        },
        Setting {
            category: "Input",
            title: "Image previews",
            path: &["prompt", "image_preview"],
            values: OFF_ON,
            default: Value::Bool(false),
        },
        Setting {
            category: "Input",
            title: "Leader timeout",
            path: &["leader", "timeout"],
            values: Values::Number(250.0, 10000.0, 250.0, ms),
            default: Value::from(2000),
        },
        Setting {
            category: "Alerts",
            title: "Notifications",
            path: &["attention", "notifications"],
            values: OFF_ON,
            default: Value::Bool(false),
        },
        Setting {
            category: "Alerts",
            title: "Sounds",
            path: &["attention", "sound"],
            values: OFF_ON,
            default: Value::Bool(false),
        },
        Setting {
            category: "Alerts",
            title: "Volume",
            path: &["attention", "volume"],
            values: Values::Number(0.0, 1.0, 0.1, percent),
            default: Value::from(0.4),
        },
        Setting {
            category: "Terminal",
            title: "Window title",
            path: &["terminal", "title"],
            values: OFF_ON,
            default: Value::Bool(true),
        },
        Setting {
            category: "Terminal",
            title: "Copy behavior",
            path: &["terminal", "copy"],
            values: choice!("manual", "select"),
            default: s(copy),
        },
        Setting {
            category: "Debug",
            title: "Developer tools",
            path: &["debug", "devtools"],
            values: OFF_ON,
            default: Value::Bool(false),
        },
    ]
}

fn read<'a>(tui: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(tui, |v, k| v.get(k))
}

impl Setting {
    /// The value in effect (`theme` may be a bare string in older files).
    pub fn value(&self, tui: &Value) -> Value {
        if self.path == ["theme", "name"]
            && let Some(name) = tui.get("theme").filter(|t| t.is_string())
        {
            return name.clone();
        }
        read(tui, self.path)
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or_else(|| self.default.clone())
    }

    pub fn label(&self, value: &Value) -> String {
        match &self.values {
            Values::Choice(choices) => {
                let text = value.as_str().unwrap_or_default();
                choices
                    .iter()
                    .find(|(v, _)| *v == text)
                    .map_or(text.to_string(), |(_, l)| l.to_string())
            }
            Values::Flag => if value.as_bool() == Some(true) {
                "on"
            } else {
                "off"
            }
            .into(),
            Values::Number(_, _, _, show) => show(value.as_f64().unwrap_or_default()),
            Values::Theme => value.as_str().unwrap_or_default().to_string(),
        }
    }

    /// The value one step forward (`+1`) or back (`-1`); themes cycle through `themes`.
    pub fn step(&self, value: &Value, delta: i32, themes: &[String]) -> Value {
        let cycle = |len: usize, at: usize| (at as i32 + delta).rem_euclid(len as i32) as usize;
        match &self.values {
            Values::Choice(choices) => {
                let text = value.as_str().unwrap_or_default();
                let at = choices.iter().position(|(v, _)| *v == text).unwrap_or(0);
                Value::String(choices[cycle(choices.len(), at)].0.into())
            }
            Values::Flag => Value::Bool(value.as_bool() != Some(true)),
            Values::Number(min, max, step, _) => {
                let n = value.as_f64().unwrap_or(*min) + f64::from(delta) * step;
                let n = (n.clamp(*min, *max) / step).round() * step;
                serde_json::Number::from_f64((n * 100.0).round() / 100.0)
                    .map_or(Value::Null, Value::Number)
            }
            Values::Theme => {
                let text = value.as_str().unwrap_or_default();
                let at = themes.iter().position(|t| t == text).unwrap_or(0);
                themes
                    .get(cycle(themes.len().max(1), at))
                    .map_or(Value::Null, |t| Value::String(t.clone()))
            }
        }
    }
}

/// The `Settings` dialog over the values in effect, focused on row `focus` (marked `●`).
pub fn dialog(tui: &Value, focus: usize) -> Select {
    let options = all()
        .iter()
        .enumerate()
        .map(|(i, s)| {
            Opt::new(s.title, i.to_string())
                .category(s.category)
                .footer(s.label(&s.value(tui)))
                .current(i == focus)
        })
        .collect();
    let mut select = Select::new(Kind::Settings, "Settings", options);
    select.actions = vec![Action::left("change", "←/→")];
    select
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn labels_and_steps() {
        let settings = all();
        let tui = json!({"scroll": {"speed": 9.9}});
        let speed = settings.iter().find(|s| s.title == "Scroll speed").unwrap();
        let next = speed.step(&speed.value(&tui), 1, &[]);
        assert_eq!(speed.label(&next), "10.00");
        let animations = &settings[2];
        assert_eq!(animations.label(&animations.value(&tui)), "on");
        let volume = settings.iter().find(|s| s.title == "Volume").unwrap();
        assert_eq!(volume.label(&volume.value(&tui)), "40%");
        let tabs = settings
            .iter()
            .find(|s| s.path == ["tabs", "scope"])
            .unwrap();
        assert_eq!(tabs.label(&tabs.step(&tabs.value(&tui), -1, &[])), "global");
    }
}
