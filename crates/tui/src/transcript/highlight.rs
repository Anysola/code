//! Code highlighting with syntect and two-face's extra syntaxes, coloured from the active
//! theme's `syntax.*`/`markdown.*` tokens. Loaded on the first code block.

use std::str::FromStr;
use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use syntect::easy::HighlightLines;
use syntect::highlighting::{
    Color as SynColor, FontStyle, ScopeSelectors, StyleModifier, Theme as SynTheme, ThemeItem,
    ThemeSettings,
};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

use crate::theme::Theme;

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(two_face::syntax::extra_newlines)
}

/// Info-string and file-type names 0.5.1 highlighted, mapped to the syntax that covers them.
fn alias(language: &str) -> &str {
    match language {
        "typescriptreact" | "tsx" => "TypeScriptReact",
        "javascriptreact" | "jsx" => "JavaScript (Babel)",
        "csharp" | "c#" | "cs" => "C#",
        "fsharp" | "f#" => "F#",
        "hcl" | "terraform" | "tf" => "Terraform",
        "make" | "makefile" => "Makefile",
        "vim" | "viml" | "vimscript" => "VimL",
        "udiff" | "patch" => "diff",
        "shell" | "sh" | "zsh" | "bash" => "Bourne Again Shell (bash)",
        "cpp" | "c++" => "C++",
        "ocaml" => "OCaml",
        "elixir" | "ex" => "Elixir",
        // No Agda grammar ships with syntect or two-face; its syntax is closest to Haskell's.
        "agda" => "Haskell",
        other => other,
    }
}

pub fn find(language: &str) -> Option<&'static SyntaxReference> {
    let set = syntaxes();
    let lower = language.to_lowercase();
    let name = alias(&lower);
    set.find_syntax_by_name(name)
        .or_else(|| set.find_syntax_by_token(name))
        .or_else(|| {
            set.syntaxes()
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(name))
        })
}

fn syn(theme: &Theme, path: &str) -> SynColor {
    // Translucent colours (some themes' comments) are flattened over the background.
    let Color::Rgb(r, g, b) = theme.fg(path) else {
        return SynColor {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        };
    };
    SynColor { r, g, b, a: 255 }
}

/// TextMate scopes mapped onto the theme tokens 0.5.1 used for the matching tree-sitter captures.
const RULES: &[(&str, &str, FontStyle)] = &[
    ("comment", "syntax.comment", FontStyle::ITALIC),
    (
        "string, constant.character",
        "syntax.string",
        FontStyle::empty(),
    ),
    (
        "constant.numeric, constant.language",
        "syntax.number",
        FontStyle::empty(),
    ),
    ("keyword.control", "syntax.keyword", FontStyle::ITALIC),
    (
        "keyword.operator, punctuation.separator",
        "syntax.operator",
        FontStyle::empty(),
    ),
    (
        "keyword, storage.modifier",
        "syntax.keyword",
        FontStyle::ITALIC,
    ),
    ("storage.type", "syntax.type", FontStyle::BOLD),
    (
        "entity.name.function, support.function, meta.function-call",
        "syntax.function",
        FontStyle::empty(),
    ),
    (
        "variable, variable.parameter, meta.property-name",
        "syntax.variable",
        FontStyle::empty(),
    ),
    (
        "entity.name.type, entity.name.class, support.type, support.class, entity.name.namespace",
        "syntax.type",
        FontStyle::empty(),
    ),
    ("punctuation", "syntax.punctuation", FontStyle::empty()),
    (
        "variable.language, support.constant",
        "text.feedback.error.base",
        FontStyle::empty(),
    ),
    (
        "entity.name.tag",
        "text.feedback.error.base",
        FontStyle::empty(),
    ),
    (
        "entity.other.attribute-name",
        "syntax.keyword",
        FontStyle::empty(),
    ),
    ("markup.heading", "markdown.heading", FontStyle::BOLD),
    ("markup.bold", "markdown.strong", FontStyle::BOLD),
    ("markup.italic", "markdown.emphasis", FontStyle::ITALIC),
    ("markup.quote", "markdown.blockQuote", FontStyle::ITALIC),
    ("markup.raw", "markdown.code", FontStyle::empty()),
    (
        "markup.underline.link",
        "markdown.link",
        FontStyle::UNDERLINE,
    ),
    ("markup.inserted", "diff.text.added", FontStyle::empty()),
    ("markup.deleted", "diff.text.removed", FontStyle::empty()),
    (
        "meta.diff.range, meta.diff.header",
        "diff.text.context",
        FontStyle::empty(),
    ),
];

fn build(theme: &Theme) -> SynTheme {
    SynTheme {
        settings: ThemeSettings {
            foreground: Some(syn(theme, "text.base")),
            ..ThemeSettings::default()
        },
        scopes: RULES
            .iter()
            .filter_map(|(scopes, token, style)| {
                Some(ThemeItem {
                    scope: ScopeSelectors::from_str(scopes).ok()?,
                    style: StyleModifier {
                        foreground: Some(syn(theme, token)),
                        background: None,
                        font_style: Some(*style),
                    },
                })
            })
            .collect(),
        ..SynTheme::default()
    }
}

/// The syntect theme for the active theme, built once per theme and mode.
fn cached(theme: &Theme) -> std::sync::Arc<SynTheme> {
    type Cache = std::sync::Mutex<Option<((String, crate::theme::Mode), std::sync::Arc<SynTheme>)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let key = (theme.name.clone(), theme.mode);
    let mut slot = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match slot.as_ref() {
        Some((k, built)) if *k == key => built.clone(),
        _ => {
            let built = std::sync::Arc::new(build(theme));
            *slot = Some((key, built.clone()));
            built
        }
    }
}

