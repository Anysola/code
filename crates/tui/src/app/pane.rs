//! The terminal pane commands: new, toggle, select, close and pane focus; keys go to the
//! terminal while it has focus.

use super::{App, Msg};
use crate::keymap::Stroke;
use crate::state::{Pane, Route, Variant};
use crate::terminal::{Terminal, key_bytes};

impl App {
    /// Pane commands; false when `id` is not one.
    pub(super) fn pane_command(&mut self, id: &str) -> bool {
        let terminal = matches!(
            id,
            "session.terminal" | "terminal.toggle" | "terminal.select" | "terminal.close"
        );
        if terminal && !matches!(self.state.route, Route::Session(_)) {
            self.toast(None, "Open a session first".into(), Variant::Warning);
            return true;
        }
        match id {
            "session.terminal" => self.new_terminal(),
            "terminal.toggle" if self.shown.is_some() => self.shown = None,
            "terminal.toggle" if self.terminals.is_empty() => self.new_terminal(),
            "terminal.toggle" => self.show(self.terminals.len() - 1, true),
            "terminal.select" if self.terminals.is_empty() => self.new_terminal(),
            "terminal.select" => {
                let next = self.shown.map_or(0, |i| (i + 1) % self.terminals.len());
                self.show(next, true);
            }
            "terminal.close" => {
                if let Some(index) = self.shown.take() {
                    self.terminals.remove(index);
                }
            }
            "pane.focus.left" => self.focus(false),
            "pane.focus.right" => self.focus(true),
            _ => return false,
        }
        true
    }

    fn new_terminal(&mut self) {
        let (rows, cols) = crate::view::terminal_size(&self.state);
        let tx = self.tx.clone();
        let wake = move || {
            let _ = tx.send(Msg::Apply(Box::new(|_: &mut App| {})));
        };
        match Terminal::spawn(self.core.directory(), rows, cols, wake) {
            Ok(terminal) => {
                self.terminals.push(terminal);
                self.show(self.terminals.len() - 1, true);
            }
            Err(error) => self.toast(
                Some("Unable to load terminal".into()),
                error,
                Variant::Error,
            ),
        }
    }

    fn show(&mut self, index: usize, focused: bool) {
        self.shown = Some(index);
        self.state.pane = Some(Pane {
            screen: self.terminals[index].screen.clone(),
            focused,
        });
    }

    fn focus(&mut self, right: bool) {
        if let Some(pane) = self.state.pane.as_mut() {
            pane.focused = right;
        }
    }

    /// Before a frame: drops exited shells and fits the shown one to the pane.
    pub(super) fn sync_pane(&mut self) {
        let before = self.terminals.len();
        let shown = self.shown.map(|i| self.terminals[i].screen.clone());
        self.terminals.retain_mut(|t| !t.exited());
        if self.terminals.len() != before {
            self.shown = shown.and_then(|s| self.terminals.iter().position(|t| t.screen == s));
        }
        let visible = matches!(self.state.route, Route::Session(_));
        match self.shown.filter(|_| visible) {
            Some(index) => {
                let focused = self.state.pane.as_ref().is_some_and(|p| p.focused);
                let (rows, cols) = crate::view::terminal_size(&self.state);
                self.terminals[index].resize(rows, cols);
                self.state.pane = Some(Pane {
                    screen: self.terminals[index].screen.clone(),
                    focused,
                });
            }
            None => self.state.pane = None,
        }
    }

    /// A key while the terminal has focus; false when it is not for the terminal.
    pub(super) fn pane_key(&mut self, stroke: &Stroke) -> bool {
        let Some(index) = self
            .shown
            .filter(|_| self.state.pane.as_ref().is_some_and(|p| p.focused))
        else {
            return false;
        };
        if let Some(bytes) = key_bytes(stroke) {
            self.terminals[index].write(&bytes);
        }
        true
    }

    /// Pasted text goes to a focused terminal.
    pub(super) fn pane_paste(&mut self, text: &str) -> bool {
        let Some(index) = self
            .shown
            .filter(|_| self.state.pane.as_ref().is_some_and(|p| p.focused))
        else {
            return false;
        };
        self.terminals[index].write(text.as_bytes());
        true
    }
}
