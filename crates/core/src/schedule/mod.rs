//! Scheduled prompts (`experimental.schedules`): a prompt that wakes its session once, at a
//! time, every interval or on a cron, optionally gated by a shell or Code Mode check. Schedules
//! fire only while some `anysola` process runs; the `schedule:leader` lease lets one process of
//! a project fire them.

pub mod check;
pub mod time;
pub mod tools;

use std::sync::{Arc, LazyLock, Weak};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::api::Shared;
use crate::storage::{Db, Guard, ScheduleRow};
use crate::tool::ToolContext;
use check::Outcome;
use time::Cron;

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const MIN_INTERVAL: i64 = MINUTE;
const MIN_CHECK_INTERVAL: i64 = 15_000;
const DEFAULT_UNTIL_AFTER: i64 = 24 * HOUR;
/// A firing this far past its due time is marked late.
const LATE_AFTER: i64 = 60_000;
/// The loop re-reads the table at least this often, so clock jumps and missed doorbells are caught.
const MAX_SLEEP: i64 = 30_000;
const LEADER_TTL: i64 = 2 * MAX_SLEEP;
/// How long a due schedule waits before the flag is read again, while schedules are off.
const DISABLED_RETRY: i64 = HOUR;
/// How long a schedule whose prompt failed waits before trying again.
const FAILED_RETRY: i64 = 5 * MINUTE;
/// How long a schedule whose check failed waits before trying again, up to MAX_CHECK_FAILURES.
const CHECK_FAILED_RETRY: i64 = MINUTE;
const MAX_CHECK_FAILURES: i64 = 3;

pub const DISABLED: &str = "Scheduled tasks are disabled; set experimental.schedules in config";

/// Rings when a schedule is created or resumed, so the loop recomputes its sleep.
static DOORBELL: LazyLock<Notify> = LazyLock::new(Notify::new);

/// Wakes the firing loop, e.g. to see that the process is closing.
pub(crate) fn ring() {
    DOORBELL.notify_waiters();
}

/// Whether config `experimental.schedules` is `true`.
pub fn enabled(config: &crate::config::Config) -> bool {
    config
        .get("experimental")
        .and_then(|e| e.get("schedules"))
        .and_then(Value::as_bool)
        == Some(true)
}

/// The lease key: one leader per project, since each process drives its own project's sessions
/// under that project's config.
fn leader_key(project_id: &str) -> String {
    format!("schedule:leader:{project_id}")
}

