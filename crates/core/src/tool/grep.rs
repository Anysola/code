//! The `grep` tool: ripgrep-style content search (see [`super::search`]).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::sinks::Lossy;
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder};
use serde::Deserialize;
use serde_json::{Map, Value};

use super::search::{self, DEFAULT_LIMIT, EXCLUDE_GIT};
use super::{
    DefinitionEnv, Tool, ToolContext, ToolDefinition, ToolError, ToolOutput, ToolResult, access,
    parse_input,
};
use crate::embedded_def;
use crate::permission::PermissionRequest;
use crate::text::{utf16_len, utf16_prefix};

embedded_def!(def, "grep");

/// Longest match line kept, in UTF-16 units; longer lines are cut and get `...`.
const MAX_LINE_CHARS: usize = 2000;

pub struct GrepTool;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Input {
    pattern: String,
    path: Option<String>,
    include: Option<String>,
    literal: Option<bool>,
    case_sensitive: Option<bool>,
    limit: Option<usize>,
}

impl Input {
    /// The permission request's metadata, keys in 0.5.1's order, absent ones left out.
    fn ask_metadata(&self) -> Value {
        let mut map = Map::new();
        map.insert("root".into(), ".".into());
        let fields = [
            ("path", self.path.clone().map(Value::from)),
            ("include", self.include.clone().map(Value::from)),
            ("literal", self.literal.map(Value::from)),
            ("caseSensitive", self.case_sensitive.map(Value::from)),
            ("limit", self.limit.map(Value::from)),
        ];
        for (key, value) in fields {
            if let Some(value) = value {
                map.insert(key.into(), value);
            }
        }
        Value::Object(map)
    }
}

#[derive(Debug, PartialEq)]
struct Match {
    path: PathBuf,
    line: u64,
    /// The whole matched line, line terminator included, as rg's JSON `lines.text`.
    text: String,
}

/// What building the matcher can end in besides a matcher.
enum Pattern {
    Matcher(RegexMatcher),
    /// rg reports a syntax error as `Invalid regex pattern: …`.
    Invalid(String),
    /// Any other rejection (a literal `\n`, a size limit) made rg exit without output: no matches.
    Rejected,
}

fn matcher(input: &Input) -> Pattern {
    let built = RegexMatcherBuilder::new()
        .case_insensitive(input.case_sensitive == Some(false))
        .fixed_strings(input.literal == Some(true))
        .line_terminator(Some(b'\n'))
        .build(&input.pattern);
    match built {
        Ok(matcher) => Pattern::Matcher(matcher),
        Err(error) => {
            let message = error.to_string();
            if message.contains("regex parse error") || message.contains("error parsing regex") {
                Pattern::Invalid(format!("rg: {message}"))
            } else {
                Pattern::Rejected
            }
        }
    }
}

fn cap_line(text: &str) -> String {
    if utf16_len(text) <= MAX_LINE_CHARS {
        return text.to_string();
    }
    let kept = utf16_prefix(text, MAX_LINE_CHARS);
    // JavaScript cut a surrogate pair in half there; decoding the lone half gave U+FFFD.
    let split_pair = utf16_len(kept) < MAX_LINE_CHARS;
    format!("{kept}{}...", if split_pair { "\u{FFFD}" } else { "" })
}

/// Searches `path`, appending matches until `found` holds `max` of them or `stop` is set.
fn search_file(
    searcher: &mut Searcher,
    matcher: &RegexMatcher,
    path: &Path,
    found: &mut Vec<Match>,
    max: usize,
    stop: &AtomicBool,
) {
    let sink = Lossy(|line, text: &str| {
        found.push(Match {
            path: path.to_path_buf(),
            line,
            text: cap_line(text),
        });
        Ok(found.len() < max && !stop.load(Ordering::Relaxed))
    });
    // Unreadable files are skipped, as rg does with `--no-messages`.
    let _ = searcher.search_path(matcher, path, sink);
}

/// Up to `max` matches under `root` (a directory, or a single file searched as given).
fn find(
    matcher: &RegexMatcher,
    root: &Path,
    is_file: bool,
    include: Option<&str>,
    max: usize,
    stop: &AtomicBool,
) -> Vec<Match> {
    let mut found = Vec::new();
    let globs: Vec<&str> = include.into_iter().chain([EXCLUDE_GIT]).collect();
    let Some(overrides) = search::overrides(root, &globs) else {
        return found;
    };
    // rg converts NUL bytes in a file it was given explicitly and skips binary files it walks to.
    let binary = if is_file {
        BinaryDetection::convert(0)
    } else {
        BinaryDetection::quit(0)
    };
    let mut searcher = SearcherBuilder::new()
        .line_number(true)
        .binary_detection(binary)
        .build();
    if is_file {
        search_file(&mut searcher, matcher, root, &mut found, max, stop);
        return found;
    }
    for file in search::files(root, overrides, true, stop) {
        search_file(&mut searcher, matcher, &file, &mut found, max, stop);
        if found.len() >= max {
            break;
        }
    }
    found
}

