//! The Code Mode catalog in system block 2: every namespace's line, and as many tool listings as
//! fit the inline budget (shortest first, round-robin across namespaces); the rest are reachable
//! through `search`. Also the texts sent when the catalog changes mid-session.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::typescript::tool_expression;

const DESCRIPTION_LIMIT: usize = 120;
const CHARACTERS_PER_TOKEN: f64 = 4.0;
const INLINE_BUDGET: i64 = 2_000;

const SEARCH_SIGNATURE: &str = include_str!("search-signature.txt");
const EMPTY: &str = "No Code Mode tools are currently available. Later Code Mode catalog updates may add or remove tools. Do not call `execute` unless there is at least one available Code Mode tool.";
pub const REMOVED: &str =
    "Code Mode tools are no longer available. Do not use any previously listed Code Mode tools.";
const CHANGED: &str = "The Code Mode tool catalog has changed.";

/// A tool as the catalog lists it.
#[derive(Debug, Clone)]
pub struct Entry {
    /// `namespace.tool`
    pub path: String,
    pub description: String,
    pub signature: String,
    /// Listed before the budget is shared out.
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Listing {
    pub path: String,
    pub line: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamespaceSummary {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub count: usize,
    pub entries: Vec<Listing>,
}

/// What the model was shown; stored so a later catalog can be described as a change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub total: usize,
    pub shown: usize,
    pub namespaces: Vec<NamespaceSummary>,
}

fn cost(text: &str) -> i64 {
    (text.encode_utf16().count() as f64 / CHARACTERS_PER_TOKEN).round() as i64
}

fn listing(entry: &Entry) -> Listing {
    let first = entry.description.split('\n').next().unwrap_or("").trim();
    let description = if first.encode_utf16().count() > DESCRIPTION_LIMIT {
        let cut: Vec<u16> = first.encode_utf16().take(DESCRIPTION_LIMIT - 3).collect();
        format!("{}...", String::from_utf16_lossy(&cut))
    } else {
        first.to_string()
    };
    let suffix = if description.is_empty() {
        String::new()
    } else {
        format!(" // {description}")
    };
    Listing {
        path: entry.path.clone(),
        line: format!("  - {}{suffix}", entry.signature),
    }
}

pub fn namespace_line(namespace: &NamespaceSummary) -> String {
    let count = if namespace.count == 1 {
        "1 tool".to_string()
    } else {
        format!("{} tools", namespace.count)
    };
    let label = if namespace.entries.len() == namespace.count {
        count
    } else if namespace.entries.is_empty() {
        format!("{count}, none shown")
    } else {
        format!("{count}, {} shown", namespace.entries.len())
    };
    let description = namespace
        .description
        .as_ref()
        .map(|d| format!(" // {d}"))
        .unwrap_or_default();
    format!("- {} ({label}){description}", namespace.name)
}

struct Selection {
    summary: NamespaceSummary,
    listings: Vec<Listing>,
    order: Vec<(usize, i64)>,
    selected: HashSet<usize>,
    next: usize,
}

/// `namespaces` gives each namespace's description; tools are grouped by their first path segment.
pub fn summarize(entries: &[Entry], namespaces: &[(String, Option<String>)]) -> Summary {
    summarize_within(entries, namespaces, INLINE_BUDGET)
}

fn summarize_within(
    entries: &[Entry],
    namespaces: &[(String, Option<String>)],
    budget: i64,
) -> Summary {
    let mut names: Vec<&str> = entries
        .iter()
        .map(|e| e.path.split('.').next().unwrap_or(&e.path))
        .collect();
    names.sort_unstable();
    names.dedup();
    let mut groups: Vec<Selection> = names
        .into_iter()
        .map(|name| {
            let mut members: Vec<&Entry> = entries
                .iter()
                .filter(|e| e.path.split('.').next() == Some(name))
                .collect();
            members.sort_by(|a, b| a.path.cmp(&b.path));
            let listings: Vec<Listing> = members.iter().map(|e| listing(e)).collect();
            let selected: HashSet<usize> =
                (0..members.len()).filter(|&i| members[i].pinned).collect();
            let mut order: Vec<(usize, i64)> = (0..listings.len())
                .filter(|i| !selected.contains(i))
                .map(|i| (i, cost(&listings[i].line)))
                .collect();
            order.sort_by(|a, b| {
                a.1.cmp(&b.1)
                    .then_with(|| listings[a.0].path.cmp(&listings[b.0].path))
            });
            Selection {
                summary: NamespaceSummary {
                    name: name.to_string(),
                    description: namespaces
                        .iter()
                        .find(|(n, _)| n == name)
                        .and_then(|(_, d)| d.clone()),
                    count: listings.len(),
                    entries: Vec::new(),
                },
                listings,
                order,
                selected,
                next: 0,
            }
        })
        .collect();

    let mut remaining = budget
        - groups
            .iter()
            .map(|g| cost(&namespace_line(&g.summary)))
            .sum::<i64>()
        - groups
            .iter()
            .flat_map(|g| g.selected.iter().map(|&i| cost(&g.listings[i].line)))
            .sum::<i64>();
    let mut active: Vec<usize> = (0..groups.len()).collect();
    while !active.is_empty() {
        active.retain(|&index| {
            let group = &mut groups[index];
            let Some(&(candidate, price)) = group.order.get(group.next) else {
                return false;
            };
            if price > remaining {
                return false;
            }
            group.selected.insert(candidate);
            group.next += 1;
            remaining -= price;
            group.next < group.order.len()
        });
    }

    let namespaces: Vec<NamespaceSummary> = groups
        .into_iter()
        .map(|g| NamespaceSummary {
            entries: (0..g.listings.len())
                .filter(|i| g.selected.contains(i))
                .map(|i| g.listings[i].clone())
                .collect(),
            ..g.summary
        })
        .collect();
    Summary {
        total: entries.len(),
        shown: namespaces.iter().map(|n| n.entries.len()).sum(),
        namespaces,
    }
}

fn prompt(partial: bool) -> String {
    let listed = if partial {
        "The tools listed below, and `search`,"
    } else {
        "The tools listed below"
    };
    let guidance = if partial {
        format!(
            "The catalog is partial. Before calling a tool that is not listed below, find it with the global `search(...)` function. Do not guess tool names. Each result's `path` is the complete call expression: write it out as code, for example `await tools.ns.tool_name({{ ... }})`. Do not use it as a key such as `tools[path]`.\n\n- {SEARCH_SIGNATURE}"
        )
    } else {
        "The catalog is complete. Do not guess tool names.".to_string()
    };
    format!(
        "# Code Mode\n\n{listed} cannot be called directly: they only work as JavaScript inside the `execute` tool. Every other tool you have is a normal tool call: call it directly, never inside `execute`.\n\n{guidance}\n\n## Code Mode tools (call only inside `execute`)"
    )
}

pub fn render(summary: &Summary) -> String {
    if summary.total == 0 {
        return EMPTY.to_string();
    }
    let lines: Vec<String> = summary
        .namespaces
        .iter()
        .flat_map(|n| {
            std::iter::once(namespace_line(n)).chain(n.entries.iter().map(|e| e.line.clone()))
        })
        .collect();
    format!(
        "{}\n\n{}",
        prompt(summary.shown < summary.total),
        lines.join("\n")
    )
}

fn described(summary: &Summary) -> Vec<&NamespaceSummary> {
    summary
        .namespaces
        .iter()
        .filter(|n| n.description.is_some())
        .collect()
}

fn entries(summary: &Summary) -> Vec<&Listing> {
    summary
        .namespaces
        .iter()
        .flat_map(|n| n.entries.iter())
        .collect()
}

fn all(summary: &Summary) -> Vec<&NamespaceSummary> {
    summary.namespaces.iter().collect()
}

struct Diff<'a, T> {
    added: Vec<&'a T>,
    removed: Vec<&'a T>,
    changed: Vec<&'a T>,
}

