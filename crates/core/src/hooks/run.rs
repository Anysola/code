//! Running one hook command: the `shell` tool's shell and process group, the event on stdin,
//! stdout and stderr read apart. The hook and its process group are killed on a timeout, an
//! interrupted turn, a dropped call and the process's own exit, the same way the `shell` tool's
//! commands are. After the hook exits on its own, whatever it left holding its output pipes is
//! killed too; a child it detached with its output redirected elsewhere survives, as in Claude
//! Code.

use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use super::output::Ran;
use crate::tool::shell::exec;

/// What is kept of each stream; the rest is read and dropped.
const MAX_OUTPUT: usize = 1 << 20;
/// How long output is still read after the hook exited while something it started holds a pipe.
const PIPE_GRACE: Duration = Duration::from_secs(1);

pub struct Job<'a> {
    pub shell: &'a str,
    pub command: &'a str,
    pub cwd: &'a Path,
    pub env: Vec<(OsString, OsString)>,
    pub input: Vec<u8>,
    /// Seconds.
    pub timeout: u64,
}

/// Kills the hook's process tree if its run is dropped before the hook was reaped (a Code Mode
/// program's pending call is aborted, not cancelled).
struct Group {
    pid: u32,
    live: bool,
    _tracked: crate::teardown::Tracked,
}

impl Group {
    fn new(pid: u32, command: &str) -> Group {
        exec::register(pid);
        Group {
            pid,
            live: true,
            _tracked: crate::teardown::track(format!("hook {pid} running: {command}")),
        }
    }

    fn reaped(&mut self) {
        self.live = false;
        exec::unregister(self.pid);
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        if self.live {
            exec::unregister(self.pid);
            exec::force_kill(self.pid);
        }
    }
}

/// The stream as text, and whether it was longer than [`MAX_OUTPUT`].
async fn read(pipe: Option<impl AsyncRead + Unpin>) -> (String, bool) {
    let Some(mut pipe) = pipe else {
        return (String::new(), false);
    };
    let mut cut = false;
    let mut out = Vec::new();
    let mut buffer = vec![0; 16 * 1024];
    loop {
        match pipe.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = MAX_OUTPUT.saturating_sub(out.len());
                cut |= n > room;
                out.extend_from_slice(&buffer[..n.min(room)]);
            }
        }
    }
    (String::from_utf8_lossy(&out).into_owned(), cut)
}

pub async fn run(job: Job<'_>, cancel: &CancellationToken) -> Ran {
    if cancel.is_cancelled() {
        return Ran::Interrupted;
    }
    let mut command = crate::console::tokio_command(job.shell);
    command
        .args(crate::tool::shell::select::args(job.shell, job.command))
        .current_dir(job.cwd)
        .env_clear()
        .envs(job.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return Ran::NotStarted(error.to_string()),
    };
    drop(command);
    let pid = child.id().unwrap_or_default();
    let mut group = Group::new(pid, job.command);
    // A hook that does not read its input must not block on a full pipe, nor fail us.
    let stdin = child.stdin.take();
    let input = job.input;
    tokio::spawn(async move {
        if let Some(mut stdin) = stdin {
            let _ = stdin.write_all(&input).await;
            let _ = stdin.shutdown().await;
        }
    });
    let mut stdout = tokio::spawn(read(child.stdout.take()));
    let mut stderr = tokio::spawn(read(child.stderr.take()));
    let timeout = Duration::from_secs(job.timeout.max(1));
    let status = tokio::select! {
        status = child.wait() => Some(status.ok()),
        () = tokio::time::sleep(timeout) => None,
        () = cancel.cancelled() => {
            exec::terminate(&mut child, pid).await;
            group.reaped();
            return Ran::Interrupted;
        }
    };
    let Some(status) = status else {
        exec::terminate(&mut child, pid).await;
        group.reaped();
        return Ran::TimedOut {
            seconds: job.timeout,
        };
    };
    group.reaped();
    // Something the hook started may still hold its pipes: it gets a moment, then the group's
    // leftovers are killed (a hook never outlives its run).
    let drained = tokio::time::timeout(PIPE_GRACE, async {
        let out = (&mut stdout).await.unwrap_or_default();
        let err = (&mut stderr).await.unwrap_or_default();
        (out, err)
    })
    .await;
    let (stdout, stderr) = match drained {
        Ok(both) => both,
        Err(_) => {
            #[cfg(unix)]
            exec::signal_group(pid, libc::SIGKILL);
            let out = tokio::time::timeout(PIPE_GRACE, stdout).await;
            let err = tokio::time::timeout(PIPE_GRACE, stderr).await;
            (
                out.ok().and_then(Result::ok).unwrap_or_default(),
                err.ok().and_then(Result::ok).unwrap_or_default(),
            )
        }
    };
    let ((stdout, cut), (stderr, _)) = (stdout, stderr);
    let code = status.and_then(|s| s.code());
    if cut && code == Some(0) {
        return Ran::OutputTooLarge;
    }
    Ran::Exited {
        code,
        stdout,
        stderr,
    }
}
