//! Durations, instants and cron expressions as 0.5.1's schedules read them (Effect 4's
//! `Duration.fromInput` and `Cron`), with jiff doing the time-zone arithmetic.

use std::collections::BTreeSet;
use std::str::FromStr;

use jiff::civil::{Date, DateTime};
use jiff::tz::{Disambiguation, TimeZone};
use jiff::{Timestamp, ToSpan};

/// Milliseconds in a duration such as `"90 seconds"` or `"1.5 hours"`; `None` when it is not one.
pub fn duration_millis(value: &str) -> Option<f64> {
    let (number, unit) = value.split_once(char::is_whitespace)?;
    let unit = unit.trim_start();
    let digits = number.strip_prefix('-').unwrap_or(number);
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, "0"));
    let numeric = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !numeric(whole) || !numeric(fraction) || (digits.contains('.') && fraction.is_empty()) {
        return None;
    }
    let scale = match unit {
        "nano" | "nanos" => 1e-6,
        "micro" | "micros" => 1e-3,
        "milli" | "millis" => 1.0,
        "second" | "seconds" => 1e3,
        "minute" | "minutes" => 60e3,
        "hour" | "hours" => 3_600e3,
        "day" | "days" => 86_400e3,
        "week" | "weeks" => 604_800e3,
        _ => return None,
    };
    let millis = number.parse::<f64>().ok()? * scale;
    millis.is_finite().then_some(millis)
}

/// The zone named `tz`, or the system's.
pub fn zone(tz: Option<&str>) -> Result<TimeZone, String> {
    match tz {
        None => Ok(TimeZone::system()),
        Some(name) => TimeZone::get(name).map_err(|_| format!("Unknown time zone: {name}")),
    }
}

/// The zone's IANA name as stored with a schedule.
pub fn zone_name(zone: &TimeZone) -> String {
    zone.iana_name().unwrap_or("UTC").to_string()
}

fn has_offset(input: &str) -> bool {
    let bytes = input.as_bytes();
    let digit_before = |at: usize| at > 0 && bytes[at - 1].is_ascii_digit();
    if input.ends_with(['z', 'Z']) {
        return digit_before(bytes.len() - 1);
    }
    for len in [6, 5] {
        if bytes.len() < len {
            continue;
        }
        let at = bytes.len() - len;
        let tail = &input[at..];
        let sign = tail.starts_with(['+', '-']);
        let shaped = if len == 6 {
            tail.as_bytes()[3] == b':'
                && tail[1..3]
                    .bytes()
                    .chain(tail[4..].bytes())
                    .all(|b| b.is_ascii_digit())
        } else {
            tail[1..].bytes().all(|b| b.is_ascii_digit())
        };
        if sign && shaped && digit_before(at) {
            return true;
        }
    }
    false
}

/// An instant with an offset, or a wall-clock time in `zone` (`"2026-10-27T09:00"`).
pub fn parse_at(input: &str, zone: &TimeZone) -> Option<i64> {
    if has_offset(input) {
        return Timestamp::from_str(input).ok().map(|t| t.as_millisecond());
    }
    let civil = DateTime::from_str(input)
        .or_else(|_| Date::from_str(input).map(|d| d.to_datetime(jiff::civil::Time::midnight())))
        .ok()?;
    zone.to_ambiguous_zoned(civil)
        .disambiguate(Disambiguation::Compatible)
        .ok()
        .map(|z| z.timestamp().as_millisecond())
}

/// `instant` as an ISO string with the zone's offset.
pub fn format_in(millis: i64, tz: &str) -> String {
    let Ok(timestamp) = Timestamp::from_millisecond(millis) else {
        return String::new();
    };
    match TimeZone::get(tz) {
        Ok(zone) => timestamp
            .to_zoned(zone)
            .strftime("%Y-%m-%dT%H:%M:%S%.3f%:z")
            .to_string(),
        Err(_) => format_utc(millis),
    }
}

pub fn format_utc(millis: i64) -> String {
    Timestamp::from_millisecond(millis)
        .map(|t| t.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
        .unwrap_or_default()
}

/// A parsed cron expression: five fields, or six with seconds first. An empty set is a wildcard.
#[derive(Debug, Clone, PartialEq)]
pub struct Cron {
    pub seconds: BTreeSet<u8>,
    minutes: BTreeSet<u8>,
    hours: BTreeSet<u8>,
    days: BTreeSet<u8>,
    months: BTreeSet<u8>,
    weekdays: BTreeSet<u8>,
    /// Day-of-month and weekday must both match (one of them was a stepped wildcard).
    and: bool,
}

struct Field {
    min: u8,
    max: u8,
    aliases: &'static [(&'static str, u8)],
    weekday: bool,
}

