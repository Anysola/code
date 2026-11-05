//! The `shell` tool: runs a command with the selected shell, captures its combined output to a
//! file and returns the tail; background commands report back through a synthetic message.

mod exec;
mod output;
pub mod parse;
pub mod scan;
mod select;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tokio::sync::oneshot;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::access::{self, Kind};
use super::{
    DefinitionEnv, Tool, ToolContent, ToolContext, ToolDefinition, ToolError, ToolOutput,
    ToolResult, parse_input,
};
use crate::embedded_def;
use crate::permission::PermissionRequest;
use exec::{Job, Running};
use output::Finished;

pub use exec::kill_background_shells;

embedded_def!(def, "shell");

const DEFAULT_TIMEOUT_MS: u64 = 120_000;

const BACKGROUND_INSTRUCTION: &str = "You will be notified automatically when the command finishes. The notification will include the command's output. Unless the user explicitly asks otherwise, DO NOT poll for completion, even if you need the final result to continue. Repeatedly sleeping and reading or searching the output file is polling, not useful work. You may read the current output if it lets you do useful work now, but do not repeatedly check it while waiting for the command to finish. Keep working on anything that does not depend on the result. If you have nothing else to do, end your response; you will be resumed automatically when the command finishes.";

/// A foreground command still running, and how to move it to the background.
struct Foreground {
    id: u64,
    session_id: String,
    command: String,
    move_now: oneshot::Sender<()>,
}

static FOREGROUND: Mutex<Vec<Foreground>> = Mutex::new(Vec::new());

fn foreground() -> std::sync::MutexGuard<'static, Vec<Foreground>> {
    FOREGROUND.lock().unwrap_or_else(|e| e.into_inner())
}

/// Removes its entry when the foreground wait ends, whatever ended it.
struct Blocking(u64);

impl Blocking {
    fn register(session_id: &str, command: &str) -> (Blocking, oneshot::Receiver<()>) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        foreground().push(Foreground {
            id,
            session_id: session_id.to_string(),
            command: command.to_string(),
            move_now: tx,
        });
        (Blocking(id), rx)
    }
}

impl Drop for Blocking {
    fn drop(&mut self) {
        foreground().retain(|entry| entry.id != self.0);
    }
}

/// A background command still running.
#[derive(Debug, Clone)]
pub struct BackgroundShell {
    entry: u64,
    pub session_id: String,
    /// The shell ID the model sees.
    pub id: String,
    pub command: String,
    /// Where its output streams.
    pub file: PathBuf,
    stop: tokio_util::sync::CancellationToken,
}

static BACKGROUND: Mutex<Vec<BackgroundShell>> = Mutex::new(Vec::new());

fn background() -> std::sync::MutexGuard<'static, Vec<BackgroundShell>> {
    BACKGROUND.lock().unwrap_or_else(|e| e.into_inner())
}

/// The commands of the session's background shells that are still running.
pub fn background_commands(session_id: &str) -> Vec<String> {
    background_shells(session_id)
        .into_iter()
        .map(|shell| shell.command)
        .collect()
}

/// The session's background shells that are still running, oldest first.
pub fn background_shells(session_id: &str) -> Vec<BackgroundShell> {
    background()
        .iter()
        .filter(|shell| shell.session_id == session_id)
        .cloned()
        .collect()
}

/// How a background shell ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellEnd {
    Exited(Option<i32>),
    Killed,
    TimedOut,
}

static ENDED: Mutex<Vec<(String, ShellEnd)>> = Mutex::new(Vec::new());

/// How the background shell with this ID ended; `None` while it runs or when unknown.
pub fn background_end(id: &str) -> Option<ShellEnd> {
    ENDED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .rev()
        .find(|(shell, _)| shell == id)
        .map(|(_, end)| *end)
}

/// Ends a background shell by its shell ID; false when none runs.
pub fn kill_background(id: &str) -> bool {
    let found = background().iter().find(|shell| shell.id == id).cloned();
    found.map(|shell| shell.stop.cancel()).is_some()
}

