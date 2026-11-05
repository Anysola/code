//! `gitGraph` as a vertical commit graph (merman's terminal layout): one lane per branch two
//! columns apart, a transition row where a branch forks, `●`/`◎`/`◆`/`⊗` per commit and its
//! label with `[branch]`/`[tag]` decorations to the right.

use std::collections::{BTreeMap, HashMap};

use super::{Grid, unquote};
use crate::text::width;

const LANE_WIDTH: usize = 2;
const LABEL_GAP: usize = 2;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Normal,
    Reverse,
    Highlight,
}

struct Commit {
    id: String,
    message: Option<String>,
    tags: Vec<String>,
    kind: Kind,
    branch: String,
    parents: Vec<String>,
}

struct Branch {
    name: String,
    order: Option<u64>,
    head: Option<String>,
}

fn is_header(line: &str) -> bool {
    let line = line.trim_end_matches(':').trim_end();
    let mut words = line.split_whitespace();
    words
        .next()
        .is_some_and(|w| w.eq_ignore_ascii_case("gitgraph"))
        && words
            .next()
            .is_none_or(|d| ["LR", "TB", "BT"].iter().any(|x| x.eq_ignore_ascii_case(d)))
        && words.next().is_none()
}

/// The line without a `%%` comment outside quotes, trimmed.
fn strip_comment(value: &str) -> &str {
    let mut quote = None;
    for (i, c) in value.char_indices() {
        if c == '"' || c == '\'' {
            if quote == Some(c) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(c);
            }
        } else if quote.is_none() && value[i..].starts_with("%%") {
            return value[..i].trim();
        }
    }
    value.trim()
}

/// Whitespace-separated tokens that keep quoted runs together; an open quote makes the whole
/// line one token.
fn tokenize(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    for c in line.chars() {
        if (c == '"' || c == '\'') && quote.is_none_or(|q| q == c) {
            quote = if quote == Some(c) { None } else { Some(c) };
            token.push(c);
        } else if c.is_whitespace() && quote.is_none() {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
        } else {
            token.push(c);
        }
    }
    if quote.is_some() {
        return vec![line.to_string()];
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

fn attributes(tokens: &[String], allowed: &[&str]) -> Result<HashMap<String, Vec<String>>, String> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        let (key, inline) = token.split_once(':').unwrap_or((token, ""));
        let key = key.to_ascii_lowercase();
        if !allowed.contains(&key.as_str()) {
            return Err(format!("Unsupported GitGraph attribute \"{key}\""));
        }
        let value = if inline.is_empty() {
            index += 1;
            tokens
                .get(index)
                .ok_or_else(|| format!("GitGraph attribute \"{key}\" requires a value"))?
                .as_str()
        } else {
            inline
        };
        out.entry(key).or_default().push(unquote(value));
        index += 1;
    }
    Ok(out)
}

fn single(attributes: &HashMap<String, Vec<String>>, key: &str) -> Result<Option<String>, String> {
    match attributes.get(key).map(Vec::as_slice) {
        Some([one]) => Ok(Some(one.clone())),
        Some([]) | None => Ok(None),
        Some(_) => Err(format!("GitGraph attribute \"{key}\" cannot repeat")),
    }
}

fn kind(value: Option<String>) -> Result<Kind, String> {
    match value.as_deref().map(str::to_ascii_uppercase).as_deref() {
        None | Some("NORMAL") => Ok(Kind::Normal),
        Some("REVERSE") => Ok(Kind::Reverse),
        Some("HIGHLIGHT") => Ok(Kind::Highlight),
        Some(_) => Err(format!(
            "Unknown GitGraph commit type \"{}\"",
            value.unwrap_or_default()
        )),
    }
}

#[derive(Default)]
struct Parser {
    branches: Vec<Branch>,
    commits: Vec<Commit>,
    heads: BTreeMap<String, Option<String>>,
    current: String,
    generated: usize,
}

impl Parser {
    fn new_id(&mut self, given: Option<String>) -> Result<String, String> {
        let id = given.unwrap_or_else(|| {
            self.generated += 1;
            format!("commit-{}", self.generated)
        });
        if id.is_empty() {
            return Err("GitGraph commit id cannot be empty".into());
        }
        if self.commits.iter().any(|c| c.id == id) {
            return Err(format!("Duplicate commit id \"{id}\""));
        }
        Ok(id)
    }

    fn push(&mut self, commit: Commit) {
        self.heads
            .insert(self.current.clone(), Some(commit.id.clone()));
        self.commits.push(commit);
    }

    fn known(&self, name: &str) -> Result<(), String> {
        if self.heads.contains_key(name) {
            Ok(())
        } else {
            Err(format!("Unknown branch \"{name}\""))
        }
    }