fn format_matches(matches: &[Match], truncated: bool) -> String {
    let mut lines = vec![if matches.is_empty() {
        "No matches found".to_string()
    } else {
        format!("Found {} matches", matches.len())
    }];
    let mut current: Option<&Path> = None;
    for found in matches {
        if current != Some(found.path.as_path()) {
            if current.is_some() {
                lines.push(String::new());
            }
            current = Some(&found.path);
            lines.push(format!("{}:", found.path.display()));
        }
        lines.push(format!("  Line {}: {}", found.line, found.text));
    }
    if truncated {
        lines.push(String::new());
        lines.push(search::truncation_notice(matches.len()));
    }
    lines.join("\n")
}

impl Tool for GrepTool {
    fn name(&self) -> &'static str {
        "grep"
    }

    fn definition(&self, _: &DefinitionEnv) -> ToolDefinition {
        let def = def();
        ToolDefinition {
            name: self.name().into(),
            description: def.description.clone(),
            input_schema: def.input_schema.clone(),
        }
    }

    fn execute<'a>(&'a self, input: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolResult> {
        async move {
            let input: Input = parse_input(self.name(), &def().input_schema, input)?;
            let shown_path = input.path.as_deref().unwrap_or(".");
            let target = access::resolve(ctx, shown_path, None);
            access::authorize_external(ctx, &[&target]).await?;
            ctx.ask(PermissionRequest {
                action: self.name().into(),
                resources: vec![input.pattern.clone()],
                save: vec!["*".into()],
                metadata: input.ask_metadata(),
            })
            .await?;
            let is_file = std::fs::metadata(&target.absolute)
                .map_err(|_| {
                    ToolError::execution(format!("Search path does not exist: {shown_path}"))
                })?
                .is_file();
            let limit = input.limit.unwrap_or(DEFAULT_LIMIT);
            let matcher = match matcher(&input) {
                Pattern::Matcher(matcher) => matcher,
                Pattern::Invalid(message) => {
                    return Err(ToolError::execution(format!(
                        "Invalid regex pattern: {message}"
                    )));
                }
                Pattern::Rejected => return Ok(output(&[], false)),
            };
            let root = target.absolute;
            let include = input.include;
            let mut matches = search::run(move |stop| {
                find(
                    &matcher,
                    &root,
                    is_file,
                    include.as_deref(),
                    limit + 1,
                    stop,
                )
            })
            .await?;
            let truncated = matches.len() > limit;
            matches.truncate(limit);
            Ok(output(&matches, truncated))
        }
        .boxed()
    }
}

