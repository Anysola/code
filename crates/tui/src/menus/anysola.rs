//! The Anysola account dialogs and texts: `/usage`, `/devices`, `/budget`, the banked-reset
//! offer, the home banners and the exit summary (`spec/tui.md §5`).

use anysola_core::gateway::account::Reply;
use anysola_core::gateway::format::usage_lines;
use anysola_core::gateway::limits::{Limits, Window, WindowId, resets_in};
use anysola_core::time::short_in;
use anysola_core::usage::{MAX_BUDGET, percent};
use chrono::FixedOffset;
use serde_json::Value;

use crate::dialog::{Action, Body, BodyKind, Kind, Opt, Select, Tone, line};

pub const ACCOUNT_URL: &str = "https://account.anysola.com";
pub const FEEDBACK_URL: &str = "https://www.anysola.com/contact";

/// An account call's failure as one line ([fix] §B21: no service hint).
pub fn error_text(reply: &Result<Reply, String>) -> String {
    match reply {
        Err(_) => "Cannot reach Anysola".into(),
        Ok(r) if r.status == 404 => "This server does not offer it yet.".into(),
        Ok(r) => r
            .body
            .get("error")
            .and_then(|e| e.get("message"))
            .or_else(|| r.body.get("message"))
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
            .map_or_else(
                || format!("Request failed (HTTP {})", r.status),
                str::to_string,
            ),
    }
}

fn str_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |v, k| v.get(k))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// What `/usage` shows besides the reply.
pub struct UsageView {
    pub days: u32,
    /// `(turns, percent of the 5-hour window)` of the open session.
    pub session: Option<(u64, f64)>,
    pub zone: FixedOffset,
    pub now: u64,
}

/// `Usage · last <days> days`: both windows, the bank, requests by day and model; `None` while
/// loading.
pub fn usage_dialog(view: &UsageView, reply: Option<&Result<Reply, String>>) -> Body {
    let days = view.days;
    let mut lines: Vec<String> = match reply {
        None => vec!["Loading…".into()],
        Some(Ok(r)) if r.ok() => usage_lines(&r.body, view.now, &view.zone),
        Some(other) => vec![error_text(other)],
    };
    if let Some((turns, used)) = view.session.filter(|(t, _)| *t > 0)
        && reply.is_some_and(|r| r.as_ref().is_ok_and(Reply::ok))
    {
        let at = lines
            .iter()
            .position(String::is_empty)
            .unwrap_or(lines.len());
        let unit = if turns == 1 { "turn" } else { "turns" };
        lines.insert(
            at,
            format!(
                "  This session   {turns} {unit} · {} of the 5-hour window",
                percent(used)
            ),
        );
    }
    let lines = lines
        .into_iter()
        .map(|l| {
            let tone = if l.starts_with("  ") {
                Tone::Base
            } else {
                Tone::Muted
            };
            line(l, tone)
        })
        .collect();
    let mut body = Body::new(
        BodyKind::Usage { days },
        &format!("Usage · last {days} days"),
        lines,
    );
    let other = if days == 7 { "30 days" } else { "7 days" };
    body.actions = vec![Action::right(other, "t")];
    let banked = match reply {
        Some(Ok(r)) if r.ok() => r
            .body
            .get("limits")
            .and_then(Limits::from_wire)
            .is_some_and(|l| !l.bank_at(view.now).is_empty()),
        _ => false,
    };
    if banked {
        body.actions.push(Action::right("use banked reset", "r"));
    }
    body
}

/// `Devices`: one row per signed-in device token.
pub fn devices_dialog(body: &Value, zone: FixedOffset) -> Select {
    let tokens = body
        .get("tokens")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let options = tokens
        .iter()
        .map(|t| {
            let name = str_at(t, &["name"]).unwrap_or("Unnamed device");
            let current = if t.get("current") == Some(&Value::Bool(true)) {
                " (this device)"
            } else {
                ""
            };
            let footer = match str_at(t, &["lastUsedAt"]) {
                Some(at) => format!("used {}", short_in(at, &zone).unwrap_or_default()),
                None => "never used".into(),
            };
            Opt::new(
                format!("{name}{current}"),
                str_at(t, &["id"]).unwrap_or_default(),
            )
            .description(str_at(t, &["client"]).unwrap_or_default())
            .footer(footer)
        })
        .collect();
    let mut select = Select::new(Kind::Devices, "Devices", options);
    select.selected = 0;
    select
}

