//! Legacy v1 themes (`defs` + `theme`) migrate to v2 documents at load: hues are inferred from
//! the accent colours by OKLCH hue distance and expanded to 9-step scales, the neutral scale is
//! interpolated from the surfaces, and literals equal to a hue step become references.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use super::color::{Oklch, Rgba, from_oklch, to_oklch};

const CHROMATIC: [&str; 7] = ["red", "orange", "yellow", "green", "cyan", "blue", "purple"];
const CATEGORICAL_TOKENS: [&str; 7] = [
    "secondary",
    "accent",
    "success",
    "warning",
    "primary",
    "error",
    "info",
];
const STEPS: [u32; 9] = [100, 200, 300, 400, 500, 600, 700, 800, 900];
const LIGHT_REFERENCES: [&str; 7] = [
    "#fca5a5", "#fdba74", "#fde047", "#86efac", "#67e8f9", "#93c5fd", "#d8b4fe",
];
const DARK_REFERENCES: [&str; 7] = [
    "#b91c1c", "#c2410c", "#a16207", "#15803d", "#0e7490", "#1d4ed8", "#7e22ce",
];

type Colors = HashMap<String, Rgba>;

struct Resolved {
    colors: Colors,
    has_selected_text: bool,
}

impl Resolved {
    fn get(&self, key: &str) -> Rgba {
        self.colors.get(key).copied().unwrap_or(Rgba::TRANSPARENT)
    }
}

pub fn migrate(source: &Value) -> Result<Value, String> {
    let light = resolve(source, "light")?;
    let dark = resolve(source, "dark")?;
    let (lb, db) = (light.get("background"), dark.get("background"));
    if lb.a > 0.0 && db.a > 0.0 && lb.same(&db) {
        let (lm, dm) = (detect(&light), detect(&dark));
        if lm == dm {
            let theme = if lm == "light" { &light } else { &dark };
            let mut definition = migrate_mode(theme, lm);
            let hue = definition.as_object_mut().and_then(|d| d.remove("hue"));
            return Ok(json!({ "base": definition, lm: { "hue": hue } }));
        }
    }
    let mut light_definition = migrate_mode(&light, "light");
    let hue = light_definition
        .as_object_mut()
        .and_then(|d| d.remove("hue"));
    Ok(json!({
        "base": light_definition,
        "light": { "hue": hue },
        "dark": migrate_mode(&dark, "dark"),
    }))
}

fn plain_luminance(color: &Rgba) -> f64 {
    0.299 * color.r + 0.587 * color.g + 0.114 * color.b
}

fn detect(theme: &Resolved) -> &'static str {
    if plain_luminance(&theme.get("text")) > plain_luminance(&theme.get("background")) {
        "dark"
    } else {
        "light"
    }
}