    fn command(&mut self, op: &str, mut rest: Vec<String>) -> Result<(), String> {
        match op {
            "commit" => {
                let quoted = rest.first().is_some_and(|t| {
                    t.len() >= 2
                        && ((t.starts_with('"') && t.ends_with('"'))
                            || (t.starts_with('\'') && t.ends_with('\'')))
                });
                let shorthand = quoted.then(|| unquote(&rest.remove(0)));
                let attrs = attributes(&rest, &["id", "msg", "tag", "type"])?;
                let id = self.new_id(single(&attrs, "id")?)?;
                let kind = kind(single(&attrs, "type")?)?;
                let parent = self.heads.get(&self.current).cloned().flatten();
                self.push(Commit {
                    id,
                    message: single(&attrs, "msg")?.or(shorthand),
                    tags: attrs.get("tag").cloned().unwrap_or_default(),
                    kind,
                    branch: self.current.clone(),
                    parents: parent.into_iter().collect(),
                });
            }
            "branch" => {
                let name = rest.first().map(|n| unquote(n)).unwrap_or_default();
                if name.is_empty() {
                    return Err("GitGraph branch name cannot be empty".into());
                }
                if self.heads.contains_key(&name) {
                    return Err(format!("Duplicate branch \"{name}\""));
                }
                let attrs = attributes(&rest[1..], &["order"])?;
                let order = match single(&attrs, "order")? {
                    None => None,
                    Some(value) => Some(value.parse::<u64>().map_err(|_| {
                        "GitGraph branch order must be a non-negative integer".to_string()
                    })?),
                };
                self.branches.push(Branch {
                    name: name.clone(),
                    order,
                    head: None,
                });
                let head = self.heads.get(&self.current).cloned().flatten();
                self.heads.insert(name.clone(), head);
                self.current = name;
            }
            "checkout" | "switch" => {
                if rest.len() != 1 {
                    return Err(format!("GitGraph {op} requires one branch"));
                }
                let name = unquote(&rest[0]);
                self.known(&name)?;
                self.current = name;
            }
            "merge" => {
                let branch = rest
                    .first()
                    .map(|n| unquote(n))
                    .ok_or("GitGraph merge requires a branch")?;
                self.known(&branch)?;
                if branch == self.current {
                    return Err("GitGraph cannot merge a branch into itself".into());
                }
                let current = self.heads[&self.current]
                    .clone()
                    .ok_or_else(|| format!("Branch \"{}\" has no commits", self.current))?;
                let merged = self.heads[&branch]
                    .clone()
                    .ok_or_else(|| format!("Branch \"{branch}\" has no commits"))?;
                if current == merged {
                    return Err(format!("Branches already share head \"{merged}\""));
                }
                let attrs = attributes(&rest[1..], &["id", "tag", "type"])?;
                let id = self.new_id(single(&attrs, "id")?)?;
                let kind = kind(single(&attrs, "type")?)?;
                self.push(Commit {
                    id,
                    message: None,
                    tags: attrs.get("tag").cloned().unwrap_or_default(),
                    kind,
                    branch: self.current.clone(),
                    parents: vec![current, merged],
                });
            }
            "cherry-pick" => return Err("Cherry-pick is not supported".into()),
            _ => return Err("Unsupported GitGraph statement".into()),
        }
        Ok(())
    }
}

