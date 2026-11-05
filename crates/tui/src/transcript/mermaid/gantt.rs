//! `gantt` as labelled tracks under a time axis (merman's terminal layout). The text has one
//! colour, so tracks use merman's dotted variant to keep the bars visible.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, NaiveDateTime};

use super::{Grid, strip_comment, unquote};
use crate::text::width;

const LABEL_GAP: usize = 2;
const MIN_CHART: usize = 24;
const MAX_CHART: usize = 64;
const LAYOUT_WIDTH: usize = 120;

struct Task {
    label: String,
    start: i64,
    end: i64,
    milestone: bool,
}

enum Entry {
    Section(String),
    Task(Task),
}

struct Gantt {
    title: Option<String>,
    axis: String,
    entries: Vec<Entry>,
}

fn parse_date(value: &str, format: &str) -> Result<i64, String> {
    let numeric = value.parse::<f64>().ok().filter(|n| n.is_finite());
    match (format, numeric) {
        ("s" | "X", Some(n)) => return Ok((n * 1000.0) as i64),
        ("x", Some(n)) => return Ok(n as i64),
        _ => {}
    }
    let calendar = ["YYYY-MM-DD", "YYYY-MM-DD HH:mm", "YYYY-MM-DD HH:mm:ss"].contains(&format);
    let parsed = calendar
        .then(|| {
            let value = value.replace('T', " ");
            NaiveDateTime::parse_from_str(&value, "%Y-%m-%d %H:%M:%S")
                .or_else(|_| NaiveDateTime::parse_from_str(&value, "%Y-%m-%d %H:%M"))
                .ok()
                .or_else(|| {
                    NaiveDate::parse_from_str(&value, "%Y-%m-%d")
                        .ok()
                        .and_then(|d| d.and_hms_opt(0, 0, 0))
                })
        })
        .flatten();
    parsed
        .map(|t| t.and_utc().timestamp_millis())
        .ok_or_else(|| format!("Unsupported date \"{value}\" for dateFormat {format}"))
}

fn parse_end(value: &str, start: i64, format: &str) -> Result<i64, String> {
    let lower = value.to_ascii_lowercase();
    for (unit, ms) in [
        ("ms", 1.0),
        ("s", 1_000.0),
        ("m", 60_000.0),
        ("h", 3_600_000.0),
        ("d", 86_400_000.0),
        ("w", 604_800_000.0),
    ] {
        if let Some(n) = lower
            .strip_suffix(unit)
            .and_then(|n| n.trim().parse::<f64>().ok())
        {
            return Ok(start + (n * ms) as i64);
        }
    }
    parse_date(value, format)
}

