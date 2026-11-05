//! Theme documents in the v2 format (`base` + `light`/`dark`): mode pick, expansion of implied
//! tokens, and resolution of `$path` references into flat colour tables.

use std::collections::HashMap;

use serde_json::{Map, Value};

use super::color::Rgba;
use super::{Mode, View};

pub const STATES: [&str; 5] = ["disabled", "pressed", "focused", "selected", "hovered"];
const VARIANTS: [&str; 3] = ["primary", "secondary", "destructive"];
const STEPS: [&str; 9] = [
    "100", "200", "300", "400", "500", "600", "700", "800", "900",
];
const TOKEN_GROUPS: [&str; 7] = [
    "text",
    "background",
    "border",
    "scrollbar",
    "diff",
    "syntax",
    "markdown",
];

/// A fully resolved theme mode.
pub struct Resolved {
    pub mode: Mode,
    pub hues: Vec<(String, [Rgba; 9])>,
    pub categorical: Vec<usize>,
    pub base: View,
    pub dialog: View,
    pub footer: View,
}

pub fn modes(document: &Value) -> Vec<Mode> {
    [Mode::Light, Mode::Dark]
        .into_iter()
        .filter(|mode| document.get(mode.key()).is_some())
        .collect()
}

pub fn merge(target: &mut Value, overlay: &Value) {
    let Value::Object(overlay) = overlay else {
        return;
    };
    if !target.is_object() {
        *target = Value::Object(Map::new());
    }
    let Value::Object(target) = target else {
        return;
    };
    for (key, item) in overlay {
        if item.is_null() {
            continue;
        }
        if item.is_object() {
            merge(target.entry(key.clone()).or_insert(Value::Null), item);
        } else {
            target.insert(key.clone(), item.clone());
        }
    }
}

