//! Keybindings: parsing of binding values, user overrides, leader sequences and display text.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;

mod defaults;
pub mod slash;

pub use defaults::DEFAULTS;

pub const LEADER_TIMEOUT_MS: u64 = 2000;

/// One key press: a key name (`a`, `return`, `f2`, `?`) and modifiers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Stroke {
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub super_: bool,
}

impl Stroke {
    pub fn parse(text: &str) -> Option<Stroke> {
        let mut stroke = Stroke::default();
        let lower = text.to_lowercase();
        // `ctrl++` style is not used; a trailing `+` would be the key itself.
        let (mods, key) = match lower.rsplit_once('+') {
            Some((mods, "")) => (mods.trim_end_matches('+'), "+"),
            Some((mods, key)) => (mods, key),
            None => ("", lower.as_str()),
        };
        for modifier in mods.split('+').filter(|m| !m.is_empty()) {
            match modifier {
                "ctrl" => stroke.ctrl = true,
                "shift" => stroke.shift = true,
                "alt" | "meta" | "option" => stroke.alt = true,
                "super" | "cmd" | "hyper" => stroke.super_ = true,
                _ => return None,
            }
        }
        stroke.key = match key {
            "enter" => "return",
            "esc" => "escape",
            "pgdown" => "pagedown",
            "pgup" => "pageup",
            "" => return None,
            other => other,
        }
        .to_string();
        Some(stroke)
    }

    /// The stroke a terminal key event stands for.
    pub fn from_event(event: &KeyEvent) -> Option<Stroke> {
        let mods = event.modifiers;
        let mut stroke = Stroke {
            ctrl: mods.contains(KeyModifiers::CONTROL),
            shift: mods.contains(KeyModifiers::SHIFT),
            alt: mods.contains(KeyModifiers::ALT) || mods.contains(KeyModifiers::META),
            super_: mods.contains(KeyModifiers::SUPER) || mods.contains(KeyModifiers::HYPER),
            ..Stroke::default()
        };
        stroke.key = match event.code {
            KeyCode::Char(' ') => "space".into(),
            KeyCode::Char(c) if c.is_ascii_uppercase() => {
                stroke.shift = true;
                c.to_ascii_lowercase().to_string()
            }
            KeyCode::Char(c) => {
                // Shifted punctuation arrives as the shifted character itself.
                if !c.is_ascii_alphanumeric() && !stroke.ctrl && !stroke.alt {
                    stroke.shift = false;
                }
                c.to_string()
            }
            KeyCode::Enter => "return".into(),
            KeyCode::Esc => "escape".into(),
            KeyCode::Tab => "tab".into(),
            KeyCode::BackTab => {
                stroke.shift = true;
                "tab".into()
            }
            KeyCode::Backspace => "backspace".into(),
            KeyCode::Delete => "delete".into(),
            KeyCode::Up => "up".into(),
            KeyCode::Down => "down".into(),
            KeyCode::Left => "left".into(),
            KeyCode::Right => "right".into(),
            KeyCode::Home => "home".into(),
            KeyCode::End => "end".into(),
            KeyCode::PageUp => "pageup".into(),
            KeyCode::PageDown => "pagedown".into(),
            KeyCode::Insert => "insert".into(),
            KeyCode::F(n) => format!("f{n}"),
            _ => return None,
        };
        Some(stroke)
    }

    pub fn display(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if self.ctrl {
            parts.push("ctrl");
        }
        if self.shift {
            parts.push("shift");
        }
        if self.alt {
            parts.push("alt");
        }
        if self.super_ {
            parts.push("super");
        }
        let key = match self.key.as_str() {
            "return" => "enter",
            "up" => "↑",
            "down" => "↓",
            "left" => "←",
            "right" => "→",
            "pageup" => "pgup",
            "pagedown" => "pgdn",
            "delete" => "del",
            other => other,
        };
        parts.push(key);
        parts.join("+")
    }
}

/// A binding alternative: strokes pressed in order (`<leader>` expanded to the leader key).
pub type Sequence = Vec<Stroke>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Match {
    /// `id` is bound to exactly these strokes.
    Exact,
    /// These strokes start a longer binding of `id`.
    Prefix,
    None,
}

pub struct Keymap {
    bindings: HashMap<String, Vec<Sequence>>,
    leader: Sequence,
    pub leader_timeout_ms: u64,
}

