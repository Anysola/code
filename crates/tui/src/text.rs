//! Width-aware text helpers and the number/time formats the TUI shows.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub fn width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Cuts `text` to `max` columns by grapheme, ending in `…` when cut.
pub fn truncate(text: &str, max: usize) -> String {
    if width(text) <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let w = width(grapheme);
        if used + w > max - 1 {
            break;
        }
        out.push_str(grapheme);
        used += w;
    }
    out.push('…');
    out
}

/// Cuts `text` to `max` columns without a marker.
pub fn clip(text: &str, max: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let w = width(grapheme);
        if used + w > max {
            break;
        }
        out.push_str(grapheme);
        used += w;
    }
    out
}

/// Keeps both ends of a path-like text: `…/my-ap…` style cuts put `…` in the middle.
/// The path with the home directory shown as `~`.
pub fn abbreviate(directory: &std::path::Path, home: &std::path::Path) -> String {
    let (dir, home) = (directory.display().to_string(), home.display().to_string());
    let sep = std::path::MAIN_SEPARATOR;
    let home = home.trim_end_matches(['/', '\\']);
    // Windows paths compare without case (`c:\users` is `C:\Users`).
    let starts = if cfg!(windows) {
        dir.to_lowercase().starts_with(&home.to_lowercase())
    } else {
        dir.starts_with(home)
    };
    let rest = if starts && !home.is_empty() {
        dir.get(home.len()..)
    } else {
        None
    };
    match rest {
        Some("") => "~".to_string(),
        Some(rest) if rest.starts_with(['/', '\\']) => format!("~{sep}{}", &rest[1..]),
        _ => dir,
    }
}

pub fn truncate_middle(text: &str, max: usize) -> String {
    if width(text) <= max {
        return text.to_string();
    }
    if max <= 1 {
        return "…".chars().take(max).collect();
    }
    let graphemes: Vec<&str> = text.graphemes(true).collect();
    let keep = max - 1;
    let tail = keep / 2;
    let head = keep - tail;
    let start: String = graphemes.iter().take(head).copied().collect();
    let end: String = graphemes[graphemes.len().saturating_sub(tail)..].concat();
    format!("{start}…{end}")
}

/// A location label cut to `max`: the leading directory becomes `…`, then the end is cut.
pub fn path(text: &str, max: usize) -> String {
    if width(text) <= max {
        return text.to_string();
    }
    let rest = text.find(['/', '\\']).map_or(text, |i| &text[i..]);
    truncate(&format!("…{rest}"), max)
}

/// Word-wraps one paragraph to `max` columns; long words are split.
pub fn wrap(text: &str, max: usize) -> Vec<String> {
    let max = max.max(1);
    let mut lines = Vec::new();
    for source in text.split('\n') {
        let mut line = String::new();
        let mut used = 0;
        for word in source.split_word_bounds() {
            let w = width(word);
            if used + w > max && !line.is_empty() {
                lines.push(line.trim_end().to_string());
                line = String::new();
                used = 0;
                if word.trim().is_empty() {
                    continue;
                }
            }
            if w > max {
                for grapheme in word.graphemes(true) {
                    let gw = width(grapheme);
                    if used + gw > max {
                        lines.push(std::mem::take(&mut line));
                        used = 0;
                    }
                    line.push_str(grapheme);
                    used += gw;
                }
                continue;
            }
            line.push_str(word);
            used += w;
        }
        lines.push(line.trim_end().to_string());
    }
    lines
}

pub fn pad_right(text: &str, total: usize) -> String {
    let w = width(text);
    format!("{text}{}", " ".repeat(total.saturating_sub(w)))
}

pub use anysola_core::gateway::format::grouped;

/// `950`, `1.2K`, `3.4M`.
pub fn compact(value: u64) -> String {
    if value >= 999_950 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1000 {
        format!("{:.1}K", value as f64 / 1000.0)
    } else {
        value.to_string()
    }
}

