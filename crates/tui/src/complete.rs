//! Autocomplete sources: `/` commands (TUI commands with their aliases plus server commands) and
//! `@` mentions (skills, subagents, files from an ignore-aware walk), fuzzy ranked with nucleo.

use std::path::Path;

use nucleo_matcher::pattern::{AtomKind as Fuzzy, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::composer::AtomKind;
use crate::keymap::slash::SLASH;
use crate::state::{PopupAction, PopupRow};

/// What decides which commands are listed.
pub struct Scope<'a> {
    pub in_session: bool,
    pub revert_staged: bool,
    /// Server commands: `(name, description)`.
    pub server: &'a [(String, String)],
}

/// Every `/` row, sorted, and the display width names are padded to.
pub fn slash_rows(scope: &Scope) -> (Vec<PopupRow>, usize) {
    let mut rows = Vec::new();
    for command in SLASH {
        if (command.session && !scope.in_session)
            || (command.id == "session.redo" && !scope.revert_staged)
        {
            continue;
        }
        let description = command.description.unwrap_or(command.title);
        for name in std::iter::once(command.name).chain(command.aliases.iter().copied()) {
            let action = if command.arguments {
                PopupAction::Insert(format!("/{name} "))
            } else {
                PopupAction::Command(command.id.to_string())
            };
            rows.push(PopupRow {
                display: format!("/{name}"),
                description: description.to_string(),
                label: String::new(),
                action,
            });
        }
    }
    for (name, description) in scope.server {
        if rows.iter().any(|r| r.display == format!("/{name}")) {
            continue;
        }
        rows.push(PopupRow {
            display: format!("/{name}"),
            description: description.clone(),
            label: String::new(),
            action: PopupAction::Insert(format!("/{name} ")),
        });
    }
    rows.sort_by_key(|r| r.display.to_lowercase());
    let pad = rows
        .iter()
        .map(|r| r.display.chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    (rows, pad)
}

/// Fuzzy filter over `keys(row)`, best first, at most `limit`; rows whose display starts with
/// `prefix + query` score double.
pub fn filter(rows: Vec<PopupRow>, query: &str, prefix: &str, limit: usize) -> Vec<PopupRow> {
    if query.is_empty() {
        return rows;
    }
    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::new(
        query,
        CaseMatching::Ignore,
        Normalization::Smart,
        Fuzzy::Fuzzy,
    );
    let mut buffer = Vec::new();
    let mut scored: Vec<(u32, usize, PopupRow)> = rows
        .into_iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let keys = [
                row.display.trim_start_matches(['/', '@']),
                row.description.as_str(),
            ];
            let score = keys
                .iter()
                .enumerate()
                .filter_map(|(k, key)| {
                    let score = pattern.score(Utf32Str::new(key, &mut buffer), &mut matcher)?;
                    // A match in the description counts for less than one in the name.
                    Some(if k == 0 { score } else { score / 2 })
                })
                .max()?;
            let starts = row
                .display
                .to_lowercase()
                .starts_with(&format!("{prefix}{}", query.to_lowercase()));
            Some((if starts { score * 2 } else { score }, index, row))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, _, row)| row)
        .collect()
}