/// A `Devices` dialog that could not list (loading or the error).
pub fn devices_pending(error: Option<String>) -> Body {
    let text = error.unwrap_or_else(|| "Loading…".into());
    Body::new(BodyKind::Status, "Devices", vec![line(text, Tone::Base)])
}

/// What `/budget <arguments>` asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Budget {
    Show,
    Set(u64),
    Off,
    ShowDefault,
    SetDefault(Option<u64>),
    Usage(&'static str),
}

pub const BUDGET_USAGE: &str =
    "Usage: /budget <percent of the 5-hour window> | off | default <percent>";
pub const DEFAULT_USAGE: &str = "Usage: /budget default <percent of the 5-hour window> | off";

/// A whole percent from 1 to 100.
fn percent_arg(text: &str) -> Option<u64> {
    text.trim_end_matches('%')
        .parse::<u64>()
        .ok()
        .filter(|n| (1..=MAX_BUDGET).contains(n))
}

pub fn parse_budget(arguments: &str) -> Budget {
    let words: Vec<&str> = arguments.split_whitespace().collect();
    match words.as_slice() {
        [] => Budget::Show,
        ["off"] => Budget::Off,
        ["default"] => Budget::ShowDefault,
        ["default", "off"] => Budget::SetDefault(None),
        ["default", n] => percent_arg(n).map_or(Budget::Usage(DEFAULT_USAGE), |n| {
            Budget::SetDefault(Some(n))
        }),
        ["default", ..] => Budget::Usage(DEFAULT_USAGE),
        [n] => percent_arg(n).map_or(Budget::Usage(BUDGET_USAGE), Budget::Set),
        _ => Budget::Usage(BUDGET_USAGE),
    }
}

pub fn budget_text(spent: f64, budget: Option<u64>) -> String {
    match budget {
        Some(b) => format!(
            "Session budget {}/{b}% of the 5-hour window",
            percent(spent)
        ),
        None => format!(
            "No budget · {} of the 5-hour window used. Set one: /budget <percent>",
            percent(spent)
        ),
    }
}

pub fn default_text(limit: Option<u64>) -> String {
    match limit {
        Some(n) => format!("New sessions start with a budget of {n}% of the 5-hour window"),
        None => "No default budget".into(),
    }
}

/// The budget `Add` raises to: one more budget's worth, at most the whole window.
pub fn raised_budget(total: f64, budget: u64) -> u64 {
    ((total.ceil() as u64).max(budget) + budget).min(MAX_BUDGET)
}

/// `Budget reached · <total>/<budget>%` before a turn that would go over.
pub fn budget_gate(total: f64, budget: u64, prompt: String) -> Select {
    let raised = raised_budget(total, budget);
    let mut options =
        vec![Opt::new("Continue once", "once").description("Ask again before the next prompt")];
    if raised > budget {
        options.push(
            Opt::new(format!("Add {}%", raised - budget), "raise")
                .description(format!("Budget {raised}% of the 5-hour window")),
        );
    }
    options.extend([
        Opt::new("Remove the budget", "remove"),
        Opt::new("Stop", "stop").description("Keep the prompt in the composer"),
    ]);
    let title = format!("Budget reached · {}/{budget}%", percent(total));
    let mut select = Select::new(Kind::BudgetGate { prompt }, &title, options);
    select.selected = 0;
    select
}

/// The offer after a refused request: `None` when the bank is empty.
pub fn banked_reset_message(
    message: &str,
    limits: &Limits,
    now: u64,
    zone: FixedOffset,
) -> Option<String> {
    let bank = limits.bank_at(now);
    let first = bank.first()?;
    let expires = first
        .expires_at
        .and_then(|at| short_in(&anysola_core::time::iso(at), &zone))
        .map(|at| format!(" The next one expires {at}."))
        .unwrap_or_default();
    let count = bank.len();
    let plural = if count == 1 { "" } else { "s" };
    let message = message.trim();
    let message = if message.contains("banked reset") {
        message.to_string()
    } else {
        format!("{message} You have {count} banked reset{plural}.")
    };
    Some(format!(
        "{message}{expires} Using one clears the window and sends your prompt again."
    ))
}

pub const NO_BANKED_RESETS: &str = "No banked resets.";

/// The `/reset` confirm text: how many, when the next one expires, and what spending it clears
/// (the one that expires first is spent); `None` when the bank is empty.
pub fn use_reset_message(limits: &Limits, now: u64, zone: FixedOffset) -> Option<String> {
    let bank = limits.bank_at(now);
    let first = bank.first()?;
    let count = bank.len();
    let plural = if count == 1 { "" } else { "s" };
    let expires = first
        .expires_at
        .and_then(|at| short_in(&anysola_core::time::iso(at), &zone))
        .map(|at| format!(" The next one expires {at}."))
        .unwrap_or_default();
    let clears = if first.both {
        "your 5-hour and weekly windows"
    } else {
        "your 5-hour window"
    };
    Some(format!(
        "You have {count} banked reset{plural}.{expires} Using one clears {clears}."
    ))
}

