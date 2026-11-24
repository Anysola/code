//! Embedded interactive terminals bound to a session's working folder (its worktree, or the
//! main folder). Each terminal owns a native PTY (`openpty` on Unix, ConPTY on Windows) and
//! streams raw bytes (base64-encoded) over the `core` event channel so multibyte UTF-8 and ANSI
//! escape sequences arrive intact.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde_json::json;
use tauri::{AppHandle, Emitter, State};

use crate::{Answer, App};

struct Entry {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

/// Live embedded terminals for this window.
#[derive(Clone, Default)]
pub(crate) struct Manager {
    next: Arc<AtomicU64>,
    entries: Arc<Mutex<HashMap<String, Entry>>>,
}

impl Manager {
    pub(crate) fn open(
        &self,
        handle: AppHandle,
        directory: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(String, String), String> {
        let dir = Path::new(directory);
        if !dir.is_dir() {
            return Err(format!("{directory} is not a folder on this computer."));
        }
        let pty = native_pty_system();
        let size = PtySize {
            rows: rows.clamp(2, 500),
            cols: cols.clamp(10, 500),
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = pty
            .openpty(size)
            .map_err(|e| format!("Couldn’t open a terminal: {e}"))?;

        let (shell, _) = anysola_core::tool::shell::invocation(None, "");
        let shell_name = anysola_core::tool::shell::shell_name(None);
        let mut cmd = CommandBuilder::new(&shell);
        if matches!(shell_name.as_str(), "pwsh" | "powershell") {
            cmd.arg("-NoLogo");
        }
        cmd.cwd(dir);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("Couldn’t start {shell_name}: {e}"))?;
        // Drop the slave handle in the parent so the reader sees EOF when the shell exits.
        drop(pair.slave);

        let killer = child.clone_killer();
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("Couldn’t read terminal output: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("Couldn’t write to terminal: {e}"))?;

        let n = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let id = format!("term_{n}");
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                id.clone(),
                Entry {
                    master: pair.master,
                    writer,
                    killer,
                },
            );

        let entries = self.entries.clone();
        let tid = id.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let data = BASE64.encode(&buf[..n]);
                        let _ = handle.emit(
                            "core",
                            json!({"type": "terminalData", "id": tid, "data": data}),
                        );
                    }
                    Err(_) => break,
                }
            }
            let code = child.wait().ok().map(|s| s.exit_code());
            entries
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&tid);
            let _ = handle.emit(
                "core",
                json!({"type": "terminalExit", "id": tid, "code": code}),
            );
        });

        Ok((id, shell_name))
    }

    pub(crate) fn write(&self, id: &str, data: &str) -> Result<(), String> {
        let mut map = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let entry = map
            .get_mut(id)
            .ok_or_else(|| "This terminal has closed.".to_string())?;
        entry
            .writer
            .write_all(data.as_bytes())
            .and_then(|_| entry.writer.flush())
            .map_err(|e| format!("Couldn’t send input to the terminal: {e}"))
    }

    pub(crate) fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let map = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let Some(entry) = map.get(id) else {
            return Ok(());
        };
        entry
            .master
            .resize(PtySize {
                rows: rows.clamp(2, 500),
                cols: cols.clamp(10, 500),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Couldn’t resize the terminal: {e}"))
    }

    pub(crate) fn close(&self, id: &str) {
        if let Some(mut entry) = self
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id)
        {
            let _ = entry.killer.kill();
        }
    }

    pub(crate) fn close_all(&self) {
        let mut drained: Vec<Entry> = self
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .map(|(_, e)| e)
            .collect();
        for entry in &mut drained {
            let _ = entry.killer.kill();
        }
    }
}

#[tauri::command]
pub async fn terminal_open(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    cols: u16,
    rows: u16,
) -> Answer {
    let manager = app.terminals.clone();
    let (id, shell) =
        tokio::task::spawn_blocking(move || manager.open(handle, &directory, cols, rows))
            .await
            .map_err(|e| format!("Terminal start failed: {e}"))??;
    Ok(json!({"id": id, "shell": shell}))
}

#[tauri::command]
pub async fn terminal_write(app: State<'_, App>, id: String, data: String) -> Result<(), String> {
    let manager = app.terminals.clone();
    tokio::task::spawn_blocking(move || manager.write(&id, &data))
        .await
        .map_err(|e| format!("Terminal write failed: {e}"))?
}

#[tauri::command]
pub async fn terminal_resize(
    app: State<'_, App>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    app.terminals.resize(&id, cols, rows)
}

#[tauri::command]
pub async fn terminal_close(app: State<'_, App>, id: String) -> Result<(), String> {
    let manager = app.terminals.clone();
    let _ = tokio::task::spawn_blocking(move || manager.close(&id)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_and_resize_handle_missing_id_cleanly() {
        let mgr = Manager::default();
        assert!(mgr.write("missing", "hi").is_err());
        assert!(mgr.resize("missing", 80, 24).is_ok());
        mgr.close("missing");
        mgr.close_all();
    }
}