fn millis(value: &str) -> Result<i64, String> {
    match time::duration_millis(value) {
        None => Err(format!("Invalid duration: {value}")),
        Some(ms) if ms <= 0.0 => Err(format!("Duration must be positive: {value}")),
        Some(ms) => Ok(ms.round() as i64),
    }
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// The first slot strictly after `now`, or `None` for one-shots.
fn following(row: &ScheduleRow, now: i64) -> Option<i64> {
    match text(&row.when, "type")? {
        "every" => {
            // Keep the phase: skip every missed slot rather than firing them in a burst.
            let every = millis(text(&row.when, "every")?).ok()?;
            let due = row.next_due_at.unwrap_or(now);
            Some(due + ((now - due).div_euclid(every) + 1) * every)
        }
        "cron" => {
            let zone = time::zone(Some(&row.tz)).ok()?;
            Cron::parse(text(&row.when, "cron")?)
                .ok()?
                .next_after(now, &zone)
        }
        _ => None,
    }
}

fn finished(row: &ScheduleRow, next: Option<i64>, remaining: Option<i64>) -> bool {
    match next {
        None => true,
        Some(next) => remaining.is_some_and(|r| r <= 0) || row.ends_at.is_some_and(|e| next > e),
    }
}

fn first_due(
    when: &Value,
    has_check: bool,
    zone: &jiff::tz::TimeZone,
    now: i64,
) -> Result<i64, String> {
    let field = |key: &str| text(when, key).unwrap_or_default();
    match text(when, "type").unwrap_or_default() {
        "in" => Ok(now + millis(field("in"))?),
        "at" => {
            let at = field("at");
            let due = time::parse_at(at, zone).ok_or_else(|| format!("Invalid time: {at}"))?;
            if due <= now {
                return Err(format!("Time is in the past: {at}"));
            }
            Ok(due)
        }
        "every" => {
            let every = millis(field("every"))?;
            let min = if has_check {
                MIN_CHECK_INTERVAL
            } else {
                MIN_INTERVAL
            };
            if every < min {
                return Err(if has_check {
                    "every must be at least 15 seconds".into()
                } else {
                    "every must be at least 1 minute".into()
                });
            }
            Ok(now + every)
        }
        _ => {
            let expression = field("cron");
            let cron = Cron::parse(expression)?;
            if cron.seconds.len() != 1 {
                return Err("cron must fire at most once a minute".into());
            }
            cron.next_after(now, zone)
                .ok_or_else(|| format!("cron never fires: {expression}"))
        }
    }
}

fn ends_at(
    ends: &Value,
    is_until: bool,
    zone: &jiff::tz::TimeZone,
    now: i64,
) -> Result<Option<i64>, String> {
    let after = match text(ends, "after") {
        Some(after) => Some(now + millis(after)?),
        None => None,
    };
    let Some(at) = text(ends, "at") else {
        return Ok(after.or(is_until.then_some(now + DEFAULT_UNTIL_AFTER)));
    };
    let at = time::parse_at(at, zone).ok_or_else(|| format!("Invalid time: {at}"))?;
    Ok(Some(after.map_or(at, |after| at.min(after))))
}

/// The stored `when` from the caller's `{in|at|every|cron, tz?}`.
fn stored_when(input: &Value) -> Result<Value, String> {
    let kinds: Vec<&str> = ["in", "at", "every", "cron"]
        .into_iter()
        .filter(|k| input.get(*k).is_some())
        .collect();
    let [kind] = kinds.as_slice() else {
        return Err("when needs exactly one of in, at, every or cron".into());
    };
    let mut when = json!({"type": kind, *kind: input[*kind]});
    if let Some(tz) = input.get("tz") {
        when["tz"] = tz.clone();
    }
    Ok(when)
}

/// The stored condition from the caller's `{run, is?, matches?, changes?}` or `{code}`.
fn stored_condition(input: &Value) -> Value {
    match input.get("code") {
        Some(code) => json!({"type": "code", "code": code}),
        None => {
            let mut condition = json!({"type": "shell", "run": input["run"]});
            for key in ["is", "matches", "changes"] {
                if let Some(value) = input.get(key) {
                    condition[key] = value.clone();
                }
            }
            condition
        }
    }
}

/// Creates a schedule for the calling session from a `schedule_create` request; the result is
/// the row and, when `until` matched right away, the check's message.
pub async fn create(
    ctx: &ToolContext,
    request: &Value,
) -> Result<(ScheduleRow, Option<String>), String> {
    let config = ctx.host.config();
    if !enabled(&config) {
        return Err(DISABLED.into());
    }
    if !ctx.directory.is_dir() {
        return Err(format!(
            "Session directory does not exist: {}",
            ctx.directory.display()
        ));
    }
    let check = match (request.get("if"), request.get("until")) {
        (Some(_), Some(_)) => return Err("Choose either if or until, not both".into()),
        (Some(c), None) => Some(json!({"mode": "if", "condition": stored_condition(c)})),
        (None, Some(c)) => Some(json!({"mode": "until", "condition": stored_condition(c)})),
        (None, None) => None,
    };
    if let Some(check) = &check {
        check::validate(&check["condition"])?;
    }
    let until = check.as_ref().is_some_and(|c| c["mode"] == "until");
    let when = match request.get("when") {
        Some(when) => stored_when(when)?,
        None if until => json!({"type": "every", "every": "30 seconds"}),
        None => return Err("when is required unless until is set".into()),
    };
    let configured = config.get("timezone").and_then(Value::as_str);
    let zone = time::zone(text(&when, "tz").or(configured))?;
    let now = crate::time::now_ms() as i64;
    let due = first_due(&when, check.is_some(), &zone, now)?;
    let ends = request.get("ends").cloned().unwrap_or(Value::Null);
    let times = ends.get("times").and_then(Value::as_i64);
    if times.is_some_and(|t| t < 1) {
        return Err("ends.times must be at least 1".into());
    }
    let ends_at = ends_at(&ends, until, &zone, now)?;
    if ends_at.is_some_and(|e| due > e) {
        return Err("The schedule ends before it first fires".into());
    }
    let id = crate::ids::schedule();
    let name = text(request, "name").map(str::to_string);
    let mut row = ScheduleRow {
        id: id.clone(),
        session_id: Some(ctx.session_id.clone()),
        name: name.clone(),
        action: json!({"type": "prompt", "prompt": request["prompt"]}),
        check: check.clone(),
        when: when.clone(),
        tz: time::zone_name(&zone),
        status: "active".into(),
        next_due_at: Some(due),
        ends_at,
        remaining_runs: if until { Some(1) } else { times },
        run_count: 0,
        checks: 0,
        failures: 0,
        last_fired_at: None,
        last_message_id: None,
        last_outcome: None,
        retry_at: None,
        state: Value::Null,
        time_created: now,
        time_updated: now,
    };
    let mut matched = None;
    if let Some(check) = &check {
        check::authorize(ctx, name.as_deref(), &when, check).await?;
        let condition = &check["condition"];
        let seed = until || condition["type"] == "code" || condition["changes"] == true;
        if seed {
            let context = check::Run {
                message_id: format!("msg_{id}_check_0"),
                now,
                checks: 0,
            };
            match check::evaluate(ctx, check, &Value::Null, &context).await {
                Outcome::Failed { error, invalid } => {
                    // A code check that fails at creation is broken (e.g. a wrong response field).
                    if invalid || condition["type"] == "code" {
                        return Err(error);
                    }
                }
                Outcome::Checked {
                    wake: true,
                    state,
                    message,
                } if until => {
                    row.state = state;
                    matched = Some(message);
                }
                Outcome::Checked { state, .. } => {
                    row.state = state;
                    row.checks = 1;
                }
            }
        }
    }
    let db = ctx.host.db();
    if let Some(message) = matched {
        row.checks = 1;
        row.status = "done".into();
        row.next_due_at = None;
        row.remaining_runs = Some(0);
        row.run_count = 1;
        row.last_fired_at = Some(now);
        row.last_outcome = Some("fired".into());
        db.schedule_insert(&row).map_err(|e| e.to_string())?;
        return Ok((row, message));
    }
    db.schedule_insert(&row).map_err(|e| e.to_string())?;
    DOORBELL.notify_one();
    Ok((row, None))
}

pub fn pause(db: &Db, row: ScheduleRow) -> Result<ScheduleRow, String> {
    if row.status != "active" {
        return Ok(row);
    }
    let mut paused = row.clone();
    paused.status = "paused".into();
    paused.time_updated = crate::time::now_ms() as i64;
    db.schedule_set_status(&paused, "active")
        .map_err(|e| e.to_string())?;
    reload(db, &row.id)
}

pub fn resume(db: &Db, row: ScheduleRow) -> Result<ScheduleRow, String> {
    if row.status != "paused" {
        return Ok(row);
    }
    let now = crate::time::now_ms() as i64;
    // A one-shot that fell due while paused fires once, late; a repeating schedule resumes at its next slot.
    let next = match row.next_due_at {
        Some(due) if due <= now => following(&row, now).or(Some(due)),
        other => other,
    };
    let done = next.is_none() || finished(&row, next, row.remaining_runs);
    let mut resumed = row.clone();
    resumed.status = if done { "done" } else { "active" }.into();
    resumed.next_due_at = if done { None } else { next };
    resumed.failures = 0;
    resumed.retry_at = None;
    resumed.time_updated = now;
    db.schedule_set_status(&resumed, "paused")
        .map_err(|e| e.to_string())?;
    DOORBELL.notify_one();
    reload(db, &row.id)
}

fn reload(db: &Db, id: &str) -> Result<ScheduleRow, String> {
    db.schedule(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Schedule not found: {id}"))
}

/// What the firing loop needs of the process.
struct Loop {
    shared: Weak<Shared>,
    db: Arc<Db>,
    key: String,
    owner: String,
}

/// Fires due schedules until the process shuts down. Only the lease holder fires, so processes
/// of one project sharing the database never fire a schedule twice.
pub(crate) async fn run(shared: Weak<Shared>) {
    let Some(strong) = shared.upgrade() else {
        return;
    };
    let me = Loop {
        db: strong.db.clone(),
        key: leader_key(&strong.project.id),
        owner: format!("{}:{}", std::process::id(), crate::ids::part()),
        shared,
    };
    drop(strong);
    let mut leading = false;
    loop {
        let wait = me.round(&mut leading).await;
        let closing = me
            .shared
            .upgrade()
            .is_none_or(|s| s.closing.load(std::sync::atomic::Ordering::Relaxed));
        if closing {
            break;
        }
        let _ = tokio::time::timeout(Duration::from_millis(wait as u64), DOORBELL.notified()).await;
    }
    let _ = me.db.lease_release(&me.key, &me.owner);
}

impl Loop {
    /// One pass; returns how long to sleep.
    async fn round(&self, leading: &mut bool) -> i64 {
        let active = self.db.schedules_active().unwrap_or(false);
        if !active {
            if *leading {
                *leading = false;
                let _ = self.db.lease_release(&self.key, &self.owner);
            }
            return MAX_SLEEP;
        }
        let now = crate::time::now_ms() as i64;
        if !self.claim(now) {
            *leading = false;
            return MAX_SLEEP;
        }
        if !*leading {
            *leading = true;
            self.rewake();
        }
        self.tick(now).await;
        match self.db.schedule_next_due(now) {
            Ok(Some(at)) => (at - crate::time::now_ms() as i64).clamp(0, MAX_SLEEP),
            _ => MAX_SLEEP,
        }
    }

    fn claim(&self, now: i64) -> bool {
        self.db
            .lease_claim(&self.key, &self.owner, now, LEADER_TTL)
            .unwrap_or(false)
    }

    /// A crash between admitting a prompt and its session claiming it leaves the session asleep.
    fn rewake(&self) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        for session in self.db.schedule_unclaimed().unwrap_or_default() {
            crate::api::wake(&shared, &session);
        }
    }

    async fn tick(&self, now: i64) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        for row in self.db.schedules_due(now).unwrap_or_default() {
            if !self.claim(crate::time::now_ms() as i64) {
                return;
            }
            if let Err(failure) = step(&shared, row.clone(), now).await {
                let mut row = row;
                row.last_outcome = Some("failed".into());
                row.time_updated = now;
                if failure == Failure::Gone {
                    // The session's folder (e.g. a removed worktree) is gone; pause rather than retry.
                    row.status = "paused".into();
                    row.retry_at = None;
                } else {
                    row.retry_at = Some(now + FAILED_RETRY);
                }
                let _ = self.db.schedule_save(&row, Guard::Active);
            }
        }
    }
}

