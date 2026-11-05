//! `/worktrees` actions: `new ctrl+a` (name prompt, blank = random) and `delete ctrl+d`
//! (two presses; a worktree with file changes asks again before forcing).

use std::path::PathBuf;

use anysola_core::worktree::RemoveError;

use super::{App, Msg};
use crate::dialog::{Confirm, ConfirmKind, Dialog, Kind, Prompt, PromptKind, Select};
use crate::state::Variant;

impl App {
    pub(super) fn worktree_prompt(&mut self) {
        self.push(Dialog::Prompt(Prompt {
            kind: PromptKind::NewWorktree,
            title: "Name worktree".into(),
            placeholder: "Worktree name".into(),
            text: String::new(),
            busy: None,
        }));
    }

    pub(super) fn create_worktree(&mut self, name: String) {
        let name = if name.is_empty() {
            anysola_core::slug::create()
        } else {
            name
        };
        if let Some(Dialog::Prompt(p)) = self.state.dialogs.last_mut() {
            p.busy = Some("Creating worktree…".into());
        }
        let (core, tx) = (self.core.clone(), self.tx.clone());
        tokio::spawn(async move {
            let created = core.create_worktree(&name).await;
            let _ = tx.send(Msg::Apply(Box::new(move |app: &mut App| {
                app.state.dialogs.pop();
                match created {
                    Ok(_) => app.refresh_worktrees(),
                    Err(error) => app.toast(
                        Some("Creating workspace failed".into()),
                        error,
                        Variant::Error,
                    ),
                }
            })));
        });
    }

    /// `ctrl+m` in a session: the session moves to the chosen worktree.
    pub(super) fn worktree_move(&mut self, value: Option<String>) {
        let session = self.state.session().map(|s| s.id.clone());
        let current = PathBuf::from(&self.state.directory);
        let home = PathBuf::from(&self.state.home);
        let target = value.and_then(|v| super::resolve_directory(&v, &current, &home));
        if let (Some(session), Some(directory)) = (session, target)
            && directory != current
        {
            self.state.dialogs.pop();
            self.relocate(directory, Some(session), true);
        }
    }

    /// `ctrl+d` on a worktree row: the second press deletes it.
    pub(super) fn worktree_delete_key(&mut self) {
        let Some(Dialog::Select(s)) = self.state.dialogs.last_mut() else {
            return;
        };
        let Some(opt) = s.current().cloned() else {
            return;
        };
        if opt.current {
            return;
        }
        if s.confirm.as_deref() != Some(&opt.value) {
            s.confirm = Some(opt.value);
            return;
        }
        s.confirm = None;
        let current = PathBuf::from(&self.state.directory);
        let home = PathBuf::from(&self.state.home);
        if let Some(directory) = super::resolve_directory(&opt.value, &current, &home) {
            self.delete_worktree(directory, false);
        }
    }

    pub(super) fn delete_worktree(&mut self, directory: PathBuf, force: bool) {
        if let Some(Dialog::Select(s)) = self.state.dialogs.last_mut()
            && s.kind == Kind::Worktrees
        {
            s.status = Some((
                format!("Deleting {}", directory.display()),
                crate::dialog::Tone::Muted,
            ));
        }
        let (core, tx) = (self.core.clone(), self.tx.clone());
        tokio::spawn(async move {
            let removed = core.remove_worktree(&directory, force).await;
            let _ = tx.send(Msg::Apply(Box::new(move |app: &mut App| match removed {
                Ok(()) => app.refresh_worktrees(),
                Err(RemoveError::Dirty) => app.push(Dialog::Confirm(Confirm {
                    kind: ConfirmKind::DeleteWorktree {
                        directory: directory.display().to_string(),
                    },
                    title: "Delete worktree?".into(),
                    message: "This worktree has file changes. Do you want to delete it anyway?"
                        .into(),
                    button: "Delete".into(),
                    yes: false,
                })),
                Err(error) => {
                    app.refresh_worktrees();
                    app.toast(
                        Some("Failed to delete worktree".into()),
                        error.to_string(),
                        Variant::Error,
                    );
                }
            })));
        });
    }

    /// Reopens the worktrees dialog with a fresh list, when it is open.
    fn refresh_worktrees(&mut self) {
        if let Some(Dialog::Select(Select {
            kind: Kind::Worktrees,
            ..
        })) = self.state.dialogs.last()
        {
            self.state.dialogs.pop();
            self.open_dialog("session.move", None);
        }
    }
}