pub fn resolve(document: &Value, requested: Mode) -> Result<Resolved, String> {
    let available = modes(document);
    let mode = if available.contains(&requested) {
        requested
    } else {
        *available
            .first()
            .ok_or("Theme must provide at least one mode")?
    };
    let mut definition = document.get("base").cloned().unwrap_or(Value::Null);
    merge(&mut definition, &document[mode.key()]);
    expand_tokens(&mut definition);
    if let Some(dialog) = definition.get_mut("@dialog") {
        expand_tokens(dialog);
        if let Some(footer) = dialog.get_mut("@footer") {
            expand_tokens(footer);
        }
    }
    let hues = resolve_hues(definition.get("hue").ok_or("Theme has no hue")?)?;
    let categorical = definition
        .get("categorical")
        .and_then(Value::as_array)
        .ok_or("Theme has no categorical hues")?
        .iter()
        .map(|name| {
            let name = name.as_str().unwrap_or_default();
            hues.iter()
                .position(|(hue, _)| hue == name)
                .ok_or(format!("Categorical hue \"{name}\" was not found"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let base = tokens(&definition);
    let dialog_override = definition.get("@dialog").map(tokens);
    let dialog = match &dialog_override {
        Some(dialog) => contextualize(&base, dialog),
        None => base.clone(),
    };
    let footer = match definition.get("@dialog").and_then(|d| d.get("@footer")) {
        Some(footer) => contextualize(&dialog, &tokens(footer)),
        None => dialog.clone(),
    };
    Ok(Resolved {
        mode,
        base: resolve_view(&base, &hues)?,
        dialog: resolve_view(&dialog, &hues)?,
        footer: resolve_view(&footer, &hues)?,
        hues,
        categorical,
    })
}

fn tokens(definition: &Value) -> Value {
    let mut out = Map::new();
    for group in TOKEN_GROUPS {
        if let Some(value) = definition.get(group) {
            out.insert(group.into(), value.clone());
        }
    }
    Value::Object(out)
}

fn set_default(object: &mut Value, key: &str, value: Option<String>) {
    if object.get(key).is_none()
        && let (Some(value), Value::Object(map)) = (value, &mut *object)
    {
        map.insert(key.into(), Value::String(value));
    }
}

fn expand_tokens(definition: &mut Value) {
    if let Some(text) = definition.get_mut("text") {
        let has_base = text.get("base").is_some();
        set_default(text, "muted", has_base.then(|| "$text.base".into()));
        let has_muted = text.get("muted").is_some();
        set_default(text, "faint", has_muted.then(|| "$text.muted".into()));
        expand_stateful_group(text.get_mut("action"), "text.action");
        expand_stateful(text.get_mut("formfield"), "text.formfield");
        if let Some(Value::Object(feedback)) = text.get_mut("feedback") {
            for (kind, value) in feedback.iter_mut() {
                let has = value.get("base").is_some();
                set_default(
                    value,
                    "muted",
                    has.then(|| format!("$text.feedback.{kind}.base")),
                );
            }
        }
    }
    if let Some(background) = definition.get_mut("background") {
        expand_stateful_group(background.get_mut("action"), "background.action");
        expand_stateful(background.get_mut("formfield"), "background.formfield");
    }
}

fn expand_stateful_group(group: Option<&mut Value>, path: &str) {
    if let Some(Value::Object(variants)) = group {
        for (variant, value) in variants.iter_mut() {
            expand_stateful(Some(value), &format!("{path}.{variant}"));
        }
    }
}

fn expand_stateful(value: Option<&mut Value>, path: &str) {
    let Some(value) = value else { return };
    if value.get("base").is_none() {
        return;
    }
    for state in STATES {
        set_default(value, &format!("${state}"), Some(format!("${path}.base")));
    }
}

fn contextualize(base: &Value, surface: &Value) -> Value {
    let mut result = base.clone();
    merge(&mut result, surface);
    for group in ["text", "background"] {
        let base_actions = base.get(group).and_then(|g| g.get("action"));
        let surface_actions = surface.get(group).and_then(|g| g.get("action"));
        let mut actions = Map::new();
        for variant in VARIANTS {
            let base_variant = base_actions.and_then(|a| a.get(variant));
            let surface_variant = surface_actions.and_then(|a| a.get(variant));
            let mut out = Map::new();
            for state in std::iter::once("base").chain(STATES) {
                let key = if state == "base" {
                    "base".to_string()
                } else {
                    format!("${state}")
                };
                let pick = surface_variant
                    .filter(|_| state != "base")
                    .and_then(|v| v.get(&key))
                    .or_else(|| surface_variant.and_then(|v| v.get("base")))
                    .or_else(|| {
                        base_variant
                            .filter(|_| state != "base")
                            .and_then(|v| v.get(&key))
                    })
                    .or_else(|| base_variant.and_then(|v| v.get("base")));
                if let Some(pick) = pick {
                    out.insert(key, pick.clone());
                }
            }
            if !out.is_empty() {
                actions.insert(variant.into(), Value::Object(out));
            }
        }
        if let Some(Value::Object(target)) = result.get_mut(group) {
            target.insert("action".into(), Value::Object(actions));
        }
    }
    result
}

fn resolve_hues(source: &Value) -> Result<Vec<(String, [Rgba; 9])>, String> {
    let Value::Object(map) = source else {
        return Err("Theme hue must be an object".into());
    };
    let names: Vec<&String> = map.keys().collect();
    let mut done: HashMap<String, [Rgba; 9]> = HashMap::new();
    fn scale(
        name: &str,
        map: &Map<String, Value>,
        done: &mut HashMap<String, [Rgba; 9]>,
        stack: &mut Vec<String>,
    ) -> Result<[Rgba; 9], String> {
        if let Some(hit) = done.get(name) {
            return Ok(*hit);
        }
        if stack.iter().any(|s| s == name) {
            return Err(format!("Circular hue reference: {name}"));
        }
        let result = match map.get(name) {
            Some(Value::String(alias)) => {
                let target = alias
                    .strip_prefix("$hue.")
                    .ok_or(format!("Hue alias \"{alias}\" must reference a hue scale"))?;
                if !map.contains_key(target) {
                    return Err(format!("Hue alias \"{alias}\" references a missing hue"));
                }
                stack.push(name.to_string());
                let scale = scale(target, map, done, stack)?;
                stack.pop();
                scale
            }
            Some(Value::Object(steps)) => {
                if let Some(extra) = steps.keys().find(|k| !STEPS.contains(&k.as_str())) {
                    return Err(format!("Unknown hue step at \"hue.{name}.{extra}\""));
                }
                let mut out = [Rgba::TRANSPARENT; 9];
                for (i, step) in STEPS.iter().enumerate() {
                    out[i] = steps
                        .get(*step)
                        .and_then(Value::as_str)
                        .and_then(Rgba::hex)
                        .ok_or(format!("Invalid hue color at \"hue.{name}.{step}\""))?;
                }
                out
            }
            _ => return Err(format!("Hue \"{name}\" was not found")),
        };
        done.insert(name.to_string(), result);
        Ok(result)
    }
    let mut out = Vec::new();
    for (index, name) in names.iter().enumerate() {
        let mut scale = scale(name, map, &mut done, &mut Vec::new())?;
        for (step, color) in scale.iter_mut().enumerate() {
            color.origin = Some((index as u16, step as u8));
        }
        out.push(((*name).clone(), scale));
    }
    Ok(out)
}

struct Resolver<'a> {
    source: &'a Value,
    hues: &'a [(String, [Rgba; 9])],
    cache: HashMap<String, Rgba>,
}

impl Resolver<'_> {
    fn color(
        &mut self,
        value: &Value,
        path: &str,
        stack: &mut Vec<String>,
    ) -> Result<Rgba, String> {
        let Value::String(text) = value else {
            return Err(format!("Invalid theme value at \"{path}\""));
        };
        if text == "transparent" {
            return Ok(Rgba::TRANSPARENT);
        }
        if let Some(color) = Rgba::hex(text) {
            return Ok(color);
        }
        let target = text
            .strip_prefix('$')
            .ok_or(format!("Invalid color \"{text}\" at \"{path}\""))?;
        if let Some(hit) = self.cache.get(target) {
            return Ok(*hit);
        }
        if stack.iter().any(|s| s == target) {
            return Err(format!("Circular theme reference: {target}"));
        }
        let result = if let Some(rest) = target.strip_prefix("hue.")
            && let Some((name, step)) = rest.rsplit_once('.')
            && let Some(step) = STEPS.iter().position(|s| *s == step)
        {
            self.hues
                .iter()
                .find(|(hue, _)| hue == name)
                .map(|(_, scale)| scale[step])
                .ok_or(format!("Theme reference \"${target}\" was not found"))?
        } else {
            let found = target
                .split('.')
                .try_fold(self.source, |value, key| value.get(key))
                .ok_or(format!("Theme reference \"${target}\" was not found"))?;
            stack.push(target.to_string());
            let color = self.color(found, target, stack);
            stack.pop();
            color.map_err(|_| {
                format!("Theme reference \"${target}\" at \"{path}\" is not a color")
            })?
        };
        self.cache.insert(target.to_string(), result);
        Ok(result)
    }

    fn walk(
        &mut self,
        value: &Value,
        path: &str,
        out: &mut HashMap<String, Rgba>,
    ) -> Result<(), String> {
        match value {
            Value::Object(map) => {
                for (key, item) in map {
                    let key = key
                        .strip_prefix('$')
                        .filter(|state| STATES.contains(state))
                        .unwrap_or(key);
                    let child = if path.is_empty() {
                        key.to_string()
                    } else {
                        format!("{path}.{key}")
                    };
                    self.walk(item, &child, out)?;
                }
                Ok(())
            }
            other => {
                let color = self.color(other, path, &mut Vec::new())?;
                out.insert(path.to_string(), color);
                Ok(())
            }
        }
    }
}

fn resolve_view(tokens: &Value, hues: &[(String, [Rgba; 9])]) -> Result<View, String> {
    let mut resolver = Resolver {
        source: tokens,
        hues,
        cache: HashMap::new(),
    };
    let mut out = HashMap::new();
    resolver.walk(tokens, "", &mut out)?;
    Ok(View(out))
}
