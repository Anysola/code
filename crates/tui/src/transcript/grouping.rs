//! Transcript groups (`routes/session/grouping/session.ts`, `group-view.tsx`). Exploration
//! (`session.grouping = auto`): adjacent read/glob/grep/webfetch/websearch calls, across the
//! steps of a turn, collapse into one `Explored: 2 reads, 1 search` row. Activity (verbosity
//! `low`): every run of thoughts and tools becomes one `+ 3 commands, 1 edit` row. A click on
//! the row opens or closes it; calls waiting for permission stay visible.

use std::collections::{HashMap, HashSet};

use super::model::{Item, Part, Status, Tool};

const EXPLORATION: [&str; 5] = ["read", "glob", "grep", "webfetch", "websearch"];

/// Where a part sits: `(item index, part index)`.
pub type At = (usize, usize);

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Exploration {
        done: bool,
    },
    /// `active`: something in the run is still going.
    Activity {
        active: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    /// The first part's ID; open state is kept under it.
    pub id: String,
    pub label: String,
    pub kind: Kind,
    pub open: bool,
}

#[derive(Debug, Default)]
pub struct Groups {
    /// Header rows before the part at this position, outermost first.
    pub headers: HashMap<At, Vec<Header>>,
    pub hidden: HashSet<At>,
}

impl Groups {
    /// What decides an item's rows beyond the item itself, for the layout cache.
    pub fn signature(&self, item: usize) -> String {
        let mut keys: Vec<String> = self
            .headers
            .iter()
            .filter(|(at, _)| at.0 == item)
            .map(|(at, h)| format!("{}:{h:?}", at.1))
            .chain(
                self.hidden
                    .iter()
                    .filter(|at| at.0 == item)
                    .map(|at| format!("-{}", at.1)),
            )
            .collect();
        keys.sort();
        keys.join(",")
    }

    /// Activity groups (verbosity `low`), then exploration groups among what they leave shown.
    pub fn new(
        items: &[Item],
        grouping: bool,
        low: bool,
        high: bool,
        toggled: &HashSet<String>,
    ) -> Groups {
        let mut groups = Groups::default();
        if low {
            for run in runs(items, |part| match part {
                Part::Tool(tool) => !tool.name.eq_ignore_ascii_case("question"),
                Part::Reasoning { .. } => true,
                Part::Text { .. } => false,
            }) {
                groups.add(&run, toggled, false, activity);
            }
        }
        if grouping {
            for run in runs(items, |part| matches!(part, Part::Tool(t) if exploring(t))) {
                let shown: Vec<_> = run
                    .into_iter()
                    .filter(|(at, _)| !groups.hidden.contains(at))
                    .collect();
                // A run split by a collapsed activity row is never on screen as a whole.
                if !shown.is_empty() {
                    groups.add(&shown, toggled, high, exploration);
                }
            }
        }
        groups
    }

    fn add(
        &mut self,
        run: &[(At, &Part)],
        toggled: &HashSet<String>,
        open_by_default: bool,
        header: fn(&[&Part]) -> Option<(String, Kind)>,
    ) {
        let parts: Vec<&Part> = run.iter().map(|(_, p)| *p).collect();
        let Some((label, kind)) = header(&parts) else {
            return;
        };
        let id = match parts[0] {
            Part::Tool(tool) => tool.call_id.clone(),
            _ => format!("{}:{}", run[0].0.0, run[0].0.1),
        };
        let open = open_by_default != toggled.contains(&id);
        if !open {
            self.hidden.extend(
                run.iter()
                    .filter(|(_, p)| !matches!(p, Part::Tool(t) if t.awaiting))
                    .map(|(at, _)| *at),
            );
        }
        self.headers.entry(run[0].0).or_default().push(Header {
            id,
            label,
            kind,
            open,
        });
    }
}

fn exploring(tool: &Tool) -> bool {
    EXPLORATION.contains(&tool.name.to_ascii_lowercase().as_str())
}

fn running(tool: &Tool) -> bool {
    matches!(tool.status, Status::Streaming | Status::Running)
}

/// Maximal runs of parts `member` accepts; blank text does not break a run, anything else does.
fn runs(items: &[Item], member: impl Fn(&Part) -> bool) -> Vec<Vec<(At, &Part)>> {
    let mut runs = Vec::new();
    let mut run = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let Item::Assistant(assistant) = item else {
            runs.push(std::mem::take(&mut run));
            continue;
        };
        for (p, part) in assistant.parts.iter().enumerate() {
            if member(part) {
                run.push(((i, p), part));
            } else if !matches!(part, Part::Text { text, .. } if text.trim().is_empty()) {
                runs.push(std::mem::take(&mut run));
            }
        }
    }
    runs.push(run);
    runs.retain(|r| !r.is_empty());
    runs
}

