//! What the interactive UI reads besides prompting: the directory and project, recent sessions,
//! a session's decoded messages, account routes, and `!` shell commands.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use super::{Core, CoreError};
use crate::gateway::account::{Method, Reply};
use crate::session::message::{AssistantTime, Message, ShellData, ShellOutput};
use crate::storage::SessionSummary;
use crate::storage::{ImportError, SessionRow};
use crate::time::now_ms;

/// The instruction file a note is added to.
const MEMORY_FILE: &str = "AGENTS.md";

/// Where `/memory` notes go: see [`Core::memory_files`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryFiles {
    /// `<root>/AGENTS.md`.
    pub project: PathBuf,
    /// `~/.anysola/AGENTS.md`.
    pub personal: PathBuf,
    /// The checkout the project file belongs to.
    pub root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryScope {
    Project,
    Personal,
}

/// Why a side question has no answer; `kind` as the window shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AsideKind {
    /// The session's budget is spent.
    Budget,
    /// Too many requests at once (after the retries for calls that never ran).
    RateLimit,
    /// The account's 5-hour or weekly window is full.
    UsageLimit,
    /// The conversation does not fit the model.
    TooLong,
    Cancelled,
    /// The model answered with no text (only a tool call, say).
    Empty,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, thiserror::Error)]
#[error("{message}")]
pub struct AsideError {
    pub kind: AsideKind,
    pub message: String,
}

impl AsideError {
    fn other(message: String) -> Self {
        AsideError {
            kind: AsideKind::Other,
            message,
        }
    }

    fn of(error: crate::gateway::error::SessionError) -> Self {
        let kind = match error.kind.as_str() {
            "budget" => AsideKind::Budget,
            "provider.rate-limit" => AsideKind::RateLimit,
            crate::gateway::error::USAGE_LIMIT_KIND => AsideKind::UsageLimit,
            _ if error.status == Some(413) => AsideKind::TooLong,
            _ => AsideKind::Other,
        };
        AsideError {
            kind,
            message: error.message,
        }
    }
}

/// Appends `line` to the instruction file `path` (see [`Core::add_memory`]); `inside`: the
/// checkout a project file must stay in.
fn append_note(path: &Path, inside: Option<&Path>, line: &str) -> Result<(), CoreError> {
    use std::io::Write;
    let fail = |what: String| CoreError::Memory(what);
    let shown = path.display();
    let existing = match std::fs::symlink_metadata(path) {
        Ok(meta) => Some(meta),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(fail(format!("Can’t read {shown}: {e}"))),
    };
    let Some(meta) = existing else {
        // A project file goes only into its checkout as it is: a worktree folder that is gone
        // must not come back as a plain folder holding a note.
        if let Some(root) = inside
            && !root.is_dir()
        {
            return Err(fail(format!(
                "{} is gone, so no note was added.",
                root.display()
            )));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| fail(format!("Can’t create {shown}: {e}")))?;
        }
        // `create_new`: a link planted in the meantime is not followed.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| fail(format!("Can’t create {shown}: {e}")))?;
        return file
            .write_all(line.as_bytes())
            .map_err(|e| fail(format!("Can’t write {shown}: {e}")));
    };
    if meta.file_type().is_symlink()
        && let Some(root) = inside
    {
        let real = std::fs::canonicalize(path)
            .map_err(|e| fail(format!("Can’t follow the link {shown}: {e}")))?;
        let root = crate::paths::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        if !real.starts_with(&root) {
            return Err(fail(format!(
                "{shown} links outside its project ({}), so no note was added.",
                real.display()
            )));
        }
    }
    let current = crate::paths::read_text(path, crate::paths::TEXT_CAP).map_err(|reason| {
        fail(match reason {
            crate::paths::Unread::Io(e) => format!("Can’t read {shown}: {e}"),
            _ => format!("{shown} isn’t a text file of at most 4 MiB, so no note was added."),
        })
    })?;
    let separator = if current.is_empty() || current.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    if (current.len() + separator.len() + line.len()) as u64 > crate::paths::TEXT_CAP {
        return Err(fail(format!(
            "{shown} would be larger than 4 MiB, which instructions can’t be."
        )));
    }
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|e| fail(format!("Can’t write {shown}: {e}")))?;
    file.write_all(format!("{separator}{line}").as_bytes())
        .map_err(|e| fail(format!("Can’t write {shown}: {e}")))
}

/// A hung `!` command must not hold the composer forever.
const SHELL_TIMEOUT: Duration = Duration::from_secs(120);