/// Moves the session's running foreground commands to the background (their tool calls return
/// the background result at once); returns their commands.
pub fn background_all(session_id: &str) -> Vec<String> {
    let (matching, rest) = std::mem::take(&mut *foreground())
        .into_iter()
        .partition::<Vec<_>, _>(|entry| entry.session_id == session_id);
    foreground().extend(rest);
    matching
        .into_iter()
        .filter_map(|entry| entry.move_now.send(()).is_ok().then_some(entry.command))
        .collect()
}

/// The name of the shell commands run with (`bash`, `zsh`, `pwsh`, …), for
/// [`DefinitionEnv::shell_name`]. `configured` is the config's `shell`.
pub fn shell_name(configured: Option<&str>) -> String {
    select::name(&select::resolve(configured))
}

/// The shell commands run with and the arguments that make it run `command` (also used for
/// ``!`cmd` `` in command templates).
pub fn invocation(configured: Option<&str>, command: &str) -> (String, Vec<String>) {
    let shell = select::resolve(configured);
    let args = select::args(&shell, command);
    (shell, args)
}

/// `process.platform` as the description names it.
fn os_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
}

pub struct ShellTool;

#[derive(Deserialize)]
struct Input {
    command: String,
    workdir: Option<String>,
    timeout: Option<u64>,
    background: Option<bool>,
}

/// The base environment's value of `name` (case-insensitive on Windows).
fn variable(ctx: &ToolContext, name: &str) -> Option<String> {
    ctx.env
        .iter()
        .find(|(key, _)| {
            let key = key.to_string_lossy();
            if cfg!(windows) {
                key.eq_ignore_ascii_case(name)
            } else {
                key == name
            }
        })
        .map(|(_, value)| value.to_string_lossy().into_owned())
}

fn scan_command(
    ctx: &ToolContext,
    command: &str,
    shell: &str,
    cwd: &Path,
) -> Result<parse::Parsed, ToolError> {
    let home = ctx.paths.home.to_string_lossy();
    let lookup = |name: &str| variable(ctx, name);
    let host = parse::Host {
        home: &home,
        env: &lookup,
    };
    parse::scan(command, shell, &cwd.to_string_lossy(), &host)
        .map_err(|reason| ToolError::unknown(parse::error_text(reason)))
}

/// Checks the working directory and every static `cd` target as external directories, then asks
/// `shell` for the scanned commands. A command the scanner cannot analyze fails before any ask.
async fn authorize(
    ctx: &ToolContext,
    command: &str,
    shell: &str,
    target: &access::Target,
) -> Result<(), ToolError> {
    let parsed = scan_command(ctx, command, shell, &target.absolute)?;
    let directories: Vec<access::Target> = parsed
        .directories
        .iter()
        .map(|dir| access::resolve_at(ctx, &target.absolute, dir, Some(Kind::Directory)))
        .collect();
    let mut targets = vec![target];
    targets.extend(directories.iter());
    access::authorize_external(ctx, &targets).await?;
    if parsed.commands.is_empty() {
        return Ok(());
    }
    let (resources, save) = parsed
        .commands
        .into_iter()
        .map(|c| (c.resource, c.save))
        .unzip();
    ctx.ask(PermissionRequest {
        action: "shell".into(),
        resources,
        save,
        ..Default::default()
    })
    .await
}

/// Checked after approval, which can outlive the directory.
fn check_workdir(path: &Path) -> Result<(), ToolError> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(ToolError::unknown(format!(
            "Working directory is not a directory: {}",
            path.display()
        ))),
        Err(_) => Err(ToolError::unknown(format!(
            "Working directory does not exist: {}",
            path.display()
        ))),
    }
}

