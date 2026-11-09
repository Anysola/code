//! The terminal pane: a shell in an in-process pseudo-terminal (ConPTY on Windows) whose output
//! is parsed into a screen grid the session view draws on the right.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

/// A running shell and its screen.
pub struct Terminal {
    pub screen: Screen,
    master: Box<dyn MasterPty + Send>,
    /// Input for the writer thread: a program that does not read would block the loop.
    input: std::sync::mpsc::Sender<Vec<u8>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

/// The parsed screen, shared with the reader thread.
#[derive(Clone)]
pub struct Screen(pub Arc<Mutex<vt100::Parser>>);

impl std::fmt::Debug for Screen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Screen")
    }
}

/// Screens are compared by identity.
impl PartialEq for Screen {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

fn shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    }
}

impl Terminal {
    /// Starts the user's shell in `directory`; `changed` runs after every chunk of output.
    pub fn spawn(
        directory: &std::path::Path,
        rows: u16,
        cols: u16,
        changed: impl Fn() + Send + 'static,
    ) -> Result<Terminal, String> {
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = native_pty_system()
            .openpty(size)
            .map_err(|e| e.to_string())?;
        let mut command = CommandBuilder::new(shell());
        command.cwd(directory);
        command.env("TERM", "xterm-256color");
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| e.to_string())?;
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let mut writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let (input, queued) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            while let Ok(bytes) = queued.recv() {
                if writer
                    .write_all(&bytes)
                    .and_then(|()| writer.flush())
                    .is_err()
                {
                    break;
                }
            }
        });
        let screen = Screen(Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 1000))));
        let parser = screen.0.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                parser
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .process(&buf[..n]);
                changed();
            }
        });
        Ok(Terminal {
            screen,
            master: pair.master,
            input,
            child,
        })
    }

    /// Queues bytes for the shell; returns at once even when it is not reading.
    pub fn write(&mut self, bytes: &[u8]) {
        let _ = self.input.send(bytes.to_vec());
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let mut parser = self.screen.0.lock().unwrap_or_else(|e| e.into_inner());
        if parser.screen().size() == (rows, cols) {
            return;
        }
        parser.screen_mut().set_size(rows, cols);
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
    }

    /// The shell exited.
    pub fn exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// The bytes a key sends to a terminal program.
pub fn key_bytes(stroke: &crate::keymap::Stroke) -> Option<Vec<u8>> {
    let key = stroke.key.as_str();
    if stroke.ctrl && key.len() == 1 {
        let c = key.as_bytes()[0].to_ascii_lowercase();
        if c.is_ascii_lowercase() {
            return Some(vec![c - b'a' + 1]);
        }
    }
    let plain = |s: &str| Some(s.as_bytes().to_vec());
    let bytes = match key {
        "return" => plain("\r"),
        "backspace" => Some(vec![0x7f]),
        "tab" => plain("\t"),
        "escape" => plain("\x1b"),
        "space" => plain(" "),
        "up" => plain("\x1b[A"),
        "down" => plain("\x1b[B"),
        "right" => plain("\x1b[C"),
        "left" => plain("\x1b[D"),
        "home" => plain("\x1b[H"),
        "end" => plain("\x1b[F"),
        "delete" => plain("\x1b[3~"),
        "pageup" => plain("\x1b[5~"),
        "pagedown" => plain("\x1b[6~"),
        k if k.chars().count() == 1 => {
            let text = if stroke.shift {
                k.to_uppercase()
            } else {
                k.to_string()
            };
            Some(text.into_bytes())
        }
        _ => None,
    }?;
    Some(if stroke.alt {
        [vec![0x1b], bytes].concat()
    } else {
        bytes
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_to_bytes() {
        let stroke = |s: &str| crate::keymap::Stroke::parse(s).unwrap();
        assert_eq!(key_bytes(&stroke("ctrl+c")), Some(vec![3]));
        assert_eq!(key_bytes(&stroke("up")), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_bytes(&stroke("a")), Some(b"a".to_vec()));
    }

    #[cfg(unix)]
    #[test]
    fn writes_return_while_the_program_does_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let mut term = Terminal::spawn(dir.path(), 10, 40, || {}).unwrap();
        term.write(b"exec sleep 30\r");
        let started = std::time::Instant::now();
        // Far more than the pty buffer holds: a blocking write would never return.
        term.write(&vec![b'x'; 1 << 20]);
        term.write(b"more");
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn runs_a_shell_command() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut term = Terminal::spawn(dir.path(), 10, 40, move || {
            let _ = tx.send(());
        })
        .unwrap();
        term.write(b"echo pane-ok\r");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let _ = rx.recv_timeout(std::time::Duration::from_millis(200));
            let text = term.screen.0.lock().unwrap().screen().contents();
            if text.lines().any(|l| l.trim() == "pane-ok") {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "no output: {text}");
        }
    }
}