impl Core {
    pub fn directory(&self) -> &Path {
        &self.shared.directory
    }

    /// The latest root sessions of this project, newest first.
    pub fn recent_sessions(&self, limit: u64) -> Result<Vec<SessionSummary>, CoreError> {
        Ok(self
            .shared
            .db
            .root_sessions(&self.shared.project.id, limit)?)
    }

    /// Top-level sessions newest first, of this project or of all, whose title contains `query`.
    pub fn sessions(
        &self,
        all: bool,
        query: &str,
        limit: u64,
    ) -> Result<Vec<SessionSummary>, CoreError> {
        let project = (!all).then_some(self.shared.project.id.as_str());
        Ok(self.shared.db.search_sessions(project, query, limit)?)
    }

    /// Deletes a session and its children; false when it did not exist.
    pub fn delete_session(&self, session_id: &str) -> Result<bool, CoreError> {
        // A command waiting on a question for it goes with it.
        if let Some(token) = self
            .shared
            .preparing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(session_id)
        {
            for (_, token) in token {
                token.cancel();
            }
        }
        let tree = self.shared.db.session_tree(session_id)?;
        let deleted = self.shared.db.delete_session(session_id)?;
        self.shared.usage.forget(&tree);
        for id in &tree {
            if let Some(cache) = self.shared.paths.sandbox_cache_dir(id) {
                let _ = std::fs::remove_dir_all(cache);
            }
        }
        Ok(deleted)
    }

    /// `{info, messages}` as `anysola session export` writes it.
    pub fn export(&self, session_id: &str) -> Result<Option<Value>, CoreError> {
        Ok(self.shared.db.export_session(session_id)?)
    }

    pub fn default_budget(&self) -> Option<u64> {
        self.shared.usage.default_budget()
    }

    pub fn set_default_budget(&self, budget: Option<u64>) -> Result<(), CoreError> {
        Ok(self.shared.usage.set_default_budget(budget)?)
    }

    /// Admits a hidden note (editor context) that goes before the next prompt; it does not
    /// start a turn by itself.
    pub fn admit_note(&self, session_id: &str, text: &str) -> Result<(), CoreError> {
        crate::session::inbox::enqueue_synthetic(
            &self.shared.db,
            session_id,
            text,
            None,
            None,
            crate::session::inbox::Delivery::Steer,
        )?;
        Ok(())
    }

    /// `/btw`: a one-shot answer to `question` from the session's context, also while a turn of
    /// it runs. Nothing is written to the session (usage is recorded, as for any model call);
    /// the request carries the session's tools unchanged (the cached prefix) but a tool call in
    /// the answer is ignored. Cancelling `cancel` ends the request at once.
    pub async fn aside(
        &self,
        session_id: &str,
        question: &str,
        cancel: &CancellationToken,
    ) -> Result<String, AsideError> {
        const INSTRUCTIONS: &str = "The user is asking a quick side question about the conversation so far. Answer directly and concisely in markdown from what you already know. Do not call any tools and do not take any actions.";
        let session = self
            .session(session_id)
            .map_err(|e| AsideError::other(e.to_string()))?
            .ok_or_else(|| AsideError::other(CoreError::SessionNotFound.to_string()))?;
        let prompt = format!("{INSTRUCTIONS}\n\n{question}");
        let how = crate::session::step::OneShot { aside: true };
        let answer = tokio::select! {
            answer = crate::session::step::generate_with(&self.shared, &session, &prompt, how) => answer,
            _ = cancel.cancelled() => return Err(AsideError {
                kind: AsideKind::Cancelled,
                message: "The side question was cancelled.".into(),
            }),
        };
        match answer {
            Ok(text) if text.trim().is_empty() => Err(AsideError {
                kind: AsideKind::Empty,
                message: "The model gave no answer. Try asking again.".into(),
            }),
            Ok(text) => Ok(text.trim().to_string()),
            Err(crate::session::step::GenerateError::Overflow) => Err(AsideError {
                kind: AsideKind::TooLong,
                message: "The conversation is too long for a side question.".into(),
            }),
            Err(crate::session::step::GenerateError::Failed(error)) => Err(AsideError::of(error)),
        }
    }

    /// SessionEnd hooks (`reason: "other"`) for these sessions, those of them that started in
    /// this process: before they are deleted. Never blocks anything; waits at most
    /// [`crate::hooks::SESSION_END_BUDGET`], then kills what still runs.
    pub async fn end_sessions(&self, session_ids: &[String]) {
        crate::hooks::fire::session_end(&self.shared, Some(session_ids), "other").await;
    }

