//! The `schedule_*` tools of the `opencode` namespace; they act only on the calling session's
//! schedules. Descriptions and schemas are 0.5.1's (`tool/plugin/schedule.ts`).

use std::sync::{Arc, OnceLock};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::codemode::{Called, NamespaceTool};
use crate::storage::ScheduleRow;
use crate::tool::{ToolContext, ToolError};

#[derive(Clone, Copy)]
enum Kind {
    Create,
    List,
    Pause,
    Resume,
    Delete,
}

struct Tool {
    kind: Kind,
    name: String,
    description: String,
    input: Value,
    output: Value,
}

/// The five tools in 0.5.1's registration order.
pub fn tools() -> Vec<Arc<dyn NamespaceTool>> {
    static DEFS: OnceLock<Vec<Value>> = OnceLock::new();
    let defs = DEFS.get_or_init(|| {
        serde_json::from_str(include_str!("tools.json")).expect("embedded schedule tools")
    });
    [
        Kind::Create,
        Kind::List,
        Kind::Pause,
        Kind::Resume,
        Kind::Delete,
    ]
    .into_iter()
    .zip(defs)
    .map(|(kind, def)| {
        let text = |key: &str| def[key].as_str().unwrap_or_default().to_string();
        Arc::new(Tool {
            kind,
            name: text("name"),
            description: text("description"),
            input: def["input_schema"].clone(),
            output: def["output_schema"].clone(),
        }) as Arc<dyn NamespaceTool>
    })
    .collect()
}

impl NamespaceTool for Tool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> &Value {
        &self.input
    }

    fn output_schema(&self) -> Option<&Value> {
        Some(&self.output)
    }

    fn call<'a>(
        &'a self,
        input: Value,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<Called, ToolError>> {
        async move {
            // A request keeps the tools it started with after config reloads, so each call rechecks.
            if !super::enabled(&ctx.host.config()) {
                return Err(ToolError::execution(super::DISABLED));
            }
            let db = ctx.host.db();
            let owned = || {
                let id = input["id"].as_str().unwrap_or_default();
                db.schedule(id)
                    .ok()
                    .flatten()
                    .filter(|row| row.session_id.as_deref() == Some(ctx.session_id.as_str()))
                    .ok_or_else(|| ToolError::execution(format!("Schedule not found: {id}")))
            };
            let failure = ToolError::execution;
            match self.kind {
                Kind::Create => {
                    let (row, message) = super::create(ctx, &input).await.map_err(failure)?;
                    let view = view(&row, message.as_deref());
                    let prefix = if row.status == "done" && view.get("until").is_some() {
                        "Matched immediately"
                    } else {
                        "Scheduled"
                    };
                    let line = format!("{prefix}: {}", summary(&view));
                    Ok(Called::output(view).with_text(line))
                }
                Kind::List => {
                    let now = super::time::format_utc(crate::time::now_ms() as i64);
                    let rows = db.schedules(Some(&ctx.session_id)).unwrap_or_default();
                    let views: Vec<Value> = rows.iter().map(|r| view(r, None)).collect();
                    let mut lines = vec![format!("Now: {now}")];
                    if views.is_empty() {
                        lines.push("No schedules.".into());
                    }
                    lines.extend(views.iter().map(summary));
                    Ok(Called::output(json!({"now": now, "schedules": views}))
                        .with_text(lines.join("\n")))
                }
                Kind::Pause | Kind::Resume => {
                    let row = owned()?;
                    let (row, verb) = match self.kind {
                        Kind::Pause => (super::pause(db, row), "Paused"),
                        _ => (super::resume(db, row), "Resumed"),
                    };
                    let view = view(&row.map_err(failure)?, None);
                    let line = format!("{verb}: {}", summary(&view));
                    Ok(Called::output(view).with_text(line))
                }
                Kind::Delete => {
                    let row = owned()?;
                    db.schedule_delete(&row.id)
                        .map_err(|e| failure(e.to_string()))?;
                    Ok(Called::output(json!({"id": row.id, "removed": true}))
                        .with_text(format!("Deleted schedule {}.", row.id)))
                }
            }
        }
        .boxed()
    }
}

/// A schedule as the model reads it, with times in the schedule's own zone.
fn view(row: &ScheduleRow, last_output: Option<&str>) -> Value {
    let local = |at: Option<i64>| at.map(|at| super::time::format_in(at, &row.tz));
    let mut view = json!({"id": row.id});
    let mut set = |key: &str, value: Option<Value>| {
        if let Some(value) = value {
            view[key] = value;
        }
    };
    set("name", row.name.clone().map(Value::from));
    set("when", Some(row.when.clone()));
    set("tz", Some(json!(row.tz)));
    let check = row.check.as_ref();
    let condition = |mode: &str| {
        check
            .filter(|c| c["mode"] == mode)
            .map(|c| c["condition"].clone())
    };
    set("if", condition("if"));
    set("until", condition("until"));
    set("prompt", Some(row.action["prompt"].clone()));
    set("status", Some(json!(row.status)));
    set("nextDueAt", local(row.next_due_at).map(Value::from));
    set("endsAt", local(row.ends_at).map(Value::from));
    set("remainingRuns", row.remaining_runs.map(Value::from));
    set("runCount", Some(json!(row.run_count)));
    set("lastFiredAt", local(row.last_fired_at).map(Value::from));
    set("lastOutcome", row.last_outcome.clone().map(Value::from));
    set("lastOutput", last_output.map(Value::from));
    view
}

fn condition_text(condition: &Value) -> String {
    if condition["type"] == "shell" {
        return condition["run"].to_string();
    }
    let first = condition["code"]
        .as_str()
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.chars().take(60).collect::<String>())
        .unwrap_or_default();
    format!("code {}", Value::from(first))
}

/// One line per schedule, for models that read tool content rather than output.
fn summary(view: &Value) -> String {
    let when = &view["when"];
    let field = |key: &str| when[key].as_str().unwrap_or_default();
    let trigger = match field("type") {
        "in" => format!("in {}", field("in")),
        "at" => format!("at {}", field("at")),
        "every" => format!("every {}", field("every")),
        _ => format!("cron \"{}\"", field("cron")),
    };
    let mut head = view["id"].as_str().unwrap_or_default().to_string();
    if let Some(name) = view["name"].as_str().filter(|n| !n.is_empty()) {
        head.push_str(&format!(" \"{name}\""));
    }
    let mut parts = vec![format!(
        "{head}: {trigger} ({})",
        view["tz"].as_str().unwrap_or_default()
    )];
    if let Some(until) = view.get("until") {
        parts.push(format!("until {}", condition_text(until)));
    } else if let Some(gate) = view.get("if") {
        parts.push(format!("if {}", condition_text(gate)));
    }
    parts.push(view["status"].as_str().unwrap_or_default().to_string());
    if let Some(next) = view["nextDueAt"].as_str() {
        parts.push(format!("next {next}"));
    }
    let prompt = view["prompt"].as_str().unwrap_or_default();
    let prompt = if prompt.chars().count() > 80 {
        format!("{}…", prompt.chars().take(79).collect::<String>())
    } else {
        prompt.to_string()
    };
    parts.push(format!("prompt: {}", Value::from(prompt)));
    if let Some(output) = view["lastOutput"].as_str().filter(|o| !o.is_empty()) {
        parts.push(format!("output: {}", Value::from(output)));
    }
    parts.join(", ")
}
