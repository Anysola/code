//! A temp home whose environment the process takes on while a test runs (tests in one binary
//! are serialized), the fake gateway's replies, and a fake stdio MCP server: the test binary
//! itself, re-run with `FAKE_MCP` set.
#![allow(dead_code)]

use std::io::{BufRead, Write};
use std::sync::{Mutex, MutexGuard};

use anysola_testkit::sse::Stream;
use anysola_testkit::{Recorded, Reply, TestHome};
use serde_json::{Value, json};

static ENV: Mutex<()> = Mutex::new(());

pub struct Env {
    pub home: TestHome,
    _guard: MutexGuard<'static, ()>,
}

pub fn env(gateway_url: &str) -> Env {
    let guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let home = TestHome::new(gateway_url);
    for (name, value) in home.env() {
        // SAFETY: tests in this binary hold `ENV` while they touch the environment.
        unsafe { std::env::set_var(name, value) };
    }
    Env {
        home,
        _guard: guard,
    }
}

pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

pub fn sse(request: &Recorded, stream: Stream) -> Reply {
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

pub fn text(request: &Recorded, text: &str) -> Reply {
    sse(request, Stream::new().text(text))
}

/// The command that runs the fake server in `test` (the name of a test in this binary that
/// calls [`serve_if_asked`]).
pub fn fake_server(test: &str) -> Value {
    let exe = std::env::current_exe().unwrap().display().to_string();
    json!({
        "type": "local",
        "command": [exe, "--exact", test, "--nocapture", "--test-threads=1", "-q"],
        "environment": {"FAKE_MCP": "1"},
    })
}

fn tool(name: &str, description: &str, input: Value) -> Value {
    json!({"name": name, "description": description, "inputSchema": input})
}

/// When this process is the fake server: answers MCP on stdin/stdout, then exits.
pub fn serve_if_asked() {
    if std::env::var("FAKE_MCP").is_err() {
        return;
    }
    let mut extra = false;
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    let send = |out: &mut std::io::StdoutLock, message: Value| {
        writeln!(out, "{message}").unwrap();
        out.flush().unwrap();
    };
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let params = &message["params"];
        let result = match message["method"].as_str().unwrap_or_default() {
            "initialize" => json!({
                "protocolVersion": params["protocolVersion"],
                "capabilities": {"tools": {"listChanged": true}, "prompts": {}, "resources": {}},
                "serverInfo": {"name": "fake", "version": "1"},
                "instructions": "Always greet politely.\nSecond line.",
            }),
            "tools/list" => {
                let mut tools = vec![
                    tool(
                        "echo",
                        "Echo the text back",
                        json!({"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}),
                    ),
                    tool(
                        "get.weather",
                        "Weather (dotted name)",
                        json!({"type": "object", "properties": {}}),
                    ),
                    tool("broken", "Malformed", json!({"type": "string"})),
                ];
                if extra {
                    tools.push(tool(
                        "add",
                        "Added later",
                        json!({"type": "object", "properties": {}}),
                    ));
                }
                json!({"tools": tools})
            }
            // `echo` grows the tool list on "grow" and ends the process on "quit".
            "tools/call" => match params["arguments"]["text"].as_str().unwrap_or_default() {
                "grow" => {
                    extra = true;
                    send(
                        &mut out,
                        json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}),
                    );
                    json!({"content": [{"type": "text", "text": "grown"}]})
                }
                "quit" => {
                    eprintln!("bye");
                    std::process::exit(3);
                }
                _ => json!({"content": [{"type": "text", "text": format!("echo: {params}")}]}),
            },
            "prompts/list" => json!({"prompts": [
                {"name": "greet", "description": "Greets", "arguments": [{"name": "who"}, {"name": "how"}]}
            ]}),
            "prompts/get" => json!({"messages": [
                {"role": "user", "content": {"type": "text", "text": format!("Hello {} {}", params["arguments"]["who"], params["arguments"]["how"])}}
            ]}),
            "resources/list" => {
                json!({"resources": [{"name": "readme", "uri": "file:///readme", "mimeType": "text/plain"}]})
            }
            "resources/templates/list" => json!({"resourceTemplates": []}),
            "resources/read" => json!({"contents": [{"uri": params["uri"], "text": "read me"}]}),
            _ => {
                send(
                    &mut out,
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Method not found"}}),
                );
                continue;
            }
        };
        send(
            &mut out,
            json!({"jsonrpc": "2.0", "id": id, "result": result}),
        );
    }
    std::process::exit(0);
}
