//! Account figures as `anysola usage`/`limits`/`status` and the TUI print them.

use serde_json::Value;

use super::limits::{Limits, human_duration, resets_in};
use crate::time::short_in;

fn number(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_f64).filter(|n| n.is_finite())
}

fn text(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// `Math.round(n).toLocaleString("en-US")`.
pub fn grouped(value: f64) -> String {
    let rounded = (value + 0.5).floor() as i64;
    let digits = rounded.unsigned_abs().to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    if rounded < 0 {
        out.insert(0, '-');
    }
    out
}

/// Left-aligned first column, right-aligned numbers, three spaces apart.
fn table<const N: usize>(header: [&str; N], rows: Vec<[String; N]>) -> Vec<String> {
    let all: Vec<[String; N]> = std::iter::once(header.map(str::to_string))
        .chain(rows)
        .collect();
    let widths: Vec<usize> = (0..N)
        .map(|i| all.iter().map(|r| r[i].chars().count()).max().unwrap_or(0))
        .collect();
    all.iter()
        .map(|row| {
            let cells: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(i, cell)| {
                    let pad = " ".repeat(widths[i] - cell.chars().count());
                    if i == 0 {
                        format!("{cell}{pad}")
                    } else {
                        format!("{pad}{cell}")
                    }
                })
                .collect();
            format!("  {}", cells.join("   ").trim_end())
        })
        .collect()
}

const BAR: usize = 10;

/// `████░░░░░░`: `percent` of 10 cells, rounded down.
pub fn bar(percent: u32) -> String {
    let filled = (percent.min(100) as usize * BAR) / 100;
    format!("{}{}", "█".repeat(filled), "░".repeat(BAR - filled))
}

/// `5-hour  ████░░░░░░   42%  resets in 2 h`, one line per window.
pub fn window_lines(limits: &Limits, now: u64) -> Vec<String> {
    [
        ("5-hour", limits.five_hour.at(now)),
        ("Weekly", limits.weekly.at(now)),
    ]
    .into_iter()
    .map(|(label, window)| {
        let resets = match window.resets_at {
            Some(at) if at > now => format!("  resets in {}", human_duration(at - now)),
            _ => String::new(),
        };
        format!(
            "  {label:<8}{}  {:>3}%{resets}",
            bar(window.percent),
            window.percent
        )
    })
    .collect()
}

/// `Banked resets  2 · next expires Nov 20, 12:00 AM`, or `none`.
pub fn bank_line<Tz: chrono::TimeZone>(limits: &Limits, now: u64, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let bank = limits.bank_at(now);
    let Some(first) = bank.first() else {
        return "  Banked resets  none".into();
    };
    let expires = first
        .expires_at
        .and_then(|at| short_in(&crate::time::iso(at), zone))
        .map(|at| format!(" · next expires {at}"))
        .unwrap_or_default();
    format!("  Banked resets  {}{expires}", bank.len())
}

/// `anysola limits`: both windows and the bank, one entry per line.
pub fn limits_lines<Tz: chrono::TimeZone>(limits: &Limits, now: u64, zone: &Tz) -> Vec<String>
where
    Tz::Offset: std::fmt::Display,
{
    let mut lines = vec!["Limits".to_string()];
    lines.extend(window_lines(limits, now));
    if let Some(window) = limits.blocked_by {
        let at = limits.window(window).resets_at;
        lines.push(format!(
            "  {} limit reached. Resets {}.",
            window.label(),
            resets_in(at, now)
        ));
    }
    lines.push(String::new());
    let bank = limits.bank_at(now);
    if bank.is_empty() {
        lines.push("Banked resets: none".into());
    } else {
        lines.push(format!("Banked resets: {}", bank.len()));
        for banked in bank {
            let scope = if banked.both {
                "5-hour and weekly"
            } else {
                "5-hour"
            };
            let expires = banked
                .expires_at
                .and_then(|at| short_in(&crate::time::iso(at), zone))
                .map(|at| format!(" · expires {at}"))
                .unwrap_or_default();
            lines.push(format!("  {scope}{expires}"));
        }
    }
    lines
}

/// `anysola usage` and the TUI's `/usage`: both windows and the bank, then requests by day and
/// by model.
pub fn usage_lines<Tz: chrono::TimeZone>(body: &Value, now: u64, zone: &Tz) -> Vec<String>
where
    Tz::Offset: std::fmt::Display,
{
    let count = |row: &Value| grouped(number(row.get("requests")).unwrap_or(0.0));
    let rows = |key: &str, name: &str| -> Vec<[String; 2]> {
        body.get(key)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        let label = text(item.get(name)).unwrap_or_default();
                        [label.to_string(), count(item)]
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut lines = Vec::new();
    if let Some(limits) = body.get("limits").and_then(Limits::from_wire) {
        lines.push("Limits".to_string());
        lines.extend(window_lines(&limits, now));
        lines.push(bank_line(&limits, now, zone));
        lines.push(String::new());
    }
    let totals = body.get("totals").cloned().unwrap_or(Value::Null);
    lines.push("Totals".to_string());
    lines.extend(table(
        ["", "requests"],
        vec![["all".into(), count(&totals)]],
    ));
    if let Some(rate) = totals.get("cacheHitRate").and_then(Value::as_f64) {
        lines.push(format!(
            "  Cache hit rate {}%",
            (rate * 100.0 + 0.5).floor()
        ));
    }
    lines.extend(["".into(), "By day".into()]);
    lines.extend(table(["date", "requests"], rows("byDay", "date")));
    lines.extend(["".into(), "By model".into()]);
    lines.extend(table(["model", "requests"], rows("byModel", "model")));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouped_numbers() {
        assert_eq!(grouped(1234.0), "1,234");
        assert_eq!(grouped(1234567.4), "1,234,567");
        assert_eq!(grouped(999.5), "1,000");
        assert_eq!(grouped(0.0), "0");
        assert_eq!(grouped(-1500.0), "-1,500");
    }

    #[test]
    fn limit_lines() {
        let now = crate::gateway::limits::parse_time("2026-11-06T10:00:00Z").unwrap();
        let limits = Limits::from_wire(&serde_json::json!({
            "plan": "pro", "allowed": false, "blockedBy": "five_hour",
            "fiveHour": {"percent": 100, "resetsAt": "2026-11-06T12:05:00Z"},
            "weekly": {"percent": 42, "resetsAt": "2026-11-09T14:00:00Z"},
            "bank": [{"id": "b1", "expiresAt": "2026-11-20T00:00:00Z", "scope": "five_hour"}]
        }))
        .unwrap();
        let utc = chrono::Utc;
        assert_eq!(
            limits_lines(&limits, now, &utc),
            vec![
                "Limits",
                "  5-hour  ██████████  100%  resets in 2 h 5 min",
                "  Weekly  ████░░░░░░   42%  resets in 3 d 4 h",
                "  5-hour limit reached. Resets in 2 h 5 min.",
                "",
                "Banked resets: 1",
                "  5-hour · expires Nov 20, 12:00 AM",
            ]
        );
        assert_eq!(
            bank_line(&limits, now, &utc),
            "  Banked resets  1 · next expires Nov 20, 12:00 AM"
        );
        assert_eq!(bar(42), "████░░░░░░");
        assert_eq!(bar(100), "██████████");
    }
}