#[derive(Debug, PartialEq)]
enum Failure {
    Gone,
    Other,
}

async fn step(shared: &Arc<Shared>, mut row: ScheduleRow, now: i64) -> Result<(), Failure> {
    let (Some(due), Some(session_id)) = (row.next_due_at, row.session_id.clone()) else {
        return Ok(());
    };
    let session = shared
        .db
        .session(&session_id)
        .map_err(|_| Failure::Other)?
        .ok_or(Failure::Other)?;
    // Project config decides, as for every session this process drives.
    if !enabled(&shared.loaded().config) {
        row.last_outcome = Some("disabled".into());
        row.retry_at = Some(now + DISABLED_RETRY);
        row.time_updated = now;
        let _ = shared.db.schedule_save(&row, Guard::Active);
        return Ok(());
    }
    if !std::path::Path::new(&session.directory).is_dir() {
        return Err(Failure::Gone);
    }
    if let Some(last) = &row.last_message_id
        && shared.db.inbox_has(last).unwrap_or(false)
    {
        // The wake that followed admission may have been lost; waking again is harmless.
        crate::api::wake(shared, &session_id);
        let next = following(&row, now);
        let done = finished(&row, next, row.remaining_runs);
        row.last_outcome = Some("skipped".into());
        row.retry_at = None;
        row.status = if done { "done" } else { "active" }.into();
        row.next_due_at = if done { None } else { next };
        row.time_updated = now;
        let _ = shared.db.schedule_save(&row, Guard::Active);
        return Ok(());
    }
    fire(shared, &session, row, due, now).await
}

