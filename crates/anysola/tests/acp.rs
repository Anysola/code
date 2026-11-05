//! `anysola acp` against the 0.5.1 transcripts: a scripted client sends the captured requests
//! (answering permission requests as the capture did) and compares what comes back after
//! normalizing ids, times, paths, token counts and the version.
#![cfg(unix)]

mod common;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Recorded, Reply, TestHome};
use base64::Engine;
use common::BIN;
use regex::Regex;
use serde_json::{Value, json};

const CAPTURED_CWD: &str = "/home/ubuntu/anysola-code/rust/testing/runs/integrations/home/my-app";
const ANSWER: &str = "This project is a small TypeScript CLI. `src/index.ts` parses the flags and `src/run.ts` does the work; tests live in `test/` and run with `bun test`.";

/// When this test binary runs as the fake MCP server: one prompt `greet`, no tools.
fn serve_mcp_if_asked() {
    if std::env::var("FAKE_ACP_MCP").is_err() {
        return;
    }
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let Ok(message) = serde_json::from_str::<Value>(&line.unwrap_or_default()) else {
            continue;
        };
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let result = match message["method"].as_str().unwrap_or_default() {
            "initialize" => json!({"protocolVersion": "2025-11-25",
                "capabilities": {"tools": {}, "prompts": {}}, "serverInfo": {"name": "f", "version": "1"}}),
            "tools/list" => json!({"tools": []}),
            "prompts/list" => {
                json!({"prompts": [{"name": "greet", "description": "Greet someone"}]})
            }
            _ => json!({}),
        };
        let reply = json!({"jsonrpc": "2.0", "id": id, "result": result});
        writeln!(out, "{reply}").unwrap();
        out.flush().unwrap();
    }
    std::process::exit(0);
}

fn fake_server() -> (String, Vec<String>) {
    let exe = std::env::current_exe().unwrap().display().to_string();
    let args = ["--exact", "mcp_server", "--nocapture", "--test-threads=1", "-q"];
    (exe, args.iter().map(|a| a.to_string()).collect())
}

#[test]
fn mcp_server() {
    serve_mcp_if_asked();
}

fn sse(request: &Recorded, stream: Stream) -> Reply {
    let model = request.json()["model"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    Reply::Sse {
        body: stream
            .usage(json!({"input_tokens": 10, "output_tokens": 1}))
            .finish(&model, Some(1)),
        before: Some(100),
    }
}

/// `TOOLTEST:<tool>:<base64 input>END` calls the tool, then answers `done`.
fn gateway() -> FakeGateway {
    FakeGateway::start(|request, _| {
        if request.is_title() {
            return sse(request, Stream::new().text("Fake title"));
        }
        let body = request.json();
        let last = body["messages"].as_array().and_then(|m| m.last()).cloned();
        let last = last.unwrap_or_default();
        let text = last.to_string();
        if text.contains("tool_result") {
            return sse(request, Stream::new().text("done"));
        }
        let marker = Regex::new(r"TOOLTEST:(\w+):([A-Za-z0-9+/=]+)END").unwrap();
        if let Some(found) = marker.captures(&text) {
            let input = base64::engine::general_purpose::STANDARD
                .decode(&found[2])
                .unwrap();
            let input: Value = serde_json::from_slice(&input).unwrap();
            return sse(request, Stream::new().tool("toolu_1", &found[1], input));
        }
        sse(request, Stream::new().text(ANSWER))
    })
}

struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: mpsc::Receiver<Value>,
}

impl Client {
    fn start(home: &TestHome, cwd: &std::path::Path) -> Client {
        let mut command = Command::new(BIN);
        command.arg("acp").current_dir(cwd).env_clear();
        if let Ok(path) = std::env::var("PATH") {
            command.env("PATH", path);
        }
        command.envs(home.env().iter().map(|(k, v)| (k, v)));
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().expect("spawn anysola acp");
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let line = line.unwrap();
                let message = serde_json::from_str(&line).unwrap_or(json!({"unparsed": line}));
                if sender.send(message).is_err() {
                    return;
                }
            }
        });
        Client {
            stdin: child.stdin.take(),
            child,
            lines,
        }
    }

    fn send(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    fn next(&self) -> Value {
        self.lines
            .recv_timeout(Duration::from_secs(30))
            .expect("a message from the agent")
    }

    /// Everything that arrives until the response to `request`; the agent's own requests are
    /// answered with `answers` in order.
    fn call(&mut self, request: &Value, answers: &mut Vec<Value>) -> Vec<Value> {
        self.send(request);
        let mut received = Vec::new();
        loop {
            let message = self.next();
            let done = message.get("method").is_none() && message["id"] == request["id"];
            if message.get("method").is_some() && message.get("id").is_some() {
                let mut answer = answers.remove(0);
                answer["id"] = message["id"].clone();
                self.send(&answer);
            }
            received.push(message);
            if done {
                return received;
            }
        }
    }
}

/// Ids by order of appearance, and the values that differ between runs.
struct Normalizer {
    ids: HashMap<String, String>,
    cwd: String,
}

impl Normalizer {
    fn new(cwd: &str) -> Normalizer {
        Normalizer {
            ids: HashMap::new(),
            cwd: cwd.to_string(),
        }
    }

    fn text(&mut self, text: &str) -> String {
        let text = text.replace(&self.cwd, "<cwd>");
        let pattern = Regex::new(r"\b(ses|msg|toolu)_[A-Za-z0-9]+").unwrap();
        let mut out = String::new();
        let mut last = 0;
        for found in pattern.find_iter(&text) {
            out.push_str(&text[last..found.start()]);
            let count = self.ids.len();
            let prefix = found.as_str().split('_').next().unwrap().to_string();
            let id = self
                .ids
                .entry(found.as_str().to_string())
                .or_insert_with(|| format!("{prefix}_<{count}>"));
            out.push_str(id);
            last = found.end();
        }
        out.push_str(&text[last..]);
        out
    }

