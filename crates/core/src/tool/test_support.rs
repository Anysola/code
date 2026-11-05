//! A [`ToolContext`] for unit tests: a temporary session directory, a gate that records every ask
//! and answers with a fixed decision.

use std::path::Path;
use std::sync::{Arc, Mutex};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;

use super::question::{Question, QuestionReply};
use super::subagent::SubagentRequest;
use super::websearch::{SearchResult, WebSearch};
use super::{
    OutputLimits, PathLocks, SessionHost, SessionNotifier, ToolContext, ToolError, ToolResult,
};
use crate::paths::Paths;
use crate::permission::{PermissionError, PermissionGate, PermissionRequest};
use crate::skill::Skill;

#[derive(Default)]
pub struct RecordingGate {
    pub asked: Mutex<Vec<PermissionRequest>>,
    /// Answer for every ask; `None` allows.
    pub answer: Mutex<Option<PermissionError>>,
    /// Resources a rule denies.
    pub denied: Mutex<Vec<String>>,
}

impl PermissionGate for RecordingGate {
    fn ask(&self, request: PermissionRequest) -> BoxFuture<'_, Result<(), PermissionError>> {
        self.asked.lock().unwrap().push(request);
        let answer = self.answer.lock().unwrap().clone();
        async move { answer.map_or(Ok(()), Err) }.boxed()
    }

    fn allows(&self, _action: &str, _resources: &[String]) -> bool {
        self.answer.lock().unwrap().is_none()
    }

    fn denies(&self, _action: &str, resources: &[String]) -> bool {
        let denied = self.denied.lock().unwrap();
        resources.iter().any(|r| denied.contains(r))
    }
}

#[derive(Default)]
pub struct RecordingNotifier {
    pub messages: Mutex<Vec<(String, String)>>,
}

impl SessionNotifier for RecordingNotifier {
    fn synthetic(&self, session_id: &str, text: String) {
        self.messages
            .lock()
            .unwrap()
            .push((session_id.to_string(), text));
    }
}

struct NoSearch;

impl WebSearch for NoSearch {
    fn search<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<Vec<SearchResult>, String>> {
        async { Err("no search in tests".to_string()) }.boxed()
    }
}

/// Records questions and subagent requests; answers questions with `question_reply`.
#[derive(Default)]
pub struct TestSession {
    pub skills: Mutex<Vec<Skill>>,
    pub questions: Mutex<Vec<Vec<Question>>>,
    pub question_reply: Mutex<Option<QuestionReply>>,
    pub subagents: Mutex<Vec<SubagentRequest>>,
}

impl SessionHost for TestSession {
    fn ask_questions(
        &self,
        _session_id: &str,
        questions: Vec<Question>,
    ) -> BoxFuture<'_, QuestionReply> {
        self.questions.lock().unwrap().push(questions);
        let reply = self
            .question_reply
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(QuestionReply::Dismissed(None));
        async move { reply }.boxed()
    }

    fn skill(&self, id: &str) -> Option<Skill> {
        self.skills
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.id == id)
            .cloned()
    }

    fn subagent<'a>(
        &'a self,
        _ctx: &'a ToolContext,
        request: SubagentRequest,
    ) -> BoxFuture<'a, ToolResult> {
        self.subagents.lock().unwrap().push(request);
        async { Err(ToolError::execution("no subagents in tests")) }.boxed()
    }
}

/// A database outside the test directory, so it never shows up in listings.
struct TestHost {
    db: crate::storage::Db,
    config: Arc<crate::config::Config>,
    _dir: tempfile::TempDir,
}

impl TestHost {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        TestHost {
            db: crate::storage::Db::open(&dir.path().join("test.db")).expect("test database"),
            config: Arc::default(),
            _dir: dir,
        }
    }
}

impl crate::codemode::Host for TestHost {
    fn db(&self) -> &crate::storage::Db {
        &self.db
    }

    fn models(&self) -> Arc<[crate::model::Model]> {
        Arc::new([])
    }

    fn config(&self) -> Arc<crate::config::Config> {
        self.config.clone()
    }
}

pub struct TestContext {
    pub ctx: ToolContext,
    pub gate: Arc<RecordingGate>,
    pub notifier: Arc<RecordingNotifier>,
    pub session: Arc<TestSession>,
}

/// A context whose session directory and project worktree are `dir`, with home and the data dir
/// under `dir/.home`.
pub fn context(dir: &Path) -> TestContext {
    let gate = Arc::new(RecordingGate::default());
    let notifier = Arc::new(RecordingNotifier::default());
    let session = Arc::new(TestSession::default());
    let home = dir.join(".home");
    let paths = Paths::isolated(home);
    let ctx = ToolContext {
        session_id: "ses_test".into(),
        message_id: "msg_test".into(),
        call_id: "toolu_test".into(),
        agent: "build".into(),
        directory: dir.to_path_buf(),
        worktree: dir.to_path_buf(),
        project_id: "global".into(),
        paths: Arc::new(paths),
        permissions: gate.clone(),
        cancel: Default::default(),
        env: Arc::new(std::env::vars_os().collect()),
        configured_shell: None,
        limits: OutputLimits::default(),
        locks: Arc::new(PathLocks::default()),
        notifier: notifier.clone(),
        web_search: Arc::new(NoSearch),
        host: Arc::new(TestHost::new()),
        session: session.clone(),
    };
    TestContext {
        ctx,
        gate,
        notifier,
        session,
    }
}
