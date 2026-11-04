//! `anysola run`: one non-interactive turn, for scripts and `Anysola/code-action`.

mod input;
mod render;
mod tools;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::Duration;

use anysola_core::api::{Core, Prompt, Target};
use anysola_core::events::{Event, InterruptReason};
use anysola_core::gateway::error::SessionError;
use anysola_core::model::ModelRef;
use anysola_core::session::gate::Reply;
use anysola_core::session::{Outcome, TurnOptions};
use serde_json::json;

use crate::cli::{Format, RunArgs};
use crate::ui;
use input::Attachment;
use render::{Mode, PERMISSION_REJECTED, Renderer};

const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(10);

/// Reports an error before a renderer exists (`sessionID` may still be unknown).
fn report(mode: Mode, message: &str, session_id: &str) -> i32 {
    if mode == Mode::Json {
        let line = json!({"type": "error", "timestamp": anysola_core::time::now_ms(), "sessionID": session_id,
            "error": {"type": "unknown", "message": message}});
        println!("{}", crate::json(&line));
    } else {
        ui::error(message);
    }
    1
}

fn budget_message(total: u64, limit: u64, session_id: &str) -> String {
    format!(
        "Budget reached: {total}/{limit} credits. Continue with `anysola run -s {session_id} --budget <credits>`."
    )
}

pub async fn run(args: RunArgs) -> i32 {
    let mode = if args.format == Format::Json {
        Mode::Json
    } else {
        Mode::Text
    };
    if args.fork && !args.continue_last && args.session.is_none() {
        return report(mode, "--fork requires --continue or --session", "");
    }
    if let Some(message) = crate::gate::minimum_version().await {
        eprint!("{message}");
        return 1;
    }
    let argument = input::message(&args.message);
    let piped = input::stdin(argument.is_some());
    let Some(message) = input::merge(argument, piped).filter(|m| !m.trim().is_empty()) else {
        return report(mode, "You must provide a message", "");
    };
    let directory = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(_) => return report(mode, "Failed to change directory to .", ""),
    };
    let mut text = message.clone();
    let mut files = Vec::new();
    for file in &args.file {
        match input::attach(file, &directory) {
            Ok(Attachment::Text(inlined)) => text = format!("{text}\n\n{inlined}"),
            Ok(Attachment::Media { mime, data, name }) => files.push((mime, data, name)),
            Err(error) => return report(mode, &error, ""),
        }
    }
    let model = match args.model.as_deref().map(|m| ModelRef::parse(m).ok_or(m)) {
        Some(Err(raw)) => return report(mode, &format!("Invalid model reference: {raw}"), ""),
        Some(Ok(model)) => Some(model),
        None => None,
    };

    let core = match Core::start(&directory).await {
        Ok(core) => core,
        Err(error) => return report(mode, &error.to_string(), ""),
    };
    let target = match (&args.session, args.continue_last) {
        (Some(id), _) => Target::Id(id.clone()),
        (None, true) => Target::Continue,
        (None, false) => Target::New,
    };
    let (session, resumed) = match core.open(&target, args.fork) {
        Ok(opened) => opened,
        Err(error) => return report(mode, &error.to_string(), ""),
    };
    let session_id = session.id.clone();
    if let Some(agent) = &args.agent
        && let Err(error) = core.select_agent(&session_id, agent)
    {
        return report(mode, &error.to_string(), &session_id);
    }
    if let Some(model) = &model
        && let Err(error) = core.select_model(&session_id, model)
    {
        return report(mode, &error.to_string(), &session_id);
    }
    if !resumed && let Some(title) = &args.title {
        let title = if title.is_empty() {
            let cut = anysola_core::text::utf16_prefix(&message, 50);
            if cut.len() < message.len() {
                format!("{cut}...")
            } else {
                cut.to_string()
            }
        } else {
            title.clone()
        };
        let _ = core.set_title(&session_id, &title);
    }
    if let Some(limit) = args.budget {
        let credits = core.set_budget(&session_id, Some(limit));
        if credits.total >= limit {
            return report(
                mode,
                &budget_message(credits.total, limit, &session_id),
                &session_id,
            );
        }
    }
    let start = core.credits(&session_id);
    let auto = args.auto || args.yolo || args.dangerously_skip_permissions;
    let code = drive(
        &core,
        &session_id,
        Prompt { text, files },
        mode,
        args.thinking,
        auto,
        args.budget,
    )
    .await;
    let end = core.credits(&session_id);
    let spent = end.total.saturating_sub(start.total);
    if mode == Mode::Text && spent > 0 {
        let left = end
            .remaining
            .or(start.remaining)
            .map(|r| format!(" · {r} left"))
            .unwrap_or_default();
        let plural = if spent == 1 { "" } else { "s" };
        ui::println(&format!(
            "{}Anysola · {spent} credit{plural}{left}{}",
            ui::DIM,
            ui::NORMAL
        ));
    }
    core.close_permissions();
    anysola_core::tool::shell::kill_background_shells();
    code
}

