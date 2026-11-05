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
use anysola_core::tool::question::QuestionReply;
use serde_json::json;

use crate::cli::{Format, RunArgs};
use crate::ui;
use input::Attachment;
use render::{Mode, PERMISSION_REJECTED, QUESTION_CANCELLED, Renderer};

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
    let mcp = anysola_mcp::Runtime::attach(&core);
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
        let credits = match core.set_budget(&session_id, Some(limit)) {
            Ok(credits) => credits,
            Err(error) => {
                let message = format!("Cannot set the budget ({error}).");
                return report(mode, &message, &session_id);
            }
        };
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
        Prompt {
            text,
            files,
            skills: Vec::new(),
        },
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
    anysola_core::mcp::Mcp::shutdown(mcp.as_ref()).await;
    core.shutdown();
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
    let cancelled_form = Arc::new(AtomicBool::new(false));
    let consumer = tokio::spawn(consume(
        core.clone(),
        events,
        renderer,
        auto,
        cancelled_form.clone(),
    ));
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
    if cancelled_form.load(Ordering::Relaxed) {
        code = code.max(1);
    }
    match exit.load(Ordering::Relaxed) {
        0 => code,
        signal => signal,
    }
}

/// Follows the session's events until its turn ends. Nobody can answer here: permission asks (of
/// the session and its subagents) are rejected with feedback the model continues on (or approved
/// once with `--auto`), questions are cancelled with feedback, and forms MCP servers ask for are
/// cancelled, which fails the run.
async fn consume(
    core: Core,
    mut events: tokio::sync::broadcast::Receiver<Event>,
    mut renderer: Renderer,
    auto: bool,
    cancelled_form: Arc<AtomicBool>,
) -> Renderer {
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return renderer,
        };
        // Asks of any session in this process (the run's and its subagents') are answered here.
        if let Event::QuestionAsked { request_id, .. } = &event {
            core.reply_question(
                request_id,
                QuestionReply::Dismissed(Some(QUESTION_CANCELLED.into())),
            );
            continue;
        }
        if let Event::FormAsked { form } = &event {
            core.reply_form(&form.id, anysola_core::mcp::FormReply::Cancelled);
            cancelled_form.store(true, Ordering::Relaxed);
            continue;
        }
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
/// terminal) interrupts it too (exit 1); a second SIGTERM exits at once. Either way the teardown
/// gets 10 s before the process reports what is still pending and exits 1.
async fn signals(
    core: Core,
    session_id: String,
    user_interrupt: Arc<AtomicBool>,
    exit: Arc<AtomicI32>,
) {
    #[cfg(unix)]
    {
        use std::io::IsTerminal;
        use tokio::signal::unix::{SignalKind, signal};
        let (Ok(mut int), Ok(mut term)) = (
            signal(SignalKind::interrupt()),
            signal(SignalKind::terminate()),
        ) else {
            return;
        };
        // Listening for SIGHUP replaces its default action (and nohup's ignore), so only a
        // terminal gets the listener.
        let tty = std::io::stdin().is_terminal() || std::io::stdout().is_terminal();
        let mut hup = tty.then(|| signal(SignalKind::hangup()).ok()).flatten();
        let mut deadline = None;
        loop {
            let code = tokio::select! {
                _ = int.recv() => 130,
                _ = term.recv() => 1,
                Some(_) = async { hup.as_mut()?.recv().await } => 1,
                () = async { deadline.as_mut().expect("guarded").await }, if deadline.is_some() => {
                    exit_pending(&format!("shutdown did not finish within {}s", SHUTDOWN_DEADLINE.as_secs()));
                }
            };
            if deadline.is_some() {
                if code == 130 {
                    std::process::exit(130);
                }
                exit_pending("received SIGTERM again during shutdown");
            }
            exit.store(code, Ordering::Relaxed);
            user_interrupt.store(true, Ordering::Relaxed);
            core.interrupt(&session_id);
            deadline = Some(Box::pin(tokio::time::sleep(SHUTDOWN_DEADLINE)));
        }
    }
    #[cfg(not(unix))]
    {
        let mut deadline = None;
        loop {
            tokio::select! {
                result = tokio::signal::ctrl_c() => if result.is_err() { return },
                () = async { deadline.as_mut().expect("guarded").await }, if deadline.is_some() => {
                    exit_pending(&format!("shutdown did not finish within {}s", SHUTDOWN_DEADLINE.as_secs()));
                }
            }
            if exit.swap(130, Ordering::Relaxed) == 130 {
                std::process::exit(130);
            }
            user_interrupt.store(true, Ordering::Relaxed);
            core.interrupt(&session_id);
            deadline = Some(Box::pin(tokio::time::sleep(SHUTDOWN_DEADLINE)));
        }
    }
}

fn exit_pending(reason: &str) -> ! {
    let pending = anysola_core::teardown::pending();
    eprint!("{}", anysola_core::teardown::report(reason, &pending));
    std::process::exit(1);
}
