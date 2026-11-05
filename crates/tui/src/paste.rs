//! Pastes that arrive as keystrokes. The Windows console API has no bracketed paste, so Windows
//! Terminal types a paste key by key and each line break would press Enter. Keys that arrive
//! together, faster than anyone types, and break a line are one paste.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// Keys that follow each other closer than this are one burst.
pub const BURST_GAP_MS: u64 = 3;

/// The text a plain key types, if it is one a paste can contain.
fn typed(key: &KeyEvent) -> Option<char> {
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }
    match key.code {
        KeyCode::Enter | KeyCode::Char('\r' | '\n') => Some('\n'),
        KeyCode::Tab => Some('\t'),
        KeyCode::Char(c) => Some(c),
        _ => None,
    }
}

/// Turns each run of typed keys in `burst` that breaks a line before its last key into one
/// `Event::Paste`; everything else passes through. Key releases are dropped.
pub fn coalesce(burst: Vec<Event>) -> Vec<Event> {
    let events: Vec<Event> = burst
        .into_iter()
        .filter(|e| !matches!(e, Event::Key(k) if k.kind == KeyEventKind::Release))
        .collect();
    let mut out = Vec::with_capacity(events.len());
    let mut run: Vec<(Event, char)> = Vec::new();
    let flush = |run: &mut Vec<(Event, char)>, out: &mut Vec<Event>| {
        // A lone Enter, or one that only ends what was typed, is a key press, not a paste.
        let pasted = run.len() > 1 && run[..run.len() - 1].iter().any(|(_, c)| *c == '\n');
        if pasted {
            out.push(Event::Paste(run.iter().map(|(_, c)| *c).collect()));
            run.clear();
        } else {
            out.extend(run.drain(..).map(|(e, _)| e));
        }
    };
    for event in events {
        match &event {
            Event::Key(key) if typed(key).is_some() => {
                let c = typed(key).expect("typed");
                run.push((event, c));
            }
            _ => {
                flush(&mut run, &mut out);
                out.push(event);
            }
        }
    }
    flush(&mut run, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn typed(text: &str) -> Vec<Event> {
        text.chars()
            .map(|c| match c {
                '\n' => key(KeyCode::Enter),
                c => key(KeyCode::Char(c)),
            })
            .collect()
    }

    #[test]
    fn a_typed_multi_line_burst_is_one_paste() {
        assert_eq!(
            coalesce(typed("fn main() {\n    ok\n}")),
            vec![Event::Paste("fn main() {\n    ok\n}".into())]
        );
        // A trailing line break is kept as text when the paste had one inside.
        assert_eq!(
            coalesce(typed("a\nb\n")),
            vec![Event::Paste("a\nb\n".into())]
        );
        assert_eq!(coalesce(typed("ç\nş")), vec![Event::Paste("ç\nş".into())]);
    }

    #[test]
    fn single_keys_and_a_final_enter_stay_keys() {
        assert_eq!(coalesce(typed("\n")), typed("\n"));
        assert_eq!(coalesce(typed("hello\n")), typed("hello\n"));
        assert_eq!(coalesce(typed("abc")), typed("abc"));
    }

    #[test]
    fn modified_keys_split_runs_and_releases_are_dropped() {
        let ctrl = Event::Key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
        let mut release = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        let mut burst = typed("a\nb");
        burst.insert(1, Event::Key(release));
        burst.push(ctrl.clone());
        assert_eq!(coalesce(burst), vec![Event::Paste("a\nb".into()), ctrl]);
    }
}