impl Keymap {
    /// Defaults with the user's `keybinds` overrides (v2 ids or v1 names) and `leader.timeout`.
    pub fn from_config(tui: &Value, windows: bool) -> Result<Keymap, String> {
        let mut values: HashMap<String, Value> = DEFAULTS
            .iter()
            .map(|(id, value, _)| (id.to_string(), Value::String(value.to_string())))
            .collect();
        if windows {
            values.insert("terminal.suspend".into(), Value::String("none".into()));
        }
        let mut user_undo = false;
        if let Some(Value::Object(overrides)) = tui.get("keybinds") {
            let unknown: Vec<&str> = overrides
                .keys()
                .filter(|k| !values.contains_key(*k) && v1_name(k).is_none())
                .map(String::as_str)
                .collect();
            match unknown.as_slice() {
                [] => {}
                [one] => return Err(format!("Unrecognized keybind: {one}")),
                many => return Err(format!("Unrecognized keybinds: {}", many.join(", "))),
            }
            for (key, value) in overrides {
                let id = v1_name(key).unwrap_or(key);
                user_undo |= id == "input.undo";
                values.insert(id.to_string(), value.clone());
            }
        }
        if windows && !user_undo {
            let undo = values["input.undo"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            values.insert("input.undo".into(), Value::String(format!("ctrl+z,{undo}")));
        }
        let leader = values
            .get("leader")
            .and_then(|v| alternatives(v).into_iter().next())
            .and_then(|text| parse_sequence(&text, &[]))
            .unwrap_or_default();
        let bindings = values
            .iter()
            .filter(|(id, _)| id.as_str() != "leader")
            .map(|(id, value)| {
                let sequences = alternatives(value)
                    .iter()
                    .filter_map(|text| parse_sequence(text, &leader))
                    .collect();
                (id.clone(), sequences)
            })
            .collect();
        let timeout = tui
            .get("leader")
            .and_then(|l| l.get("timeout"))
            .and_then(Value::as_u64)
            .filter(|t| *t > 0)
            .unwrap_or(LEADER_TIMEOUT_MS);
        Ok(Keymap {
            bindings,
            leader,
            leader_timeout_ms: timeout,
        })
    }

    pub fn defaults() -> Keymap {
        Keymap::from_config(&Value::Null, cfg!(windows)).expect("default keymap")
    }

    pub fn is_leader(&self, strokes: &[Stroke]) -> bool {
        !self.leader.is_empty() && strokes == self.leader.as_slice()
    }

    pub fn matches(&self, id: &str, strokes: &[Stroke]) -> Match {
        let Some(sequences) = self.bindings.get(id) else {
            return Match::None;
        };
        if sequences.iter().any(|s| s.as_slice() == strokes) {
            Match::Exact
        } else if sequences
            .iter()
            .any(|s| s.len() > strokes.len() && s.starts_with(strokes))
        {
            Match::Prefix
        } else {
            Match::None
        }
    }

    /// Every binding of `id` as shown in the palette: `ctrl+c, ctrl+d, ctrl+x q`.
    pub fn all(&self, id: &str) -> String {
        let mut shown: Vec<String> = Vec::new();
        for sequence in self.bindings.get(id).into_iter().flatten() {
            let text = self.show(sequence);
            if !shown.contains(&text) {
                shown.push(text);
            }
        }
        shown.join(", ")
    }

    /// The first binding of `id` (footer hints), if any.
    pub fn first(&self, id: &str) -> Option<String> {
        self.bindings
            .get(id)
            .and_then(|s| s.first())
            .map(|s| self.show(s))
    }

    fn show(&self, sequence: &Sequence) -> String {
        let (lead, rest) = if !self.leader.is_empty() && sequence.starts_with(&self.leader) {
            (
                Some(
                    self.leader
                        .iter()
                        .map(Stroke::display)
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
                &sequence[self.leader.len()..],
            )
        } else {
            (None, sequence.as_slice())
        };
        lead.into_iter()
            .chain(rest.iter().map(Stroke::display))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn v1_name(name: &str) -> Option<&'static str> {
    defaults::V1_NAMES
        .iter()
        .find(|(v1, _)| *v1 == name)
        .map(|(_, v2)| *v2)
}

/// The alternatives of a binding value: `false`/`"none"` → none, a comma string, a stroke
/// object, `{key: …}`, or an array of those.
fn alternatives(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) if text == "none" => Vec::new(),
        Value::String(text) => split_commas(text),
        Value::Array(items) => items.iter().flat_map(alternatives).collect(),
        Value::Object(map) => match map.get("key") {
            Some(key) => alternatives(key),
            None => stroke_object(map).into_iter().collect(),
        },
        _ => Vec::new(),
    }
}

/// Splits on commas that separate alternatives (a lone `,` key stays a key).
fn split_commas(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for c in text.chars() {
        if c == ',' && !current.is_empty() && !current.ends_with('+') {
            out.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out.into_iter().map(|s| s.trim().to_string()).collect()
}

fn stroke_object(map: &serde_json::Map<String, Value>) -> Option<String> {
    let name = map.get("name")?.as_str()?;
    let flag = |k: &str| map.get(k).and_then(Value::as_bool) == Some(true);
    let mut parts = Vec::new();
    for (key, label) in [
        ("ctrl", "ctrl"),
        ("shift", "shift"),
        ("meta", "alt"),
        ("super", "super"),
        ("hyper", "hyper"),
    ] {
        if flag(key) {
            parts.push(label.to_string());
        }
    }
    parts.push(name.to_string());
    Some(parts.join("+"))
}

fn parse_sequence(text: &str, leader: &[Stroke]) -> Option<Sequence> {
    let mut sequence = Vec::new();
    for part in text.split_whitespace() {
        let mut rest = part;
        while let Some(after) = rest.strip_prefix("<leader>") {
            sequence.extend_from_slice(leader);
            rest = after;
        }
        if !rest.is_empty() {
            sequence.push(Stroke::parse(rest)?);
        }
    }
    (!sequence.is_empty()).then_some(sequence)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strokes(text: &str) -> Vec<Stroke> {
        let keymap = Keymap::defaults();
        parse_sequence(text, &keymap.leader).unwrap()
    }

    /// Every default of `keybinds-default.md` resolves to its command.
    #[test]
    fn every_default_resolves() {
        let keymap = Keymap::from_config(&Value::Null, false).unwrap();
        for (id, value, _) in DEFAULTS {
            if *id == "leader" {
                continue;
            }
            for alternative in alternatives(&Value::String(value.to_string())) {
                assert_eq!(
                    keymap.matches(id, &strokes(&alternative)),
                    Match::Exact,
                    "{id} {alternative}"
                );
            }
        }
    }

    #[test]
    fn leader_q_only_exits() {
        let keymap = Keymap::defaults();
        assert_eq!(
            keymap.matches("app.exit", &strokes("ctrl+x q")),
            Match::Exact
        );
        assert_eq!(
            keymap.matches("session.queued_prompts", &strokes("ctrl+x q")),
            Match::None
        );
        assert_eq!(
            keymap.matches("app.exit", &strokes("ctrl+x")),
            Match::Prefix
        );
        assert_eq!(keymap.all("app.exit"), "ctrl+c, ctrl+d, ctrl+x q");
        assert_eq!(keymap.first("session.list").unwrap(), "ctrl+x l");
        assert_eq!(keymap.first("agent.cycle").unwrap(), "shift+tab");
        assert_eq!(keymap.first("prompt.queue").unwrap(), "ctrl+x enter");
    }

    #[test]
    fn overrides_accept_v1_names_and_reject_unknown_ids() {
        let tui = serde_json::json!({"keybinds": {"command_list": "ctrl+k", "session.new": false}, "leader": {"timeout": 500}});
        let keymap = Keymap::from_config(&tui, false).unwrap();
        assert_eq!(keymap.first("command.palette.show").unwrap(), "ctrl+k");
        assert_eq!(keymap.first("session.new"), None);
        assert_eq!(keymap.leader_timeout_ms, 500);
        let bad = serde_json::json!({"keybinds": {"nope": "a"}});
        assert_eq!(
            Keymap::from_config(&bad, false).err().unwrap(),
            "Unrecognized keybind: nope"
        );
    }

    #[test]
    fn windows_binds_ctrl_z_to_undo() {
        let keymap = Keymap::from_config(&Value::Null, true).unwrap();
        assert_eq!(
            keymap.matches("input.undo", &strokes("ctrl+z")),
            Match::Exact
        );
        assert_eq!(keymap.first("terminal.suspend"), None);
    }

    #[test]
    fn events_map_to_strokes() {
        let event = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(Stroke::from_event(&event), Stroke::parse("shift+g"));
        let event = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(Stroke::from_event(&event), Stroke::parse("shift+tab"));
    }
}