async fn drive(
    core: &Core,
    session_id: &str,
    prompt: Prompt,
    mode: Mode,
    thinking: bool,
    auto: bool,
    budget: Option<u64>,
) -> i32 {
    let directory = std::env::current_dir().unwrap_or_default();
    let renderer = Renderer::new(mode, thinking, session_id.to_string(), directory);
    let user_interrupt = renderer.user_interrupt.clone();
    let exit = Arc::new(AtomicI32::new(0));
    let events = core.subscribe();
    let consumer = tokio::spawn(consume(core.clone(), events, renderer, auto));
    let signals = tokio::spawn(signals(
        core.clone(),
        session_id.to_string(),
        user_interrupt,
        exit.clone(),
    ));

    let options = TurnOptions {
        stop_at_budget: budget.is_some(),
    };
    let outcome = match core.prompt(session_id, prompt, options).await {
        Ok(outcome) => outcome,
        Err(error) => {
            consumer.abort();
            signals.abort();
            return report(mode, &error.to_string(), session_id);
        }
    };
    let mut renderer = consumer.await.expect("the event consumer does not panic");
    signals.abort();
    let mut code = i32::from(renderer.emitted_error);
    match outcome {
        Outcome::Interrupted(InterruptReason::Budget) => {
            let credits = core.credits(session_id);
            let message = budget_message(credits.total, budget.unwrap_or_default(), session_id);
            renderer.error(&SessionError::new("unknown", message));
            code = 1;
        }
        Outcome::Interrupted(InterruptReason::User) => code = 130,
        Outcome::Failed(_) => code = 1,
        Outcome::Succeeded | Outcome::Interrupted(InterruptReason::Declined) => {}
    }
    match exit.load(Ordering::Relaxed) {
        0 => code,
        signal => signal,
    }
}

/// Follows the session's events until its turn ends. Nobody can answer here: permission asks are
/// rejected with feedback the model continues on (or approved once with `--auto`).
async fn consume(
    core: Core,
    mut events: tokio::sync::broadcast::Receiver<Event>,
    mut renderer: Renderer,
    auto: bool,
) -> Renderer {
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return renderer,
        };
        // Asks of any session in this process (the run's and its subagents') are answered here.
        if let Event::PermissionAsked {
            request_id,
            action,
            resources,
            ..
        } = &event
        {
            if auto {
                core.reply_permission(request_id, Reply::Once);
            } else {
                ui::println(&format!(
                    "{}! {}permission requested: {action} ({}); auto-rejecting",
                    ui::WARNING_BOLD,
                    ui::NORMAL,
                    resources.join(", ")
                ));
                core.reply_permission(request_id, Reply::Reject(Some(PERMISSION_REJECTED.into())));
            }
            continue;
        }
        if event.session_id() != renderer.session_id {
            continue;
        }
        if renderer.handle(&event) {
            return renderer;
        }
    }
}

/// SIGINT interrupts the turn (exit 130; a second SIGINT exits at once). SIGTERM (and SIGHUP on a
/// terminal) interrupts it too and gives the teardown 10 s.
async fn signals(
    core: Core,
    session_id: String,
    user_interrupt: Arc<AtomicBool>,
    exit: Arc<AtomicI32>,
) {
    let interrupt = |code: i32| {
        if exit.swap(code, Ordering::Relaxed) == 130 && code == 130 {
            std::process::exit(130);
        }
        user_interrupt.store(true, Ordering::Relaxed);
        core.interrupt(&session_id);
    };
    #[cfg(unix)]
    {
        use std::io::IsTerminal;
        use tokio::signal::unix::{SignalKind, signal};
        let (Ok(mut int), Ok(mut term), Ok(mut hup)) = (
            signal(SignalKind::interrupt()),
            signal(SignalKind::terminate()),
            signal(SignalKind::hangup()),
        ) else {
            return;
        };
        let tty = std::io::stdin().is_terminal();
        loop {
            tokio::select! {
                _ = int.recv() => interrupt(130),
                _ = term.recv() => break,
                _ = hup.recv(), if tty => break,
            }
        }
        interrupt(1);
        tokio::time::sleep(SHUTDOWN_DEADLINE).await;
        ui::println(&format!(
            "shutdown did not finish within 10s; exiting. Still pending:\n  session {session_id}"
        ));
        std::process::exit(1);
    }
    #[cfg(not(unix))]
    {
        while tokio::signal::ctrl_c().await.is_ok() {
            interrupt(130);
        }
    }
}