/// Files and directories under `root` (gitignore-aware), relative, directories with a `/`.
pub fn walk(root: &Path, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    for entry in ignore::WalkBuilder::new(root)
        .hidden(true)
        .build()
        .flatten()
    {
        if out.len() >= limit {
            break;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        if relative.as_os_str().is_empty() {
            continue;
        }
        let mut text = relative.to_string_lossy().replace('\\', "/");
        if entry.file_type().is_some_and(|t| t.is_dir()) {
            text.push('/');
        }
        out.push(text);
    }
    out
}

/// `@` rows: skills, subagents, then the files matching `query` (at most 20).
/// `@` rows: skills, references (`(name, description)`), agents, then files. A query that
/// starts with a reference name and `/` lists only that reference.
pub fn mention_rows(
    query: &str,
    skills: &[(String, String)],
    references: &[(String, String)],
    agents: &[(String, String)],
    files: &[String],
) -> Vec<PopupRow> {
    let named = |items: &[(String, String)], label: &str, kind: AtomKind| -> Vec<PopupRow> {
        items
            .iter()
            .map(|(name, description)| PopupRow {
                display: format!("@{name}"),
                description: description.clone(),
                label: label.to_string(),
                action: PopupAction::Mention(kind.clone(), format!("@{name}")),
            })
            .collect()
    };
    let alias = query.split_once('/').map(|(head, _)| head);
    if let Some(reference) = references.iter().find(|(name, _)| Some(name.as_str()) == alias) {
        return named(std::slice::from_ref(reference), "reference", AtomKind::File);
    }
    let mut rows = named(skills, "skill", AtomKind::Skill);
    rows.extend(named(references, "reference", AtomKind::File));
    rows.extend(named(agents, "agent", AtomKind::Agent));
    let (path, range) = match query.split_once('#') {
        Some((path, range)) => (path, Some(range)),
        None => (query, None),
    };
    let file_rows: Vec<PopupRow> = files
        .iter()
        .map(|file| {
            let suffix = range.map_or(String::new(), |r| format!("#{r}"));
            let action = if file.ends_with('/') {
                PopupAction::Directory(format!("@{file}"))
            } else {
                PopupAction::Mention(AtomKind::File, format!("@{file}{suffix}"))
            };
            PopupRow {
                display: format!("@{file}"),
                description: String::new(),
                label: String::new(),
                action,
            }
        })
        .collect();
    if query.is_empty() {
        rows.extend(file_rows);
        return rows;
    }
    let mut ranked = filter(rows, path, "@", 10);
    ranked.extend(filter(file_rows, path, "@", 20));
    ranked.truncate(10);
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The home `/` list equals 0.5.1's (`slash-list-home-context.txt`) minus `/plugins`, which
    /// 1.0 drops with the plugin host.
    #[test]
    fn home_slash_list_matches_0_5_1() {
        let server = [
            (
                "goal".to_string(),
                "keep working until an objective is met [pause|resume|clear]".to_string(),
            ),
            ("init".to_string(), "guided AGENTS.md setup".to_string()),
            (
                "review".to_string(),
                "review changes [commit|branch|pr], defaults to uncommitted".to_string(),
            ),
        ];
        let scope = Scope {
            in_session: false,
            revert_staged: false,
            server: &server,
        };
        let (rows, pad) = slash_rows(&scope);
        let capture =
            include_str!("../../../spec/verbatim/tui/screens/slash-list-home-context.txt");
        let expected: Vec<String> = capture
            .lines()
            .filter_map(|l| l.strip_prefix("┃ "))
            .map(|l| l.split_whitespace().next().unwrap_or_default().to_string())
            .filter(|name| name != "/plugins")
            .collect();
        let actual: Vec<String> = rows.iter().map(|r| r.display.clone()).collect();
        assert_eq!(actual, expected);
        assert_eq!(pad, 12);
    }

    #[test]
    fn session_only_commands_appear_in_a_session() {
        let scope = Scope {
            in_session: true,
            revert_staged: false,
            server: &[],
        };
        let names: Vec<String> = slash_rows(&scope)
            .0
            .into_iter()
            .map(|r| r.display)
            .collect();
        assert!(names.contains(&"/compact".to_string()));
        assert!(!names.contains(&"/redo".to_string()));
    }

    #[test]
    fn filters_prefer_prefix_matches() {
        let scope = Scope {
            in_session: false,
            revert_staged: false,
            server: &[],
        };
        let rows = filter(slash_rows(&scope).0, "mod", "/", 10);
        assert_eq!(rows[0].display, "/models");
    }

    #[test]
    fn reference_alias_lists_only_that_reference() {
        let references = vec![("docs".to_string(), " /srv/docs".to_string())];
        let rows = mention_rows("docs/guide", &[], &references, &[], &["a.rs".into()]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "reference");
        let all = mention_rows("", &[], &references, &[], &[]);
        assert_eq!(all[0].display, "@docs");
    }

    #[test]
    fn mentions_list_files_with_ranges() {
        let files = vec!["src/".to_string(), "src/main.rs".to_string()];
        let rows = mention_rows("main#12-20", &[], &[], &[], &files);
        assert_eq!(
            rows[0].action,
            PopupAction::Mention(AtomKind::File, "@src/main.rs#12-20".into())
        );
    }
}
