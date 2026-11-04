//! Running one command. The shell gets its own process group (Unix), stdin from the null device,
//! and one pipe for both stdout and stderr, so the output file holds them in arrival order. The
//! exit, the timeout and a kill request race; ending a command means SIGTERM to its group and
//! SIGKILL 3 s later (`taskkill /T /F` on Windows).

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, PipeReader, Read, Write};
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::process::Child;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_util::sync::{CancellationToken, DropGuard};

const FORCE_KILL_AFTER: Duration = Duration::from_secs(3);
/// How long output is still captured after the shell exits while descendants hold the pipe.
const PIPE_GRACE: Duration = Duration::from_secs(1);

/// Process ids (= process group ids on Unix) of every command still running.
static RUNNING: Mutex<Vec<u32>> = Mutex::new(Vec::new());

fn running() -> std::sync::MutexGuard<'static, Vec<u32>> {
    RUNNING.lock().unwrap_or_else(|e| e.into_inner())
}

/// Kills every shell command still running (background ones in particular). Call it when the
/// process exits; nothing else would stop them.
pub fn kill_background_shells() {
    for pid in std::mem::take(&mut *running()) {
        force_kill(pid);
    }
}

pub struct Job<'a> {
    pub program: &'a str,
    pub args: Vec<String>,
    pub cwd: &'a Path,
    pub env: Vec<(OsString, OsString)>,
    pub file: &'a Path,
    /// Milliseconds; 0 runs without a limit.
    pub timeout: u64,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Exited {
        code: Option<i32>,
        signal: Option<String>,
    },
    TimedOut,
}

#[derive(Debug)]
pub struct Ended {
    pub outcome: Outcome,
    /// Bytes and newlines captured in the output file.
    pub size: u64,
    pub newlines: u64,
}

/// A started command. Dropping it before the command ended kills the command.
pub struct Running {
    capture: Arc<Capture>,
    supervisor: JoinHandle<Ended>,
    untimed: CancellationToken,
    _kill: DropGuard,
}

impl Running {
    pub async fn wait(&mut self) -> Ended {
        (&mut self.supervisor)
            .await
            .unwrap_or_else(|error| std::panic::resume_unwind(error.into_panic()))
    }

    /// Lets the command run without its time limit (moved to the background).
    pub fn clear_timeout(&self) {
        self.untimed.cancel();
    }

    /// Stops capturing and deletes the output file (an interrupted command leaves nothing).
    pub fn discard(&self, file: &Path) {
        self.capture.stop();
        let _ = std::fs::remove_file(file);
    }
}

pub fn spawn(job: Job<'_>) -> io::Result<Running> {
    let output = File::create(job.file)?;
    let (reader, writer) = io::pipe()?;
    let mut command = tokio::process::Command::new(job.program);
    command
        .args(&job.args)
        .current_dir(job.cwd)
        .env_clear()
        .envs(job.env)
        .stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer);
    #[cfg(unix)]
    command.process_group(0);
    let child = command.spawn()?;
    // The command still holds our copies of the pipe's write end; the pipe only reaches EOF once
    // they are closed.
    drop(command);
    let pid = child.id().unwrap_or_default();
    running().push(pid);
    let tracked = crate::teardown::track(format!(
        "child {pid} running: {} {}",
        job.program,
        job.args.join(" ")
    ));

    let capture = Arc::new(Capture::new(output));
    let (eof_tx, eof_rx) = oneshot::channel();
    let pump_capture = capture.clone();
    std::thread::spawn(move || pump(reader, &pump_capture, eof_tx));
    let kill = CancellationToken::new();
    let untimed = CancellationToken::new();
    let supervised = supervise(
        child,
        pid,
        job.timeout,
        untimed.clone(),
        kill.clone(),
        capture.clone(),
        eof_rx,
    );
    let supervisor = tokio::spawn(async move {
        let ended = supervised.await;
        drop(tracked);
        ended
    });
    Ok(Running {
        capture,
        supervisor,
        untimed,
        _kill: kill.drop_guard(),
    })
}

async fn supervise(
    mut child: Child,
    pid: u32,
    timeout: u64,
    untimed: CancellationToken,
    kill: CancellationToken,
    capture: Arc<Capture>,
    eof: oneshot::Receiver<()>,
) -> Ended {
    let timer = async {
        if timeout == 0 {
            std::future::pending::<()>().await;
        }
        tokio::select! {
            () = tokio::time::sleep(Duration::from_millis(timeout)) => {}
            () = untimed.cancelled() => std::future::pending::<()>().await,
        }
    };
    let outcome = tokio::select! {
        status = child.wait() => exited(status.ok()),
        () = timer => {
            terminate(&mut child, pid).await;
            Outcome::TimedOut
        }
        () = kill.cancelled() => exited(terminate(&mut child, pid).await),
    };
    running().retain(|&other| other != pid);
    let _ = tokio::time::timeout(PIPE_GRACE, eof).await;
    let (size, newlines) = capture.stop();
    Ended {
        outcome,
        size,
        newlines,
    }
}

