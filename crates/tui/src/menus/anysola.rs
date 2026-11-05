//! The Anysola account dialogs and texts: `/credits`, `/usage`, `/devices`, `/budget`, the
//! home banners and the exit summary (`spec/tui.md §5`).

use anysola_core::gateway::account::Reply;
use anysola_core::gateway::format::{credits, usage_lines};
use anysola_core::time::short_in;
use chrono::FixedOffset;
use serde_json::Value;

use crate::dialog::{Action, Body, BodyKind, Kind, Line, Opt, Select, Tone, line};
use crate::text::{short_date, title_case};

pub const BILLING_URL: &str = "https://account.anysola.com/billing";
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

fn num_at(value: &Value, path: &[&str]) -> Option<f64> {
    path.iter()
        .try_fold(value, |v, k| v.get(k))
        .and_then(Value::as_f64)
}

/// What `/credits` shows besides `/me`.
pub struct CreditsView<'a> {
    pub me: Result<&'a Value, Option<&'a str>>,
    pub balance: Option<f64>,
    /// `(turns, total)` of the open session.
    pub session: Option<(u64, u64)>,
    pub low: bool,
    pub zone: FixedOffset,
    pub now: u64,
}

pub fn credits_dialog(view: &CreditsView) -> Body {
    let mut rows: Vec<(&str, String)> = Vec::new();
    match view.me {
        Err(error) => rows.push(("Account", error.unwrap_or("Loading…").to_string())),
        Ok(me) => {
            let free = me.get("access").and_then(Value::as_str) == Some("free");
            let plan = str_at(me, &["plan"]).unwrap_or("unknown");
            let suffix = if free { " (free)" } else { "" };
            rows.push(("Plan", format!("{}{suffix}", title_case(plan))));
            if me.get("credits").is_some_and(Value::is_object) {
                let balance = view
                    .balance
                    .or_else(|| num_at(me, &["credits", "available"]))
                    .unwrap_or(0.0);
                rows.push(("Balance", format!("{} credits", credits(balance))));
            }
            if let Some(at) = str_at(me, &["credits", "renewsAt"]).and_then(crate::text::parse_iso)
            {
                rows.push(("Renews", short_date(at, view.now)));
            }
            if me.get("freeQuota").is_some_and(Value::is_object) {
                // [fix] §B4: `/me` sends `turns`.
                let used = num_at(me, &["freeQuota", "turns"])
                    .or_else(|| num_at(me, &["freeQuota", "usedTurns"]))
                    .unwrap_or(0.0);
                let daily = num_at(me, &["freeQuota", "dailyTurns"]).unwrap_or(0.0);
                rows.push(("Free quota", format!("{used}/{daily} turns today")));
            }
            if let Some(at) = str_at(me, &["freeQuota", "resetsAt"]) {
                rows.push(("Resets", short_in(at, &view.zone).unwrap_or_default()));
            }
            if me.get("usageToday").is_some_and(Value::is_object) {
                let requests = num_at(me, &["usageToday", "requests"]).unwrap_or(0.0);
                let spent = num_at(me, &["usageToday", "credits"]).unwrap_or(0.0);
                rows.push((
                    "Today",
                    format!("{requests} requests · {} credits", credits(spent)),
                ));
            }
            if let Some((turns, total)) = view.session.filter(|(t, _)| *t > 0) {
                rows.push((
                    "This session",
                    format!("{turns} turns · {} credits", credits(total as f64)),
                ));
            }
            if let Some(label) = str_at(me, &["event", "label"]) {
                let until = str_at(me, &["event", "until"])
                    .and_then(|at| short_in(at, &view.zone))
                    .map(|at| format!(" until {at}"))
                    .unwrap_or_default();
                rows.push(("Event", format!("{label}{until}")));
            }
        }
    }
    let mut lines: Vec<Line> = rows
        .into_iter()
        .map(|(label, value)| {
            vec![
                (crate::text::pad_right(label, 16), Tone::Muted),
                (value, Tone::Base),
            ]
        })
        .collect();
    lines.push(Vec::new());
    let tone = if view.low { Tone::Warning } else { Tone::Muted };
    lines.push(line(format!("Upgrade or top up: {BILLING_URL}"), tone));
    Body::new(BodyKind::Credits, "Credits", lines)
}

