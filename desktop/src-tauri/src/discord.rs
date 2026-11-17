//! Discord Rich Presence: what the open session is, on the user's Discord profile, as far as the
//! preferences allow. Discord's local IPC: frames of `u32 opcode LE + u32 length LE + JSON`, a
//! handshake (op 0) and then `SET_ACTIVITY` commands (op 1), each answered before the next.
//!
//! One thread owns the connection and only ever writes then reads: on Windows a read parked on a
//! synchronous pipe handle blocks writes to it. It takes the latest wanted state from a channel,
//! sends at most one update every few seconds, and looks for Discord again while it isn't there.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};
use tauri::State;

use crate::prefs;
use crate::{Answer, App};

/// The app's own Discord application, when it has one. There is none yet: the user brings their
/// own in Settings until one is registered.
const DEFAULT_APPLICATION_ID: Option<&str> = None;

const LOGO: &str = "https://www.anysola.com/anysola-logo.png";
const SITE: &str = "https://www.anysola.com/code";
/// Discord drops updates sent faster than this.
const COALESCE: Duration = Duration::from_secs(4);
/// How often to look for Discord while it isn't running.
const RETRY: Duration = Duration::from_secs(15);
/// A Discord that stops answering must not hold the thread.
const IO_TIMEOUT: Duration = Duration::from_secs(5);

const NO_ID: &str = "Add a Discord application ID to turn this on.";
const NOT_RUNNING: &str = "Discord isn’t running, or this app can’t reach it.";
const REFUSED: &str =
    "Discord refused this application ID. Check it in the Discord Developer Portal.";

/// What the window is showing.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Presence {
    pub directory: Option<String>,
    pub title: Option<String>,
    pub model: Option<String>,
    pub working: bool,
}

/// The activity Discord shows, or `None` to clear it.
#[derive(Debug, Clone, PartialEq)]
struct Wanted {
    client_id: Option<String>,
    activity: Option<Value>,
}

enum Message {
    Want(Wanted),
    Exit(Sender<()>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub state: &'static str,
    pub detail: Option<&'static str>,
}

struct Shared {
    prefs: prefs::Discord,
    presence: Presence,
    /// When the open session became the open one: `(directory, title, epoch seconds)`.
    since: Option<(Option<String>, Option<String>, u64)>,
}

pub struct Handle {
    sender: Mutex<Sender<Message>>,
    shared: Mutex<Shared>,
    status: Arc<Mutex<Status>>,
}

/// Where the transport looks, and how often it sends and retries.
struct Settings {
    dirs: Vec<PathBuf>,
    coalesce: Duration,
    retry: Duration,
}

fn now_secs() -> u64 {
    anysola_core::time::now_ms() / 1000
}

fn client_id(prefs: &prefs::Discord) -> Option<String> {
    prefs
        .application_id
        .clone()
        .or(DEFAULT_APPLICATION_ID.map(str::to_string))
}

fn folder_name(directory: &str) -> String {
    let trimmed = directory.trim_end_matches(['/', '\\']);
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or(trimmed)
        .to_string()
}

/// A new session's placeholder title gives way to its real one: still the same session.
fn same_session(before: &(Option<String>, Option<String>), now: &Presence) -> bool {
    let placeholder =
        |t: &Option<String>| t.as_deref().is_none_or(|t| t.starts_with("New session"));
    before.0 == now.directory && (before.1 == now.title || placeholder(&before.1))
}

/// The activity for what the window shows, as the preferences allow.
fn activity(prefs: &prefs::Discord, presence: &Presence, started: u64) -> Value {
    let details = match (&presence.directory, prefs.project, presence.working) {
        (Some(directory), true, true) => format!("Working in {}", folder_name(directory)),
        (Some(directory), true, false) => format!("In {}", folder_name(directory)),
        (_, _, true) => "Working".into(),
        (_, _, false) => "Idle".into(),
    };
    let state = presence
        .title
        .as_ref()
        .filter(|_| prefs.session)
        .or(presence.model.as_ref().filter(|_| prefs.model));
    let mut activity = json!({
        "type": 0,
        "details": details,
        "assets": {"large_image": LOGO, "large_text": "Anysola Code"},
        "buttons": [{"label": "Anysola Code", "url": SITE}],
    });
    if let Some(state) = state {
        activity["state"] = json!(state);
    }
    if prefs.elapsed {
        activity["timestamps"] = json!({"start": started});
    }
    activity
}

impl Handle {
    pub fn start(prefs: prefs::Discord) -> Handle {
        Handle::with(prefs, socket_dirs(), COALESCE, RETRY)
    }

