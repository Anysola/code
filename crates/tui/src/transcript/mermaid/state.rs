//! State diagrams (`stateDiagram`, `stateDiagram-v2`) drawn with the flowchart's layered layout
//! rather than 0.5.1 merman's orthogonal state router. The parser follows merman's
//! `state/parser.ts`: states, `state "Name" as Id`, `Id : description`, transitions with labels,
//! `[*]` as one start (`●`) and one end (`◉`) per scope, `<<choice>>` (`◇`), `<<fork>>` and
//! `<<join>>` (a bar). Composite states are flattened: their inner states join the chart, and a
//! transition to (from) a composite goes to its inner start (from its inner end) when it has one,
//! else the composite is a plain state. Notes, `--` separators, classes and styles are skipped.
//! `direction` is `TB` unless `LR`/`RL` asks for a left-to-right chain; a left-to-right chart that
//! is not a chain is drawn top-down. Diagrams that cannot be read or laid out (transitions that
//! loop back) are errors, and the block stays source.

use std::collections::HashMap;

use super::flowchart::{self, Chart, Direction, Node, Shape};
use super::{lines, strip_comment, unquote};

struct State {
    id: String,
    label: String,
    shape: Shape,
}

#[derive(Default)]
struct Diagram {
    direction: Option<Direction>,
    states: Vec<State>,
    index: HashMap<String, usize>,
    transitions: Vec<(String, String, Option<String>)>,
    composites: Vec<String>,
}

impl Diagram {
    /// The state `id`, made on first mention.
    fn state(&mut self, id: &str) -> &mut State {
        let at = match self.index.get(id) {
            Some(at) => *at,
            None => {
                self.states.push(State {
                    id: id.to_string(),
                    label: id.to_string(),
                    shape: Shape::Box,
                });
                self.index.insert(id.to_string(), self.states.len() - 1);
                self.states.len() - 1
            }
        };
        &mut self.states[at]
    }

    /// A transition end: `[*]` is the scope's start as a source and its end as a target.
    fn endpoint(&mut self, raw: &str, source: bool, scope: Option<&str>) -> Result<String, String> {
        let raw = raw.split(":::").next().unwrap_or_default().trim();
        if raw != "[*]" {
            let id = id(raw)?;
            self.state(&id);
            return Ok(id);
        }
        let (marker, glyph) = if source {
            ("__start", "●")
        } else {
            ("__end", "◉")
        };
        let id = scope.map_or(marker.to_string(), |s| format!("{s}.{marker}"));
        let state = self.state(&id);
        state.label = glyph.to_string();
        state.shape = Shape::Point;
        Ok(id)
    }
}

/// A state id: one word, without a `:::class` suffix.
fn id(text: &str) -> Result<String, String> {
    let id = text.split(":::").next().unwrap_or_default().trim();
    if id.is_empty() || id.contains(char::is_whitespace) || id.contains('"') {
        return Err(format!("cannot read state {text}"));
    }
    Ok(id.to_string())
}

/// A label on one line: quotes and entities resolved, `<br>` breaks joined by a space.
fn label(text: &str) -> String {
    lines(&unquote(text)).join(" ")
}

fn parse(source: &str) -> Result<Chart, String> {
    let mut rows = source.lines().map(strip_comment).filter(|l| !l.is_empty());
    let header = rows.next().ok_or("empty")?;
    if !matches!(
        header.to_ascii_lowercase().as_str(),
        "statediagram" | "statediagram-v2"
    ) {
        return Err("not a state diagram".into());
    }
    let mut diagram = Diagram::default();
    let mut scope: Vec<String> = Vec::new();
    let mut in_note = false;
    for line in rows {
        let lower = line.to_ascii_lowercase();
        if in_note {
            in_note = lower != "end note";
            continue;
        }
        let word = lower.split_whitespace().next().unwrap_or_default();
        if line == "}" {
            scope.pop().ok_or("unexpected }")?;
            continue;
        }
        if line.starts_with("--") && !line.contains("-->") {
            continue;
        }
        let parent = scope.last().cloned();
        match word {
            "direction" => {
                let value = lower.split_whitespace().nth(1).unwrap_or_default();
                let direction = match value {
                    "tb" | "td" | "bt" => Direction::Down,
                    "lr" | "rl" => Direction::Right,
                    other => return Err(format!("direction {other}")),
                };
                if scope.is_empty() {
                    diagram.direction = Some(direction);
                }
                continue;
            }
            "note" => {
                // `note left of X : text` and `note "text" as N` are one line, else up to `end note`.
                in_note = !line.contains(':') && !line[4..].trim_start().starts_with('"');
                continue;
            }
            "classdef" | "class" | "style" | "hide" | "scale" | "acctitle" | "accdescr"
            | "linkstyle" => continue,
            "state" => {
                state_line(&mut diagram, &mut scope, line[5..].trim())?;
                continue;
            }
            _ => {}
        }
        if let Some((from, rest)) = line.split_once("-->") {
            let (to, text) = match rest.split_once(':') {
                Some((to, text)) => (to, Some(label(text)).filter(|t| !t.is_empty())),
                None => (rest, None),
            };
            if to.contains("-->") {
                return Err(format!("cannot read transition {line}"));
            }
            let from = diagram.endpoint(from, true, parent.as_deref())?;
            let to = diagram.endpoint(to, false, parent.as_deref())?;
            diagram.transitions.push((from, to, text));
        } else if let Some((name, text)) = line.split_once(':').filter(|(_, t)| !t.contains(':')) {
            let text = label(text);
            let state = diagram.state(&id(name)?);
            if state.shape == Shape::Box {
                state.label = if state.label == state.id {
                    text
                } else {
                    format!("{} {text}", state.label)
                };
            }
        } else {
            diagram.state(&id(line)?);
        }
    }
    if in_note {
        return Err("unclosed note".into());
    }
    if !scope.is_empty() {
        return Err("unclosed composite state".into());
    }
    chart(diagram)
}