    /// SessionEnd hooks for every session that started in this process, as the app stops this
    /// core: at most [`crate::hooks::SESSION_END_BUDGET`] for all of them together.
    pub async fn end_all_sessions(&self) {
        crate::hooks::fire::session_end(&self.shared, None, "other").await;
    }

    /// Imports a session export (untrusted; see [`crate::storage::import`]) as a new root
    /// session of this core's folder and announces it (`session.created`).
    pub fn import_session(&self, bytes: &[u8]) -> Result<SessionRow, ImportError> {
        let directory = self.shared.directory.display().to_string();
        let row = self
            .shared
            .db
            .import_session(bytes, &self.shared.project, &directory)?;
        let _ = self
            .shared
            .events
            .send(crate::events::Event::SessionCreated {
                session_id: row.id.clone(),
                parent_id: None,
                title: row.title.clone(),
            });
        Ok(row)
    }

    /// [`Core::import_session`] of the file at `path` (a regular file of at most
    /// [`crate::storage::import::MAX_FILE_BYTES`], read no further).
    pub fn import_session_file(&self, path: &Path) -> Result<SessionRow, ImportError> {
        let bytes = crate::storage::import::read_file(path)?;
        self.import_session(&bytes)
    }

    /// The instruction files a note can go to: the project's `AGENTS.md` at the root of the
    /// checkout the session works in (its own worktree for a worktree session; without a
    /// session, this core's project), and the personal `~/.anysola/AGENTS.md`.
    pub fn memory_files(&self, session_id: Option<&str>) -> Result<MemoryFiles, CoreError> {
        let root = match session_id {
            Some(id) => {
                let session = self.session(id)?.ok_or(CoreError::SessionNotFound)?;
                self.shared.checkout_of(&session).root
            }
            None => self.shared.project.worktree.clone(),
        };
        Ok(MemoryFiles {
            project: root.join(MEMORY_FILE),
            personal: self.shared.paths.data.join(MEMORY_FILE),
            root,
        })
    }