/// `code` as styled lines; unknown languages come back in the base text colour.
pub fn highlight(code: &str, language: &str, theme: &Theme) -> Vec<Vec<Span<'static>>> {
    let plain = || {
        code.lines()
            .map(|l| {
                vec![Span::styled(
                    l.to_string(),
                    Style::new().fg(theme.fg("text.base")),
                )]
            })
            .collect()
    };
    let Some(syntax) = find(language) else {
        return plain();
    };
    let synth = cached(theme);
    let mut lines = HighlightLines::new(syntax, &synth);
    let mut out = Vec::new();
    for line in LinesWithEndings::from(code) {
        let Ok(ranges) = lines.highlight_line(line, syntaxes()) else {
            return plain();
        };
        out.push(
            ranges
                .into_iter()
                .map(|(style, text)| {
                    let mut s = Style::new().fg(Color::Rgb(
                        style.foreground.r,
                        style.foreground.g,
                        style.foreground.b,
                    ));
                    if style.font_style.contains(FontStyle::BOLD) {
                        s = s.add_modifier(Modifier::BOLD);
                    }
                    if style.font_style.contains(FontStyle::ITALIC) {
                        s = s.add_modifier(Modifier::ITALIC);
                    }
                    if style.font_style.contains(FontStyle::UNDERLINE) {
                        s = s.add_modifier(Modifier::UNDERLINED);
                    }
                    Span::styled(text.trim_end_matches(['\n', '\r']).to_string(), s)
                })
                .collect(),
        );
    }
    out
}

/// The info-string language for a file path, as 0.5.1's file-type map names it.
pub fn filetype(path: &str) -> &'static str {
    let name = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_lowercase();
    if name == "makefile" {
        return "make";
    }
    let extension = name.rsplit_once('.').map_or("", |(_, e)| e);
    match extension {
        "ts" | "mts" | "cts" | "js" | "mjs" | "cjs" | "jsx" => "typescript",
        "tsx" => "tsx",
        "md" | "mdx" | "markdown" => "markdown",
        "py" | "pyi" => "python",
        "rs" => "rust",
        "go" => "go",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "cs" => "csharp",
        "sh" | "bash" | "zsh" => "bash",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "rb" => "ruby",
        "php" => "php",
        "scala" | "sc" => "scala",
        "html" | "htm" => "html",
        "vue" => "vue",
        "tf" | "hcl" => "hcl",
        "json" | "jsonc" => "json",
        "yaml" | "yml" => "yaml",
        "hs" => "haskell",
        "css" | "scss" => "css",
        "jl" => "julia",
        "lua" => "lua",
        "ml" | "mli" => "ocaml",
        "clj" | "cljs" | "edn" => "clojure",
        "swift" => "swift",
        "toml" => "toml",
        "nix" => "nix",
        "diff" | "patch" => "diff",
        "ex" | "exs" => "elixir",
        "fs" | "fsx" | "fsi" => "fsharp",
        "r" => "r",
        "mk" => "make",
        "vim" => "vim",
        "xml" | "svg" => "xml",
        "agda" => "agda",
        "zig" => "zig",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every language 0.5.1 highlighted has a built-in syntax.
    #[test]
    fn covers_the_languages_of_0_5_1() {
        let languages = [
            "javascript",
            "typescript",
            "tsx",
            "markdown",
            "zig",
            "python",
            "rust",
            "go",
            "cpp",
            "csharp",
            "bash",
            "c",
            "java",
            "kotlin",
            "ruby",
            "php",
            "scala",
            "html",
            "vue",
            "hcl",
            "json",
            "yaml",
            "haskell",
            "css",
            "julia",
            "lua",
            "ocaml",
            "clojure",
            "swift",
            "toml",
            "nix",
            "diff",
            "elixir",
            "fsharp",
            "r",
            "make",
            "vim",
            "xml",
            "agda",
        ];
        let missing: Vec<&str> = languages
            .iter()
            .copied()
            .filter(|l| find(l).is_none())
            .collect();
        assert!(missing.is_empty(), "missing {missing:?}");
    }

    /// Comments, strings, numbers and control keywords take the theme's own `syntax.*` colours
    /// in every bundled theme and mode.
    #[test]
    fn token_mapping_per_theme() {
        let catalog = crate::theme::Catalog::bundled();
        let code = "// note\nfunction f() { return \"x\" + 42 }\n";
        for name in catalog.names() {
            for mode in [crate::theme::Mode::Dark, crate::theme::Mode::Light] {
                let Ok(theme) = catalog.load(&name, mode) else {
                    continue;
                };
                let lines = highlight(code, "javascript", &theme);
                let colour = |line: usize, text: &str| {
                    lines[line]
                        .iter()
                        .find(|s| s.content.contains(text))
                        .and_then(|s| s.style.fg)
                };
                for (line, text, token) in [
                    (0, "note", "syntax.comment"),
                    (1, "return", "syntax.keyword"),
                    (1, "x", "syntax.string"),
                    (1, "42", "syntax.number"),
                ] {
                    assert_eq!(
                        colour(line, text),
                        Some(theme.fg(token)),
                        "{name} {mode:?} {text}"
                    );
                }
            }
        }
    }
}