fn parse(source: &str) -> Result<(Vec<Branch>, Vec<Commit>), String> {
    let mut parser = Parser {
        branches: vec![Branch {
            name: "main".into(),
            order: Some(0),
            head: None,
        }],
        current: "main".into(),
        ..Parser::default()
    };
    parser.heads.insert("main".into(), None);
    let mut header = false;
    let mut description = false;
    for raw in source.lines() {
        let line = strip_comment(raw);
        if description {
            description = line != "}";
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("accdescr") && line.ends_with('{') {
            description = true;
            continue;
        }
        let first = lower.split([' ', ':']).next().unwrap_or_default();
        if line.is_empty() || ["acctitle", "accdescr", "title"].contains(&first) {
            continue;
        }
        if is_header(line) {
            if header {
                return Err("GitGraph header can only appear once".into());
            }
            header = true;
            continue;
        }
        if !header {
            return Err("GitGraph header is required".into());
        }
        let mut tokens = tokenize(line);
        let op = tokens.remove(0).to_ascii_lowercase();
        parser.command(&op, tokens)?;
    }
    if !header {
        return Err("GitGraph header is required".into());
    }
    let Parser {
        mut branches,
        commits,
        heads,
        ..
    } = parser;
    for branch in &mut branches {
        branch.head = heads.get(&branch.name).cloned().flatten();
    }
    // Unordered branches come before ordered ones; `main` stays first.
    let main = branches.remove(0);
    let (mut ordered, unordered): (Vec<_>, Vec<_>) =
        branches.into_iter().partition(|b| b.order.is_some());
    ordered.sort_by_key(|b| b.order);
    let branches = std::iter::once(main)
        .chain(unordered)
        .chain(ordered)
        .collect();
    Ok((branches, commits))
}

#[derive(Default, Clone, Copy)]
struct Joins {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
}

fn glyph(j: Joins) -> char {
    match (j.up, j.down, j.left, j.right) {
        (true, true, false, false) | (true, false, false, false) | (false, true, false, false) => {
            '│'
        }
        (false, false, _, _) => '─',
        (false, true, false, true) => '╭',
        (false, true, true, false) => '╮',
        (true, false, false, true) => '╰',
        (true, false, true, false) => '╯',
        (true, true, false, true) => '├',
        (true, true, true, false) => '┤',
        (false, true, true, true) => '┬',
        (true, false, true, true) => '┴',
        (true, true, true, true) => '┼',
    }
}

type Row = BTreeMap<usize, Joins>;

fn join(row: &mut Row, x: usize, add: Joins) {
    let cell = row.entry(x).or_default();
    cell.up |= add.up;
    cell.down |= add.down;
    cell.left |= add.left;
    cell.right |= add.right;
}

/// A horizontal run from one lane to another with the given vertical stubs at each end.
fn across(
    row: &mut Row,
    from: usize,
    to: usize,
    from_vertical: (bool, bool),
    to_vertical: (bool, bool),
) {
    if from == to {
        return;
    }
    let (source, target) = (from * LANE_WIDTH, to * LANE_WIDTH);
    let rightward = target > source;
    let stub = |(up, down): (bool, bool), toward_right: bool| Joins {
        up,
        down,
        left: !toward_right,
        right: toward_right,
    };
    join(row, source, stub(from_vertical, rightward));
    let (low, high) = (source.min(target), source.max(target));
    for x in low + 1..high {
        join(
            row,
            x,
            Joins {
                left: true,
                right: true,
                ..Joins::default()
            },
        );
    }
    join(row, target, stub(to_vertical, !rightward));
}

pub fn render(source: &str) -> Result<Vec<String>, String> {
    let (branches, commits) = parse(source)?;
    if commits.is_empty() {
        return Ok(Vec::new());
    }
    let lane: HashMap<&str, usize> = branches
        .iter()
        .enumerate()
        .map(|(i, b)| (b.name.as_str(), i))
        .collect();
    let index: HashMap<&str, usize> = commits
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id.as_str(), i))
        .collect();
    let mut spans: HashMap<&str, (usize, usize)> = HashMap::new();
    for (i, commit) in commits.iter().enumerate() {
        spans.entry(&commit.branch).or_insert((i, i)).1 = i;
        for parent in &commit.parents {
            let parent = &commits[index[parent.as_str()]];
            if parent.branch != commit.branch
                && let Some(span) = spans.get_mut(parent.branch.as_str())
            {
                span.1 = span.1.max(i);
            }
        }
    }
    let label = |commit: &Commit| {
        let subject = commit.message.clone().unwrap_or_else(|| commit.id.clone());
        let decorations: Vec<String> = branches
            .iter()
            .filter(|b| b.head.as_deref() == Some(&commit.id))
            .map(|b| &b.name)
            .chain(&commit.tags)
            .map(|v| format!("[{v}]"))
            .collect();
        if decorations.is_empty() {
            subject
        } else {
            format!("{subject}  {}", decorations.join(" "))
        }
    };
    let parent_lane = |commit: &Commit, n: usize| {
        commit
            .parents
            .get(n)
            .map(|p| lane[commits[index[p.as_str()]].branch.as_str()])
    };
    let forks: Vec<bool> = commits
        .iter()
        .map(|c| parent_lane(c, 0).is_some_and(|l| l != lane[c.branch.as_str()]))
        .collect();
    let graph = (branches.len() - 1) * LANE_WIDTH + 1;
    let labels: Vec<String> = commits.iter().map(label).collect();
    let label_width = labels.iter().map(|l| width(l)).max().unwrap_or(0);
    let height = commits.len() + forks.iter().filter(|f| **f).count();
    let mut grid = Grid::new(graph + LABEL_GAP + label_width, height);
    let paint = |grid: &mut Grid, row: Row, y: usize| {
        for (x, joins) in row {
            grid.set(x, y, glyph(joins));
        }
    };
    let vertical = |up, down| Joins {
        up,
        down,
        ..Joins::default()
    };
    let mut y = 0;
    for (i, commit) in commits.iter().enumerate() {
        let own = lane[commit.branch.as_str()];
        if forks[i] {
            let mut row = Row::new();
            for (name, &(first, last)) in &spans {
                if first < i && last >= i {
                    join(&mut row, lane[name] * LANE_WIDTH, vertical(true, true));
                }
            }
            if let Some(from) = parent_lane(commit, 0) {
                across(&mut row, from, own, (true, true), (false, true));
            }
            paint(&mut grid, row, y);
            y += 1;
        }
        let mut row = Row::new();
        for branch in &branches {
            let Some(&(first, last)) = spans.get(branch.name.as_str()) else {
                continue;
            };
            if first > i || (last <= i && branch.name != commit.branch) {
                continue;
            }
            join(
                &mut row,
                lane[branch.name.as_str()] * LANE_WIDTH,
                vertical(i > 0, last > i),
            );
        }
        if let Some(merged) = parent_lane(commit, 1) {
            across(&mut row, own, merged, (true, false), (true, false));
        }
        paint(&mut grid, row, y);
        let mark = match commit.kind {
            Kind::Reverse => '⊗',
            Kind::Highlight => '◆',
            Kind::Normal if commit.parents.len() > 1 => '◎',
            Kind::Normal => '●',
        };
        grid.set(own * LANE_WIDTH, y, mark);
        grid.text(graph + LABEL_GAP, y, &labels[i]);
        y += 1;
    }
    Ok(grid.rows())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(source: &str) -> String {
        render(source).unwrap().join("\n")
    }

    /// merman's own expectations (`gitgraph/diagram.test.ts`).
    #[test]
    fn branches_and_merges_like_merman() {
        assert_eq!(
            text(
                "gitGraph\n  commit id: \"baseline\"\n  branch refactor\n  commit id: \"extract-seam\" msg: \"Extract seam\"\n  commit id: \"add-tests\" tag: \"ready\"\n  checkout main\n  commit id: \"unrelated-fix\"\n  merge refactor id: \"land-refactor\" tag: \"v2\""
            ),
            "●    baseline\n├─╮\n│ ●  Extract seam\n│ ●  add-tests  [refactor] [ready]\n● │  unrelated-fix\n◎─╯  land-refactor  [main] [v2]"
        );
        assert_eq!(
            text("gitGraph\n commit id: base\n branch one\n branch two\n commit id: work"),
            "●      base  [main] [one]\n├───╮\n    ●  work  [two]"
        );
        assert!(
            text("gitGraph\n commit id: base\n branch feature\n checkout main\n commit id: next")
                .contains("base  [feature]")
        );
        assert_eq!(text("gitGraph BT:\n commit id: one"), "●  one  [main]");
    }

    #[test]
    fn ids_order_and_comments() {
        let (_, commits) = parse("gitGraph\n commit\n commit").unwrap();
        let ids: Vec<_> = commits.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["commit-1", "commit-2"]);
        let (branches, _) = parse(
            "gitGraph\n commit id: base\n branch later order: 2\n checkout main\n branch ordinary\n checkout main\n branch earlier order: 1",
        )
        .unwrap();
        let names: Vec<_> = branches.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["main", "ordinary", "earlier", "later"]);
        let (_, commits) =
            parse("gitGraph\n commit id: \"release%%candidate\" %% comment").unwrap();
        assert_eq!(commits[0].id, "release%%candidate");
        let (_, commits) = parse("gitGraph\n commit \"Initial release\"").unwrap();
        assert_eq!(commits[0].message.as_deref(), Some("Initial release"));
    }

    #[test]
    fn semantic_errors() {
        let error = |s: &str| render(s).unwrap_err();
        assert_eq!(
            error("gitGraph\n checkout missing"),
            "Unknown branch \"missing\""
        );
        assert_eq!(
            error("gitGraph\n cherry-pick id: one"),
            "Cherry-pick is not supported"
        );
        assert_eq!(
            error("gitGraph\n commit id: same\n commit id: same"),
            "Duplicate commit id \"same\""
        );
        assert_eq!(
            error("gitGraph\n branch feature\n checkout main\n branch feature"),
            "Duplicate branch \"feature\""
        );
        assert_eq!(
            error("gitGraph\n branch feature\n commit id: work\n checkout main\n merge feature"),
            "Branch \"main\" has no commits"
        );
        assert_eq!(
            error("commit id: missing-header"),
            "GitGraph header is required"
        );
        assert_eq!(
            error("gitGraph\n commit\n gitGraph"),
            "GitGraph header can only appear once"
        );
    }

    #[test]
    fn commit_types() {
        assert_eq!(
            text("gitGraph\n commit id: a type: REVERSE\n commit id: b type: HIGHLIGHT"),
            "⊗  a\n◆  b  [main]"
        );
    }
}