    fn value(&mut self, value: &Value) -> Value {
        match value {
            Value::String(s) => Value::String(self.text(s)),
            Value::Array(items) => Value::Array(items.iter().map(|v| self.value(v)).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| {
                        let v = match k.as_str() {
                            "used" | "inputTokens" | "outputTokens" | "totalTokens" => json!(0),
                            "updatedAt" | "title" if v.is_string() && map.contains_key("sessionId") => {
                                json!("<varies>")
                            }
                            "version" if map.contains_key("name") => json!("<ver>"),
                            "sessions" => self.value(&json!(v.as_array().unwrap()[..1])),
                            _ => self.value(v),
                        };
                        (k.clone(), v)
                    })
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    /// The messages to compare: commands updates (which arrive as MCP servers connect) are
    /// left out and repeated updates collapsed.
    fn sequence(&mut self, messages: &[Value]) -> Vec<Value> {
        let mut out: Vec<Value> = Vec::new();
        for message in messages {
            if message["params"]["update"]["sessionUpdate"] == "available_commands_update" {
                continue;
            }
            let message = self.value(message);
            if out.last() != Some(&message) {
                out.push(message);
            }
        }
        out
    }
}

fn transcript(name: &str) -> Vec<(bool, Value)> {
    let text = std::fs::read_to_string(common::spec(&format!("integrations/{name}"))).unwrap();
    text.lines()
        .filter_map(|line| {
            let (outgoing, json) = match line.split_at(3) {
                (">> ", json) => (true, json),
                ("<< ", json) => (false, json),
                _ => return None,
            };
            Some((outgoing, serde_json::from_str(json).unwrap()))
        })
        .collect()
}

fn last_commands(messages: &[Value]) -> Option<Value> {
    messages
        .iter()
        .rev()
        .find(|m| m["params"]["update"]["sessionUpdate"] == "available_commands_update")
        .map(|m| m["params"]["update"]["availableCommands"].clone())
}

fn replay(name: &str) {
    let gateway = gateway();
    let home = TestHome::new(&gateway.url);
    let cwd = common::my_app(&home);
    let (exe, args) = fake_server();
    let mut server = json!({"type": "local", "command": [exe], "environment": {"FAKE_ACP_MCP": "1"}});
    server["command"]
        .as_array_mut()
        .unwrap()
        .extend(args.iter().map(|a| json!(a)));
    std::fs::write(
        cwd.join("opencode.json"),
        json!({"mcp": {"testsrv": server}}).to_string(),
    )
    .unwrap();
    let cwd = cwd.display().to_string();
    let mut client = Client::start(&home, std::path::Path::new(&cwd));
    let entries = transcript(name);
    let mut theirs = Normalizer::new(CAPTURED_CWD);
    let mut ours = Normalizer::new(&cwd);
    let mut sessions: HashMap<String, String> = HashMap::new();
    let mut their_commands = None;
    let mut our_commands = Vec::new();
    let mut index = 0;
    while index < entries.len() {
        let (outgoing, request) = &entries[index];
        index += 1;
        assert!(*outgoing && request.get("method").is_some());
        let mut expected = Vec::new();
        let mut answers = Vec::new();
        while index < entries.len() && !(entries[index].0 && entries[index].1.get("method").is_some()) {
            let (outgoing, message) = &entries[index];
            if *outgoing {
                answers.push(message.clone());
            } else {
                expected.push(message.clone());
            }
            index += 1;
        }
        let mut sent = request.to_string().replace(CAPTURED_CWD, &cwd);
        for (theirs, ours) in &sessions {
            sent = sent.replace(theirs, ours);
        }
        let mut sent: Value = serde_json::from_str(&sent).unwrap();
        for server in sent["params"]["mcpServers"].as_array_mut().into_iter().flatten() {
            server["command"] = json!(exe);
            server["args"] = json!(args);
            server["env"] = json!([{"name": "FAKE_ACP_MCP", "value": "1"}]);
        }
        let received = client.call(&sent, &mut answers);
        if request["method"] == "session/new" {
            let theirs = expected.last().unwrap()["result"]["sessionId"].as_str().unwrap();
            let id = received.last().unwrap()["result"]["sessionId"].as_str().unwrap();
            sessions.insert(theirs.to_string(), id.to_string());
            // MCP servers connect in the background; wait until both servers' prompts are in.
            for _ in 0..200 {
                if last_commands(&our_commands)
                    .is_some_and(|c| c.to_string().contains("acpmcp:greet") && c.to_string().contains("testsrv:greet"))
                {
                    break;
                }
                if let Ok(message) = client.lines.recv_timeout(Duration::from_millis(100)) {
                    our_commands.push(message);
                }
            }
        }
        our_commands.extend(received.iter().cloned());
        their_commands = last_commands(&expected).or(their_commands);
        assert_eq!(
            ours.sequence(&received),
            theirs.sequence(&expected),
            "{name}: {request}"
        );
    }
    assert_eq!(
        last_commands(&our_commands),
        their_commands,
        "{name}: final commands"
    );
    drop(client.stdin.take());
    let status = client.child.wait().unwrap();
    assert_eq!(status.code(), Some(0));
}

#[test]
fn session_transcript() {
    replay("acp-transcript-session.log");
}

#[test]
fn permission_transcript() {
    replay("acp-transcript-permission.log");
}
