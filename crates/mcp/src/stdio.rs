//! The stdio transport: a child process speaking newline-delimited JSON on stdin/stdout.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdout};
use tokio::sync::mpsc;

use crate::rpc::{Error, Incoming, Transport};

const CLOSE_GRACE: Duration = Duration::from_secs(2);
const FORCE_KILL_AFTER: Duration = Duration::from_secs(2);
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const EXIT_WAIT: Duration = Duration::from_secs(1);
const STDERR_TAIL: usize = 1_000;

pub struct Stdio {
    outgoing: Mutex<Option<mpsc::UnboundedSender<String>>>,
    child: Arc<tokio::sync::Mutex<Option<Child>>>,
    /// The process group to signal, which outlives the child's own exit.
    pid: Option<u32>,
    closing: Arc<std::sync::atomic::AtomicBool>,
}

/// `NotFound: ChildProcess.spawn (<command> <args>)`, as 0.5.1 words a failed spawn.
fn spawn_error(error: &std::io::Error, command: &[String]) -> Error {
    let kind = match error.kind() {
        std::io::ErrorKind::NotFound => "NotFound",
        std::io::ErrorKind::PermissionDenied => "PermissionDenied",
        _ => "Unknown",
    };
    Error::Other(format!(
        "{kind}: ChildProcess.spawn ({})",
        command.join(" ")
    ))
}

impl Stdio {
    pub fn spawn(
        server: &str,
        command: &[String],
        cwd: &Path,
        environment: &[(String, String)],
        incoming: mpsc::UnboundedSender<Incoming>,
    ) -> Result<Stdio, Error> {
        let mut process = tokio::process::Command::new(&command[0]);
        process
            .args(&command[1..])
            .current_dir(cwd)
            .envs(environment.iter().map(|(k, v)| (k, v)))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        process.process_group(0);
        let mut child = process.spawn().map_err(|e| spawn_error(&e, command))?;
        let pid = child.id();
        let (mut stdin, stdout, mut stderr) = (
            child.stdin.take().expect("piped stdin"),
            child.stdout.take().expect("piped stdout"),
            child.stderr.take().expect("piped stderr"),
        );
        let (outgoing, mut queue) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(line) = queue.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
        let tail = Arc::new(Mutex::new(String::new()));
        let drain = {
            let (tail, server) = (tail.clone(), server.to_string());
            tokio::spawn(async move {
                let mut buffer = [0u8; 8192];
                while let Ok(n) = stderr.read(&mut buffer).await {
                    if n == 0 {
                        break;
                    }
                    let output = String::from_utf8_lossy(&buffer[..n]).into_owned();
                    if !output.trim().is_empty() {
                        anysola_core::log::write(
                            anysola_core::log::Level::Debug,
                            "mcp server stderr",
                            &[("server", &server), ("output", &output)],
                        );
                    }
                    let mut tail = tail.lock().unwrap_or_else(|e| e.into_inner());
                    tail.push_str(&output);
                    let excess = tail.chars().count().saturating_sub(STDERR_TAIL);
                    if excess > 0 {
                        *tail = tail.chars().skip(excess).collect();
                    }
                }
            })
        };
        let child = Arc::new(tokio::sync::Mutex::new(Some(child)));
        let closing = Arc::new(std::sync::atomic::AtomicBool::new(false));
        tokio::spawn(read(
            server.to_string(),
            stdout,
            incoming,
            Exit {
                child: child.clone(),
                closing: closing.clone(),
                drain,
                tail,
            },
        ));
        Ok(Stdio {
            outgoing: Mutex::new(Some(outgoing)),
            child,
            pid,
            closing,
        })
    }
}

/// What reading needs to report an unexpected exit.
struct Exit {
    child: Arc<tokio::sync::Mutex<Option<Child>>>,
    closing: Arc<std::sync::atomic::AtomicBool>,
    drain: tokio::task::JoinHandle<()>,
    tail: Arc<Mutex<String>>,
}