const MONTHS: &[(&str, u8)] = &[
    ("jan", 1),
    ("feb", 2),
    ("mar", 3),
    ("apr", 4),
    ("may", 5),
    ("jun", 6),
    ("jul", 7),
    ("aug", 8),
    ("sep", 9),
    ("oct", 10),
    ("nov", 11),
    ("dec", 12),
];
const WEEKDAYS: &[(&str, u8)] = &[
    ("sun", 0),
    ("mon", 1),
    ("tue", 2),
    ("wed", 3),
    ("thu", 4),
    ("fri", 5),
    ("sat", 6),
];

fn value(text: &str, aliases: &[(&str, u8)]) -> Option<u32> {
    let lower = text.to_lowercase();
    if let Some((_, v)) = aliases.iter().find(|(name, _)| *name == lower) {
        return Some(u32::from(*v));
    }
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

/// One field, as Effect's `parseSegment`: `(values, wildcard)`.
fn segment(input: &str, field: &Field) -> Result<(BTreeSet<u8>, bool), String> {
    let mut values = BTreeSet::new();
    let parts: Vec<&str> = input.split(',').collect();
    let split_step = |part: &str| -> (String, Option<Option<u32>>) {
        match part.split_once('/') {
            Some((raw, step)) => (
                raw.to_string(),
                Some(
                    (!step.is_empty() && step.bytes().all(|b| b.is_ascii_digit()))
                        .then(|| step.parse().ok())
                        .flatten(),
                ),
            ),
            None => (part.to_string(), None),
        }
    };
    let (first_raw, first_step) = split_step(parts[0]);
    let wildcard = first_raw == "*";
    let everything = wildcard && matches!(first_step, None | Some(Some(1)));
    let (min, max) = (u32::from(field.min), u32::from(field.max));
    let mut add = |v: u32| {
        if !everything {
            let v = if field.weekday && v == 7 { 0 } else { v as u8 };
            values.insert(v);
        }
    };
    for (index, part) in parts.iter().enumerate() {
        let (raw, step) = if index == 0 {
            (first_raw.clone(), first_step)
        } else {
            split_step(part)
        };
        let step = match step {
            None => None,
            Some(None) => return Err("Expected step value to be a positive integer".into()),
            Some(Some(0)) => return Err("Expected step value to be greater than 0".into()),
            Some(Some(s)) if s > max => {
                return Err(format!(
                    "Expected step value to be less than or equal to {max}"
                ));
            }
            Some(Some(s)) => Some(s),
        };
        if raw == "*" {
            if index == 0 && matches!(step, None | Some(1)) {
                continue;
            }
            let mut i = min;
            while i <= max {
                add(i);
                i += step.unwrap_or(1);
            }
            continue;
        }
        let (left, right) = match raw.split_once('-') {
            Some((l, r)) => (value(l, field.aliases), Some(value(r, field.aliases))),
            None => (value(&raw, field.aliases), None),
        };
        let left = left.ok_or("Expected a positive integer")?;
        let range = format!("Expected a value between {min} and {max}");
        if left < min || left > max {
            return Err(range);
        }
        let end = match right {
            None => {
                if step.is_none() {
                    left
                } else {
                    max
                }
            }
            Some(right) => {
                let right = right.ok_or("Expected a positive integer")?;
                if right < min || right > max {
                    return Err(range);
                }
                if left > right {
                    return Err("Invalid value range".into());
                }
                right
            }
        };
        let mut i = left;
        while i <= end {
            add(i);
            i += step.unwrap_or(1);
        }
    }
    Ok((values, wildcard))
}

/// How far ahead [`Cron::next_after`] looks before deciding an expression never fires.
const SEARCH_DAYS: i32 = 4_000;

impl Cron {
    pub fn parse(expression: &str) -> Result<Cron, String> {
        let mut fields: Vec<&str> = expression.split_whitespace().collect();
        if fields.len() != 5 && fields.len() != 6 {
            return Err("Invalid number of segments in cron expression".into());
        }
        if fields.len() == 5 {
            fields.insert(0, "0");
        }
        let field = |min, max, aliases, weekday| Field {
            min,
            max,
            aliases,
            weekday,
        };
        let (seconds, _) = segment(fields[0], &field(0, 59, &[], false))?;
        let (minutes, _) = segment(fields[1], &field(0, 59, &[], false))?;
        let (hours, _) = segment(fields[2], &field(0, 23, &[], false))?;
        let (days, days_wild) = segment(fields[3], &field(1, 31, &[], false))?;
        let (months, _) = segment(fields[4], &field(1, 12, MONTHS, false))?;
        let (weekdays, weekdays_wild) = segment(fields[5], &field(0, 7, WEEKDAYS, true))?;
        let and = (days_wild || weekdays_wild) && !days.is_empty() && !weekdays.is_empty();
        Ok(Cron {
            seconds,
            minutes,
            hours,
            days,
            months,
            weekdays,
            and,
        })
    }

    fn day_matches(&self, date: Date) -> bool {
        if !self.months.is_empty() && !self.months.contains(&(date.month() as u8)) {
            return false;
        }
        let day = self.days.contains(&(date.day() as u8));
        let weekday = self
            .weekdays
            .contains(&(date.weekday().to_sunday_zero_offset() as u8));
        match (self.days.is_empty(), self.weekdays.is_empty()) {
            (true, true) => true,
            _ if self.and => (self.days.is_empty() || day) && (self.weekdays.is_empty() || weekday),
            (false, true) => day,
            (true, false) => weekday,
            (false, false) => day || weekday,
        }
    }

    /// The first slot strictly after `now` (epoch ms) in `zone`, or `None` if none comes. Wall
    /// times skipped by a forward clock change resolve later, as Effect's `Cron.next` does; a
    /// repeated wall time resolves to its first occurrence, so a slot already past during the
    /// repeated hour is skipped rather than fired twice.
    pub fn next_after(&self, now: i64, zone: &TimeZone) -> Option<i64> {
        let start = Timestamp::from_millisecond(now)
            .ok()?
            .to_zoned(zone.clone());
        let start = start.datetime();
        let range = |set: &BTreeSet<u8>, max: u8| -> Vec<u8> {
            if set.is_empty() {
                (0..=max).collect()
            } else {
                set.iter().copied().collect()
            }
        };
        let (hours, minutes, seconds) = (
            range(&self.hours, 23),
            range(&self.minutes, 59),
            range(&self.seconds, 59),
        );
        let mut date = start.date();
        for _ in 0..SEARCH_DAYS {
            if self.day_matches(date) {
                for &h in &hours {
                    for &m in &minutes {
                        for &s in &seconds {
                            let civil = date.at(h as i8, m as i8, s as i8, 0);
                            if civil <= start {
                                continue;
                            }
                            let at = zone
                                .to_ambiguous_zoned(civil)
                                .disambiguate(Disambiguation::Compatible)
                                .ok()?
                                .timestamp()
                                .as_millisecond();
                            if at > now {
                                return Some(at);
                            }
                        }
                    }
                }
            }
            date = date.checked_add(1.day()).ok()?;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> i64 {
        Timestamp::from_str(text).unwrap().as_millisecond()
    }

    fn next(cron: &str, now: &str, tz: &str) -> String {
        let zone = TimeZone::get(tz).unwrap();
        let next = Cron::parse(cron)
            .unwrap()
            .next_after(at(now), &zone)
            .unwrap();
        format_utc(next)
    }

    #[test]
    fn durations() {
        assert_eq!(duration_millis("90 seconds"), Some(90_000.0));
        assert_eq!(duration_millis("1 hour"), Some(3_600_000.0));
        assert_eq!(duration_millis("1.5 minutes"), Some(90_000.0));
        assert_eq!(duration_millis("2 weeks"), Some(1_209_600_000.0));
        assert_eq!(duration_millis("-1 minute"), Some(-60_000.0));
        for invalid in ["5m", "5 mins", "five minutes", "1. hour", "1 Hour", ""] {
            assert_eq!(duration_millis(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn instants_with_and_without_offsets() {
        let berlin = TimeZone::get("Europe/Berlin").unwrap();
        assert_eq!(
            parse_at("2026-10-27T09:00", &berlin),
            Some(at("2026-10-27T08:00:00Z"))
        );
        assert_eq!(
            parse_at("2026-10-27T09:00:00Z", &berlin),
            Some(at("2026-10-27T09:00:00Z"))
        );
        assert_eq!(
            parse_at("2026-10-27T09:00+03:00", &berlin),
            Some(at("2026-10-27T06:00:00Z"))
        );
        assert_eq!(parse_at("tomorrow", &berlin), None);
        assert_eq!(
            format_in(at("2026-10-27T08:00:00Z"), "Europe/Berlin"),
            "2026-10-27T09:00:00.000+01:00"
        );
    }

    #[test]
    fn parse_errors_are_effects() {
        let error = |c: &str| Cron::parse(c).unwrap_err();
        assert_eq!(
            error("* * * *"),
            "Invalid number of segments in cron expression"
        );
        assert_eq!(error("60 * * * *"), "Expected a value between 0 and 59");
        assert_eq!(
            error("*/0 * * * *"),
            "Expected step value to be greater than 0"
        );
        assert_eq!(
            error("*/x * * * *"),
            "Expected step value to be a positive integer"
        );
        assert_eq!(
            error("*/60 * * * *"),
            "Expected step value to be less than or equal to 59"
        );
        assert_eq!(error("5-1 * * * *"), "Invalid value range");
        assert_eq!(error("a * * * *"), "Expected a positive integer");
        let parsed = Cron::parse("0 9 * jan-mar mon-fri").unwrap();
        assert_eq!(parsed.seconds.len(), 1);
        assert_eq!(parsed.weekdays, BTreeSet::from([1, 2, 3, 4, 5]));
        assert_eq!(
            Cron::parse("0 0 * * 7").unwrap().weekdays,
            BTreeSet::from([0])
        );
        assert!(Cron::parse("* * * * * *").unwrap().seconds.is_empty());
    }

    #[test]
    fn next_slots_in_zones() {
        assert_eq!(
            next("0 9 * * 1-5", "2026-11-06T10:00:00Z", "UTC"),
            "2026-11-09T09:00:00.000Z"
        );
        assert_eq!(
            next("0 9 * * *", "2026-11-05T12:00:00Z", "Asia/Tokyo"),
            "2026-11-06T00:00:00.000Z"
        );
        assert_eq!(
            next("*/15 * * * *", "2026-11-05T12:07:30Z", "UTC"),
            "2026-11-05T12:15:00.000Z"
        );
        // Day-of-month or weekday, unless one of them is a stepped wildcard.
        assert_eq!(
            next("0 0 13 * 5", "2026-11-01T00:00:00Z", "UTC"),
            "2026-11-06T00:00:00.000Z"
        );
        assert_eq!(
            next("0 0 */2 * 5", "2026-11-01T00:00:00Z", "UTC"),
            "2026-11-13T00:00:00.000Z"
        );
        let leap = Cron::parse("0 0 29 2 *").unwrap();
        assert_eq!(
            leap.next_after(at("2026-03-01T00:00:00Z"), &TimeZone::UTC)
                .map(format_utc),
            Some("2028-02-29T00:00:00.000Z".into())
        );
        let never = Cron::parse("0 0 31 2 *").unwrap();
        assert_eq!(
            never.next_after(at("2026-03-01T00:00:00Z"), &TimeZone::UTC),
            None
        );
    }

    #[test]
    fn daylight_saving_gaps_and_folds() {
        // New York springs forward at 02:00 on 2026-03-08: 02:30 does not exist and fires at 03:30.
        assert_eq!(
            next("30 2 * * *", "2026-03-08T05:00:00Z", "America/New_York"),
            "2026-03-08T07:30:00.000Z"
        );
        // It falls back at 02:00 on 2026-11-01: 01:30 happens twice and fires once, at the first.
        assert_eq!(
            next("30 1 * * *", "2026-11-01T04:00:00Z", "America/New_York"),
            "2026-11-01T05:30:00.000Z"
        );
        // During the repeated hour (01:10 EST) the 01:30 slot is already past.
        assert_eq!(
            next("30 1 * * *", "2026-11-01T06:10:00Z", "America/New_York"),
            "2026-11-02T06:30:00.000Z"
        );
        assert_eq!(
            next("*/30 * * * *", "2026-11-01T06:10:00Z", "America/New_York"),
            "2026-11-01T07:00:00.000Z"
        );
    }
}
