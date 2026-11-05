//! The `/diff` viewer's loading and keys.

use super::{App, Msg};
use crate::dialog::{Body, BodyKind, Dialog, Kind, Opt, Select, Tone, line};
use crate::diffview::{Diff, Source, load};
use crate::keymap::Stroke;

impl App {
    pub(super) fn open_diff(&mut self, source: Source) {
        let mut view = Diff::new(source);
        if let Some(old) = &self.state.diff {
            view.tree = old.tree;
            view.single = old.single;
            view.base = old.base.clone();
            view.fullscreen = old.fullscreen;
        } else {
            view.fullscreen = self.tui.diff_fullscreen();
        }
        let base = view.base.clone();
        self.state.diff = Some(view);
        let directory = self.core.directory().to_path_buf();
        let tx = self.tx.clone();
        let core = self.core.clone();
        let session = self.state.session().map(|s| s.id.clone());
        tokio::task::spawn_blocking(move || {
            let files = match (source, session) {
                (Source::Turn, Some(session)) => core
                    .last_turn_patch(&session)
                    .map(|patch| crate::diffview::split(&patch))
                    .map_err(|e| e.to_string()),
                _ => load(&directory, source, base.as_deref()),
            };
            let _ = tx.send(Msg::Apply(Box::new(move |app: &mut App| {
                if let Some(view) = app.state.diff.as_mut()
                    && view.source == source
                {
                    view.files = Some(files);
                }
            })));
        });
    }

    pub(super) fn diff_key(&mut self, stroke: &Stroke) {
        if let Some(view) = self.state.diff.as_mut()
            && view.menu.take().is_some()
        {
            return;
        }
        let page = self.state.height.saturating_sub(4) as usize;
        let width = crate::view::diff_width(&self.state);
        let Some(view) = self.state.diff.as_mut() else {
            return;
        };
        let count = view.list().len();
        match (stroke.key.as_str(), stroke.ctrl) {
            ("escape" | "q", false) | ("c", true) => self.state.diff = None,
            ("down" | "up", false) if stroke.alt && count > 0 => {
                let next = if stroke.key == "down" { "n" } else { "p" };
                let mut stroke = stroke.clone();
                stroke.key = next.into();
                stroke.alt = false;
                self.diff_key(&stroke)
            }
            ("j" | "down", false) => view.scroll += 1,
            ("k" | "up", false) => view.scroll = view.scroll.saturating_sub(1),
            ("pagedown", _) | ("f", true) => view.scroll += page,
            ("pageup", _) | ("b", true) => view.scroll = view.scroll.saturating_sub(page),
            ("d", true) => view.scroll += page / 2,
            ("u", true) => view.scroll = view.scroll.saturating_sub(page / 2),
            ("home" | "g", false) => view.scroll = 0,
            ("end" | "G", _) => view.scroll = view.length(width, &self.theme),
            ("]", false) => {
                let starts = view.hunks(width, &self.theme);
                if let Some(next) = starts.into_iter().find(|row| *row > view.scroll) {
                    view.scroll = next;
                }
            }
            ("[", false) => {
                let starts = view.hunks(width, &self.theme);
                if let Some(previous) = starts.into_iter().rev().find(|row| *row < view.scroll) {
                    view.scroll = previous;
                }
            }
            ("n" | "p", false) if count > 0 => {
                let current = view.current(width, &self.theme);
                let step = if stroke.key == "n" { 1 } else { count - 1 };
                view.selected = (current + step) % count;
                view.scroll = if view.single {
                    0
                } else {
                    view.starts(width, &self.theme)[view.selected]
                };
            }
            ("b", false) => view.tree = !view.tree,
            ("f", false) => view.fullscreen = !view.fullscreen,
            ("s", false) => {
                view.selected = view.current(width, &self.theme);
                view.single = !view.single;
                view.scroll = if view.single {
                    0
                } else {
                    view.starts(width, &self.theme)
                        .get(view.selected)
                        .copied()
                        .unwrap_or(0)
                };
            }
            ("?", _) => self.diff_help(),
            ("m", false) => {
                let selected = view.current(width, &self.theme);
                if let Some(Ok(files)) = view.files.as_mut()
                    && let Some(file) = files.get_mut(selected)
                {
                    file.reviewed = !file.reviewed;
                }
            }
            ("d", false) => self.diff_sources(),
            _ => {}
        }
    }