/// Admits the wake prompt as `msg_<schedule>_<run>`, so a retry after a crash finds the earlier
/// admission instead of prompting twice.
/// One firing: its run number, due time and whether it is late.
#[derive(Clone, Copy)]
struct Slot {
    run: i64,
    due: i64,
    late: bool,
}

fn admit(
    shared: &Arc<Shared>,
    row: &ScheduleRow,
    session_id: &str,
    slot: Slot,
    text: String,
    check: Option<Value>,
) -> Result<String, Failure> {
    let Slot { run, due, late } = slot;
    let id = format!("msg_{}_{run}", row.id);
    let known =
        shared.db.inbox_has(&id).unwrap_or(false) || shared.db.message_exists(&id).unwrap_or(false);
    if !known {
        let prompt = crate::api::Prompt {
            text,
            ..Default::default()
        };
        let mut payload = crate::session::inbox::user_data(&prompt);
        let mut schedule = json!({"id": row.id, "run": run, "dueAt": due, "late": late});
        if let Some(name) = &row.name {
            schedule["name"] = json!(name);
        }
        if let Some(check) = check {
            schedule["check"] = check;
        }
        payload["metadata"] = json!({"schedule": schedule});
        shared
            .db
            .inbox_push(session_id, &id, "user", &payload, "steer")
            .map_err(|_| Failure::Other)?;
    }
    crate::api::wake(shared, session_id);
    Ok(id)
}