fn parse(source: &str) -> Result<Gantt, String> {
    let mut gantt = Gantt {
        title: None,
        axis: "%Y-%m-%d".into(),
        entries: Vec::new(),
    };
    let mut format = "YYYY-MM-DD".to_string();
    let mut ends: HashMap<String, i64> = HashMap::new();
    let mut header = false;
    for raw in source.lines() {
        let line = strip_comment(raw);
        let lower = line.to_ascii_lowercase();
        if line.is_empty() || lower.starts_with("acctitle") || lower.starts_with("accdescr") {
            continue;
        }
        if lower == "gantt" {
            if header {
                return Err("Gantt header can only appear once".into());
            }
            header = true;
            continue;
        }
        if !header {
            return Err("Gantt header is required".into());
        }
        let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        match word.to_ascii_lowercase().as_str() {
            "section" if !rest.trim().is_empty() => {
                gantt.entries.push(Entry::Section(unquote(rest)));
                continue;
            }
            "title" => {
                gantt.title = Some(unquote(rest));
                continue;
            }
            "dateformat" => {
                format = rest.trim().to_string();
                continue;
            }
            "axisformat" => {
                gantt.axis = rest.trim().to_string();
                continue;
            }
            "tickinterval" | "excludes" | "todaymarker" => {
                return Err(format!("{word} is not supported"));
            }
            _ => {}
        }
        let (label, spec) = line
            .split_once(':')
            .filter(|(l, _)| !l.trim().is_empty())
            .ok_or("Gantt task needs a label and a schedule")?;
        let mut fields: Vec<&str> = spec
            .split(',')
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .collect();
        let mut milestone = false;
        while let Some(first) = fields.first() {
            let flag = first.to_ascii_lowercase();
            if !["active", "done", "crit", "milestone", "vert"].contains(&flag.as_str()) {
                break;
            }
            milestone |= flag == "milestone";
            fields.remove(0);
        }
        if !(2..=3).contains(&fields.len()) {
            return Err("Gantt tasks require a start and end or duration".into());
        }
        let id = (fields.len() == 3).then(|| fields.remove(0).to_string());
        let start = match fields[0].strip_prefix("after ") {
            Some(ids) => ids
                .split_whitespace()
                .map(|id| {
                    ends.get(id)
                        .copied()
                        .ok_or(format!("Unknown Gantt task id \"{id}\""))
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .max()
                .unwrap_or(0),
            None => parse_date(fields[0], &format)?,
        };
        let end = parse_end(fields[1], start, &format)?;
        if end < start {
            return Err("Gantt task cannot end before it starts".into());
        }
        if let Some(id) = id {
            ends.insert(id, end);
        }
        gantt.entries.push(Entry::Task(Task {
            label: unquote(label),
            start,
            end,
            milestone,
        }));
    }
    if !header {
        return Err("Gantt header is required".into());
    }
    Ok(gantt)
}

fn tick_step(span: i64) -> i64 {
    const STEPS: [i64; 15] = [
        1_000,
        5_000,
        10_000,
        30_000,
        60_000,
        300_000,
        900_000,
        3_600_000,
        21_600_000,
        43_200_000,
        86_400_000,
        172_800_000,
        604_800_000,
        2_592_000_000,
        31_536_000_000,
    ];
    STEPS
        .into_iter()
        .find(|s| *s as f64 >= span as f64 / 4.0)
        .unwrap_or(STEPS[14])
}

fn format_time(ms: i64, format: &str) -> String {
    let Some(t) = DateTime::from_timestamp_millis(ms) else {
        return String::new();
    };
    [
        ("%Y", "%Y"),
        ("%m", "%m"),
        ("%d", "%d"),
        ("%H", "%H"),
        ("%M", "%M"),
        ("%S", "%S"),
    ]
    .iter()
    .fold(format.to_string(), |out, (token, spec)| {
        out.replace(token, &t.format(spec).to_string())
    })
}

pub fn render(source: &str) -> Result<Vec<String>, String> {
    let gantt = parse(source)?;
    if gantt.entries.is_empty() {
        return Err("Gantt chart is empty".into());
    }
    let label = |e: &Entry| match e {
        Entry::Section(s) => s.clone(),
        Entry::Task(t) => t.label.clone(),
    };
    let label_width = gantt
        .entries
        .iter()
        .map(|e| width(&label(e)))
        .max()
        .unwrap_or(0);
    let mut rows = Vec::new();
    let mut body = 0;
    for (i, entry) in gantt.entries.iter().enumerate() {
        if matches!(entry, Entry::Section(_)) && i > 0 {
            body += 1;
        }
        rows.push(body);
        body += 1;
    }
    let tasks: Vec<&Task> = gantt
        .entries
        .iter()
        .filter_map(|e| match e {
            Entry::Task(t) => Some(t),
            Entry::Section(_) => None,
        })
        .collect();
    if tasks.is_empty() {
        let mut grid = Grid::new(label_width, body);
        for (entry, y) in gantt.entries.iter().zip(&rows) {
            grid.text(0, *y, &label(entry));
        }
        return Ok(grid.rows());
    }
    let available = LAYOUT_WIDTH.saturating_sub(label_width + LABEL_GAP);
    let chart = available.clamp(MIN_CHART, MAX_CHART);
    let minimum = tasks.iter().map(|t| t.start).min().unwrap_or(0);
    let maximum = tasks.iter().map(|t| t.end).max().unwrap_or(0);
    let span = (maximum - minimum).max(1);
    let title_height = if gantt.title.is_some() { 2 } else { 0 };
    let x0 = label_width + LABEL_GAP;
    let mut grid = Grid::new(x0 + chart, title_height + 2 + body);
    if let Some(title) = &gantt.title {
        grid.text(x0 + chart.saturating_sub(width(title)) / 2, 0, title);
    }
    let column = |value: i64| {
        (((value - minimum) as f64 / span as f64) * (chart - 1) as f64).round() as usize
    };
    for offset in 0..chart {
        grid.set(x0 + offset, title_height + 1, '─');
    }
    let step = tick_step(span);
    let mut ticks = Vec::new();
    let mut value = (minimum as f64 / step as f64).ceil() as i64 * step;
    while value <= minimum + span {
        ticks.push(value);
        value += step;
    }
    if ticks.is_empty() {
        ticks = vec![minimum, minimum + span];
    }
    for value in ticks {
        let at = column(value);
        let text = format_time(value, &gantt.axis);
        let w = width(&text);
        let lx = (x0 + at)
            .saturating_sub(w / 2)
            .clamp(x0, (x0 + chart).saturating_sub(w).max(x0));
        grid.text(lx, title_height, &text);
        grid.set(x0 + at, title_height + 1, '┬');
    }
    for (entry, row) in gantt.entries.iter().zip(&rows) {
        let y = title_height + 2 + row;
        grid.text(0, y, &label(entry));
        let Entry::Task(task) = entry else { continue };
        let (start, end) = (column(task.start), column(task.end));
        if task.milestone || start == end {
            grid.set(x0 + start, y, '◆');
            continue;
        }
        for offset in 0..chart {
            grid.set(x0 + offset, y, '·');
        }
        for offset in start..=end {
            grid.set(x0 + offset, y, '─');
        }
    }
    Ok(grid.rows())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_under_an_axis() {
        let rows = render(
            "gantt\n  dateFormat s\n  axisFormat %Ss\n  section Old\n  create :crit, a, 0, 15s\n  stream :after a, 5s",
        )
        .unwrap();
        assert_eq!(
            rows[0].trim_start(),
            "00s            05s             10s            15s            20s"
        );
        assert!(rows[1].trim_start().starts_with('┬'));
        assert_eq!(rows[2], "Old");
        assert!(rows[3].starts_with("create  ───"));
        assert!(rows[3].ends_with('·'));
        assert!(rows[4].starts_with("stream  ···"));
        assert!(rows[4].ends_with('─'));
    }

    #[test]
    fn milestones_dependencies_and_sections_only() {
        let rows = render(
            "gantt\n dateFormat YYYY-MM-DD\n task one :first, 2026-08-01, 2d\n deploy :milestone, after first, 0d",
        )
        .unwrap();
        assert!(rows.iter().any(|r| r.contains('◆')));
        assert_eq!(render("gantt\n section Planning").unwrap(), ["Planning"]);
        assert!(render("gantt\n x :after nope, 1d").is_err());
    }
}
