//! Dialog state: the shared `DialogSelect` list (search, fuzzy filter, categories, scrolling)
//! and the plain, prompt, confirm and export dialogs. Drawing is `view::dialog`; what a choice
//! does is `app::dialogs`.

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Medium,
    Large,
    XLarge,
}

impl Size {
    pub fn width(self) -> u16 {
        match self {
            Size::Medium => 60,
            Size::Large => 88,
            Size::XLarge => 116,
        }
    }
}

/// How a piece of dialog text is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Base,
    Muted,
    Bold,
    Accent,
    Info,
    Success,
    Warning,
    Error,
}

pub type Line = Vec<(String, Tone)>;

pub fn line(text: impl Into<String>, tone: Tone) -> Line {
    vec![(text.into(), tone)]
}

/// A footer action, drawn `<title> <key>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub title: String,
    pub key: String,
    pub right: bool,
}

impl Action {
    pub fn left(title: &str, key: &str) -> Action {
        Action {
            title: title.into(),
            key: key.into(),
            right: false,
        }
    }

    pub fn right(title: &str, key: &str) -> Action {
        Action {
            right: true,
            ..Action::left(title, key)
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Opt {
    pub title: String,
    pub description: String,
    pub footer: String,
    pub footer_tone: Option<Tone>,
    pub category: String,
    /// Extra text the search matches (command ids, descriptions).
    pub search: String,
    pub current: bool,
    /// A marker in the gutter instead of `●` (`!`, `?`, `✓`).
    pub gutter: Option<(String, Tone)>,
    /// What the dialog acts on (an id, a name).
    pub value: String,
    /// Listed only while searching (`Some(true)`) or only without a query (`Some(false)`).
    pub searching: Option<bool>,
}

impl Opt {
    pub fn new(title: impl Into<String>, value: impl Into<String>) -> Opt {
        Opt {
            title: title.into(),
            value: value.into(),
            ..Opt::default()
        }
    }

    pub fn description(mut self, text: impl Into<String>) -> Opt {
        self.description = text.into();
        self
    }

    pub fn footer(mut self, text: impl Into<String>) -> Opt {
        self.footer = text.into();
        self
    }

    pub fn category(mut self, text: impl Into<String>) -> Opt {
        self.category = text.into();
        self
    }

    pub fn current(mut self, current: bool) -> Opt {
        self.current = current;
        self
    }
}

/// What choosing a row of a list dialog does.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Palette,
    Models,
    Variants,
    Agents,
    Sessions { all: bool },
    Themes { original: String },
    Settings,
    Mcps,
    Timeline,
    MessageActions { message: String },
    Fork,
    Skills,
    Open,
    Worktrees,
    Connect,
    Devices,
    BudgetGate { prompt: String },
    Stash,
    Queue { session: String },
}

/// One drawn row of a list dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Header(usize),
    Blank,
    Option(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Select {
    pub kind: Kind,
    pub title: String,
    /// Muted text after the bold title (` for my-app`).
    pub suffix: String,
    pub placeholder: String,
    pub query: String,
    pub options: Vec<Opt>,
    /// Position among the matching options.
    pub selected: usize,
    /// First drawn row.
    pub offset: usize,
    pub size: Size,
    pub actions: Vec<Action>,
    /// Shown instead of the list (`Loading sessions…`, an error).
    pub status: Option<(String, Tone)>,
    /// The option whose destructive action waits for a second press.
    pub confirm: Option<String>,
    pub empty: &'static str,
    /// Options keep their categories while searching (otherwise the result is one flat list).
    pub grouped_search: bool,
    /// The panel keeps the height of a full list (`Worktrees`).
    pub tall: bool,
}

/// Rows a list may use: `floor(height / 2) − 6`.
pub fn list_limit(height: u16) -> usize {
    (height / 2).saturating_sub(6).max(1) as usize
}

fn score(matcher: &mut Matcher, pattern: &Pattern, text: &str, buf: &mut Vec<char>) -> u32 {
    if text.is_empty() {
        return 0;
    }
    pattern
        .score(Utf32Str::new(text, buf), matcher)
        .unwrap_or(0)
}

impl Select {
    pub fn new(kind: Kind, title: &str, options: Vec<Opt>) -> Select {
        let selected = options.iter().position(|o| o.current).unwrap_or(0);
        Select {
            kind,
            title: title.into(),
            suffix: String::new(),
            placeholder: "Search".into(),
            query: String::new(),
            options,
            selected,
            offset: 0,
            size: Size::Medium,
            actions: Vec::new(),
            status: None,
            confirm: None,
            empty: "No items available",
            grouped_search: false,
            tall: false,
        }
    }

    /// Indexes of the options the query keeps, best first (all, in order, without a query).
    pub fn matches(&self) -> Vec<usize> {
        let query = self.query.trim();
        if query.is_empty() {
            return self
                .grouped_order()
                .into_iter()
                .filter(|&i| self.options[i].searching != Some(true))
                .collect();
        }
        let pattern = Pattern::new(
            query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let mut matcher = Matcher::new(Config::DEFAULT);
        let mut buf = Vec::new();
        let mut scored: Vec<(u32, usize)> = self
            .options
            .iter()
            .enumerate()
            .filter(|(_, o)| o.searching != Some(false))
            .filter_map(|(i, o)| {
                let title = score(&mut matcher, &pattern, &o.title, &mut buf);
                let category = score(&mut matcher, &pattern, &o.category, &mut buf);
                let search = score(&mut matcher, &pattern, &o.search, &mut buf);
                let total = title * 2 + category + search;
                (total > 0).then_some((total, i))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, i)| i).collect()
    }

    /// Options grouped by category in order of first appearance.
    fn grouped_order(&self) -> Vec<usize> {
        let mut categories: Vec<&str> = Vec::new();
        for o in &self.options {
            if !categories.contains(&o.category.as_str()) {
                categories.push(&o.category);
            }
        }
        categories
            .iter()
            .flat_map(|c| {
                self.options
                    .iter()
                    .enumerate()
                    .filter(move |(_, o)| o.category == *c)
                    .map(|(i, _)| i)
            })
            .collect()
    }

    /// The drawn rows: category headers (blank row before every one but the first) and options.
    pub fn rows(&self) -> Vec<Row> {
        let matches = self.matches();
        let categories = self.query.trim().is_empty() || self.grouped_search;
        let mut rows = Vec::new();
        let mut last: Option<&str> = None;
        for index in matches {
            let category = self.options[index].category.as_str();
            if categories && !category.is_empty() && last != Some(category) {
                if last.is_some() {
                    rows.push(Row::Blank);
                }
                rows.push(Row::Header(index));
            }
            last = Some(category);
            rows.push(Row::Option(index));
        }
        rows
    }

    /// The option under the cursor.
    pub fn current(&self) -> Option<&Opt> {
        self.matches().get(self.selected).map(|&i| &self.options[i])
    }

    /// Moves the cursor (wrapping) and scrolls it into a list of `limit` rows.
    pub fn step(&mut self, delta: isize, limit: usize) {
        let count = self.matches().len();
        if count == 0 {
            return;
        }
        let at = (self.selected as isize + delta).rem_euclid(count as isize);
        self.selected = at as usize;
        self.confirm = None;
        self.reveal(limit);
    }

    /// Moves without wrapping (page keys, home/end).
    pub fn jump(&mut self, delta: isize, limit: usize) {
        let count = self.matches().len();
        if count == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).clamp(0, count as isize - 1) as usize;
        self.confirm = None;
        self.reveal(limit);
    }

    /// Scrolls so the cursor row (and its category header when it starts one) is visible.
    pub fn reveal(&mut self, limit: usize) {
        let rows = self.rows();
        let Some(target) = self.current().and_then(|_| {
            let index = self.matches()[self.selected];
            rows.iter().position(|r| *r == Row::Option(index))
        }) else {
            self.offset = 0;
            return;
        };
        let top = match target.checked_sub(1).map(|i| rows[i]) {
            Some(Row::Header(_)) => target - 1,
            _ => target,
        };
        if top < self.offset {
            self.offset = top;
        } else if target >= self.offset + limit {
            self.offset = target + 1 - limit;
        }
        self.offset = self.offset.min(rows.len().saturating_sub(limit));
    }

    pub fn set_query(&mut self, query: String, limit: usize) {
        self.query = query;
        self.selected = 0;
        self.offset = 0;
        self.confirm = None;
        if self.query.is_empty()
            && let Some(at) = self.matches().iter().position(|&i| self.options[i].current)
        {
            self.selected = at;
            self.reveal(limit);
        }
    }
}

/// What a plain dialog is, for its keys.
#[derive(Debug, Clone, PartialEq)]
pub enum BodyKind {
    Credits,
    Usage { days: u32 },
    Help,
    Debug { report: String },
    Status,
    Update,
    Exported { path: String },
    Btw { answer: String },
    Image { index: usize },
    Error { details: String },
}

/// A dialog with a title, free rows and a footer.
#[derive(Debug, Clone, PartialEq)]
pub struct Body {
    pub kind: BodyKind,
    pub title: Line,
    pub close: String,
    pub size: Size,
    pub centered: bool,
    pub lines: Vec<Line>,
    pub footer: Line,
    pub actions: Vec<Action>,
    /// Right-aligned buttons; the last is the active one unless `active` says otherwise.
    pub buttons: Vec<String>,
    pub active: usize,
    /// Columns kept free right of the buttons.
    pub button_margin: u16,
    pub scroll: usize,
}

impl Body {
    pub fn new(kind: BodyKind, title: &str, lines: Vec<Line>) -> Body {
        Body {
            kind,
            title: line(title, Tone::Bold),
            close: "esc".into(),
            size: Size::Medium,
            centered: false,
            lines,
            footer: Vec::new(),
            actions: Vec::new(),
            buttons: Vec::new(),
            active: 0,
            button_margin: 0,
            scroll: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PromptKind {
    Rename { session: String },
    Btw,
}

/// A one-line text prompt (`Rename session`, `/btw`).
#[derive(Debug, Clone, PartialEq)]
pub struct Prompt {
    pub kind: PromptKind,
    pub title: String,
    pub placeholder: String,
    pub text: String,
    pub busy: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfirmKind {
    Revoke { id: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Confirm {
    pub kind: ConfirmKind,
    pub title: String,
    pub message: String,
    pub button: String,
    /// The confirm button (true) or `Cancel` has focus.
    pub yes: bool,
}

/// `Export session` options.
#[derive(Debug, Clone, PartialEq)]
pub struct Export {
    pub json: bool,
    pub thinking: bool,
    pub tools: bool,
    pub sanitize: bool,
    /// 0 format, 1–2 checkboxes, then the `Copy` and `Export` buttons.
    pub focus: usize,
}

impl Export {
    pub fn fields(&self) -> usize {
        if self.json { 2 } else { 3 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Dialog {
    Select(Select),
    Body(Body),
    Prompt(Prompt),
    Confirm(Confirm),
    Export(Export),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Select {
        let options = vec![
            Opt::new("Switch model", "a").category("Suggested"),
            Opt::new("Switch session", "b").category("Session"),
            Opt::new("New session", "c").category("Session"),
            Opt::new("Switch agent", "d").category("Agent"),
        ];
        Select::new(Kind::Palette, "Commands", options)
    }

    #[test]
    fn groups_and_filters() {
        let mut s = palette();
        assert_eq!(
            s.rows(),
            vec![
                Row::Header(0),
                Row::Option(0),
                Row::Blank,
                Row::Header(1),
                Row::Option(1),
                Row::Option(2),
                Row::Blank,
                Row::Header(3),
                Row::Option(3),
            ]
        );
        s.set_query("agent".into(), 10);
        assert_eq!(s.current().unwrap().value, "d");
        assert_eq!(s.rows(), vec![Row::Option(3)]);
    }

    #[test]
    fn wraps_and_scrolls() {
        let mut s = palette();
        s.step(-1, 3);
        assert_eq!(s.current().unwrap().value, "d");
        assert_eq!(s.offset, 6);
        s.step(1, 3);
        assert_eq!((s.selected, s.offset), (0, 0));
    }
}