async fn read(
    server: String,
    stdout: ChildStdout,
    incoming: mpsc::UnboundedSender<Incoming>,
    exit: Exit,
) {
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    loop {
        line.clear();
        let limited = (&mut reader).take(MAX_FRAME_BYTES as u64 + 1);
        let mut limited = limited;
        match limited.read_until(b'\n', &mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if line.last() != Some(&b'\n') && line.len() > MAX_FRAME_BYTES {
            let _ = incoming.send(Incoming::Error(format!(
                "MCP server {server} wrote more than 16 MiB to stdout without a message boundary"
            )));
            let _ = incoming.send(Incoming::Closed);
            return;
        }
        let mut text = line.as_slice();
        text = text.strip_suffix(b"\n").unwrap_or(text);
        text = text.strip_suffix(b"\r").unwrap_or(text);
        let text = String::from_utf8_lossy(text);
        if text.trim().is_empty() {
            continue;
        }
        let item = match serde_json::from_str::<Value>(&text) {
            Ok(message) if message.is_object() => Incoming::Message(message),
            Ok(_) => Incoming::Error(format!(
                "Invalid MCP stdio message from {server}: not a JSON-RPC message"
            )),
            Err(error) => Incoming::Error(format!(
                "Invalid MCP stdio message from {server}: SyntaxError: {error}"
            )),
        };
        let _ = incoming.send(item);
    }
    if !exit.closing.load(std::sync::atomic::Ordering::SeqCst) {
        // stdout can end before the final stderr output has been read.
        let code = tokio::time::timeout(EXIT_WAIT, async {
            let _ = exit.drain.await;
            let mut child = exit.child.lock().await;
            match child.as_mut() {
                Some(child) => child.wait().await.ok().and_then(|s| s.code()),
                None => None,
            }
        })
        .await
        .ok()
        .flatten();
        let exited = match code {
            Some(code) => format!("MCP server process exited with code {code}"),
            None => "MCP server process exited".to_string(),
        };
        let output = exit
            .tail
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .trim()
            .to_string();
        let _ = incoming.send(Incoming::Error(if output.is_empty() {
            exited
        } else {
            format!("{exited}: {output}")
        }));
    }
    let _ = incoming.send(Incoming::Closed);
}

#[cfg(unix)]
fn signal_group(pid: Option<u32>, signal: i32) {
    if let Some(pid) = pid {
        // SAFETY: kill(2) with a negative pid signals the process group the child leads.
        unsafe {
            libc::kill(-(pid as i32), signal);
        }
    }
}

impl Transport for Stdio {
    fn send(&self, message: Value) -> BoxFuture<'_, Result<(), Error>> {
        let sent = self
            .outgoing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|o| o.send(format!("{message}\n")).is_ok());
        async move {
            if sent {
                Ok(())
            } else {
                Err(Error::Other("Not connected".into()))
            }
        }
        .boxed()
    }

    /// Ends stdin, then waits 2 s for the exit, SIGTERM, 2 s more, SIGKILL; the whole process
    /// group goes.
    fn close(&self) -> BoxFuture<'_, ()> {
        self.closing
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.outgoing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        async move {
            let Some(mut child) = self.child.lock().await.take() else {
                return;
            };
            if tokio::time::timeout(CLOSE_GRACE, child.wait())
                .await
                .is_ok()
            {
                #[cfg(unix)]
                signal_group(self.pid, libc::SIGKILL);
                return;
            }
            #[cfg(unix)]
            {
                signal_group(self.pid, libc::SIGTERM);
                if tokio::time::timeout(FORCE_KILL_AFTER, child.wait())
                    .await
                    .is_ok()
                {
                    signal_group(self.pid, libc::SIGKILL);
                    return;
                }
                signal_group(self.pid, libc::SIGKILL);
            }
            let _ = child.kill().await;
        }
        .boxed()
    }
}
