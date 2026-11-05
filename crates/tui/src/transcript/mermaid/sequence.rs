//! `sequenceDiagram` in merman's compact terminal layout: participant names, lifelines, and
//! one arrow row per message with its label inline. Notes, activations, blocks and
//! self-messages are errors, and the block stays source.

use super::{Grid, strip_comment, unquote};
use crate::text::width;

struct Message {
    from: usize,
    to: usize,
    label: String,
}

struct Sequence {
    /// `(id, shown name)` in order of appearance.
    participants: Vec<(String, String)>,
    messages: Vec<Message>,
}

/// Arrows from longest to shortest so `-->>` is not read as `->>`.
const ARROWS: [&str; 4] = ["-->>", "->>", "-->", "->"];

impl Sequence {
    fn index(&mut self, id: &str) -> usize {
        match self.participants.iter().position(|(p, _)| p == id) {
            Some(at) => at,
            None => {
                self.participants.push((id.to_string(), id.to_string()));
                self.participants.len() - 1
            }
        }
    }
}

fn parse(source: &str) -> Result<Sequence, String> {
    let mut sequence = Sequence {
        participants: Vec::new(),
        messages: Vec::new(),
    };
    let mut header = false;
    for raw in source.lines() {
        let line = strip_comment(raw);
        if line.is_empty() {
            continue;
        }
        if !header {
            if !line.eq_ignore_ascii_case("sequenceDiagram") {
                return Err("sequenceDiagram header is required".into());
            }
            header = true;
            continue;
        }
        let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        if word == "participant" || word == "actor" {
            let (id, name) = match rest.split_once(" as ") {
                Some((id, name)) => (id.trim(), unquote(name)),
                None => (rest.trim(), unquote(rest)),
            };
            let at = sequence.index(id);
            sequence.participants[at].1 = name;
            continue;
        }
        let Some((arrow_at, arrow)) = ARROWS
            .iter()
            .filter_map(|a| line.find(a).map(|at| (at, *a)))
            .min_by_key(|(at, a)| (*at, std::cmp::Reverse(a.len())))
        else {
            return Err(format!("unsupported sequence line: {line}"));
        };
        let from = line[..arrow_at].trim();
        let (to, label) = line[arrow_at + arrow.len()..]
            .split_once(':')
            .ok_or("sequence messages need a label")?;
        let to = to.trim().trim_start_matches(['+', '-']);
        if from.is_empty() || to.is_empty() || from == to {
            return Err("unsupported sequence message".into());
        }
        let (from, to) = (sequence.index(from), sequence.index(to));
        sequence.messages.push(Message {
            from,
            to,
            label: unquote(label),
        });
    }
    if sequence.participants.is_empty() {
        return Err("sequence diagram is empty".into());
    }
    Ok(sequence)
}

/// Columns between two lifelines an inline label needs: head and tee, a dash and a space on
/// each side.
fn needed(label: &str) -> usize {
    width(label) + 6
}

pub fn render(source: &str) -> Result<Vec<String>, String> {
    let sequence = parse(source)?;
    let names: Vec<&str> = sequence
        .participants
        .iter()
        .map(|(_, n)| n.as_str())
        .collect();
    let count = names.len();
    // Lifeline columns: names three apart, then widened so every label fits inline.
    let mut centers = vec![width(names[0]) / 2; count];
    for i in 1..count {
        let previous_end = centers[i - 1] - width(names[i - 1]) / 2 + width(names[i - 1]);
        centers[i] = previous_end + 3 + width(names[i]) / 2;
    }
    let mut ordered: Vec<&Message> = sequence.messages.iter().collect();
    ordered.sort_by_key(|m| m.from.abs_diff(m.to));
    for message in ordered {
        let (lo, hi) = (message.from.min(message.to), message.from.max(message.to));
        let need = needed(&message.label);
        let have = centers[hi] - centers[lo];
        if have < need {
            for center in &mut centers[hi..] {
                *center += need - have;
            }
        }
    }
    let total = (0..count)
        .map(|i| centers[i] - width(names[i]) / 2 + width(names[i]))
        .max()
        .unwrap_or(0)
        .max(centers[count - 1] + 1);
    let height = 2 + sequence.messages.len() * 2;
    let mut grid = Grid::new(total, height);
    for (i, name) in names.iter().enumerate() {
        grid.text(centers[i] - width(name) / 2, 0, name);
    }
    for y in 1..height {
        for center in &centers {
            grid.set(*center, y, '│');
        }
    }
    for (n, message) in sequence.messages.iter().enumerate() {
        let y = 2 + n * 2;
        let leftward = message.to < message.from;
        let (lo, hi) = (
            centers[message.from.min(message.to)],
            centers[message.from.max(message.to)],
        );
        for x in lo + 1..hi {
            grid.set(x, y, '─');
        }
        let (tail, head) = if leftward { (hi, lo) } else { (lo, hi) };
        grid.set(tail, y, if leftward { '┤' } else { '├' });
        grid.set(head, y, if leftward { '◄' } else { '►' });
        let label = &message.label;
        let room = hi - lo - 1;
        let rest = room.saturating_sub(width(label) + 2);
        let left = rest.div_ceil(2).saturating_sub(1).max(1);
        grid.text(lo + 1 + left, y, &format!(" {label} "));
    }
    Ok(grid.rows())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// merman's compact expectation (`sequence/diagram.test.ts`).
    #[test]
    fn compact_layout_like_merman() {
        let rows = render(
            "sequenceDiagram\n  participant Leaf as leaf tool\n  participant Location as LocationMutation\n  participant File as FileMutation\n  Leaf->>Location: resolve(path)\n  Location-->>Leaf: Plan(target, authority anchor)\n  Leaf->>File: commit(plan)\n  File->>Location: revalidate(plan)\n  Location-->>File: same target or reject",
        )
        .unwrap();
        assert_eq!(
            rows,
            [
                "leaf tool                       LocationMutation             FileMutation",
                "    │                                   │                          │",
                "    ├───────── resolve(path) ───────────►                          │",
                "    │                                   │                          │",
                "    ◄─ Plan(target, authority anchor) ──┤                          │",
                "    │                                   │                          │",
                "    ├─────────────────────── commit(plan) ─────────────────────────►",
                "    │                                   │                          │",
                "    │                                   ◄─── revalidate(plan) ─────┤",
                "    │                                   │                          │",
                "    │                                   ├─ same target or reject ──►",
                "    │                                   │                          │",
            ]
        );
    }

    #[test]
    fn unsupported_lines_fall_back() {
        assert!(render("sequenceDiagram\nA->>A: self").is_err());
        assert!(render("sequenceDiagram\nNote over A: hi").is_err());
    }
}