/// `5-hour limit 84% used · resets in 2 h · /usage`.
pub fn limit_banner(window: WindowId, w: Window, now: u64) -> String {
    let resets = w
        .resets_at
        .map(|at| format!(" · resets {}", resets_in(Some(at), now)))
        .unwrap_or_default();
    if w.percent >= 100 {
        format!("{} limit reached{resets} · /usage", window.label())
    } else {
        format!(
            "{} limit {}% used{resets} · /usage",
            window.label(),
            w.percent
        )
    }
}

/// Home footer banners: the event in info colour, windows near their limit in warning colour.
pub fn banners(
    me: Option<&Value>,
    warnings: &[(WindowId, Window)],
    now: u64,
    zone: FixedOffset,
) -> Vec<(String, Tone)> {
    let mut out = Vec::new();
    if let Some(me) = me
        && let Some(label) = str_at(me, &["event", "label"])
    {
        let until = str_at(me, &["event", "until"])
            .and_then(|at| short_in(at, &zone))
            .map(|at| format!(" · until {at}"))
            .unwrap_or_default();
        out.push((format!("{label}{until}"), Tone::Info));
    }
    if let Some((window, w)) = warnings.first() {
        out.push((limit_banner(*window, *w, now), Tone::Warning));
    }
    out
}

/// What the exit summary reports about the session that was open.
pub struct Summary {
    pub title: String,
    /// Empty when no session was open.
    pub id: String,
    pub turns: u64,
    /// Percent of the 5-hour window the session used.
    pub spent: f64,
    /// The status line's windows (`5h 42% · week 18%`), when known.
    pub limits: Option<String>,
    /// The version installed by `/update`, to run next.
    pub updated: Option<String>,
}

/// The lines printed after leaving the alternate screen (with SGR bold/dim).
pub fn exit_summary(summary: &Summary) -> String {
    let mut out = String::new();
    for line in crate::view::WORDMARK {
        out.push_str(&format!("  \x1b[1m{line}\x1b[0m\n"));
    }
    out.push_str(&format!("{}\x1b[90mCode\x1b[0m\n\n", " ".repeat(58)));
    if let Some(version) = &summary.updated {
        out.push_str(&format!("  Run anysola to use {version}\n\n"));
    }
    if summary.id.is_empty() {
        return out;
    }
    let row =
        |label: &str, value: &str| format!("  \x1b[90m{label:<10}\x1b[0m\x1b[1m{value}\x1b[0m\n");
    out.push_str(&row("Session", &summary.title));
    if summary.spent > 0.0 {
        let turns = if summary.turns == 1 { "turn" } else { "turns" };
        out.push_str(&row(
            "Used",
            &format!(
                "{} {turns} · {} of the 5-hour window",
                summary.turns,
                percent(summary.spent)
            ),
        ));
    }
    if let Some(limits) = &summary.limits {
        out.push_str(&row("Limits", limits));
    }
    out.push_str(&row("Continue", &format!("anysola -s {}", summary.id)));
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_arguments() {
        assert_eq!(parse_budget(""), Budget::Show);
        assert_eq!(parse_budget("50"), Budget::Set(50));
        assert_eq!(parse_budget("20%"), Budget::Set(20));
        assert_eq!(parse_budget("0"), Budget::Usage(BUDGET_USAGE));
        assert_eq!(parse_budget("101"), Budget::Usage(BUDGET_USAGE));
        assert_eq!(parse_budget("100"), Budget::Set(100));
        assert_eq!(parse_budget("off"), Budget::Off);
        assert_eq!(parse_budget("default"), Budget::ShowDefault);
        assert_eq!(parse_budget("default 20"), Budget::SetDefault(Some(20)));
        assert_eq!(parse_budget("default off"), Budget::SetDefault(None));
        assert_eq!(parse_budget("default x"), Budget::Usage(DEFAULT_USAGE));
        assert_eq!(
            budget_text(12.0, Some(20)),
            "Session budget 12%/20% of the 5-hour window"
        );
        assert_eq!(raised_budget(21.2, 20), 42);
        assert_eq!(raised_budget(60.0, 60), 100);
        assert_eq!(raised_budget(100.0, 100), 100);
    }
}
