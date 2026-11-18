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

/// Effect's `DurationFromString`: `<number> <unit>` with units `nanos`, `micros`, `millis`,
/// `seconds`, `minutes`, `hours`, `days`, `weeks` (singular or plural), e.g. `4 minutes`.
pub fn parse_duration(text: &str) -> Option<std::time::Duration> {
    let (number, unit) = text.split_once(char::is_whitespace)?;
    let value: f64 = number.parse().ok()?;
    let unit = unit.trim_start();
    let nanos = match unit.strip_suffix('s').unwrap_or(unit) {
        "nano" => 1.0,
        "micro" => 1e3,
        "milli" => 1e6,
        "second" => 1e9,
        "minute" => 60e9,
        "hour" => 3600e9,
        "day" => 86_400e9,
        "week" => 604_800e9,
        _ => return None,
    };
    let total = value * nanos;
    (total.is_finite() && total >= 0.0).then(|| std::time::Duration::from_nanos(total as u64))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::parse_duration;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("4 minutes"), Some(Duration::from_secs(240)));
        assert_eq!(parse_duration("1 minute"), Some(Duration::from_secs(60)));
        assert_eq!(
            parse_duration("250 millis"),
            Some(Duration::from_millis(250))
        );
        assert_eq!(
            parse_duration("1.5 seconds"),
            Some(Duration::from_millis(1500))
        );
        assert_eq!(parse_duration("4m"), None);
        assert_eq!(parse_duration("four minutes"), None);
    }
}