async fn fire(
    shared: &Arc<Shared>,
    session: &crate::storage::SessionRow,
    mut row: ScheduleRow,
    due: i64,
    now: i64,
) -> Result<(), Failure> {
    let late = now - due >= LATE_AFTER;
    let next = following(&row, now);
    let prompt = row.action["prompt"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    row.time_updated = now;
    let Some(check) = row.check.clone() else {
        let run = row.run_count + 1;
        let id = admit(
            shared,
            &row,
            &session.id,
            Slot { run, due, late },
            prompt,
            None,
        )?;
        let remaining = row.remaining_runs.map(|r| r - 1);
        let done = finished(&row, next, remaining);
        record_fire(
            &mut row,
            Slot { run, due, late },
            id,
            now,
            remaining,
            done,
            next,
        );
        let _ = shared.db.schedule_save(&row, Guard::Any);
        return Ok(());
    };
    let context = check::Run {
        message_id: format!("msg_{}_check_{}", row.id, row.checks + 1),
        now,
        checks: row.checks,
    };
    let ctx = check::unattended(shared, session).map_err(|_| Failure::Other)?;
    let outcome = check::evaluate(&ctx, &check, &row.state, &context).await;
    let until = check["mode"] == "until";
    match outcome {
        Outcome::Failed { error, invalid } => {
            let strikes = row.failures + 1;
            if !invalid && strikes < MAX_CHECK_FAILURES {
                row.failures = strikes;
                row.last_outcome = Some("failed".into());
                row.retry_at = Some(now + CHECK_FAILED_RETRY);
                let _ = shared.db.schedule_save(&row, Guard::Active);
                return Ok(());
            }
            let run = row.run_count + 1;
            let note = if invalid {
                format!("[Schedule paused after invalid check: {error}]")
            } else {
                format!("[Schedule paused after {strikes} failed checks: {error}]")
            };
            let meta = json!({"error": error, "failures": strikes});
            let id = admit(
                shared,
                &row,
                &session.id,
                Slot { run, due, late },
                format!("{prompt}\n\n{note}"),
                Some(meta),
            )?;
            row.status = "paused".into();
            row.failures = strikes;
            row.run_count = run;
            row.last_message_id = Some(id);
            row.last_fired_at = Some(now);
            row.last_outcome = Some("failed".into());
            row.retry_at = None;
            let _ = shared.db.schedule_save(&row, Guard::Any);
        }
        Outcome::Checked {
            wake: false, state, ..
        } => {
            let ended = next.is_none() || row.ends_at.zip(next).is_some_and(|(e, n)| n > e);
            row.checks += 1;
            row.failures = 0;
            row.state = state;
            row.retry_at = None;
            if until && ended {
                let run = row.run_count + 1;
                let text = format!(
                    "{prompt}\n\n[Schedule timed out without matching: {}]",
                    check::describe(&check["condition"])
                );
                let id = admit(
                    shared,
                    &row,
                    &session.id,
                    Slot { run, due, late },
                    text,
                    Some(json!({"expired": true})),
                )?;
                row.status = "done".into();
                row.run_count = run;
                row.last_message_id = Some(id);
                row.last_fired_at = Some(now);
                row.last_outcome = Some("expired".into());
                row.next_due_at = None;
                row.remaining_runs = Some(0);
                let _ = shared.db.schedule_save(&row, Guard::Any);
                return Ok(());
            }
            row.last_outcome = Some("checked".into());
            if ended {
                row.status = "done".into();
                row.next_due_at = None;
            } else {
                row.next_due_at = next;
            }
            let _ = shared.db.schedule_save(&row, Guard::Active);
        }
        Outcome::Checked {
            wake: true,
            state,
            message,
        } => {
            let run = row.run_count + 1;
            let text = match message.as_deref().filter(|m| !m.trim().is_empty()) {
                Some(message) => format!("{prompt}\n\n{message}"),
                None => prompt,
            };
            let meta = message
                .as_ref()
                .map_or(json!({}), |m| json!({"message": m}));
            let id = admit(
                shared,
                &row,
                &session.id,
                Slot { run, due, late },
                text,
                Some(meta),
            )?;
            let remaining = if until {
                Some(0)
            } else {
                row.remaining_runs.map(|r| r - 1)
            };
            let done = until || finished(&row, next, remaining);
            row.checks += 1;
            row.failures = 0;
            row.state = state;
            // Recorded even if a pause landed during the prompt, or the next firing would reuse this id.
            record_fire(
                &mut row,
                Slot { run, due, late },
                id,
                now,
                remaining,
                done,
                next,
            );
            let _ = shared.db.schedule_save(&row, Guard::Any);
        }
    }
    Ok(())
}

fn record_fire(
    row: &mut ScheduleRow,
    slot: Slot,
    id: String,
    now: i64,
    remaining: Option<i64>,
    done: bool,
    next: Option<i64>,
) {
    let Slot { run, late, .. } = slot;
    row.run_count = run;
    row.last_message_id = Some(id);
    row.last_fired_at = Some(now);
    row.last_outcome = Some(if late { "late" } else { "fired" }.into());
    row.retry_at = None;
    row.remaining_runs = remaining;
    if done {
        row.status = "done".into();
    }
    row.next_due_at = if done { None } else { next };
}