/// The rest of a `state …` line.
fn state_line(diagram: &mut Diagram, scope: &mut Vec<String>, rest: &str) -> Result<(), String> {
    let (rest, composite) = match rest.strip_suffix('{') {
        Some(rest) => (rest.trim(), true),
        None => (rest, false),
    };
    // `"Name" as Id` or `Id`, then an optional `<<kind>>`.
    let (name, rest) = match rest.strip_prefix('"') {
        Some(after) => {
            let (name, tail) = after.split_once('"').ok_or("unclosed quote")?;
            let tail = tail.trim_start();
            let tail = tail
                .strip_prefix("as ")
                .or_else(|| tail.strip_prefix("AS "))
                .ok_or_else(|| format!("cannot read state \"{name}\""))?;
            (Some(name), tail.trim())
        }
        None => (None, rest),
    };
    let (word, kind) = match rest.split_once("<<") {
        Some((word, kind)) => (word.trim(), Some(kind.trim_end_matches(">>").trim())),
        None => (rest.trim(), None),
    };
    let id = id(word)?;
    if composite {
        diagram.composites.push(id.clone());
        scope.push(id.clone());
    }
    let state = diagram.state(&id);
    if let Some(name) = name {
        state.label = label(&format!("\"{name}\""));
    }
    match kind.map(str::to_ascii_lowercase).as_deref() {
        None => {}
        Some("choice") => {
            state.label = "◇".into();
            state.shape = Shape::Point;
        }
        Some("fork" | "join") => {
            state.label = "━━━━━".into();
            state.shape = Shape::Point;
        }
        Some(other) => return Err(format!("state kind {other}")),
    }
    Ok(())
}

