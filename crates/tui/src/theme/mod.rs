//! Themes: the bundled set, custom `themes/*.json` files, mode selection and the resolved
//! colour tables the views draw with.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use ratatui::style::Color;
use serde_json::Value;

pub mod color;
mod v1;
mod v2;

pub use color::Rgba;

pub const DEFAULT: &str = "anysola";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
}

impl Mode {
    fn key(self) -> &'static str {
        match self {
            Mode::Dark => "dark",
            Mode::Light => "light",
        }
    }
}

/// One resolved token tree (`text.base`, `background.action.primary.focused`, …).
#[derive(Debug, Clone, Default)]
pub struct View(HashMap<String, Rgba>);

impl View {
    pub fn get(&self, path: &str) -> Rgba {
        self.0.get(path).copied().unwrap_or(Rgba::TRANSPARENT)
    }
}

pub struct Theme {
    pub name: String,
    pub mode: Mode,
    pub modes: Vec<Mode>,
    hues: Vec<(String, [Rgba; 9])>,
    categorical: Vec<usize>,
    pub base: View,
    pub dialog: View,
    pub footer: View,
}

const BUNDLED: &[(&str, &str)] = &[
    ("anysola", include_str!("../../themes/anysola.json")),
    ("opencode", include_str!("../../themes/opencode.json")),
    ("aura", include_str!("../../themes/aura.json")),
    ("ayu", include_str!("../../themes/ayu.json")),
    ("catppuccin", include_str!("../../themes/catppuccin.json")),
    (
        "catppuccin-frappe",
        include_str!("../../themes/catppuccin-frappe.json"),
    ),
    (
        "catppuccin-macchiato",
        include_str!("../../themes/catppuccin-macchiato.json"),
    ),
    ("cobalt2", include_str!("../../themes/cobalt2.json")),
    ("cursor", include_str!("../../themes/cursor.json")),
    ("dracula", include_str!("../../themes/dracula.json")),
    ("everforest", include_str!("../../themes/everforest.json")),
    ("flexoki", include_str!("../../themes/flexoki.json")),
    ("github", include_str!("../../themes/github.json")),
    ("gruvbox", include_str!("../../themes/gruvbox.json")),
    ("kanagawa", include_str!("../../themes/kanagawa.json")),
    ("material", include_str!("../../themes/material.json")),
    ("matrix", include_str!("../../themes/matrix.json")),
    ("mercury", include_str!("../../themes/mercury.json")),
    ("monokai", include_str!("../../themes/monokai.json")),
    ("nightowl", include_str!("../../themes/nightowl.json")),
    ("nord", include_str!("../../themes/nord.json")),
    ("one-dark", include_str!("../../themes/one-dark.json")),
    ("osaka-jade", include_str!("../../themes/osaka-jade.json")),
    ("orng", include_str!("../../themes/orng.json")),
    ("lucent-orng", include_str!("../../themes/lucent-orng.json")),
    ("palenight", include_str!("../../themes/palenight.json")),
    ("rosepine", include_str!("../../themes/rosepine.json")),
    ("solarized", include_str!("../../themes/solarized.json")),
    ("synthwave84", include_str!("../../themes/synthwave84.json")),
    ("tokyonight", include_str!("../../themes/tokyonight.json")),
    ("vesper", include_str!("../../themes/vesper.json")),
    ("vercel", include_str!("../../themes/vercel.json")),
    ("zenburn", include_str!("../../themes/zenburn.json")),
    ("carbonfox", include_str!("../../themes/carbonfox.json")),
];

/// The theme sources by name: bundled ones, then custom files (which may override them), then
/// `system`, which renders as `anysola` until the terminal palette is known.
pub struct Catalog {
    sources: BTreeMap<String, Value>,
}

impl Catalog {
    pub fn bundled() -> Catalog {
        let mut sources: BTreeMap<String, Value> = BUNDLED
            .iter()
            .map(|(name, text)| {
                (
                    name.to_string(),
                    serde_json::from_str(text).expect("bundled theme"),
                )
            })
            .collect();
        let anysola = sources[DEFAULT].clone();
        sources.insert("system".into(), anysola);
        Catalog { sources }
    }

    /// Adds `themes/*.json` of each directory, later directories overriding earlier ones.
    /// Files that are not theme documents are skipped; a JSON error fails discovery.
    pub fn discover(&mut self, directories: &[impl AsRef<Path>]) -> Result<(), String> {
        for directory in directories {
            let Ok(entries) = std::fs::read_dir(directory.as_ref().join("themes")) else {
                continue;
            };
            let mut files: Vec<_> = entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "json"))
                .collect();
            files.sort();
            for file in files {
                let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
                let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                let name = file.file_stem().unwrap_or_default().to_string_lossy();
                if value.get("theme").is_some() || value.get("base").is_some() {
                    self.sources.insert(name.into_owned(), value);
                }
            }
        }
        Ok(())
    }

    /// Names as `/themes` lists them (case-insensitive order).
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.sources.keys().cloned().collect();
        names.sort_by_key(|n| n.to_lowercase());
        names
    }

    pub fn contains(&self, name: &str) -> bool {
        self.sources.contains_key(name)
    }

    /// Resolves `name` (unknown names fall back to `anysola`) in `mode`, or that theme's only mode.
    pub fn load(&self, name: &str, mode: Mode) -> Result<Theme, String> {
        let name = if self.contains(name) { name } else { DEFAULT };
        let source = &self.sources[name];
        let document = if source.get("theme").is_some() {
            v1::migrate(source)?
        } else {
            source.clone()
        };
        let resolved = v2::resolve(&document, mode)
            .map_err(|_| format!("Invalid theme: {name} value is an invalid value"))?;
        Ok(Theme {
            name: name.to_string(),
            mode: resolved.mode,
            modes: v2::modes(&document),
            hues: resolved.hues,
            categorical: resolved.categorical,
            base: resolved.base,
            dialog: resolved.dialog,
            footer: resolved.footer,
        })
    }
}