fn ints(color: &Rgba) -> (u8, u8, u8, u8) {
    let (r, g, b) = color.bytes();
    (r, g, b, (color.a.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn hex(color: &Rgba) -> String {
    let (r, g, b, a) = ints(color);
    hex_ints(r, g, b, a)
}

fn hex_ints(r: u8, g: u8, b: u8, a: u8) -> String {
    let alpha = if a == 255 {
        String::new()
    } else {
        format!("{a:02x}")
    };
    format!("#{r:02x}{g:02x}{b:02x}{alpha}")
}

fn oklch(color: &Rgba) -> Oklch {
    let (r, g, b, _) = ints(color);
    to_oklch(&Rgba::rgb(r, g, b))
}

fn selected_foreground(theme: &Resolved, background: &Rgba) -> Rgba {
    if theme.has_selected_text {
        return theme.get("selectedListItemText");
    }
    let base = theme.get("background");
    if base.a != 0.0 {
        return base;
    }
    if plain_luminance(background) > 0.5 {
        Rgba::rgb(0, 0, 0)
    } else {
        Rgba::rgb(255, 255, 255)
    }
}

fn hue_distance(a: f64, b: f64) -> f64 {
    let d = (a - b).abs();
    d.min(360.0 - d)
}

/// The nearest chromatic hue and its distance; none for transparent or greyish colours.
fn infer_hue(color: &Rgba) -> Option<(&'static str, f64)> {
    let value = oklch(color);
    if ints(color).3 == 0 || value.c < 0.03 {
        return None;
    }
    let references = if value.l >= 0.6 {
        LIGHT_REFERENCES
    } else {
        DARK_REFERENCES
    };
    let mut best: Option<(&'static str, f64)> = None;
    for (name, reference) in CHROMATIC.iter().zip(references) {
        let angle = to_oklch(&Rgba::hex(reference).unwrap_or(Rgba::TRANSPARENT)).h;
        let distance = hue_distance(value.h, angle);
        if best.is_none_or(|(_, d)| distance < d) {
            best = Some((name, distance));
        }
    }
    best
}

struct Hues {
    by_hue: Vec<(&'static str, Rgba, f64)>,
    by_token: HashMap<&'static str, &'static str>,
}

fn infer_hues(theme: &Resolved) -> Hues {
    let mut hues = Hues {
        by_hue: Vec::new(),
        by_token: HashMap::new(),
    };
    let tokens = [
        "accent",
        "success",
        "warning",
        "primary",
        "error",
        "info",
        "secondary",
    ];
    for token in tokens {
        let color = theme.get(token);
        let Some((name, distance)) = infer_hue(&color) else {
            continue;
        };
        match hues.by_hue.iter_mut().find(|(hue, _, _)| *hue == name) {
            Some(current) if current.2 <= distance => {}
            Some(current) => *current = (name, color, distance),
            None => hues.by_hue.push((name, color, distance)),
        }
        hues.by_token.insert(token, name);
    }
    for token in ["accent", "primary"] {
        let color = theme.get(token);
        let Some((name, distance)) = infer_hue(&color) else {
            continue;
        };
        match hues.by_hue.iter_mut().find(|(hue, _, _)| *hue == name) {
            Some(current) => *current = (name, color, distance),
            None => hues.by_hue.push((name, color, distance)),
        }
        hues.by_token.insert(token, name);
    }
    hues
}

fn hue_scale(color: &Rgba, mode: &str) -> Value {
    let value = oklch(color);
    let endpoint = if mode == "light" {
        value.l.max(0.97)
    } else {
        value.l.min(0.18)
    };
    let alpha = ints(color).3;
    let mut scale = Map::new();
    for step in STEPS {
        let text = if step == 200 {
            hex(color)
        } else {
            let progress = (f64::from(step) - 200.0) / 700.0;
            let generated = from_oklch(Oklch {
                l: value.l + (endpoint - value.l) * progress,
                c: value.c * (1.0 - progress * 0.5),
                h: value.h,
            })
            .to_hex();
            with_alpha(generated, alpha)
        };
        scale.insert(step.to_string(), Value::String(text));
    }
    Value::Object(scale)
}

fn with_alpha(hex: String, alpha: u8) -> String {
    if alpha == 255 {
        hex
    } else {
        format!("{hex}{alpha:02x}")
    }
}

fn interpolate(first: &Rgba, second: &Rgba, amount: f64) -> String {
    let start = oklch(first);
    let end = oklch(second);
    let hue = (((end.h - start.h) % 360.0) + 540.0) % 360.0 - 180.0;
    let generated = from_oklch(Oklch {
        l: start.l + (end.l - start.l) * amount,
        c: start.c + (end.c - start.c) * amount,
        h: start.h + hue * amount,
    })
    .to_hex();
    let (a1, a2) = (f64::from(ints(first).3), f64::from(ints(second).3));
    let alpha = (a1 + (a2 - a1) * amount).round().clamp(0.0, 255.0) as u8;
    with_alpha(generated, alpha)
}

fn neutral_scale(theme: &Resolved) -> Value {
    let anchors = [
        (200, theme.get("text")),
        (400, theme.get("textMuted")),
        (600, theme.get("backgroundElement")),
        (700, theme.get("backgroundPanel")),
        (800, theme.get("background")),
    ];
    let mut scale = Map::new();
    for step in STEPS {
        let text = if let Some((_, color)) = anchors.iter().find(|(s, _)| *s == step) {
            hex(color)
        } else {
            let (lower, upper) = if step < anchors[0].0 {
                (anchors[0], anchors[1])
            } else if step > anchors[4].0 {
                (anchors[3], anchors[4])
            } else {
                let lower = *anchors
                    .iter()
                    .rev()
                    .find(|(s, _)| *s < step)
                    .unwrap_or(&anchors[0]);
                let upper = *anchors
                    .iter()
                    .find(|(s, _)| *s > step)
                    .unwrap_or(&anchors[4]);
                (lower, upper)
            };
            let amount = (f64::from(step) - f64::from(lower.0)) / f64::from(upper.0 - lower.0);
            interpolate(&lower.1, &upper.1, amount)
        };
        scale.insert(step.to_string(), Value::String(text));
    }
    Value::Object(scale)
}

fn dialog_background(theme: &Resolved) -> String {
    if theme.get("backgroundPanel").a == 1.0 {
        return "$background.raised.base".into();
    }
    let visible = [
        "backgroundPanel",
        "backgroundMenu",
        "backgroundElement",
        "background",
    ]
    .iter()
    .map(|k| theme.get(k))
    .find(|c| c.a > 0.0);
    match visible {
        Some(color) => {
            let (r, g, b) = color.bytes();
            hex_ints(r, g, b, 255)
        }
        None if plain_luminance(&theme.get("text")) > 0.5 => "#000000".into(),
        None => "#ffffff".into(),
    }
}

fn migrate_mode(theme: &Resolved, mode: &str) -> Value {
    let c = |key: &str| hex(&theme.get(key));
    let selected = hex(&selected_foreground(theme, &theme.get("primary")));
    let destructive = hex(&selected_foreground(theme, &theme.get("error")));
    let hues = infer_hues(theme);
    let mut categorical: Vec<&str> = Vec::new();
    for token in CATEGORICAL_TOKENS {
        if let Some(hue) = hues.by_token.get(token)
            && !categorical.contains(hue)
        {
            categorical.push(hue);
        }
    }
    if categorical.is_empty() {
        categorical.push("neutral");
    }
    let mut hue = Map::new();
    hue.insert("gray".into(), neutral_scale(theme));
    for name in CHROMATIC {
        if let Some((_, color, _)) = hues.by_hue.iter().find(|(h, _, _)| *h == name) {
            hue.insert(name.into(), hue_scale(color, mode));
        }
    }
    let alias = |token: &str| {
        hues.by_token
            .get(token)
            .map_or("$hue.gray".to_string(), |h| format!("$hue.{h}"))
    };
    hue.insert("accent".into(), Value::String(alias("accent")));
    hue.insert("interactive".into(), Value::String(alias("primary")));
    hue.insert("neutral".into(), Value::String("$hue.gray".into()));
    let (text, muted, primary) = (
        "$hue.neutral.200",
        "$hue.neutral.400",
        "$hue.interactive.200",
    );
    let panel = "$hue.neutral.700";
    let mut definition = json!({
        "hue": hue,
        "categorical": categorical,
        "text": {
            "base": text,
            "muted": muted,
            "action": {
                "primary": { "base": "$text.base", "$disabled": muted, "$focused": selected, "$selected": primary },
                "secondary": { "base": "$text.muted", "$hovered": "$text.base" },
                "destructive": { "base": destructive, "$disabled": muted },
            },
            "formfield": { "base": text, "$hovered": primary, "$focused": primary, "$pressed": primary, "$disabled": muted, "$selected": primary },
            "feedback": {
                "error": { "base": c("error") }, "warning": { "base": c("warning") },
                "success": { "base": c("success") }, "info": { "base": c("info") },
            },
        },
        "background": {
            "base": "$hue.neutral.800",
            "raised": { "base": panel, "high": "$hue.neutral.600", "max": "$hue.neutral.500" },
            "action": {
                "primary": { "base": "transparent", "$hovered": panel, "$focused": primary, "$selected": "transparent" },
                "secondary": { "base": "transparent" },
                "destructive": { "base": c("error") },
            },
            "formfield": { "base": "$background.base" },
            "feedback": {
                "error": { "base": "$background.base" }, "warning": { "base": "$background.base" },
                "success": { "base": "$background.base" }, "info": { "base": "$background.base" },
            },
        },
        "border": { "base": c("border") },
        "scrollbar": { "base": c("borderActive") },
        "diff": {
            "text": { "added": c("diffAdded"), "removed": c("diffRemoved"), "context": c("diffContext"), "hunkHeader": c("diffHunkHeader") },
            "background": { "added": c("diffAddedBg"), "removed": c("diffRemovedBg"), "context": c("diffContextBg") },
            "highlight": { "added": c("diffHighlightAdded"), "removed": c("diffHighlightRemoved") },
            "lineNumber": { "text": c("diffLineNumber"), "background": { "added": c("diffAddedLineNumberBg"), "removed": c("diffRemovedLineNumberBg") } },
        },
        "syntax": {
            "comment": c("syntaxComment"), "keyword": c("syntaxKeyword"), "function": c("syntaxFunction"),
            "variable": c("syntaxVariable"), "string": c("syntaxString"), "number": c("syntaxNumber"),
            "type": c("syntaxType"), "operator": c("syntaxOperator"), "punctuation": c("syntaxPunctuation"),
        },
        "markdown": {
            "text": c("markdownText"), "heading": c("markdownHeading"), "link": c("markdownLink"),
            "linkText": c("markdownLinkText"), "code": c("markdownCode"), "blockQuote": c("markdownBlockQuote"),
            "emphasis": c("markdownEmph"), "strong": c("markdownStrong"), "horizontalRule": c("markdownHorizontalRule"),
            "listItem": c("markdownListItem"), "listEnumeration": c("markdownListEnumeration"),
            "image": c("markdownImage"), "imageText": c("markdownImageText"), "codeBlock": c("markdownCodeBlock"),
        },
        "@dialog": {
            "background": {
                "base": dialog_background(theme),
                "action": { "primary": { "$hovered": "$background.raised.high" } },
            },
        },
    });
    reference_hues(&mut definition);
    definition
}

/// Rewrites literals equal to a hue step into `$hue.<name>.<step>`; the role hues win ties.
fn reference_hues(definition: &mut Value) {
    let Some(hues) = definition.get("hue").cloned() else {
        return;
    };
    let scale_of = |name: &str| -> Option<Map<String, Value>> {
        let mut name = name.to_string();
        for _ in 0..8 {
            match hues.get(&name)? {
                Value::Object(scale) => return Some(scale.clone()),
                Value::String(alias) => name = alias.strip_prefix("$hue.")?.to_string(),
                _ => return None,
            }
        }
        None
    };
    let mut references: HashMap<String, String> = HashMap::new();
    let mut index = |name: &str, overwrite: bool| {
        let Some(scale) = scale_of(name) else { return };
        for step in STEPS {
            let Some(color) = scale.get(&step.to_string()).and_then(Value::as_str) else {
                continue;
            };
            let key = color.to_lowercase();
            if overwrite || !references.contains_key(&key) {
                references.insert(key, format!("$hue.{name}.{step}"));
            }
        }
    };
    for name in CHROMATIC {
        index(name, false);
    }
    index("gray", false);
    for name in ["accent", "interactive", "neutral"] {
        index(name, true);
    }
    fn replace(value: &mut Value, references: &HashMap<String, String>) {
        match value {
            Value::String(text) => {
                if let Some(reference) = references.get(&text.to_lowercase()) {
                    *text = reference.clone();
                }
            }
            Value::Object(map) => map.values_mut().for_each(|v| replace(v, references)),
            _ => {}
        }
    }
    if let Value::Object(map) = definition {
        for (key, value) in map.iter_mut() {
            if key != "hue" && key != "categorical" {
                replace(value, &references);
            }
        }
    }
}

fn ansi(code: u64) -> Rgba {
    const BASIC: [&str; 16] = [
        "#000000", "#800000", "#008000", "#808000", "#000080", "#800080", "#008080", "#c0c0c0",
        "#808080", "#ff0000", "#00ff00", "#ffff00", "#0000ff", "#ff00ff", "#00ffff", "#ffffff",
    ];
    match code {
        0..16 => Rgba::hex(BASIC[code as usize]).unwrap_or(Rgba::TRANSPARENT),
        16..232 => {
            let index = code - 16;
            let value = |part: u64| if part == 0 { 0 } else { (part * 40 + 55) as u8 };
            Rgba::rgb(value(index / 36), value((index / 6) % 6), value(index % 6))
        }
        232..256 => {
            let gray = ((code - 232) * 10 + 8) as u8;
            Rgba::rgb(gray, gray, gray)
        }
        _ => Rgba::rgb(0, 0, 0),
    }
}

fn resolve(source: &Value, mode: &str) -> Result<Resolved, String> {
    let defs = source.get("defs");
    let theme = source
        .get("theme")
        .and_then(Value::as_object)
        .ok_or("Invalid V1 theme")?;
    fn color(
        value: &Value,
        mode: &str,
        defs: Option<&Value>,
        theme: &Map<String, Value>,
        chain: &mut Vec<String>,
    ) -> Result<Rgba, String> {
        match value {
            Value::String(text) if text == "transparent" || text == "none" => Ok(Rgba::TRANSPARENT),
            Value::String(text) if text.starts_with('#') => {
                Rgba::hex(text).ok_or(format!("Invalid color {text}"))
            }
            Value::String(text) => {
                if chain.contains(text) {
                    return Err(format!("Circular color reference: {text}"));
                }
                let next = defs
                    .and_then(|d| d.get(text))
                    .or_else(|| theme.get(text))
                    .ok_or(format!(
                        "Color reference \"{text}\" not found in defs or theme"
                    ))?;
                chain.push(text.clone());
                color(next, mode, defs, theme, chain)
            }
            Value::Number(code) => Ok(ansi(code.as_u64().unwrap_or(0))),
            Value::Object(variants) => {
                let next = variants.get(mode).ok_or("Invalid V1 theme color")?;
                color(next, mode, defs, theme, chain)
            }
            _ => Err("Invalid V1 theme color".into()),
        }
    }
    let mut colors = Colors::new();
    for (key, value) in theme {
        if matches!(
            key.as_str(),
            "selectedListItemText" | "backgroundMenu" | "thinkingOpacity"
        ) {
            continue;
        }
        colors.insert(
            key.clone(),
            color(value, mode, defs, theme, &mut Vec::new())?,
        );
    }
    let has_selected_text = theme.contains_key("selectedListItemText");
    let selected = match theme.get("selectedListItemText") {
        Some(value) => color(value, mode, defs, theme, &mut Vec::new())?,
        None => colors
            .get("background")
            .copied()
            .unwrap_or(Rgba::TRANSPARENT),
    };
    colors.insert("selectedListItemText".into(), selected);
    let menu = match theme.get("backgroundMenu") {
        Some(value) => color(value, mode, defs, theme, &mut Vec::new())?,
        None => colors
            .get("backgroundElement")
            .copied()
            .unwrap_or(Rgba::TRANSPARENT),
    };
    colors.insert("backgroundMenu".into(), menu);
    Ok(Resolved {
        colors,
        has_selected_text,
    })
}
