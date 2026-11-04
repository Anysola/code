//! Work a shutdown waits for (running turns, shell commands), so a teardown that outlives its
//! deadline can say what it is stuck on.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

static ENTRIES: Mutex<Vec<(u64, String, Instant)>> = Mutex::new(Vec::new());
static NEXT: AtomicU64 = AtomicU64::new(0);

/// Pending work; it settles when the guard drops.
#[must_use]
pub struct Tracked(u64);

impl Drop for Tracked {
    fn drop(&mut self) {
        entries().retain(|(id, ..)| *id != self.0);
    }
}

fn entries() -> std::sync::MutexGuard<'static, Vec<(u64, String, Instant)>> {
    ENTRIES.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn track(label: impl Into<String>) -> Tracked {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    entries().push((id, label.into(), Instant::now()));
    Tracked(id)
}

/// Pending work, oldest first, with how long each has been pending.
pub fn pending() -> Vec<(String, std::time::Duration)> {
    entries()
        .iter()
        .map(|(_, label, since)| (label.clone(), since.elapsed()))
        .collect()
}

/// `<reason>; exiting. Still pending:` and one `  <seconds>s  <label>` line per entry.
pub fn report(reason: &str, pending: &[(String, std::time::Duration)]) -> String {
    let mut lines = vec![format!("{reason}; exiting. Still pending:")];
    if pending.is_empty() {
        lines.push("  nothing tracked".into());
    }
    for (label, elapsed) in pending {
        lines.push(format!("  {:.1}s  {label}", elapsed.as_secs_f64()));
    }
    lines.push(String::new());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn tracks_until_dropped_and_reports() {
        let shell = track("child 42 running: bash -c sleep 9");
        let label = |p: &[(String, Duration)]| p.iter().any(|(l, _)| l.starts_with("child 42"));
        assert!(label(&pending()));
        drop(shell);
        assert!(!label(&pending()));
        let report = report(
            "shutdown did not finish within 10s",
            &[(
                "session ses_1 running".into(),
                Duration::from_millis(10_260),
            )],
        );
        assert_eq!(
            report,
            "shutdown did not finish within 10s; exiting. Still pending:\n  10.3s  session ses_1 running\n"
        );
        assert_eq!(
            super::report("received SIGTERM again during shutdown", &[]),
            "received SIGTERM again during shutdown; exiting. Still pending:\n  nothing tracked\n"
        );
    }
}
