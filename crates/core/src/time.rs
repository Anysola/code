//! Clock helpers. Every stored timestamp is integer epoch milliseconds.

use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `Date.prototype.toDateString()` in local time, e.g. `Wed Nov 04 2026`.
pub fn date_string() -> String {
    chrono::Local::now().format("%a %b %d %Y").to_string()
}

/// `Date.prototype.toISOString()` for an epoch-ms value, e.g. `2026-11-04T10:20:03.620Z`.
pub fn iso(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

pub fn local_year() -> i32 {
    use chrono::Datelike;
    chrono::Local::now().year()
}