fn output(matches: &[Match], truncated: bool) -> ToolOutput {
    ToolOutput::text(format_matches(matches, truncated))
        .with_metadata("matches", matches.len())
        .with_metadata("truncated", truncated)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::tool::ToolContent;
    use crate::tool::search::fixture;
    use crate::tool::test_support::context;

    async fn grep(dir: &Path, input: Value) -> ToolResult {
        let test = context(dir);
        GrepTool.execute(input, &test.ctx).await
    }

    fn text(output: &ToolOutput) -> &str {
        let [ToolContent::Text { text }] = output.content.as_slice() else {
            panic!("one text block expected: {:?}", output.content)
        };
        text
    }

    #[tokio::test]
    async fn captured_matches() {
        let dir = tempfile::tempdir().unwrap();
        fixture::project(dir.path());
        let d = dir.path().display();
        let s = std::path::MAIN_SEPARATOR;
        let out = grep(dir.path(), json!({"pattern": "foo"})).await.unwrap();
        assert_eq!(
            text(&out),
            format!(
                "Found 3 matches\n{d}{s}.hid{s}h.ts:\n  Line 1: foo\n\n\n{d}{s}src{s}x.ts:\n  Line 1: foo one\n\n  Line 3: foo two\n"
            )
        );
        assert_eq!(
            Value::Object(out.metadata),
            json!({"matches": 3, "truncated": false})
        );
    }

    #[tokio::test]
    async fn captured_no_match_and_limit() {
        let dir = tempfile::tempdir().unwrap();
        fixture::project(dir.path());
        let d = dir.path().display();
        let s = std::path::MAIN_SEPARATOR;
        let none = grep(dir.path(), json!({"pattern": "zzz"})).await.unwrap();
        assert_eq!(text(&none), "No matches found");
        let one = grep(dir.path(), json!({"pattern": "foo", "limit": 1}))
            .await
            .unwrap();
        assert_eq!(
            text(&one),
            format!(
                "Found 1 matches\n{d}{s}.hid{s}h.ts:\n  Line 1: foo\n\n\n(Results are truncated: showing first 1 results. Consider using a more specific path or pattern.)"
            )
        );
        assert_eq!(
            Value::Object(one.metadata),
            json!({"matches": 1, "truncated": true})
        );
    }

    #[tokio::test]
    async fn captured_errors() {
        let dir = tempfile::tempdir().unwrap();
        fixture::project(dir.path());
        let error = grep(dir.path(), json!({"pattern": "(", "path": "src"}))
            .await
            .unwrap_err();
        assert_eq!(error.type_tag(), "tool.execution");
        assert_eq!(
            error.message,
            "Invalid regex pattern: rg: regex parse error:\n    (?:()\n    ^\nerror: unclosed group"
        );
        let error = grep(dir.path(), json!({"pattern": "foo", "path": "nope"}))
            .await
            .unwrap_err();
        assert_eq!(error.type_tag(), "tool.execution");
        assert_eq!(error.message, "Search path does not exist: nope");
    }

    #[tokio::test]
    async fn options() {
        let dir = tempfile::tempdir().unwrap();
        fixture::project(dir.path());
        std::fs::write(dir.path().join("src/y.js"), "FOO.bar\nfooxbar").unwrap();
        let d = dir.path().display();
        let s = std::path::MAIN_SEPARATOR;
        let out = grep(
            dir.path(),
            json!({"pattern": "foo.bar", "literal": true, "caseSensitive": false, "include": "*.js"}),
        )
        .await
        .unwrap();
        assert_eq!(
            text(&out),
            format!("Found 1 matches\n{d}{s}src{s}y.js:\n  Line 1: FOO.bar\n")
        );
        // The last line has no terminator, so no blank line follows it.
        let out = grep(dir.path(), json!({"pattern": "x.ar", "path": "src/y.js"}))
            .await
            .unwrap();
        assert_eq!(
            text(&out),
            format!("Found 1 matches\n{d}{s}src{s}y.js:\n  Line 2: fooxbar")
        );
        // An include glob that matches the ignored directory re-includes it.
        let out = grep(dir.path(), json!({"pattern": "foo", "include": "i*"}))
            .await
            .unwrap();
        assert_eq!(
            text(&out),
            format!("Found 1 matches\n{d}{s}ign{s}i.ts:\n  Line 1: foo\n")
        );
    }

    #[tokio::test]
    async fn rejected_patterns_find_nothing() {
        let dir = tempfile::tempdir().unwrap();
        fixture::project(dir.path());
        for input in [
            json!({"pattern": "a\\nb"}),
            json!({"pattern": "foo", "include": "a{b"}),
        ] {
            let out = grep(dir.path(), input).await.unwrap();
            assert_eq!(text(&out), "No matches found");
        }
    }

    #[test]
    fn long_lines_are_capped() {
        let line = format!("{}\n", "a".repeat(2500));
        assert_eq!(cap_line(&line), format!("{}...", "a".repeat(2000)));
        let line = format!("{}😀x", "a".repeat(1999));
        assert_eq!(cap_line(&line), format!("{}\u{FFFD}...", "a".repeat(1999)));
        assert_eq!(cap_line("short\n"), "short\n");
    }

    #[tokio::test]
    async fn permission_order() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let test = context(dir.path());
        let path = outside.path().display().to_string();
        GrepTool
            .execute(json!({"pattern": "x", "path": path, "limit": 5}), &test.ctx)
            .await
            .unwrap();
        let asked = test.gate.asked.lock().unwrap();
        assert_eq!(asked[0].action, "external_directory");
        // Rule resources always use `/`.
        assert_eq!(
            asked[0].resources,
            [format!("{}/*", path.replace('\\', "/"))]
        );
        assert_eq!(asked[1].action, "grep");
        assert_eq!(asked[1].resources, ["x"]);
        assert_eq!(asked[1].save, ["*"]);
        assert_eq!(
            asked[1].metadata,
            json!({"root": ".", "path": path, "limit": 5})
        );
    }
}