fn environment(ctx: &ToolContext) -> Vec<(OsString, OsString)> {
    let mut env: HashMap<OsString, OsString> = ctx.env.iter().cloned().collect();
    let mut set = |key: &str, value: &str| {
        env.insert(key.into(), value.into());
    };
    set("TERM", "xterm-256color");
    set("OPENCODE_TERMINAL", "1");
    set("AGENT", "1");
    set("OPENCODE", "1");
    set("OPENCODE_SESSION_ID", &ctx.session_id);
    env.entry("AI_AGENT".into())
        .and_modify(|value| {
            if value.is_empty() {
                *value = "opencode".into();
            }
        })
        .or_insert_with(|| "opencode".into());
    env.into_iter().collect()
}

fn completed(finished: &Finished) -> ToolOutput {
    ToolOutput {
        content: finished
            .content()
            .into_iter()
            .map(ToolContent::text)
            .collect(),
        metadata: finished.metadata(),
    }
}

fn moved_to_background(id: &str, file: &Path) -> ToolOutput {
    let mut metadata = Map::new();
    metadata.insert("status".into(), "running".into());
    metadata.insert("truncated".into(), false.into());
    metadata.insert("shellID".into(), id.into());
    ToolOutput {
        content: vec![
            ToolContent::text(format!(
                "Command moved to the background (shell ID: {id}).\nOutput is streaming to: {}",
                file.display()
            )),
            ToolContent::text(BACKGROUND_INSTRUCTION),
        ],
        metadata,
    }
}

/// A started command and what its result needs.
struct Started {
    id: String,
    file: PathBuf,
    timeout: u64,
    running: Running,
}

impl ShellTool {
    async fn start(&self, input: &Input, ctx: &ToolContext) -> Result<Started, ToolError> {
        let directory = ctx.directory.to_string_lossy();
        let workdir = input.workdir.as_deref().unwrap_or(&directory);
        let target = access::resolve(ctx, workdir, Some(Kind::Directory));
        let shell = select::resolve(ctx.configured_shell.as_deref());
        authorize(ctx, &input.command, &shell, &target).await?;
        check_workdir(&target.absolute)?;

        let background = input.background == Some(true);
        let default_timeout = if background { 0 } else { DEFAULT_TIMEOUT_MS };
        let timeout = input.timeout.unwrap_or(default_timeout);
        output::sweep(&ctx.paths.data.join("shell"));
        let directory = ctx.paths.shell_dir(&ctx.project_id);
        std::fs::create_dir_all(&directory).map_err(|e| ToolError::unknown(e.to_string()))?;
        let id = crate::ids::shell();
        let file = directory.join(format!("{id}.out"));
        let job = Job {
            program: &shell,
            args: select::args(&shell, &input.command),
            cwd: &target.absolute,
            env: environment(ctx),
            file: &file,
            timeout,
        };
        let running = exec::spawn(job).map_err(|e| ToolError::unknown(e.to_string()))?;
        Ok(Started {
            id,
            file,
            timeout,
            running,
        })
    }
}

impl Tool for ShellTool {
    fn name(&self) -> &'static str {
        "shell"
    }

    fn definition(&self, env: &DefinitionEnv) -> ToolDefinition {
        let def = def();
        ToolDefinition {
            name: self.name().into(),
            description: def
                .description
                .replace("{os}", os_name())
                .replace("{shell}", &env.shell_name),
            input_schema: def.input_schema.clone(),
        }
    }

    fn execute<'a>(&'a self, input: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolResult> {
        async move {
            let input: Input = parse_input(self.name(), &def().input_schema, input)?;
            let started = self.start(&input, ctx).await?;
            if input.background == Some(true) {
                let result = moved_to_background(&started.id, &started.file);
                notify_when_done(started, input.command, ctx);
                return Ok(result);
            }
            let mut started = started;
            let (blocking, moved) = Blocking::register(&ctx.session_id, &input.command);
            let ended = tokio::select! {
                ended = started.running.wait() => Some(ended),
                () = ctx.cancel.cancelled() => None,
                Ok(()) = moved => {
                    drop(blocking);
                    started.running.clear_timeout();
                    let result = moved_to_background(&started.id, &started.file);
                    notify_when_done(started, input.command, ctx);
                    return Ok(result);
                }
            };
            let Started {
                file,
                timeout,
                running,
                ..
            } = started;
            // Dropping `running` on the way out kills a command that is still running.
            let Some(ended) = ended else {
                running.discard(&file);
                return Err(ToolError::interrupted());
            };
            let finished = Finished::read(&file, ended, ctx.limits, timeout)
                .map_err(|e| ToolError::unknown(e.to_string()))?;
            Ok(completed(&finished))
        }
        .boxed()
    }
}