    /// Adds `text` as a line (`- text`) at the end of an instruction file (see
    /// [`Core::memory_files`]), creating it when missing. The user asks for this, so nothing is
    /// asked; the file must be what instruction files may be: a regular file (a project file
    /// may be a link only to a regular file inside its checkout) of at most 4 MiB, in UTF-8.
    /// Returns the file written.
    pub fn add_memory(
        &self,
        session_id: Option<&str>,
        scope: MemoryScope,
        text: &str,
    ) -> Result<PathBuf, CoreError> {
        let files = self.memory_files(session_id)?;
        let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            return Err(CoreError::Memory("Write the note to add.".into()));
        }
        let (path, inside) = match scope {
            MemoryScope::Project => (files.project, Some(files.root)),
            MemoryScope::Personal => (files.personal, None),
        };
        append_note(&path, inside.as_deref(), &format!("- {line}\n"))?;
        Ok(path)
    }
    /// The project's other checkouts.
    pub fn worktrees(&self) -> Vec<crate::worktree::Worktree> {
        crate::worktree::list(&self.shared.db, &self.shared.project.id)
    }

    /// Adds a detached worktree of the project named `name` (suffixed when taken).
    pub async fn create_worktree(&self, name: &str) -> Result<std::path::PathBuf, String> {
        let shared = &self.shared;
        let base = crate::worktree::base(
            &self.loaded().config,
            &shared.paths,
            &shared.project.worktree,
            &shared.project.id,
        );
        crate::worktree::create(
            &shared.db,
            &shared.paths,
            &base,
            &shared.project.id,
            &shared.project.worktree,
            name,
        )
        .await
    }

    /// Removes a worktree this project created; `force` drops its file changes.
    pub async fn remove_worktree(
        &self,
        directory: &Path,
        force: bool,
    ) -> Result<(), crate::worktree::RemoveError> {
        crate::worktree::remove(&self.shared.db, &self.shared.project.id, directory, force).await
    }

    /// The session's stored messages, decoded, oldest first.
    pub fn messages(&self, session_id: &str) -> Result<Vec<(String, Message)>, CoreError> {
        Ok(self
            .shared
            .db
            .messages(session_id)?
            .into_iter()
            .map(|row| {
                let message = Message::decode(&row.kind, &row.data);
                (row.id, message)
            })
            .collect())
    }

    /// The staged revert boundary (`messageID`), if any.
    pub fn revert_boundary(&self, session_id: &str) -> Result<Option<String>, CoreError> {
        Ok(self.shared.db.revert(session_id)?.and_then(|r| {
            r.get("messageID")
                .and_then(Value::as_str)
                .map(str::to_string)
        }))
    }

    /// The staged revert's file changes (`file`, `status`, `additions`, `deletions`).
    pub fn revert_files(&self, session_id: &str) -> Result<Vec<Value>, CoreError> {
        Ok(self
            .shared
            .db
            .revert(session_id)?
            .and_then(|r| r.get("files").and_then(Value::as_array).cloned())
            .unwrap_or_default())
    }

    pub fn signed_in(&self) -> bool {
        self.shared.gateway.token().is_some()
    }

    /// An account route (`/me`, `/usage?days=7`, …); `Err` is the transport failure text.
    pub async fn account(&self, method: Method, route: &str) -> Result<Reply, String> {
        self.shared.gateway.account_call(method, route, None).await
    }

    /// Runs a `!` command in the session directory and stores it as a `shell` message: its
    /// combined output, and `exit` (or `timedOut`) in the metadata.
    pub async fn run_shell(&self, session_id: &str, command: &str) -> Result<(), CoreError> {
        let session = self
            .session(session_id)?
            .ok_or(CoreError::SessionNotFound)?;
        let configured = self.loaded().config.shell().map(str::to_string);
        let (program, args) = crate::tool::shell::invocation(configured.as_deref(), command);
        let created = now_ms();
        let mut process = crate::console::tokio_command(program);
        process
            .args(args)
            .current_dir(&session.directory)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        process.process_group(0);
        let mark = crate::children::mark_tokio(&mut process);
        let child = process.spawn().map(|child| {
            let pid = child.id();
            let ours = crate::children::adopt_tokio(&child, crate::children::Leads::Group, mark);
            (child, pid, ours)
        });
        struct KillGroup(Option<u32>);
        impl Drop for KillGroup {
            fn drop(&mut self) {
                if let Some(pid) = self.0.take() {
                    crate::tool::shell::exec::force_kill(pid);
                }
            }
        }
        let output = async move {
            use tokio::io::AsyncReadExt;
            let (mut child, pid, ours) = child?;
            // Declared after `child` so on timeout/cancel `guard` drops (killing the whole tree)
            // before `child` (`kill_on_drop`) terminates and closes only the shell pid.
            let mut guard = KillGroup(pid);
            let mut stdout = child.stdout.take();
            let mut stderr = child.stderr.take();
            let read_out = async move {
                let mut buf = Vec::new();
                if let Some(ref mut s) = stdout {
                    let _ = s.read_to_end(&mut buf).await;
                }
                buf
            };
            let read_err = async move {
                let mut buf = Vec::new();
                if let Some(ref mut s) = stderr {
                    let _ = s.read_to_end(&mut buf).await;
                }
                buf
            };
            let (status, stdout, stderr) = tokio::join!(child.wait(), read_out, read_err);
            guard.0 = None;
            drop(ours);
            Ok::<_, std::io::Error>(std::process::Output {
                status: status?,
                stdout,
                stderr,
            })
        };
        let (output, metadata) = match tokio::time::timeout(SHELL_TIMEOUT, output).await {
            Ok(Ok(out)) => {
                let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
                text.push_str(&String::from_utf8_lossy(&out.stderr));
                (text, json!({ "exit": out.status.code() }))
            }
            Ok(Err(error)) => (error.to_string(), json!({ "exit": 1 })),
            Err(_) => (String::new(), json!({ "timedOut": true })),
        };
        let data = ShellData {
            time: AssistantTime {
                created,
                streamed: None,
                completed: Some(now_ms()),
            },
            command: command.to_string(),
            output: Some(ShellOutput { output }),
            metadata: Some(metadata),
            extra: Map::new(),
        };
        let value = serde_json::to_value(&data).unwrap_or(Value::Null);
        self.shared
            .db
            .append_message(session_id, &crate::ids::message(), "shell", &value)?;
        Ok(())
    }
}

/// Session metadata key of the directories an ACP client granted.
const GRANTED: &str = "opencode.acp.additionalDirectories";

impl Core {
    /// The directories granted with [`Core::grant_directories`].
    pub fn granted_directories(&self, session_id: &str) -> Result<Vec<String>, CoreError> {
        let metadata = self.shared.db.session_metadata(session_id)?;
        Ok(metadata
            .get(GRANTED)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default())
    }