    fn with(
        prefs: prefs::Discord,
        dirs: Vec<PathBuf>,
        coalesce: Duration,
        retry: Duration,
    ) -> Handle {
        let (sender, receiver) = channel();
        let status = Arc::new(Mutex::new(Status {
            state: "off",
            detail: None,
        }));
        let settings = Settings {
            dirs,
            coalesce,
            retry,
        };
        let thread_status = status.clone();
        let _ = std::thread::Builder::new()
            .name("discord".into())
            .spawn(move || run(receiver, settings, thread_status));
        let handle = Handle {
            sender: Mutex::new(sender),
            shared: Mutex::new(Shared {
                prefs,
                presence: Presence::default(),
                since: None,
            }),
            status,
        };
        handle.push();
        handle
    }

    pub fn set_prefs(&self, prefs: prefs::Discord) {
        self.shared.lock().unwrap_or_else(|e| e.into_inner()).prefs = prefs;
        self.push();
    }

    pub fn set_presence(&self, presence: Presence) {
        {
            let mut shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
            let kept = shared
                .since
                .as_ref()
                .is_some_and(|(d, t, _)| same_session(&(d.clone(), t.clone()), &presence));
            if !kept {
                shared.since = Some((
                    presence.directory.clone(),
                    presence.title.clone(),
                    now_secs(),
                ));
            } else if let Some(since) = &mut shared.since {
                since.1 = presence.title.clone();
            }
            shared.presence = presence;
        }
        self.push();
    }

    fn push(&self) {
        let wanted = {
            let shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
            let id = client_id(&shared.prefs);
            let activity = (shared.prefs.enabled && id.is_some()).then(|| {
                let started = shared.since.as_ref().map_or_else(now_secs, |s| s.2);
                activity(&shared.prefs, &shared.presence, started)
            });
            Wanted {
                client_id: id.filter(|_| shared.prefs.enabled),
                activity,
            }
        };
        let _ = self
            .sender
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(Message::Want(wanted));
    }