/// Posts the `<shell …>` message into the session once a background command ends. The wait is
/// detached from the turn: interrupting the turn does not stop a background command.
fn notify_when_done(started: Started, command: String, ctx: &ToolContext) {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let notifier = ctx.notifier.clone();
    let session_id = ctx.session_id.clone();
    let limits = ctx.limits;
    let entry = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let stop = started.running.stopper();
    background().push(BackgroundShell {
        entry,
        session_id: session_id.clone(),
        id: started.id.clone(),
        command: command.clone(),
        file: started.file.clone(),
        stop: stop.clone(),
    });
    tokio::spawn(async move {
        let Started {
            id,
            file,
            timeout,
            mut running,
        } = started;
        let ended = running.wait().await;
        let end = match ended.outcome {
            exec::Outcome::TimedOut => ShellEnd::TimedOut,
            _ if stop.is_cancelled() => ShellEnd::Killed,
            exec::Outcome::Exited { code, .. } => ShellEnd::Exited(code),
        };
        ENDED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((id.clone(), end));
        background().retain(|shell| shell.entry != entry);
        let (state, text) = match Finished::read(&file, ended, limits, timeout) {
            Ok(finished) => ("completed", finished.content().join("\n\n")),
            Err(error) => ("error", error.to_string()),
        };
        let text = output::notification(&id, state, &command, &text);
        notifier.synthetic(&session_id, text, Some(command));
    });
}