/// The flowchart model: composite ends resolved to their inner start/end, composites without a
/// remaining transition dropped.
fn chart(diagram: Diagram) -> Result<Chart, String> {
    let resolve = |id: &String, marker: &str| -> String {
        let inner = format!("{id}.{marker}");
        if diagram.composites.contains(id) && diagram.index.contains_key(&inner) {
            inner
        } else {
            id.clone()
        }
    };
    let transitions: Vec<(String, String, Option<String>)> = diagram
        .transitions
        .iter()
        .map(|(from, to, text)| (resolve(from, "__end"), resolve(to, "__start"), text.clone()))
        .collect();
    let mut chart = Chart {
        direction: diagram.direction.unwrap_or(Direction::Down),
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    let mut index: HashMap<&str, usize> = HashMap::new();
    for state in &diagram.states {
        let used = transitions
            .iter()
            .any(|(a, b, _)| *a == state.id || *b == state.id);
        if diagram.composites.contains(&state.id) && !used {
            continue;
        }
        index.insert(&state.id, chart.nodes.len());
        chart.nodes.push(Node {
            label: state.label.clone(),
            shape: state.shape,
        });
    }
    for (from, to, text) in transitions {
        chart
            .edges
            .push((index[from.as_str()], index[to.as_str()], text));
    }
    if chart.nodes.is_empty() {
        return Err("no states".into());
    }
    Ok(chart)
}

/// The drawing's rows, or why the diagram cannot be drawn (the block then stays source).
pub fn render(source: &str) -> Result<Vec<String>, String> {
    let mut chart = parse(source)?;
    match flowchart::draw(&chart) {
        Err(_) if chart.direction == Direction::Right => {
            chart.direction = Direction::Down;
            flowchart::draw(&chart)
        }
        drawn => drawn,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(source: &str) -> Vec<String> {
        render(source).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn chain_with_start_and_end() {
        assert_eq!(
            draw(
                "stateDiagram-v2\n  [*] --> Cart\n  Cart --> Payment: checkout\n  Payment --> [*]"
            ),
            vec![
                "     ●",
                "     │",
                "     │",
                "     ▼",
                " ┌──────┐",
                " │ Cart │",
                " └───┬──┘",
                "     │ checkout",
                "     │",
                "     ▼",
                "┌─────────┐",
                "│ Payment │",
                "└────┬────┘",
                "     │",
                "     │",
                "     ▼",
                "     ◉",
            ]
        );
    }

    #[test]
    fn left_to_right_chain() {
        assert_eq!(
            draw(
                "stateDiagram\n  %% request lifecycle\n  direction LR\n  [*] --> Idle\n  \
                 Idle --> Loading: submit\n  Loading --> [*]"
            ),
            vec![
                "    ┌──────┐         ┌─────────┐",
                "●──▶│ Idle │─submit─▶│ Loading │──▶◉",
                "    └──────┘         └─────────┘",
            ]
        );
    }

    #[test]
    fn left_to_right_branches_are_drawn_top_down() {
        let rows = draw("stateDiagram-v2\ndirection LR\nA --> B\nA --> C");
        assert_eq!(rows[1].trim(), "│ A │");
        assert_eq!(rows.len(), 9);
    }

    #[test]
    fn aliases_descriptions_and_entities() {
        let rows = draw(
            "stateDiagram-v2\n  state \"Ready &amp; waiting\" as Ready\n  Done : all<br/>finished\n  \
             Ready --> Done: elapsed &lt;3s",
        );
        assert!(
            rows.contains(&"│ Ready & waiting │".to_string()),
            "{rows:?}"
        );
        assert!(rows.iter().any(|r| r.contains("elapsed <3s")), "{rows:?}");
        assert!(
            rows.iter().any(|r| r.contains("│ all finished │")),
            "{rows:?}"
        );
    }

    #[test]
    fn choice_branches_and_shared_end() {
        assert_eq!(
            draw(
                "stateDiagram-v2\n  [*] --> Check\n  state Check <<choice>>\n  \
                 Check --> Accepted: yes\n  Check --> Rejected: no\n  Accepted --> [*]\n  \
                 Rejected --> [*]"
            ),
            vec![
                "            ●",
                "            │",
                "            │",
                "            ▼",
                "            ◇",
                "            │",
                "            │",
                "            │",
                "      ┌─────┴───────┐",
                "      ▼ yes         ▼ no",
                "┌──────────┐  ┌──────────┐",
                "│ Accepted │  │ Rejected │",
                "└─────┬────┘  └─────┬────┘",
                "      │             │",
                "      └─────┬───────┘",
                "            ▼",
                "            ◉",
            ]
        );
    }

    #[test]
    fn labels_beside_narrow_neighbours_stay_whole() {
        let rows = draw(
            "stateDiagram-v2\n[*] --> C\nstate C <<choice>>\nC --> A: timeout\nC --> B: retry",
        );
        assert!(rows.iter().any(|r| r.contains("▼ timeout")), "{rows:?}");
        assert!(rows.iter().any(|r| r.contains("▼ retry")), "{rows:?}");
    }

    #[test]
    fn composites_flatten_and_notes_are_skipped() {
        let rows = draw(
            "stateDiagram-v2\n  [*] --> Auth\n  state Auth {\n    [*] --> Idle\n    \
             Idle --> Editing: open\n    --\n    Editing --> [*]\n  }\n  \
             note right of Editing\n    Draft changes\n  end note\n  note left of Idle : waits\n  \
             Auth --> Closed\n  classDef hot fill:#f00\n  class Closed hot\n  Closed:::hot --> [*]",
        );
        let text = rows.join("\n");
        assert!(!text.contains("Auth"), "{text}");
        assert!(!text.contains("Draft"), "{text}");
        for name in ["Idle", "Editing", "Closed"] {
            assert!(text.contains(&format!("│ {name} │")), "{text}");
        }
        // Outer start and end, plus the composite's own start and end.
        assert_eq!(text.matches('●').count(), 2, "{text}");
        assert_eq!(text.matches('◉').count(), 2, "{text}");
    }

    #[test]
    fn fork_and_join_are_bars() {
        let rows = draw(
            "stateDiagram-v2\nstate f <<fork>>\nstate j <<join>>\n[*] --> f\nf --> A\nf --> B\n\
             A --> j\nB --> j\nj --> [*]",
        );
        assert_eq!(rows.iter().filter(|r| r.contains("━━━━━")).count(), 2);
    }

    #[test]
    fn unreadable_and_looping_diagrams_fall_back() {
        assert!(render("stateDiagram-v2\n  A --> B --> C").is_err());
        assert!(render("stateDiagram-v2\n  state A <<history>>").is_err());
        assert!(render("stateDiagram-v2\n  state A {\n  A1 --> A2").is_err());
        assert!(render("stateDiagram-v2\n  note left of A\n  text").is_err());
        assert!(render("stateDiagram-v2\n  Idle --> Busy\n  Busy --> Idle").is_err());
        assert!(render("stateDiagram-v2").is_err());
    }
}