    /// `d`: the "Diff source" list, with the base branch row.
    fn diff_sources(&mut self) {
        let Some(view) = self.state.diff.as_ref() else {
            return;
        };
        let mut sources = vec![Source::All, Source::Committed, Source::Uncommitted];
        if self.state.session().is_some() {
            sources.push(Source::Turn);
        }
        let mut options: Vec<Opt> = sources
            .into_iter()
            .map(|source| {
                let mut opt = Opt::new(format!("{:<11}", source.title()), source.title())
                    .description(source.description());
                opt.current = source == view.source;
                opt
            })
            .collect();
        let base = view.base.clone().unwrap_or_else(|| "Choose…".into());
        options.push(Opt::new(format!("{:<11}", "Base"), "base").description(base));
        self.push(Dialog::Select(Select::new(
            Kind::DiffSource,
            "Diff source",
            options,
        )));
    }

    pub(super) fn diff_source_chosen(&mut self, value: &str) {
        self.state.dialogs.pop();
        let source = match value {
            "All" => Source::All,
            "Committed" => Source::Committed,
            "Uncommitted" => Source::Uncommitted,
            "Last turn" => Source::Turn,
            _ => return self.diff_bases(),
        };
        self.open_diff(source);
    }

    /// The "Base branch" picker over local and remote branches.
    fn diff_bases(&mut self) {
        let current = self.state.diff.as_ref().and_then(|v| v.base.clone());
        let options = match crate::diffview::branches(self.core.directory()) {
            Ok(branches) => branches
                .into_iter()
                .map(|name| {
                    let mut opt = Opt::new(name.clone(), name.clone());
                    opt.current = current.as_deref() == Some(name.as_str());
                    opt
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        let mut select = Select::new(Kind::DiffBase, "Base branch", options);
        select.placeholder = "Search local and remote branches".into();
        select.empty = "No branches found";
        self.push(Dialog::Select(select));
    }

    pub(super) fn diff_base_chosen(&mut self, branch: String) {
        self.state.dialogs.clear();
        let Some(view) = self.state.diff.as_mut() else {
            return;
        };
        view.base = Some(branch);
        let source = view.source;
        self.open_diff(source);
    }

    /// `?`: the "Diff shortcuts" dialog.
    fn diff_help(&mut self) {
        let groups: [(&str, Vec<(&str, &str)>); 3] = [
            (
                "Navigate",
                vec![
                    ("n / p", "Next / previous file"),
                    ("] / [", "Next / previous hunk"),
                    ("m", "Mark reviewed"),
                ],
            ),
            (
                "Scroll",
                vec![
                    ("j / k", "Down / up"),
                    ("ctrl+d / ctrl+u", "Half page down / up"),
                    ("pgdn / pgup", "Page down / up"),
                    ("gg / G", "First / last"),
                ],
            ),
            (
                "View",
                vec![
                    ("s", "All files / single file"),
                    ("b", "Show / hide file tree"),
                    ("d", "Switch diff source"),
                    ("esc / q", "Close diff viewer"),
                ],
            ),
        ];
        let mut lines = Vec::new();
        for (title, rows) in groups {
            if !lines.is_empty() {
                lines.push(Vec::new());
            }
            lines.push(line(title, Tone::Bold));
            for (key, label) in rows {
                lines.push(vec![
                    (format!("{key:<17}  "), Tone::Base),
                    (label.to_string(), Tone::Muted),
                ]);
            }
        }
        let body = Body::new(BodyKind::DiffHelp, "Diff shortcuts", lines);
        self.push(Dialog::Body(body));
    }

    /// A right click on a file tree row opens its menu; a left click on the menu toggles the mark.
    pub(super) fn diff_click(&mut self, right: bool, at: (u16, u16)) -> bool {
        let width = crate::view::diff_width(&self.state);
        let left = if crate::view::diff_docked(&self.state) {
            crate::view::pane_width(&self.state)
        } else {
            0
        };
        let top = if left > 0 {
            u16::from(self.state.strip())
        } else {
            0
        };
        if at.0 < left {
            return false;
        }
        let at = (at.0 - left, at.1.saturating_sub(top));
        let Some(view) = self.state.diff.as_mut() else {
            return false;
        };
        if let Some((x, y, index)) = view.menu.take() {
            if !right
                && at.1 == y
                && (x..x + 19).contains(&at.0)
                && let Some(Ok(files)) = view.files.as_mut()
                && let Some(file) = files.get_mut(index)
            {
                file.reviewed = !file.reviewed;
            }
            return true;
        }
        let tree = crate::view::diff_tree_width(view, width);
        let index = at.1.checked_sub(2).map(usize::from);
        match index {
            Some(index) if at.0 < tree && index < view.list().len() => {
                if right {
                    view.menu = Some((at.0.min(width.saturating_sub(19)), at.1 + 1, index));
                } else {
                    view.selected = index;
                    view.scroll = if view.single {
                        0
                    } else {
                        view.starts(width, &self.theme)[index]
                    };
                }
                true
            }
            _ => true,
        }
    }
}