/// `Usage · last <days> days`; `None` while loading.
pub fn usage_dialog(days: u32, reply: Option<&Result<Reply, String>>, zone: FixedOffset) -> Body {
    let lines: Vec<String> = match reply {
        None => vec!["Loading…".into()],
        Some(Ok(r)) if r.ok() => usage_lines(&r.body, &zone),
        Some(other) => vec![error_text(other)],
    };
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

pub const BUDGET_USAGE: &str = "Usage: /budget <credits> | off | default <credits>";
pub const DEFAULT_USAGE: &str = "Usage: /budget default <credits> | off";

fn positive(text: &str) -> Option<u64> {
    text.parse::<u64>().ok().filter(|n| *n >= 1)
}

pub fn parse_budget(arguments: &str) -> Budget {
    let words: Vec<&str> = arguments.split_whitespace().collect();
    match words.as_slice() {
        [] => Budget::Show,
        ["off"] => Budget::Off,
        ["default"] => Budget::ShowDefault,
        ["default", "off"] => Budget::SetDefault(None),
        ["default", n] => positive(n).map_or(Budget::Usage(DEFAULT_USAGE), |n| {
            Budget::SetDefault(Some(n))
        }),
        ["default", ..] => Budget::Usage(DEFAULT_USAGE),
        [n] => positive(n).map_or(Budget::Usage(BUDGET_USAGE), Budget::Set),
        _ => Budget::Usage(BUDGET_USAGE),
    }
}

pub fn budget_text(spent: u64, budget: Option<u64>) -> String {
    match budget {
        Some(b) => format!(
            "Session budget {}/{} credits",
            credits(spent as f64),
            credits(b as f64)
        ),
        None => format!(
            "No budget · {} credits spent. Set one: /budget <credits>",
            credits(spent as f64)
        ),
    }
}

pub fn default_text(limit: Option<u64>) -> String {
    match limit {
        Some(n) => format!(
            "New sessions start with a {} credit budget",
            credits(n as f64)
        ),
        None => "No default budget".into(),
    }
}

/// `Budget reached · <total>/<budget> credits` before a turn that would go over.
pub fn budget_gate(total: u64, budget: u64, prompt: String) -> Select {
    let raised = total.max(budget) + budget;
    let options = vec![
        Opt::new("Continue once", "once").description("Ask again before the next prompt"),
        Opt::new(format!("Add {} credits", credits(budget as f64)), "raise")
            .description(format!("Budget {}", credits(raised as f64))),
        Opt::new("Remove the budget", "remove"),
        Opt::new("Stop", "stop").description("Keep the prompt in the composer"),
    ];
    let title = format!(
        "Budget reached · {}/{} credits",
        credits(total as f64),
        credits(budget as f64)
    );
    let mut select = Select::new(Kind::BudgetGate { prompt }, &title, options);
    select.selected = 0;
    select
}

/// Home footer banners: the event in info colour, low balance in warning colour.
pub fn banners(me: Option<&Value>, low: Option<f64>, zone: FixedOffset) -> Vec<(String, Tone)> {
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
    if let Some(balance) = low {
        out.push((
            format!("Low balance: {} credits left · /credits", credits(balance)),
            Tone::Warning,
        ));
    }
    out
}

/// What the exit summary reports about the session that was open.
pub struct Summary {
    pub title: String,
    /// Empty when no session was open.
    pub id: String,
    pub turns: u64,
    pub spent: u64,
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
    if summary.spent > 0 {
        let turns = if summary.turns == 1 { "turn" } else { "turns" };
        out.push_str(&row(
            "Spent",
            &format!(
                "{} {turns} · {} credits",
                summary.turns,
                credits(summary.spent as f64)
            ),
        ));
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
        assert_eq!(parse_budget("0"), Budget::Usage(BUDGET_USAGE));
        assert_eq!(parse_budget("off"), Budget::Off);
        assert_eq!(parse_budget("default"), Budget::ShowDefault);
        assert_eq!(parse_budget("default 20"), Budget::SetDefault(Some(20)));
        assert_eq!(parse_budget("default off"), Budget::SetDefault(None));
        assert_eq!(parse_budget("default x"), Budget::Usage(DEFAULT_USAGE));
        assert_eq!(budget_text(12, Some(50)), "Session budget 12/50 credits");
    }
}