fn exited(status: Option<ExitStatus>) -> Outcome {
    Outcome::Exited {
        code: status.and_then(|s| s.code()),
        signal: status.and_then(signal_name),
    }
}

/// SIGTERM to the group, SIGKILL if it is still alive 3 s later; returns the reaped status.
async fn terminate(child: &mut Child, pid: u32) -> Option<ExitStatus> {
    #[cfg(unix)]
    if !signal_group(pid, libc::SIGTERM) {
        let _ = child.start_kill();
    }
    #[cfg(not(unix))]
    force_kill(pid);
    match tokio::time::timeout(FORCE_KILL_AFTER, child.wait()).await {
        Ok(status) => status.ok(),
        Err(_) => {
            force_kill(pid);
            let _ = child.start_kill();
            child.wait().await.ok()
        }
    }
}

#[cfg(unix)]
fn signal_group(pid: u32, signal: i32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: kill(2) with a negative pid signals that process group; no memory is involved.
    unsafe { libc::kill(-pid, signal) == 0 }
}

#[cfg(unix)]
fn force_kill(pid: u32) {
    signal_group(pid, libc::SIGKILL);
}

#[cfg(windows)]
fn force_kill(pid: u32) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("taskkill")
        .args(["/pid", &pid.to_string(), "/T", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(unix)]
fn signal_name(status: ExitStatus) -> Option<String> {
    use std::os::unix::process::ExitStatusExt;
    let signal = status.signal()?;
    let names = [
        (libc::SIGHUP, "SIGHUP"),
        (libc::SIGINT, "SIGINT"),
        (libc::SIGQUIT, "SIGQUIT"),
        (libc::SIGILL, "SIGILL"),
        (libc::SIGTRAP, "SIGTRAP"),
        (libc::SIGABRT, "SIGABRT"),
        (libc::SIGBUS, "SIGBUS"),
        (libc::SIGFPE, "SIGFPE"),
        (libc::SIGKILL, "SIGKILL"),
        (libc::SIGUSR1, "SIGUSR1"),
        (libc::SIGSEGV, "SIGSEGV"),
        (libc::SIGUSR2, "SIGUSR2"),
        (libc::SIGPIPE, "SIGPIPE"),
        (libc::SIGALRM, "SIGALRM"),
        (libc::SIGTERM, "SIGTERM"),
        (libc::SIGCHLD, "SIGCHLD"),
        (libc::SIGCONT, "SIGCONT"),
        (libc::SIGSTOP, "SIGSTOP"),
        (libc::SIGTSTP, "SIGTSTP"),
        (libc::SIGTTIN, "SIGTTIN"),
        (libc::SIGTTOU, "SIGTTOU"),
        (libc::SIGURG, "SIGURG"),
        (libc::SIGXCPU, "SIGXCPU"),
        (libc::SIGXFSZ, "SIGXFSZ"),
        (libc::SIGVTALRM, "SIGVTALRM"),
        (libc::SIGPROF, "SIGPROF"),
        (libc::SIGWINCH, "SIGWINCH"),
        (libc::SIGIO, "SIGIO"),
        (libc::SIGSYS, "SIGSYS"),
    ];
    Some(
        names
            .iter()
            .find(|(number, _)| *number == signal)
            .map_or_else(|| format!("SIG{signal}"), |(_, name)| (*name).to_string()),
    )
}

#[cfg(not(unix))]
fn signal_name(_: ExitStatus) -> Option<String> {
    None
}

/// The output file and its running counters, shared by the pipe reader and the supervisor.
struct Capture {
    state: Mutex<CaptureState>,
}

struct CaptureState {
    /// `None` once capturing stopped; later output is drained and dropped.
    file: Option<File>,
    size: u64,
    newlines: u64,
}

impl Capture {
    fn new(file: File) -> Self {
        Capture {
            state: Mutex::new(CaptureState {
                file: Some(file),
                size: 0,
                newlines: 0,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CaptureState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self, chunk: &[u8]) {
        let mut state = self.lock();
        let Some(file) = state.file.as_mut() else {
            return;
        };
        let _ = file.write_all(chunk);
        state.size += chunk.len() as u64;
        state.newlines += chunk.iter().filter(|&&b| b == b'\n').count() as u64;
    }

    /// Closes the file and returns the counters.
    fn stop(&self) -> (u64, u64) {
        let mut state = self.lock();
        state.file = None;
        (state.size, state.newlines)
    }
}

/// Copies the pipe into the capture until every writer (the shell and its descendants) closed it.
fn pump(mut reader: PipeReader, capture: &Capture, eof: oneshot::Sender<()>) {
    let mut buffer = vec![0; 64 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => capture.write(&buffer[..n]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    let _ = eof.send(());
}