/// `850ms`, `12.4s`, `3m 5s`, `2h 10m`, `1d 4h`.
pub fn duration(ms: u64) -> String {
    if ms < 1000 {
        return format!("{ms}ms");
    }
    if ms < 60_000 {
        return format!("{:.1}s", ms as f64 / 1000.0);
    }
    let seconds = ms / 1000;
    if seconds < 3600 {
        return format!("{}m {}s", seconds / 60, seconds % 60);
    }
    if seconds < 86_400 {
        return format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60);
    }
    format!("{}d {}h", seconds / 86_400, (seconds % 86_400) / 3600)
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// The first graphemes of `text` that fit in `max` cells.
fn take_start(text: &str, max: usize) -> String {
    let mut out = String::new();
    for grapheme in text.graphemes(true) {
        if width(&out) + width(grapheme) > max {
            break;
        }
        out.push_str(grapheme);
    }
    out
}

/// A path cut to `max` cells keeping the root and the basename: `/…/dir…/parent/name`.
pub fn truncate_file_path(value: &str, max: usize) -> String {
    if width(value) <= max {
        return value.to_string();
    }
    // Windows paths: the same cut over `\` separators.
    if value.contains('\\') && !value.contains('/') {
        return truncate_file_path(&value.replace('\\', "/"), max).replace('/', "\\");
    }
    let root = if value.starts_with('/') { "/" } else { "" };
    let segments: Vec<&str> = value[root.len()..]
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let basename = segments.last().copied().unwrap_or(value);
    let basename_fit = |room: usize| -> String {
        if width(basename) <= room {
            return basename.to_string();
        }
        let (stem, extension) = match basename.rfind('.') {
            Some(dot) if dot > 0 => basename.split_at(dot),
            _ => (basename, ""),
        };
        if width(extension) + 1 >= room {
            return take_start("…", room);
        }
        format!(
            "{}…{extension}",
            take_start(stem, room - width(extension) - 1)
        )
    };
    if segments.len() < 2 {
        return format!("{root}{}", basename_fit(max.saturating_sub(width(root))));
    }
    let prefix = format!("{root}…/");
    let room = max.saturating_sub(width(&prefix));
    if room == 0 {
        return take_start(&prefix, max);
    }
    let compact = basename_fit(room);
    if compact != basename {
        return prefix + &compact;
    }
    let mut selected = vec![basename.to_string()];
    let mut used = width(&prefix) + width(basename);
    for segment in segments[..segments.len() - 1].iter().rev() {
        let next = width(segment) + 1;
        if used + next > max {
            let available = max.saturating_sub(used + 1);
            if available > 1 {
                selected.insert(0, format!("{}…", take_start(segment, available - 1)));
            }
            break;
        }
        selected.insert(0, segment.to_string());
        used += next;
    }
    prefix + &selected.join("/")
}

/// `18 Nov`, with the year when it is not the current one.
pub fn short_date(ms: u64, now: u64) -> String {
    let (y, m, d) = civil(ms);
    let (now_y, _, _) = civil(now);
    if y == now_y {
        format!("{d} {}", MONTHS[m as usize - 1])
    } else {
        format!("{d} {} {y}", MONTHS[m as usize - 1])
    }
}

/// `now`, `40m ago`, `5h ago`, `2d ago`, then the date.
pub fn ago(ms: u64, now: u64) -> String {
    let minutes = now.saturating_sub(ms) / 60_000;
    match minutes {
        0 => "now".into(),
        1..60 => format!("{minutes}m ago"),
        60..1440 => format!("{}h ago", minutes / 60),
        1440..10_080 => format!("{}d ago", minutes / 1440),
        _ => short_date(ms, now),
    }
}