impl<T> Diff<'_, T> {
    fn any(&self) -> bool {
        !self.added.is_empty() || !self.removed.is_empty() || !self.changed.is_empty()
    }
}

fn diff_by_key<'a, T>(
    previous: &[&'a T],
    current: &[&'a T],
    key: impl Fn(&T) -> &str,
    changed: impl Fn(&T, &T) -> bool,
) -> Diff<'a, T> {
    let find = |list: &[&'a T], k: &str| list.iter().find(|v| key(v) == k).copied();
    Diff {
        added: current
            .iter()
            .filter(|v| find(previous, key(v)).is_none())
            .copied()
            .collect(),
        removed: previous
            .iter()
            .filter(|v| find(current, key(v)).is_none())
            .copied()
            .collect(),
        changed: current
            .iter()
            .filter(|v| find(previous, key(v)).is_some_and(|before| changed(before, v)))
            .copied()
            .collect(),
    }
}

/// The system text announcing a changed catalog: a delta when it is shorter than a replacement.
pub fn update(previous: &Summary, current: &Summary) -> String {
    let replacement = format!(
        "The Code Mode tool catalog has changed. This catalog supersedes the previous Code Mode tool catalog.\n\n{}",
        render(current)
    );
    if current.total == 0 {
        return replacement;
    }
    let previous_complete = previous.shown == previous.total;
    let current_complete = current.shown == current.total;
    if previous_complete != current_complete {
        return replacement;
    }
    let descriptions = diff_by_key(
        &described(previous),
        &described(current),
        |n| &n.name,
        |a, b| a.description != b.description,
    );
    if descriptions.any() {
        return replacement;
    }
    let diff = diff_by_key(
        &entries(previous),
        &entries(current),
        |e| &e.path,
        |a, b| a.line != b.line,
    );
    let mut parts = vec![CHANGED.to_string()];
    if !current_complete {
        if diff.any() {
            return replacement;
        }
        let namespaces = diff_by_key(
            &all(previous),
            &all(current),
            |n| &n.name,
            |a, b| a.count != b.count,
        );
        if !namespaces.any() {
            return replacement;
        }
        let list = |items: &[&NamespaceSummary], show: &dyn Fn(&NamespaceSummary) -> String| {
            items.iter().map(|n| show(n)).collect::<Vec<_>>().join(", ")
        };
        if !namespaces.added.is_empty() {
            parts.push(format!(
                "New tool namespaces are available: {}.",
                list(&namespaces.added, &|n| format!(
                    "`{}` ({} tools)",
                    n.name, n.count
                ))
            ));
        }
        if !namespaces.changed.is_empty() {
            parts.push(format!(
                "The following namespace inventories changed; search them again before relying on previous results: {}.",
                list(&namespaces.changed, &|n| format!("`{}` now has {} tools", n.name, n.count))
            ));
        }
        if !namespaces.removed.is_empty() {
            parts.push(format!(
                "The following tool namespaces are no longer available and must not be used: {}.",
                list(&namespaces.removed, &|n| format!("`{}`", n.name))
            ));
        }
    } else {
        if !diff.any() {
            return replacement;
        }
        let lines = |items: &[&Listing]| items.iter().map(|e| e.line.clone()).collect::<Vec<_>>();
        if !diff.added.is_empty() {
            let mut block =
                vec!["New tools are available in addition to those previously listed:".to_string()];
            block.extend(lines(&diff.added));
            parts.push(block.join("\n"));
        }
        if !diff.changed.is_empty() {
            let mut block =
                vec!["Changed tool listings supersede the previously listed ones:".to_string()];
            block.extend(lines(&diff.changed));
            parts.push(block.join("\n"));
        }
        if !diff.removed.is_empty() {
            parts.push(format!(
                "The following tools are no longer available and must not be called: {}.",
                diff.removed
                    .iter()
                    .map(|e| tool_expression(&e.path))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    let delta = parts.join("\n\n");
    if delta.encode_utf16().count() < replacement.encode_utf16().count() {
        delta
    } else {
        replacement
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, signature_len: usize, pinned: bool) -> Entry {
        Entry {
            path: path.into(),
            description: format!("Tool {path}.\nSecond line."),
            signature: format!("{}()", "x".repeat(signature_len)),
            pinned,
        }
    }

    #[test]
    fn budget_round_robin_shortest_first() {
        let entries = vec![
            entry("a.big", 400, false),
            entry("a.small", 10, false),
            entry("b.mid", 100, false),
            entry("b.pinned", 300, true),
        ];
        let summary = summarize_within(&entries, &[("a".into(), Some("A tools".into()))], 200);
        let shown: Vec<&str> = summary
            .namespaces
            .iter()
            .flat_map(|n| n.entries.iter().map(|e| e.path.as_str()))
            .collect();
        assert_eq!(shown, vec!["a.small", "b.mid", "b.pinned"]);
        assert_eq!(
            namespace_line(&summary.namespaces[0]),
            "- a (2 tools, 1 shown) // A tools"
        );
        assert_eq!(namespace_line(&summary.namespaces[1]), "- b (2 tools)");
        assert!(render(&summary).contains("The catalog is partial."));
        assert!(render(&summary).contains("\n\n- search({\n  query?: string,"));
    }

    #[test]
    fn descriptions_are_cut() {
        let mut long = entry("a.t", 1, false);
        long.description = "y".repeat(130);
        assert_eq!(
            listing(&long).line,
            format!("  - x() // {}...", "y".repeat(117))
        );
    }

    #[test]
    fn change_texts() {
        let one = summarize(&[entry("a.one", 5, false)], &[]);
        let two = summarize(&[entry("a.one", 5, false), entry("a.two", 5, false)], &[]);
        assert_eq!(
            update(&one, &two),
            "The Code Mode tool catalog has changed.\n\nNew tools are available in addition to those previously listed:\n  - xxxxx() // Tool a.two."
        );
        assert_eq!(
            update(&two, &one),
            "The Code Mode tool catalog has changed.\n\nThe following tools are no longer available and must not be called: tools.a.two."
        );
        assert!(
            update(&one, &one)
                .starts_with("The Code Mode tool catalog has changed. This catalog supersedes")
        );
        assert_eq!(render(&summarize(&[], &[])), EMPTY);
    }
}