/// Count phrases in first-seen order: `2 reads, 1 search`.
fn phrase(counts: &[(&str, usize)], plural: fn(&str) -> String) -> String {
    counts
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(name, n)| {
            let word = if *n == 1 {
                name.to_string()
            } else {
                plural(name)
            };
            format!("{n} {word}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn bump<'a>(counts: &mut Vec<(&'a str, usize)>, name: &'a str) {
    match counts.iter_mut().find(|(n, _)| *n == name) {
        Some(entry) => entry.1 += 1,
        None => counts.push((name, 1)),
    }
}

/// `Explored: 2 reads, 1 search` (`Exploring:` while a call runs).
fn exploration(parts: &[&Part]) -> Option<(String, Kind)> {
    let tools: Vec<&Tool> = parts
        .iter()
        .filter_map(|p| match p {
            Part::Tool(t) => Some(t),
            _ => None,
        })
        .collect();
    let mut counts = Vec::new();
    for tool in &tools {
        bump(
            &mut counts,
            match tool.name.to_ascii_lowercase().as_str() {
                "grep" | "glob" | "websearch" => "search",
                "webfetch" => "fetch",
                _ => "read",
            },
        );
    }
    let done = !tools.iter().any(|t| running(t));
    let verb = if done { "Explored" } else { "Exploring" };
    let names = phrase(&counts, |name| match name {
        "search" | "fetch" => format!("{name}es"),
        _ => format!("{name}s"),
    });
    Some((format!("{verb}: {names}"), Kind::Exploration { done }))
}

/// `3 commands, 1 edit, 2 thoughts, 4 reads, 1 tool` over finished work; until something
/// finishes, the first running part's status (`Thinking…`, `Running command…`).
fn activity(parts: &[&Part]) -> Option<(String, Kind)> {
    let mut counts = vec![
        ("command", 0),
        ("edit", 0),
        ("thought", 0),
        ("read", 0),
        ("tool", 0),
    ];
    let mut busy = None;
    for part in parts {
        let name = match part {
            Part::Reasoning { end: None, .. } => {
                busy.get_or_insert_with(|| "Thinking…".to_string());
                continue;
            }
            Part::Reasoning { text, .. } if text.replace("[REDACTED]", "").trim().is_empty() => {
                continue;
            }
            Part::Reasoning { .. } => "thought",
            Part::Tool(tool) if tool.awaiting => continue,
            Part::Tool(tool) if running(tool) => {
                let canonical = match tool.name.as_str() {
                    "bash" | "shell" => "command",
                    "execute" => "code",
                    "task" => "subagent",
                    "apply_patch" => "patch",
                    other => other,
                };
                let verb = if tool.status == Status::Streaming {
                    "Preparing"
                } else {
                    "Running"
                };
                busy.get_or_insert_with(|| format!("{verb} {canonical}…"));
                continue;
            }
            Part::Tool(tool) => match tool.name.as_str() {
                "bash" | "shell" => "command",
                "read" => "read",
                "edit" | "write" | "patch" | "apply_patch" => "edit",
                _ => "tool",
            },
            Part::Text { .. } => continue,
        };
        bump(&mut counts, name);
    }
    let label = phrase(&counts, |name| format!("{name}s"));
    let active = busy.is_some();
    let label = if label.is_empty() { busy? } else { label };
    Some((label, Kind::Activity { active }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::model::Assistant;

    fn tool(id: &str, name: &str, status: Status) -> Part {
        Part::Tool(Tool {
            call_id: id.into(),
            name: name.into(),
            input: serde_json::Value::Null,
            status,
            output: String::new(),
            metadata: Default::default(),
            error: None,
            started: 0,
            awaiting: false,
        })
    }

    fn step(parts: Vec<Part>) -> Item {
        Item::Assistant(Assistant {
            parts,
            ..Assistant::default()
        })
    }

    fn items() -> Vec<Item> {
        vec![
            step(vec![tool("a", "read", Status::Completed)]),
            step(vec![
                tool("b", "grep", Status::Completed),
                tool("c", "read", Status::Completed),
                tool("d", "webfetch", Status::Running),
            ]),
            step(vec![tool("e", "shell", Status::Completed)]),
            step(vec![tool("f", "glob", Status::Completed)]),
        ]
    }

    #[test]
    fn exploration_runs_cross_steps_and_count_like_0_5_1() {
        let none = HashSet::new();
        let groups = Groups::new(&items(), true, false, false, &none);
        assert_eq!(
            groups.headers[&(0, 0)][0].label,
            "Exploring: 2 reads, 1 search, 1 fetch"
        );
        assert_eq!(groups.headers[&(3, 0)][0].label, "Explored: 1 search");
        assert!(groups.hidden.contains(&(1, 2)) && !groups.hidden.contains(&(2, 0)));
        let toggled = HashSet::from(["a".to_string()]);
        let groups = Groups::new(&items(), true, false, false, &toggled);
        assert!(groups.headers[&(0, 0)][0].open && !groups.hidden.contains(&(0, 0)));
        assert!(
            !Groups::new(&items(), true, false, true, &none)
                .hidden
                .contains(&(3, 0))
        );
        assert!(
            Groups::new(&items(), false, false, false, &none)
                .headers
                .is_empty()
        );
    }

    #[test]
    fn low_verbosity_summarises_each_run() {
        let mut items = items();
        items.push(Item::Notice("break".into()));
        items.push(step(vec![
            Part::Reasoning {
                text: "plan".into(),
                start: 0,
                end: None,
            },
            tool("g", "shell", Status::Running),
        ]));
        let groups = Groups::new(&items, true, true, false, &HashSet::new());
        let first = &groups.headers[&(0, 0)];
        assert_eq!(
            first.len(),
            1,
            "exploration rows stay inside the closed activity row"
        );
        assert_eq!(first[0].label, "1 command, 2 reads, 2 tools");
        assert_eq!(first[0].kind, Kind::Activity { active: true });
        assert_eq!(groups.headers[&(5, 0)][0].label, "Thinking…");
    }
}