/// `text` without ANSI escape sequences (CSI and OSC).
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for next in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&next) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(next) = chars.next() {
                    if next == '\x07' || (next == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Locale short time in the viewer's timezone (`3:04 PM`; the 0.5.1 default locale is en-US).
pub fn time(ms: u64) -> String {
    local(ms).format("%-I:%M %p").to_string()
}

/// `<time> · <locale date>` (`3:04 PM · 11/5/2026`).
pub fn datetime(ms: u64) -> String {
    format!("{} · {}", time(ms), local(ms).format("%-m/%-d/%Y"))
}

fn local(ms: u64) -> chrono::DateTime<chrono::Local> {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .unwrap_or_default()
        .with_timezone(&chrono::Local)
}

/// UTC calendar date of a millisecond timestamp.
fn civil(ms: u64) -> (i64, u32, u32) {
    let days = (ms / 86_400_000) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Milliseconds of an ISO-8601 UTC timestamp (`2026-11-23T00:00:00Z`, fraction optional).
pub fn parse_iso(text: &str) -> Option<u64> {
    let (date, time) = text.split_once('T').unwrap_or((text, "00:00:00"));
    let mut parts = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    let time = time.trim_end_matches('Z');
    let time = time.split(['+']).next().unwrap_or(time);
    let mut clock = time.split(':');
    let h: i64 = clock.next()?.parse().ok()?;
    let min: i64 = clock.next().unwrap_or("0").parse().ok()?;
    let sec: f64 = clock.next().unwrap_or("0").parse().ok()?;
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let ms = ((days * 86_400 + h * 3600 + min * 60) as f64 + sec) * 1000.0;
    (ms >= 0.0).then_some(ms as u64)
}

pub fn title_case(id: &str) -> String {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn windows_paths_keep_their_separators() {
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(
            super::abbreviate(
                std::path::Path::new("/home/u/proj/src"),
                std::path::Path::new("/home/u")
            ),
            format!("~{sep}proj/src")
        );
        let cut = super::truncate_file_path(r"C:\Users\me\proj\src\Button.tsx", 24);
        assert!(cut.ends_with(r"\Button.tsx") && !cut.contains('/'), "{cut}");
        assert_eq!(super::path(r"C:\Users\me\proj", 12), r"…\Users\me\…");
    }

    use super::*;

    #[test]
    fn formats() {
        assert_eq!(grouped(12345.4), "12,345");
        assert_eq!(compact(1260), "1.3K");
        assert_eq!(duration(7100), "7.1s");
        assert_eq!(duration(4), "4ms");
        assert_eq!(duration(185_000), "3m 5s");
        let renews = parse_iso("2026-11-23T10:00:00Z").unwrap();
        let now = parse_iso("2026-11-05T00:00:00Z").unwrap();
        assert_eq!(short_date(renews, now), "23 Nov");
        assert_eq!(ago(now - 90_000, now), "1m ago");
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m \x1b]8;;u\x07x"), "red x");
        let stamp = datetime(now);
        let (clock, date) = stamp.split_once(" · ").unwrap();
        assert!(clock.ends_with("AM") || clock.ends_with("PM"));
        assert!(date.ends_with("2026") || date.ends_with("2025"));
        assert_eq!(time(now), clock);
        assert_eq!(path("~/my-app:master", 8), "…/my-ap…");
        assert_eq!(truncate_middle("abcdefghij", 5), "ab…ij");
        assert_eq!(truncate("Fix typo in hello.txt", 16), "Fix typo in hel…");
        assert_eq!(wrap("a bb ccc", 4), vec!["a bb", "ccc"]);
    }
}

#[cfg(test)]
mod path_tests {
    use super::truncate_file_path;

    #[test]
    fn keeps_root_and_basename() {
        let long = "/tmp/claude-1001/-home-ubuntu/baa4cc54-bc0a-55b0-9ae5-1c58ec01d2f3/scratchpad/shots2/home-0.5.1/my-app";
        assert_eq!(
            truncate_file_path(long, 70),
            "/…/baa4cc54-bc0a-55b0-9ae5-1c58ec…/scratchpad/shots2/home-0.5.1/my-app"
        );
        assert_eq!(truncate_file_path("~/my-app", 70), "~/my-app");
        assert_eq!(
            truncate_file_path("/a/averyverylongname.txt", 12),
            "/…/aver….txt"
        );
    }
}
