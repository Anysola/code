//! The footer's `◎ goal` status and the `Goal` dialog (`feature-plugins/prompt/goal.tsx`):
//! objective, state and reason, with pause/resume/clear sent as `/goal <action>`.

use anysola_core::session::goal::{Goal, Status};

use super::App;
use crate::dialog::{Action, Body, BodyKind, Dialog, Tone, line};
use crate::keymap::Stroke;

/// `◎ goal`, `◎ goal paused` or `◎ goal met` and its colour.
pub fn status(goal: &Goal) -> (&'static str, &'static str) {
    match goal.status {
        Status::Active => ("◎ goal", "text.muted"),
        Status::Paused => ("◎ goal paused", "text.feedback.warning.base"),
        Status::Complete => ("◎ goal met", "text.feedback.success.base"),
    }
}

fn body(goal: &Goal) -> Body {
    let (word, tone) = match goal.status {
        Status::Active => ("Active", Tone::Muted),
        Status::Paused => ("Paused", Tone::Warning),
        Status::Complete => ("Met", Tone::Success),
    };
    let mut lines = vec![line(goal.objective.clone(), Tone::Base), Vec::new()];
    lines.push(line(format!("◎ {word}"), tone));
    if let Some(reason) = &goal.reason {
        lines.push(line(reason.clone(), Tone::Muted));
    }
    let mut body = Body::new(BodyKind::Goal, "Goal", lines);
    body.actions = match goal.status {
        Status::Active => vec![Action::left("pause", "p")],
        Status::Paused => vec![Action::left("resume", "r")],
        Status::Complete => Vec::new(),
    };
    body.actions.push(Action::left("clear", "x"));
    body
}

impl App {
    pub(super) fn open_goal(&mut self) {
        if let Some(goal) = self.state.session().and_then(|s| s.goal.clone()) {
            self.push(Dialog::Body(body(&goal)));
        }
    }

    /// Keeps an open Goal dialog in step with the goal; closes it once the goal is cleared.
    pub(super) fn refresh_goal_dialog(&mut self) {
        let goal = self.state.session().and_then(|s| s.goal.clone());
        let open = matches!(
            self.state.dialogs.last(),
            Some(Dialog::Body(b)) if b.kind == BodyKind::Goal
        );
        if !open {
            return;
        }
        self.state.dialogs.pop();
        if let Some(goal) = goal {
            self.push(Dialog::Body(body(&goal)));
        }
    }

    pub(super) fn goal_key(&mut self, stroke: &Stroke) {
        let Some(goal) = self.state.session().and_then(|s| s.goal.clone()) else {
            return;
        };
        let action = match (stroke.key.as_str(), goal.status) {
            ("p", Status::Active) => "pause",
            ("r", Status::Paused) => "resume",
            ("x", _) => "clear",
            _ => return,
        };
        if action == "clear" {
            self.state.dialogs.clear();
        }
        let Some(id) = self.state.session().map(|s| s.id.clone()) else {
            return;
        };
        let core = self.core.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = core
                .command(
                    &id,
                    "goal",
                    action,
                    anysola_core::session::TurnOptions::default(),
                )
                .await;
            if let Err(error) = result {
                let message = error.to_string();
                let _ = tx.send(super::Msg::Apply(Box::new(move |app: &mut App| {
                    app.toast(
                        Some("Failed to update the goal".into()),
                        message,
                        crate::state::Variant::Error,
                    )
                })));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn goal(status: Status) -> Goal {
        Goal {
            id: "g".into(),
            objective: "Ship it".into(),
            status,
            reason: Some("Tests pass".into()),
            rounds: 0,
            max_rounds: 50,
            stalls: None,
            boot: String::new(),
            updated_at: 0,
        }
    }

    #[test]
    fn status_words_and_dialog_actions() {
        assert_eq!(status(&goal(Status::Paused)).0, "◎ goal paused");
        assert_eq!(status(&goal(Status::Complete)).0, "◎ goal met");
        let dialog = body(&goal(Status::Active));
        let keys: Vec<_> = dialog.actions.iter().map(|a| a.key.as_str()).collect();
        assert_eq!(keys, ["p", "x"]);
        assert_eq!(dialog.lines[2], line("◎ Active", Tone::Muted));
    }
}
