//! The `/diff` viewer's loading and keys.

use super::{App, Msg};
use crate::diffview::{Diff, Source, load};
use crate::keymap::Stroke;

impl App {
    pub(super) fn open_diff(&mut self, source: Source) {
        let mut view = Diff::new(source);
        if let Some(old) = &self.state.diff {
            view.tree = old.tree;
        }
        self.state.diff = Some(view);
        let directory = self.core.directory().to_path_buf();
        let tx = self.tx.clone();
        tokio::task::spawn_blocking(move || {
            let files = load(&directory, source);
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
        let page = self.state.height.saturating_sub(4) as usize;
        let Some(view) = self.state.diff.as_mut() else {
            return;
        };
        let count = view.list().len();
        match (stroke.key.as_str(), stroke.ctrl) {
            ("escape" | "q", false) | ("c", true) => self.state.diff = None,
            ("j" | "down", false) => view.scroll += 1,
            ("k" | "up", false) => view.scroll = view.scroll.saturating_sub(1),
            ("pagedown", _) | ("f", true) => view.scroll += page,
            ("pageup", _) | ("b", true) => view.scroll = view.scroll.saturating_sub(page),
            ("d", true) => view.scroll += page / 2,
            ("u", true) => view.scroll = view.scroll.saturating_sub(page / 2),
            ("home" | "g", false) => view.scroll = 0,
            ("n", false) if count > 0 => {
                view.selected = (view.selected + 1) % count;
                view.scroll = 0;
            }
            ("p", false) if count > 0 => {
                view.selected = (view.selected + count - 1) % count;
                view.scroll = 0;
            }
            ("b", false) => view.tree = !view.tree,
            ("m", false) => {
                let selected = view.selected;
                if let Some(Ok(files)) = view.files.as_mut()
                    && let Some(file) = files.get_mut(selected)
                {
                    file.reviewed = !file.reviewed;
                }
            }
            ("d", false) => {
                let next = view.source.next();
                self.open_diff(next);
            }
            _ => {}
        }
    }
}
