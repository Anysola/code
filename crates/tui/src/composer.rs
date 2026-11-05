//! The prompt buffer: multiline text with atomic placeholders (collapsed pastes, mentions, image
//! chips), cursor movement and editing, in-input undo, and prompt history.

use std::path::{Path, PathBuf};

use unicode_segmentation::UnicodeSegmentation;

use crate::text::width;

#[derive(Debug, Clone, PartialEq)]
pub enum AtomKind {
    /// `[Pasted ~N lines]` standing for the full pasted text.
    Paste(String),
    File,
    Agent,
    Skill,
    /// `[Image N]` / `[PDF N]`: `(mime, base64 data, file name)`.
    Attachment(String, String, String),
}

/// A mention before an external edit: kind, text and offset in the expanded text.
pub type Mark = (AtomKind, String, usize);

/// A placeholder occupying `start..end` of the text; edits never split it.
#[derive(Debug, Clone, PartialEq)]
pub struct Atom {
    pub start: usize,
    pub end: usize,
    pub kind: AtomKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Normal,
    Shell,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Snapshot {
    text: String,
    cursor: usize,
    atoms: Vec<Atom>,
}

#[derive(Debug, Clone, Default)]
pub struct Composer {
    pub text: String,
    /// Byte offset of the cursor.
    pub cursor: usize,
    pub atoms: Vec<Atom>,
    pub mode: Mode,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

/// Order-preserving pairs of `(item, candidate)` with the most matches, then the least total
/// distance between an item's old offset and its candidate.
fn assign(items: &[usize], candidates: &[usize]) -> Vec<(usize, usize)> {
    let (n, m) = (items.len(), candidates.len());
    // best[i][j] = (matches, -cost) for items i.. and candidates j..
    let mut best = vec![vec![(0usize, 0i64); m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let (matches, cost) = best[i + 1][j + 1];
            let distance = items[i].abs_diff(candidates[j]) as i64;
            let matched = (matches + 1, cost - distance);
            best[i][j] = [matched, best[i + 1][j], best[i][j + 1]]
                .into_iter()
                .reduce(|a, b| {
                    if b.0 > a.0 || (b.0 == a.0 && b.1 > a.1) {
                        b
                    } else {
                        a
                    }
                })
                .unwrap_or_default();
        }
    }
    let (mut i, mut j, mut pairs) = (0, 0, Vec::new());
    while i < n && j < m {
        let (matches, cost) = best[i][j];
        let distance = items[i].abs_diff(candidates[j]) as i64;
        if (matches, cost) == (best[i + 1][j + 1].0 + 1, best[i + 1][j + 1].1 - distance) {
            pairs.push((i, candidates[j]));
            i += 1;
            j += 1;
        } else if best[i + 1][j] == (matches, cost) {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

/// A paste at least this big collapses into `[Pasted ~N lines]`.
pub fn collapses(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.lines().count() >= 3 || trimmed.chars().count() > 150
}

/// Removes ANSI escape sequences and normalises line endings.
pub fn clean_paste(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .collect::<Vec<_>>()
        .into_iter()
        .peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            } else if chars.peek() == Some(&']') {
                for n in chars.by_ref() {
                    if n == '\x07' {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

impl Composer {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            cursor: self.cursor,
            atoms: self.atoms.clone(),
        }
    }

    fn save(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn restore(&mut self, s: Snapshot) {
        self.text = s.text;
        self.cursor = s.cursor;
        self.atoms = s.atoms;
    }

    pub fn undo(&mut self) {
        if let Some(previous) = self.undo.pop() {
            let current = self.snapshot();
            self.redo.push(current);
            self.restore(previous);
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            let current = self.snapshot();
            self.undo.push(current);
            self.restore(next);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn clear(&mut self) {
        self.save();
        self.text.clear();
        self.cursor = 0;
        self.atoms.clear();
    }

    pub fn set(&mut self, text: &str) {
        self.save();
        self.text = text.to_string();
        self.cursor = text.len();
        self.atoms.clear();
    }

    fn atom_at(&self, offset: usize) -> Option<&Atom> {
        self.atoms
            .iter()
            .find(|a| a.start < offset && offset < a.end)
    }

    fn shift(&mut self, from: usize, delta: isize) {
        for atom in &mut self.atoms {
            if atom.start >= from {
                atom.start = (atom.start as isize + delta) as usize;
                atom.end = (atom.end as isize + delta) as usize;
            }
        }
    }

    pub fn insert(&mut self, text: &str) {
        self.save();
        self.insert_raw(text);
    }

    fn insert_raw(&mut self, text: &str) {
        if let Some(atom) = self.atom_at(self.cursor) {
            self.cursor = atom.end;
        }
        self.shift(self.cursor, text.len() as isize);
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    /// Inserts `label` as an atom followed by a space.
    pub fn insert_atom(&mut self, label: &str, kind: AtomKind) {
        self.save();
        if let Some(atom) = self.atom_at(self.cursor) {
            self.cursor = atom.end;
        }
        let start = self.cursor;
        self.insert_raw(label);
        self.atoms.push(Atom {
            start,
            end: start + label.len(),
            kind,
        });
        if !self.text[self.cursor..].starts_with(' ') {
            self.insert_raw(" ");
        } else {
            self.cursor += 1;
        }
    }

    /// Removes `range` and any atom it touches.
    fn remove(&mut self, mut start: usize, mut end: usize) {
        for atom in &self.atoms {
            if atom.start < end && start < atom.end {
                start = start.min(atom.start);
                end = end.max(atom.end);
            }
        }
        self.atoms.retain(|a| a.end <= start || a.start >= end);
        self.text.replace_range(start..end, "");
        self.shift(end, -((end - start) as isize));
        self.cursor = start;
    }

    fn prev_boundary(&self, offset: usize) -> usize {
        self.text[..offset]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.text[offset..]
            .graphemes(true)
            .next()
            .map_or(offset, |g| offset + g.len())
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.save();
        let start = self.prev_boundary(self.cursor);
        self.remove(start, self.cursor);
    }

    pub fn delete(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        self.save();
        let end = self.next_boundary(self.cursor);
        self.remove(self.cursor, end);
    }

    pub fn left(&mut self) {
        self.cursor = self.prev_boundary(self.cursor);
        if let Some(atom) = self.atom_at(self.cursor) {
            self.cursor = atom.start;
        }
    }

    pub fn right(&mut self) {
        self.cursor = self.next_boundary(self.cursor);
        if let Some(atom) = self.atom_at(self.cursor) {
            self.cursor = atom.end;
        }
    }

    fn line_start(&self) -> usize {
        self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1)
    }

    fn line_end(&self) -> usize {
        self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |i| self.cursor + i)
    }

    pub fn home(&mut self) {
        self.cursor = self.line_start();
    }

    pub fn end(&mut self) {
        self.cursor = self.line_end();
    }

    pub fn buffer_home(&mut self) {
        self.cursor = 0;
    }

    pub fn buffer_end(&mut self) {
        self.cursor = self.text.len();
    }

    /// Moves to the previous logical line; false at the first line.
    pub fn up(&mut self) -> bool {
        let start = self.line_start();
        if start == 0 {
            return false;
        }
        let column = width(&self.text[start..self.cursor]);
        let previous = self.text[..start - 1].rfind('\n').map_or(0, |i| i + 1);
        self.cursor = previous + offset_at(&self.text[previous..start - 1], column);
        true
    }

    /// Moves to the next logical line; false at the last line.
    pub fn down(&mut self) -> bool {
        let end = self.line_end();
        if end >= self.text.len() {
            return false;
        }
        let column = width(&self.text[self.line_start()..self.cursor]);
        let next = end + 1;
        let next_end = self.text[next..]
            .find('\n')
            .map_or(self.text.len(), |i| next + i);
        self.cursor = next + offset_at(&self.text[next..next_end], column);
        true
    }

    fn word_back(&self) -> usize {
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end();
        trimmed
            .unicode_word_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn word_forward(&self) -> usize {
        let after = &self.text[self.cursor..];
        after
            .unicode_word_indices()
            .next()
            .map_or(self.text.len(), |(i, w)| self.cursor + i + w.len())
    }

    pub fn word_left(&mut self) {
        self.cursor = self.word_back();
    }

    pub fn word_right(&mut self) {
        self.cursor = self.word_forward();
    }

    pub fn delete_word_back(&mut self) {
        let start = self.word_back();
        if start < self.cursor {
            self.save();
            self.remove(start, self.cursor);
        }
    }

    pub fn delete_word_forward(&mut self) {
        let end = self.word_forward();
        if end > self.cursor {
            self.save();
            self.remove(self.cursor, end);
        }
    }

    pub fn delete_to_line_end(&mut self) {
        let end = self.line_end();
        if end > self.cursor {
            self.save();
            self.remove(self.cursor, end);
        }
    }

    pub fn delete_to_line_start(&mut self) {
        let start = self.line_start();
        if start < self.cursor {
            self.save();
            self.remove(start, self.cursor);
        }
    }

    pub fn delete_line(&mut self) {
        let start = self.line_start();
        let end = (self.line_end() + 1).min(self.text.len());
        self.save();
        self.remove(start, end);
    }

    /// Pastes text: large pastes collapse into a placeholder; pasting the same text right after
    /// its placeholder expands it instead.
    pub fn paste(&mut self, raw: &str, compact: bool) {
        let text = clean_paste(raw);
        if text.is_empty() {
            return;
        }
        let repeat = self.atoms.iter().position(|a| {
            matches!(&a.kind, AtomKind::Paste(full) if *full == text)
                && (self.cursor == a.end || self.cursor == a.end + 1)
        });
        if let Some(index) = repeat {
            self.save();
            let atom = self.atoms.remove(index);
            self.text.replace_range(atom.start..atom.end, &text);
            let delta = text.len() as isize - (atom.end - atom.start) as isize;
            self.shift(atom.end, delta);
            self.cursor = (self.cursor as isize + delta) as usize;
            return;
        }
        if compact && collapses(&text) {
            let lines = text.matches('\n').count() + 1;
            self.insert_atom(&format!("[Pasted ~{lines} lines]"), AtomKind::Paste(text));
        } else {
            self.insert(&text);
        }
    }

    /// The text with placeholders expanded, as submitted and copied.
    pub fn expanded(&self) -> String {
        let mut out = String::new();
        let mut at = 0;
        let mut atoms: Vec<&Atom> = self.atoms.iter().collect();
        atoms.sort_by_key(|a| a.start);
        for atom in atoms {
            out.push_str(&self.text[at..atom.start]);
            match &atom.kind {
                AtomKind::Paste(full) => out.push_str(full),
                _ => out.push_str(&self.text[atom.start..atom.end]),
            }
            at = atom.end;
        }
        out.push_str(&self.text[at..]);
        out
    }

    /// File, agent and skill mentions as `(kind, text, offset in `expanded()`)`, for
    /// [`Composer::realign`] after the text was edited elsewhere.
    pub fn mention_marks(&self) -> Vec<Mark> {
        let mut atoms: Vec<&Atom> = self.atoms.iter().collect();
        atoms.sort_by_key(|a| a.start);
        let mut delta = 0isize;
        let mut marks = Vec::new();
        for atom in atoms {
            match &atom.kind {
                AtomKind::Paste(full) => {
                    delta += full.len() as isize - (atom.end - atom.start) as isize;
                }
                AtomKind::Attachment(..) => {}
                kind => marks.push((
                    kind.clone(),
                    self.text[atom.start..atom.end].to_string(),
                    (atom.start as isize + delta) as usize,
                )),
            }
        }
        marks
    }

    /// Replaces the text with `content` and re-attaches earlier mentions to the occurrences of
    /// their text that keep as many mentions as possible, nearest to where each used to be.
    pub fn realign(&mut self, content: &str, marks: &[Mark]) {
        self.set(content);
        let mut groups: Vec<(&str, Vec<&Mark>)> = Vec::new();
        for mark in marks {
            match groups.iter_mut().find(|(text, _)| *text == mark.1) {
                Some((_, items)) => items.push(mark),
                None => groups.push((&mark.1, vec![mark])),
            }
        }
        groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));
        let mut protected: Vec<(usize, usize)> = Vec::new();
        for (text, mut items) in groups {
            if text.is_empty() {
                continue;
            }
            items.sort_by_key(|m| m.2);
            let candidates: Vec<usize> = content.match_indices(text).map(|(at, _)| at).collect();
            let free: Vec<usize> = candidates
                .iter()
                .copied()
                .filter(|&c| !protected.iter().any(|&(s, e)| c < e && c + text.len() > s))
                .collect();
            let starts: Vec<usize> = items.iter().map(|m| m.2).collect();
            for (item, at) in assign(&starts, &free) {
                self.atoms.push(Atom {
                    start: at,
                    end: at + text.len(),
                    kind: items[item].0.clone(),
                });
            }
            protected.extend(candidates.iter().map(|&c| (c, c + text.len())));
        }
        self.atoms.sort_by_key(|a| a.start);
    }

    /// `(mime, data, name)` of the attached images and PDFs.
    pub fn attachments(&self) -> Vec<(String, String, String)> {
        self.atoms
            .iter()
            .filter_map(|a| match &a.kind {
                AtomKind::Attachment(mime, data, name) => {
                    Some((mime.clone(), data.clone(), name.clone()))
                }
                _ => None,
            })
            .collect()
    }

    /// The `@<skill>` mentions, in order of appearance.
    pub fn skills(&self) -> Vec<String> {
        let mut atoms: Vec<&Atom> = self
            .atoms
            .iter()
            .filter(|a| a.kind == AtomKind::Skill)
            .collect();
        atoms.sort_by_key(|a| a.start);
        atoms
            .iter()
            .map(|a| self.text[a.start..a.end].to_string())
            .collect()
    }

    /// The `@`/`/` token the cursor is in: `(trigger, start offset, query)`.
    pub fn token(&self) -> Option<(char, usize, String)> {
        let before = &self.text[..self.cursor];
        if self.mode == Mode::Normal
            && before.starts_with('/')
            && !before.contains(char::is_whitespace)
            && !before[1..].contains('/')
        {
            return Some(('/', 0, before[1..].to_string()));
        }
        let at = before.rfind('@')?;
        if at > 0 && !before[..at].ends_with(char::is_whitespace) {
            return None;
        }
        let query = &before[at + 1..];
        if query.contains(char::is_whitespace) || self.atom_at(at + 1).is_some() {
            return None;
        }
        Some(('@', at, query.to_string()))
    }

    /// The argument being typed after `/cd `: where it starts and its text.
    pub fn cd_argument(&self) -> Option<(usize, String)> {
        let before = &self.text[..self.cursor];
        let argument = before.strip_prefix("/cd ")?;
        (self.mode == Mode::Normal && !argument.contains(char::is_whitespace))
            .then(|| (4, argument.to_string()))
    }

    /// Replaces `start..cursor` with `text`.
    pub fn replace_token(&mut self, start: usize, text: &str) {
        self.save();
        self.remove(start, self.cursor);
        self.insert_raw(text);
    }
}

fn offset_at(line: &str, column: usize) -> usize {
    let mut used = 0;
    for (i, g) in line.grapheme_indices(true) {
        if used >= column {
            return i;
        }
        used += width(g);
    }
    line.len()
}

/// One stored prompt (`prompt-history.jsonl`).
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub text: String,
    pub shell: bool,
}

pub const HISTORY_LIMIT: usize = 50;

pub struct History {
    path: PathBuf,
    pub entries: Vec<Entry>,
    /// Position while browsing: `entries.len()` means "not browsing".
    pub index: usize,
}

impl History {
    /// Loads the file, dropping lines that do not parse, and keeps the last 50.
    pub fn load(path: &Path) -> History {
        let entries: Vec<Entry> = std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter_map(|v| {
                Some(Entry {
                    text: v.get("text")?.as_str()?.to_string(),
                    shell: v.get("mode").and_then(|m| m.as_str()) == Some("shell"),
                })
            })
            .collect();
        let skip = entries.len().saturating_sub(HISTORY_LIMIT);
        let entries: Vec<Entry> = entries.into_iter().skip(skip).collect();
        let index = entries.len();
        History {
            path: path.to_path_buf(),
            entries,
            index,
        }
    }

    pub fn push(&mut self, entry: Entry) {
        if self.entries.last() != Some(&entry) {
            self.entries.push(entry);
            let skip = self.entries.len().saturating_sub(HISTORY_LIMIT);
            self.entries.drain(..skip);
            self.write();
        }
        self.index = self.entries.len();
    }

    fn write(&self) {
        let body: String = self
            .entries
            .iter()
            .map(|e| {
                let mut value = serde_json::json!({"text": e.text, "files": [], "agents": [], "skills": [], "pasted": []});
                if e.shell {
                    value["mode"] = "shell".into();
                }
                format!("{value}\n")
            })
            .collect();
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.path, body);
    }

    /// The previous entry, when browsing may start or continue (`current` is empty or shown).
    pub fn previous(&mut self, current: &str) -> Option<Entry> {
        let showing = self
            .entries
            .get(self.index)
            .is_some_and(|e| e.text == current);
        if !(current.is_empty() || showing) || self.index == 0 {
            return None;
        }
        self.index -= 1;
        self.entries.get(self.index).cloned()
    }

    /// The next entry; past the newest an empty prompt.
    pub fn next(&mut self, current: &str) -> Option<Entry> {
        let showing = self
            .entries
            .get(self.index)
            .is_some_and(|e| e.text == current);
        if !showing {
            return None;
        }
        self.index += 1;
        Some(self.entries.get(self.index).cloned().unwrap_or(Entry {
            text: String::new(),
            shell: false,
        }))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn mentions_follow_an_external_edit() {
        let mut c = Composer::default();
        c.insert_atom("@a.rs", AtomKind::File);
        c.insert("then ");
        c.insert_atom("@a.rs", AtomKind::File);
        let marks = c.mention_marks();
        c.realign("intro @a.rs and @a.rs and more", &marks);
        let spans: Vec<_> = c.atoms.iter().map(|a| (a.start, a.end)).collect();
        assert_eq!(spans, vec![(6, 11), (16, 21)]);
        c.realign("@a.rs gone? @b.rs", &marks);
        assert_eq!(c.atoms.len(), 1);
    }

    use super::*;

    #[test]
    fn large_pastes_collapse_and_expand() {
        let mut c = Composer::default();
        c.insert("see ");
        let big = "a\nb\nc\nd";
        c.paste(big, true);
        assert_eq!(c.text, "see [Pasted ~4 lines] ");
        assert_eq!(c.expanded(), "see a\nb\nc\nd ");
        c.paste(big, true);
        assert_eq!(c.text, "see a\nb\nc\nd ");
    }

    #[test]
    fn atoms_delete_whole() {
        let mut c = Composer::default();
        c.insert_atom("@src/main.rs", AtomKind::File);
        assert_eq!(c.text, "@src/main.rs ");
        c.backspace();
        c.backspace();
        assert_eq!(c.text, "");
        c.insert("x");
        c.undo();
        assert_eq!(c.text, "");
    }

    #[test]
    fn tokens() {
        let mut c = Composer::default();
        c.insert("/mod");
        assert_eq!(c.token(), Some(('/', 0, "mod".into())));
        c.set("look at @src");
        assert_eq!(c.token(), Some(('@', 8, "src".into())));
        c.set("mail a@b");
        assert_eq!(c.token(), None);
    }

    #[test]
    fn history_browses_and_caps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prompt-history.jsonl");
        let mut h = History::load(&path);
        for i in 0..55 {
            h.push(Entry {
                text: format!("p{i}"),
                shell: false,
            });
        }
        let mut h = History::load(&path);
        assert_eq!(h.entries.len(), 50);
        assert_eq!(h.previous("").unwrap().text, "p54");
        assert_eq!(h.previous("p54").unwrap().text, "p53");
        assert_eq!(h.next("p53").unwrap().text, "p54");
        assert_eq!(h.next("p54").unwrap().text, "");
        assert!(h.previous("typed").is_none());
    }
}