    pub fn status(&self) -> Status {
        let shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if !shared.prefs.enabled {
            return Status {
                state: "off",
                detail: None,
            };
        }
        if client_id(&shared.prefs).is_none() {
            return Status {
                state: "off",
                detail: Some(NO_ID),
            };
        }
        drop(shared);
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Clears the activity and closes the connection (the app is closing); waits at most
    /// `grace`.
    pub fn stop(&self, grace: Duration) {
        let (done, wait) = channel();
        let sent = self
            .sender
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(Message::Exit(done));
        if sent.is_ok() {
            let _ = wait.recv_timeout(grace);
        }
    }
}

/// Where Discord's sockets are on Linux and macOS, Flatpak and Snap installs included.
#[cfg(unix)]
fn socket_dirs() -> Vec<PathBuf> {
    let mut bases: Vec<PathBuf> = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
        .iter()
        .filter_map(|v| std::env::var_os(v).filter(|v| !v.is_empty()))
        .map(PathBuf::from)
        .collect();
    bases.push(PathBuf::from("/tmp"));
    let mut dirs = Vec::new();
    for base in bases {
        for sub in ["", "app/com.discordapp.Discord", "snap.discord"] {
            let dir = if sub.is_empty() {
                base.clone()
            } else {
                base.join(sub)
            };
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    dirs
}

#[cfg(windows)]
fn socket_dirs() -> Vec<PathBuf> {
    vec![PathBuf::from(r"\\.\pipe")]
}

/// One connection to Discord: whole frames out, and the next frame in within [`IO_TIMEOUT`].
trait Pipe: Send {
    fn send(&mut self, frame: &[u8]) -> std::io::Result<()>;
    fn receive(&mut self) -> std::io::Result<(u32, Value)>;
}

/// A socket with read and write timeouts of its own.
#[cfg(unix)]
impl Pipe for std::os::unix::net::UnixStream {
    fn send(&mut self, frame: &[u8]) -> std::io::Result<()> {
        self.write_all(frame)?;
        self.flush()
    }

    fn receive(&mut self) -> std::io::Result<(u32, Value)> {
        decode(self)
    }
}

#[cfg(unix)]
fn open(path: &std::path::Path) -> std::io::Result<Box<dyn Pipe>> {
    let stream = std::os::unix::net::UnixStream::connect(path)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    Ok(Box::new(stream))
}

/// The named pipe, opened as an ordinary file for reading and writing. A synchronous pipe
/// handle has no read timeout, so reads go through [`timed`].
#[cfg(windows)]
struct NamedPipe(std::fs::File);

#[cfg(windows)]
impl Pipe for NamedPipe {
    fn send(&mut self, frame: &[u8]) -> std::io::Result<()> {
        self.0.write_all(frame)?;
        self.0.flush()
    }

    fn receive(&mut self) -> std::io::Result<(u32, Value)> {
        timed(self.0.try_clone()?, IO_TIMEOUT)
    }
}

#[cfg(windows)]
fn open(path: &std::path::Path) -> std::io::Result<Box<dyn Pipe>> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    Ok(Box::new(NamedPipe(file)))
}

/// Reads one frame on a helper thread and waits at most `timeout` for it. On a timeout the
/// helper is left parked on its read and the caller drops the connection (a parked read blocks
/// writes to that pipe); it ends when Discord answers or the pipe closes.
#[cfg(any(windows, test))]
fn timed<R: Read + Send + 'static>(
    mut reader: R,
    timeout: Duration,
) -> std::io::Result<(u32, Value)> {
    let (sender, result) = channel();
    std::thread::Builder::new()
        .name("discord-read".into())
        .spawn(move || {
            let _ = sender.send(decode(&mut reader));
        })?;
    result.recv_timeout(timeout).unwrap_or_else(|_| {
        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "discord did not answer",
        ))
    })
}

struct Connection {
    pipe: Box<dyn Pipe>,
    client_id: String,
    nonce: u64,
}

fn encode(op: u32, payload: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(payload).unwrap_or_default();
    let mut frame = Vec::with_capacity(8 + body.len());
    frame.extend_from_slice(&op.to_le_bytes());
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    frame
}

fn decode(reader: &mut impl Read) -> std::io::Result<(u32, Value)> {
    let mut header = [0u8; 8];
    reader.read_exact(&mut header)?;
    let op = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let length = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    if length > 1 << 20 {
        return Err(std::io::Error::other("oversized frame"));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    Ok((op, serde_json::from_slice(&body).unwrap_or(Value::Null)))
}

const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_CLOSE: u32 = 2;

impl Connection {
    /// The first Discord that takes the handshake; `Err` with why there is none.
    fn open(dirs: &[PathBuf], client_id: &str) -> Result<Connection, &'static str> {
        for dir in dirs {
            for n in 0..10 {
                let Ok(mut pipe) = open(&dir.join(format!("discord-ipc-{n}"))) else {
                    continue;
                };
                let hello = json!({"v": 1, "client_id": client_id});
                if pipe.send(&encode(OP_HANDSHAKE, &hello)).is_err() {
                    continue;
                }
                match pipe.receive() {
                    Ok((OP_FRAME, _)) => {
                        return Ok(Connection {
                            pipe,
                            client_id: client_id.to_string(),
                            nonce: 0,
                        });
                    }
                    Ok((_, body)) => {
                        // A wrong application ID is refused here; the next pipe would say the same.
                        anysola_core::log::warn(
                            "discord refused the handshake",
                            &[("reply", &body.to_string())],
                        );
                        return Err(REFUSED);
                    }
                    Err(_) => continue,
                }
            }
        }
        Err(NOT_RUNNING)
    }

    /// Sends the activity (`None` clears it) and waits for Discord's answer to it.
    fn set_activity(&mut self, activity: Option<&Value>) -> std::io::Result<()> {
        self.nonce += 1;
        let nonce = self.nonce.to_string();
        let command = json!({
            "cmd": "SET_ACTIVITY",
            "args": {"pid": std::process::id(), "activity": activity},
            "nonce": nonce,
        });
        self.pipe.send(&encode(OP_FRAME, &command))?;
        loop {
            let (op, body) = self.pipe.receive()?;
            if op == OP_CLOSE {
                return Err(std::io::Error::other("discord closed the connection"));
            }
            if body["nonce"] != nonce {
                continue;
            }
            if body["evt"] == "ERROR" {
                // Discord kept the connection; only this activity was refused.
                anysola_core::log::warn(
                    "discord refused the activity",
                    &[("reply", &body["data"].to_string())],
                );
            }
            return Ok(());
        }
    }

    fn close(mut self) {
        let _ = self.set_activity(None);
        let _ = self.pipe.send(&encode(OP_CLOSE, &json!({})));
    }
}