// The tests drive GNU bash and coreutils.
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use serde_json::json;

    use super::*;
    use crate::tool::test_support::{TestContext, context};

    /// Process tests run one at a time: `kill_background_shells` kills every running command.
    static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn setup(dir: &Path) -> TestContext {
        let mut test = context(dir);
        test.ctx.configured_shell = Some("bash".into());
        let path = std::env::var_os("PATH").unwrap_or_default();
        test.ctx.env = Arc::new(vec![("PATH".into(), path), ("LC_ALL".into(), "C".into())]);
        test
    }

    async fn run(test: &TestContext, input: Value) -> ToolResult {
        ShellTool.execute(input, &test.ctx).await
    }

    fn texts(output: &ToolOutput) -> Vec<&str> {
        output
            .content
            .iter()
            .map(|item| match item {
                ToolContent::Text { text } => text.as_str(),
                ToolContent::File { .. } => panic!("text expected"),
            })
            .collect()
    }

    fn metadata(output: &ToolOutput) -> String {
        serde_json::to_string(&output.metadata).unwrap()
    }

    fn output_files(test: &TestContext) -> Vec<PathBuf> {
        let dir = test.ctx.paths.shell_dir(&test.ctx.project_id);
        std::fs::read_dir(dir)
            .map(|entries| entries.flatten().map(|e| e.path()).collect())
            .unwrap_or_default()
    }

    async fn notification(test: &TestContext) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some((session, text)) = test.notifier.messages.lock().unwrap().pop() {
                assert_eq!(session, "ses_test");
                return text;
            }
            assert!(Instant::now() < deadline, "no notification");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[test]
    fn description_names_os_and_shell() {
        let env = DefinitionEnv {
            shell_name: shell_name(Some("bash")),
            year: 2026,
            subagents: vec![],
        };
        assert_eq!(
            ShellTool.definition(&env).description,
            "Execute a shell command and return its output. Commands run on Linux using bash. Quote file paths containing spaces or special characters. Prefer dedicated tools over shell commands when possible. When output is large, the full result is saved to a file and a truncated preview is returned. Rely on automatic truncation unless filtering the output is more useful. Commands accept an optional timeout, background commands have no timeout by default. Background commands return immediately, and you will be notified when they complete."
        );
    }

    #[tokio::test]
    async fn captured_outputs() {
        let _serial = SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let test = setup(dir.path());
        let out = run(&test, json!({"command": "echo hi; echo err 1>&2"}))
            .await
            .unwrap();
        assert_eq!(texts(&out), ["hi\nerr\n"]);
        assert_eq!(
            metadata(&out),
            r#"{"status":"completed","truncated":false,"exit":0}"#
        );
        let out = run(&test, json!({"command": "ls /nonexistent_zz; exit 3"}))
            .await
            .unwrap();
        assert_eq!(
            texts(&out),
            [
                "ls: cannot access '/nonexistent_zz': No such file or directory\n",
                "Exited with code 3"
            ]
        );
        let out = run(&test, json!({"command": "true"})).await.unwrap();
        assert_eq!(texts(&out), ["(no output)"]);
        // stdin is the null device, not a terminal.
        let out = run(&test, json!({"command": "cat; tty"})).await.unwrap();
        assert_eq!(texts(&out), ["not a tty\n", "Exited with code 1"]);
    }

    #[tokio::test]
    async fn captured_timeout() {
        let _serial = SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let test = setup(dir.path());
        let started = Instant::now();
        let out = run(
            &test,
            json!({"command": "echo start; sleep 3; echo end", "timeout": 500}),
        )
        .await
        .unwrap();
        assert!(started.elapsed() < Duration::from_millis(2500));
        assert_eq!(
            texts(&out),
            [
                "start\n\n\nCommand exceeded timeout of 500 ms. Retry with a larger timeout if the command is expected to take longer.",
                "Timed out before completion"
            ]
        );
        assert_eq!(
            metadata(&out),
            r#"{"status":"completed","truncated":false,"timeout":true}"#
        );
    }

    #[tokio::test]
    async fn killed_by_signal() {
        let _serial = SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let test = setup(dir.path());
        let out = run(&test, json!({"command": "kill -TERM $$"}))
            .await
            .unwrap();
        assert_eq!(texts(&out), ["(no output)", "Killed by SIGTERM"]);
        assert_eq!(
            metadata(&out),
            r#"{"status":"completed","truncated":false,"signal":"SIGTERM"}"#
        );
    }

    #[tokio::test]
    async fn captured_truncation() {
        let _serial = SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let test = setup(dir.path());
        let out = run(&test, json!({"command": "seq 1 100000"}))
            .await
            .unwrap();
        let [file] = output_files(&test).try_into().unwrap();
        let text = texts(&out)[0];
        assert!(text.ends_with(&format!(
            "\n99999\n100000\n\n[showing lines 98001-100000 of 100000; full output saved to {}]",
            file.display()
        )));
        assert!(text.starts_with("98001\n"));
        assert_eq!(std::fs::metadata(&file).unwrap().len(), 588_895);
        assert_eq!(out.metadata["truncated"], json!(true));
        std::fs::remove_file(&file).unwrap();

        let command = r"head -c 120000 /dev/zero | tr '\0' a; printf '\ntail\n'";
        let out = run(&test, json!({"command": command})).await.unwrap();
        let [file] = output_files(&test).try_into().unwrap();
        assert_eq!(
            texts(&out),
            [format!(
                "{}\ntail\n\n\n[showing lines 1-2 of 2; full output saved to {}]",
                "a".repeat(51194),
                file.display()
            )]
        );
    }

    #[tokio::test]
    async fn environment_and_workdir() {
        let _serial = SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let mut test = setup(dir.path());
        let command =
            "echo $TERM $OPENCODE_TERMINAL $AGENT $OPENCODE $AI_AGENT $OPENCODE_SESSION_ID";
        let out = run(&test, json!({"command": command})).await.unwrap();
        assert_eq!(texts(&out), ["xterm-256color 1 1 1 opencode ses_test\n"]);
        let mut env = (*test.ctx.env).clone();
        env.push(("AI_AGENT".into(), "claude".into()));
        test.ctx.env = Arc::new(env);
        let out = run(&test, json!({"command": "echo $AI_AGENT"}))
            .await
            .unwrap();
        assert_eq!(texts(&out), ["claude\n"]);

        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::create_dir_all(dir.path().join(".home")).unwrap();
        let out = run(&test, json!({"command": "pwd", "workdir": "sub"}))
            .await
            .unwrap();
        assert_eq!(texts(&out), [format!("{}/sub\n", dir.path().display())]);
        let out = run(&test, json!({"command": "pwd", "workdir": "~"}))
            .await
            .unwrap();
        assert_eq!(texts(&out), [format!("{}/.home\n", dir.path().display())]);
    }

    #[tokio::test]
    async fn workdir_errors_after_approval() {
        let dir = tempfile::tempdir().unwrap();
        let test = setup(dir.path());
        std::fs::write(dir.path().join("f"), "").unwrap();
        let error = run(&test, json!({"command": "ls", "workdir": "nope"}))
            .await
            .unwrap_err();
        assert_eq!(error.type_tag(), "unknown");
        assert_eq!(
            error.message,
            format!(
                "Working directory does not exist: {}/nope",
                dir.path().display()
            )
        );
        let error = run(&test, json!({"command": "ls", "workdir": "f"}))
            .await
            .unwrap_err();
        assert_eq!(
            error.message,
            format!(
                "Working directory is not a directory: {}/f",
                dir.path().display()
            )
        );
        assert_eq!(test.gate.asked.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn permission_requests() {
        let _serial = SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let test = setup(dir.path());
        let workdir = outside.path().display().to_string();
        run(
            &test,
            json!({"command": "  git status --short ", "workdir": workdir}),
        )
        .await
        .unwrap();
        run(&test, json!({"command": " "})).await.unwrap();
        let asked = test.gate.asked.lock().unwrap();
        assert_eq!(asked.len(), 2);
        assert_eq!(asked[0].action, "external_directory");
        assert_eq!(asked[0].resources, [format!("{workdir}/*")]);
        assert_eq!(asked[1].action, "shell");
        assert_eq!(asked[1].resources, ["git status --short"]);
        assert_eq!(asked[1].save, ["git status *"]);
    }

    #[tokio::test]
    async fn static_cd_targets_are_external_directories() {
        let _serial = SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let test = setup(dir.path());
        let target = outside.path().display().to_string();
        run(
            &test,
            json!({"command": format!("cd {target} && npm run build --x")}),
        )
        .await
        .unwrap();
        let asked = test.gate.asked.lock().unwrap();
        assert_eq!(asked[0].action, "external_directory");
        assert_eq!(asked[0].resources, [format!("{target}/*")]);
        assert_eq!(asked[1].resources, ["npm run build --x"]);
        assert_eq!(asked[1].save, ["npm run build *"]);
    }

    #[tokio::test]
    async fn unanalyzable_commands_fail_before_running() {
        let _serial = SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let test = setup(dir.path());
        let error = run(&test, json!({"command": "touch made; echo 'open"}))
            .await
            .unwrap_err();
        assert_eq!(error.type_tag(), "unknown");
        assert_eq!(
            error.message,
            "Portable shell scanner cannot analyze command: unterminated-quote"
        );
        assert!(!dir.path().join("made").exists());
        assert!(test.gate.asked.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn descendants_holding_the_pipe() {
        let _serial = SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let test = setup(dir.path());
        let started = Instant::now();
        let out = run(&test, json!({"command": "(sleep 5 &); echo hi"}))
            .await
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(texts(&out), ["hi\n"]);
    }

    #[tokio::test]
    async fn interruption_kills_and_removes_output() {
        let _serial = SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let test = setup(dir.path());
        let ctx = test.ctx.clone();
        let call = tokio::spawn(async move {
            let input = json!({"command": "echo $$ > pid; sleep 30"});
            ShellTool.execute(input, &ctx).await
        });
        let pid_file = dir.path().join("pid");
        while std::fs::read_to_string(&pid_file).map_or(true, |s| !s.ends_with('\n')) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let pid: i32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        test.ctx.cancel.cancel();
        let error = call.await.unwrap().unwrap_err();
        assert_eq!(error, ToolError::interrupted());
        assert!(output_files(&test).is_empty());
        let deadline = Instant::now() + Duration::from_secs(5);
        // SAFETY: signal 0 only checks that the process exists.
        while unsafe { libc::kill(pid, 0) } == 0 {
            assert!(Instant::now() < deadline, "shell still running");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn background_commands() {
        let _serial = SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let test = setup(dir.path());
        let command = "echo started; sleep 1; echo finished; exit 4";
        let out = run(&test, json!({"command": command, "background": true}))
            .await
            .unwrap();
        let id = out.metadata["shellID"].as_str().unwrap().to_string();
        let file = test.ctx.paths.shell_dir("global").join(format!("{id}.out"));
        assert_eq!(
            metadata(&out),
            format!(r#"{{"status":"running","truncated":false,"shellID":"{id}"}}"#)
        );
        assert_eq!(
            texts(&out),
            [
                format!(
                    "Command moved to the background (shell ID: {id}).\nOutput is streaming to: {}",
                    file.display()
                ),
                BACKGROUND_INSTRUCTION.to_string()
            ]
        );
        assert_eq!(
            notification(&test).await,
            format!(
                "<shell id=\"{id}\" state=\"completed\" command=\"{command}\">\nstarted\nfinished\n\n\nExited with code 4\n</shell>"
            )
        );
        assert_eq!(background_end(&id), Some(ShellEnd::Exited(Some(4))));

        // Background commands outlive the turn and have no timeout; the process exit kills them.
        test.ctx.cancel.cancel();
        let out = run(&test, json!({"command": "sleep 30", "background": true}))
            .await
            .unwrap();
        let id = out.metadata["shellID"].as_str().unwrap().to_string();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(test.notifier.messages.lock().unwrap().is_empty());
        kill_background_shells();
        assert_eq!(
            notification(&test).await,
            format!(
                "<shell id=\"{id}\" state=\"completed\" command=\"sleep 30\">\n(no output)\n\nKilled by SIGKILL\n</shell>"
            )
        );
    }

    /// ctrl+b: a running foreground command returns the background result at once, keeps running
    /// past its timeout, and reports back when it ends.
    #[tokio::test]
    async fn foreground_moved_to_background() {
        let _serial = SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let test = setup(dir.path());
        assert!(background_all("ses_test").is_empty());
        let command = "sleep 1; echo late";
        let call = run(&test, json!({"command": command, "timeout": 500}));
        let mover = async {
            while foreground().is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(background_all("ses_other").is_empty());
            background_all("ses_test")
        };
        let (out, moved) = tokio::join!(call, mover);
        assert_eq!(moved, [command]);
        let out = out.unwrap();
        let id = out.metadata["shellID"].as_str().unwrap().to_string();
        assert!(texts(&out)[0].starts_with(&format!(
            "Command moved to the background (shell ID: {id})."
        )));
        assert!(foreground().is_empty());
        assert_eq!(
            notification(&test).await,
            format!(
                "<shell id=\"{id}\" state=\"completed\" command=\"{command}\">\nlate\n\n</shell>"
            )
        );
    }
}