impl Theme {
    pub fn get(&self, path: &str) -> Rgba {
        self.base.get(path)
    }

    pub fn hue(&self, name: &str, step: usize) -> Rgba {
        self.hues
            .iter()
            .find(|(hue, _)| hue == name)
            .map(|(_, scale)| scale[step / 100 - 1])
            .unwrap_or(Rgba::TRANSPARENT)
    }

    /// Step 200 of the n-th categorical hue (agent colours, mention styles).
    pub fn categorical(&self, index: usize) -> Rgba {
        let hue = self.categorical[index % self.categorical.len()];
        self.hues[hue].1[1]
    }

    pub fn categorical_len(&self) -> usize {
        self.categorical.len()
    }

    /// Moves a colour `amount` steps along the hue scale it came from; literals stay unchanged.
    pub fn shift(&self, color: Rgba, amount: i32) -> Rgba {
        match color.origin {
            Some((hue, step)) => {
                let step = (i32::from(step) + amount).clamp(0, 8) as usize;
                self.hues[hue as usize].1[step]
            }
            None => color,
        }
    }

    pub fn decrease(&self, color: Rgba, amount: i32) -> Rgba {
        self.shift(color, -amount)
    }

    /// The terminal colour for `color`, composited over the base background when translucent.
    pub fn term(&self, color: Rgba) -> Color {
        if color.a <= 0.0 {
            return Color::Reset;
        }
        let color = if color.a < 1.0 {
            color.over(self.get("background.base"))
        } else {
            color
        };
        let (r, g, b) = color.bytes();
        Color::Rgb(r, g, b)
    }

    pub fn fg(&self, path: &str) -> Color {
        self.term(self.get(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every token row of `theme-anysola.md` resolves to the listed dark and light colour.
    #[test]
    fn anysola_matches_the_token_table() {
        let spec = include_str!("../../../../spec/verbatim/tui/theme-anysola.md");
        let catalog = Catalog::bundled();
        let dark = catalog.load("anysola", Mode::Dark).unwrap();
        let light = catalog.load("anysola", Mode::Light).unwrap();
        let mut section = "";
        let mut checked = 0;
        for line in spec.lines() {
            if line.starts_with("## ") {
                section = line;
            }
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            if cells.len() != 7 || !cells[1].starts_with('`') || section.contains("Syntax") {
                continue;
            }
            let token = cells[1].trim_matches('`');
            let pick = |theme: &Theme| {
                if section.contains("Dialog") {
                    theme.dialog.get(token)
                } else {
                    theme.base.get(token)
                }
            };
            let hex = |color: Rgba| {
                if color.a == 0.0 {
                    "transparent".to_string()
                } else {
                    color.to_hex()
                }
            };
            assert_eq!(hex(pick(&dark)), cells[3], "dark {token}");
            assert_eq!(hex(pick(&light)), cells[5], "light {token}");
            checked += 1;
        }
        assert!(checked > 100, "checked {checked}");
        assert_eq!(dark.get("background.base").to_hex(), "#110f0d");
        assert_eq!(light.get("background.base").to_hex(), "#fbfaf8");
    }

    #[test]
    fn lists_the_35_bundled_themes() {
        let names = Catalog::bundled().names();
        assert_eq!(names.len(), 35);
        assert_eq!(names[0], "anysola");
        assert!(names.contains(&"system".to_string()));
    }

    #[test]
    fn every_bundled_theme_resolves_in_both_modes() {
        let catalog = Catalog::bundled();
        for name in catalog.names() {
            for mode in [Mode::Dark, Mode::Light] {
                let theme = catalog
                    .load(&name, mode)
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
                assert!(theme.get("text.base").a > 0.0, "{name} text.base");
            }
        }
    }

    #[test]
    fn shift_follows_the_hue_and_keeps_literals() {
        let theme = Catalog::bundled().load("anysola", Mode::Dark).unwrap();
        let text = theme.get("text.base");
        assert_eq!(theme.shift(text, 1).to_hex(), "#c9c3bc");
        let warning = theme.get("text.feedback.warning.base");
        assert_eq!(theme.shift(warning, 2).to_hex(), "#eeba70");
    }

    /// v1 migration matches 0.5.1's `migrateV1` + `resolveThemeDocument` (reference values
    /// produced by running the TypeScript source).
    #[test]
    fn v1_migration_matches_the_reference() {
        let reference: serde_json::Map<String, Value> =
            serde_json::from_str(include_str!("v1-reference.json")).unwrap();
        let catalog = Catalog::bundled();
        for (key, expected) in reference {
            let parts: Vec<&str> = key.split(' ').collect();
            let mode = if parts[1] == "dark" {
                Mode::Dark
            } else {
                Mode::Light
            };
            let theme = catalog.load(parts[0], mode).unwrap();
            let color = match parts[2] {
                "cat0" => theme.categorical(0),
                path => match path.strip_prefix("dialog.") {
                    Some(path) => theme.dialog.get(path),
                    None => theme.get(path),
                },
            };
            let actual = if color.a == 0.0 {
                "transparent".to_string()
            } else {
                Rgba { a: 1.0, ..color }.to_hex()
            };
            assert_eq!(actual, expected.as_str().unwrap(), "{key}");
        }
    }
}