fn set_status(status: &Mutex<Status>, state: &'static str, detail: Option<&'static str>) {
    *status.lock().unwrap_or_else(|e| e.into_inner()) = Status { state, detail };
}

/// The connection's thread: the latest wanted state, sent no more often than `coalesce`.
fn run(receiver: Receiver<Message>, settings: Settings, status: Arc<Mutex<Status>>) {
    let mut wanted = Wanted {
        client_id: None,
        activity: None,
    };
    let mut connection: Option<Connection> = None;
    let mut dirty = false;
    let mut last_sent: Option<Instant> = None;
    let mut next_attempt = Instant::now();
    let mut wait: Option<Duration> = None;
    loop {
        let message = match wait {
            Some(wait) => receiver.recv_timeout(wait),
            None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        let mut exit = None;
        match message {
            Ok(Message::Want(next)) => {
                wanted = next;
                dirty = true;
            }
            Ok(Message::Exit(done)) => exit = Some(Some(done)),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => exit = Some(None),
        }
        // Only the latest state matters.
        while exit.is_none() {
            match receiver.try_recv() {
                Ok(Message::Want(next)) => {
                    wanted = next;
                    dirty = true;
                }
                Ok(Message::Exit(done)) => exit = Some(Some(done)),
                Err(_) => break,
            }
        }
        if let Some(done) = exit {
            if let Some(connection) = connection.take() {
                connection.close();
            }
            set_status(&status, "off", None);
            if let Some(done) = done {
                let _ = done.send(());
            }
            return;
        }
        let Some(id) = wanted.client_id.clone() else {
            if let Some(connection) = connection.take() {
                connection.close();
            }
            set_status(&status, "off", None);
            dirty = false;
            wait = None;
            continue;
        };
        if connection.as_ref().is_some_and(|c| c.client_id != id) {
            if let Some(connection) = connection.take() {
                connection.close();
            }
            next_attempt = Instant::now();
        }
        if connection.is_none() && Instant::now() >= next_attempt {
            match Connection::open(&settings.dirs, &id) {
                Ok(open) => {
                    connection = Some(open);
                    set_status(&status, "connected", None);
                    dirty = true;
                    last_sent = None;
                }
                Err(why) => {
                    set_status(&status, "waiting", Some(why));
                    next_attempt = Instant::now() + settings.retry;
                }
            }
        }
        if let Some(open) = connection.as_mut()
            && dirty
        {
            let due = last_sent.map_or(Instant::now(), |at| at + settings.coalesce);
            if Instant::now() >= due {
                last_sent = Some(Instant::now());
                dirty = false;
                if open.set_activity(wanted.activity.as_ref()).is_err() {
                    // Discord went away: the next attempt sends the state again.
                    connection = None;
                    dirty = true;
                    set_status(&status, "waiting", Some(NOT_RUNNING));
                    next_attempt = Instant::now() + settings.retry;
                }
            }
        }
        let now = Instant::now();
        wait = match (&connection, dirty) {
            (Some(_), true) => Some(
                last_sent
                    .map_or(now, |at| at + settings.coalesce)
                    .saturating_duration_since(now),
            ),
            (Some(_), false) => None,
            (None, _) => Some(next_attempt.saturating_duration_since(now)),
        };
    }
}

#[tauri::command]
pub async fn discord_status(app: State<'_, App>) -> Answer {
    let status = app.discord.get().map_or(
        Status {
            state: "off",
            detail: None,
        },
        Handle::status,
    );
    Ok(json!({"state": status.state, "detail": status.detail}))
}

#[tauri::command]
pub async fn presence(app: State<'_, App>, presence: Presence) -> Answer {
    if let Some(discord) = app.discord.get() {
        discord.set_presence(presence);
    }
    Ok(Value::Null)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    /// A fake Discord on `<dir>/discord-ipc-0`: answers the handshake and every command, and
    /// passes on the commands it got.
    fn fake(dir: &std::path::Path) -> Receiver<(u32, Value)> {
        let listener = UnixListener::bind(dir.join("discord-ipc-0")).unwrap();
        let (sender, frames) = channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let sender = sender.clone();
                std::thread::spawn(move || {
                    loop {
                        let Ok((op, body)) = decode(&mut stream) else {
                            return;
                        };
                        let reply = match op {
                            OP_HANDSHAKE => json!({"cmd": "DISPATCH", "evt": "READY"}),
                            OP_FRAME => {
                                json!({"cmd": body["cmd"], "nonce": body["nonce"], "evt": null})
                            }
                            _ => return,
                        };
                        if stream.write_all(&encode(OP_FRAME, &reply)).is_err() {
                            return;
                        }
                        if sender.send((op, body)).is_err() {
                            return;
                        }
                    }
                });
            }
        });
        frames
    }

    fn temp() -> PathBuf {
        // Socket paths are short on Unix (~100 bytes).
        let dir = PathBuf::from("/tmp").join(format!(
            "ad-{}-{}",
            std::process::id(),
            anysola_core::ids::part()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn enabled() -> prefs::Discord {
        prefs::Discord {
            enabled: true,
            application_id: Some("42".into()),
            ..prefs::Discord::default()
        }
    }

    fn next(frames: &Receiver<(u32, Value)>) -> (u32, Value) {
        frames
            .recv_timeout(Duration::from_secs(5))
            .expect("a frame")
    }

    #[test]
    fn activity_follows_the_preferences() {
        let presence = Presence {
            directory: Some("/home/u/shop/".into()),
            title: Some("Fix checkout".into()),
            model: Some("Claude Sonnet 5.5".into()),
            working: true,
        };
        let mut prefs = enabled();
        let all = activity(&prefs, &presence, 100);
        assert_eq!(all["details"], "Working in shop");
        assert_eq!(
            all["state"], "Claude Sonnet 5.5",
            "the session is hidden by default"
        );
        assert_eq!(all["timestamps"]["start"], 100);
        assert_eq!(all["assets"]["large_image"], LOGO);
        assert_eq!(all["buttons"][0]["url"], SITE);
        assert!(all.get("name").is_none());
        prefs.session = true;
        assert_eq!(activity(&prefs, &presence, 1)["state"], "Fix checkout");
        prefs.project = false;
        prefs.session = false;
        prefs.model = false;
        prefs.elapsed = false;
        let bare = activity(&prefs, &presence, 1);
        assert_eq!(bare["details"], "Working");
        assert!(bare.get("state").is_none() && bare.get("timestamps").is_none());
        let idle = Presence {
            working: false,
            ..presence
        };
        assert_eq!(activity(&enabled(), &idle, 1)["details"], "In shop");
        prefs.project = false;
        assert_eq!(activity(&prefs, &idle, 1)["details"], "Idle");
    }

    #[test]
    fn handshake_activity_coalescing_and_clear() {
        let dir = temp();
        let frames = fake(&dir);
        let coalesce = Duration::from_millis(400);
        let handle = Handle::with(
            enabled(),
            vec![dir.clone()],
            coalesce,
            Duration::from_millis(50),
        );
        let (op, hello) = next(&frames);
        assert_eq!(op, OP_HANDSHAKE);
        assert_eq!(hello, json!({"v": 1, "client_id": "42"}));
        let (op, first) = next(&frames);
        assert_eq!(op, OP_FRAME);
        assert_eq!(first["cmd"], "SET_ACTIVITY");
        assert_eq!(first["args"]["pid"], std::process::id());
        assert_eq!(first["args"]["activity"]["details"], "Idle");
        assert_eq!(handle.status().state, "connected");

        // A burst becomes one update, the last one, after the pause.
        let start = Instant::now();
        for n in 0..5 {
            handle.set_presence(Presence {
                directory: Some("/p/shop".into()),
                title: Some(format!("t{n}")),
                model: Some(format!("m{n}")),
                working: true,
            });
        }
        let (_, update) = next(&frames);
        assert!(start.elapsed() >= coalesce - Duration::from_millis(50));
        assert_eq!(update["args"]["activity"]["state"], "m4");
        assert_eq!(update["args"]["activity"]["details"], "Working in shop");
        assert!(frames.recv_timeout(coalesce * 2).is_err(), "nothing more");

        // Turning it off clears the activity.
        handle.set_prefs(prefs::Discord::default());
        let (_, clear) = next(&frames);
        assert_eq!(clear["args"]["activity"], Value::Null);
        assert_eq!(handle.status().state, "off");
        handle.stop(Duration::from_secs(2));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn exit_clears_and_no_discord_is_waiting() {
        let dir = temp();
        let handle = Handle::with(
            enabled(),
            vec![dir.clone()],
            Duration::ZERO,
            Duration::from_millis(50),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while handle.status().state != "waiting" && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(handle.status().detail, Some(NOT_RUNNING));
        // Discord starts: the next attempt finds it.
        let frames = fake(&dir);
        assert_eq!(next(&frames).0, OP_HANDSHAKE);
        assert_eq!(next(&frames).1["cmd"], "SET_ACTIVITY");
        handle.stop(Duration::from_secs(2));
        let (_, clear) = next(&frames);
        assert_eq!(clear["args"]["activity"], Value::Null);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_refused_application_id_says_so() {
        let dir = temp();
        let listener = UnixListener::bind(dir.join("discord-ipc-0")).unwrap();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                if decode(&mut stream).is_ok() {
                    let _ = stream.write_all(&encode(
                        OP_CLOSE,
                        &json!({"code": 4000, "message": "Invalid Client ID"}),
                    ));
                }
            }
        });
        let handle = Handle::with(
            enabled(),
            vec![dir.clone()],
            Duration::ZERO,
            Duration::from_secs(60),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while handle.status().state != "waiting" && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(handle.status().detail, Some(REFUSED));
        handle.stop(Duration::from_secs(1));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_read_that_never_answers_gives_up() {
        // What the Windows pipe relies on: a peer that accepts and never replies.
        let (mine, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        let start = Instant::now();
        let error = timed(mine, Duration::from_millis(200)).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(2));
        let (mine, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
        peer.write_all(&encode(OP_FRAME, &json!({"evt": "READY"})))
            .unwrap();
        assert_eq!(
            timed(mine, Duration::from_secs(5)).unwrap(),
            (OP_FRAME, json!({"evt": "READY"}))
        );
        drop(theirs);
    }

    #[test]
    fn without_an_application_id_it_is_off() {
        let prefs = prefs::Discord {
            enabled: true,
            ..prefs::Discord::default()
        };
        let handle = Handle::with(prefs, Vec::new(), Duration::ZERO, Duration::ZERO);
        assert_eq!(
            handle.status(),
            Status {
                state: "off",
                detail: Some(NO_ID)
            }
        );
        handle.stop(Duration::from_secs(1));
    }
}