    /// Sets what the session asks about even where its rules allow it (see [`Floor`](crate::session::gate::Floor)).
    pub fn set_permission_floor(
        &self,
        session_id: &str,
        floor: crate::session::gate::Floor,
    ) -> Result<(), CoreError> {
        let db = &self.shared.db;
        let mut metadata = db.session_metadata(session_id)?;
        let current = metadata.get(crate::session::gate::FLOOR_KEY).cloned();
        let wanted = (floor != crate::session::gate::Floor::None).then(|| json!(floor.as_str()));
        if current == wanted {
            return Ok(());
        }
        match wanted {
            Some(value) => metadata.insert(crate::session::gate::FLOOR_KEY.into(), value),
            None => metadata.remove(crate::session::gate::FLOOR_KEY),
        };
        let rules = db.session_permissions(session_id)?;
        Ok(db.set_session_access(session_id, &rules, &metadata)?)
    }

    /// Replaces the directories outside the session's own that its tools may use without asking.
    /// The grants lead the session's rules, so its other rules keep precedence for those paths.
    pub fn grant_directories(
        &self,
        session_id: &str,
        directories: &[String],
    ) -> Result<(), CoreError> {
        use crate::permission::{Effect, Rule};
        let db = &self.shared.db;
        let rules = |directories: &[String]| -> Vec<Rule> {
            let mut resources: Vec<String> = Vec::new();
            for directory in directories {
                // Tools compare written paths without following symlinks: grant both spellings.
                let resolved = crate::paths::canonicalize(directory)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| directory.clone());
                for root in [directory.clone(), resolved] {
                    let resource = Path::new(&root).join("*").display().to_string();
                    if !resources.contains(&resource) {
                        resources.push(resource);
                    }
                }
            }
            resources
                .into_iter()
                .map(|r| Rule::new("external_directory", &r, Effect::Allow))
                .collect()
        };
        let previous = self.granted_directories(session_id)?;
        if previous == directories {
            return Ok(());
        }
        let owned = rules(&previous);
        let mut granted = rules(directories);
        granted.extend(
            db.session_permissions(session_id)?
                .into_iter()
                .filter(|rule| !owned.contains(rule)),
        );
        let mut metadata = db.session_metadata(session_id)?;
        metadata.remove(GRANTED);
        if !directories.is_empty() {
            metadata.insert(GRANTED.into(), json!(directories));
        }
        Ok(db.set_session_access(session_id, &granted, &metadata)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_is_a_line_at_the_end_of_a_regular_file_in_its_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("AGENTS.md");
        append_note(&file, Some(&root), "- one\n").unwrap();
        append_note(&file, Some(&root), "- two\n").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "- one\n- two\n");
        std::fs::write(&file, "# Rules").unwrap();
        append_note(&file, Some(&root), "- three\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "# Rules\n- three\n"
        );
        #[cfg(unix)]
        {
            // A link to a file in the checkout is written through (AGENTS.md -> CLAUDE.md).
            let claude = root.join("CLAUDE.md");
            std::fs::write(&claude, "c\n").unwrap();
            let linked = root.join("sub/AGENTS.md");
            std::fs::create_dir_all(linked.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&claude, &linked).unwrap();
            append_note(&linked, Some(&root), "- four\n").unwrap();
            assert_eq!(std::fs::read_to_string(&claude).unwrap(), "c\n- four\n");
            assert!(
                std::fs::symlink_metadata(&linked)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            // A link out of the checkout, or to something that is not a regular file, is refused.
            let outside = dir.path().join("config.json");
            std::fs::write(&outside, "{}").unwrap();
            let escape = root.join("x/AGENTS.md");
            std::fs::create_dir_all(escape.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&outside, &escape).unwrap();
            assert!(append_note(&escape, Some(&root), "- no\n").is_err());
            assert_eq!(std::fs::read_to_string(&outside).unwrap(), "{}");
            let device = root.join("y/AGENTS.md");
            std::fs::create_dir_all(device.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink("/dev/zero", &device).unwrap();
            assert!(append_note(&device, None, "- no\n").is_err());
        }
        let folder = root.join("z/AGENTS.md");
        std::fs::create_dir_all(&folder).unwrap();
        assert!(append_note(&folder, Some(&root), "- no\n").is_err());
        // A worktree that is gone is not made again as a plain folder for a note.
        let gone = dir.path().join("worktree");
        assert!(append_note(&gone.join("AGENTS.md"), Some(&gone), "- no\n").is_err());
        assert!(!gone.exists());
    }
}
