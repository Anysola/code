//! A fake `api.anysola.com/v1/code` on 127.0.0.1: `/config`, `/me`, `/version`, `/search` and
//! scripted `/messages` streams. Every request is recorded.

use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    /// Lower-case names, in arrival order.
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }

    /// A title request: no tools.
    /// Title requests carry no tools and ask for only the title.
    pub fn is_title(&self) -> bool {
        self.json().get("tools").is_none() && self.body.contains("Reply with only the title.")
    }
}

#[derive(Debug, Clone)]
pub enum Reply {
    /// `text/event-stream` with `x-credits-available-before` when given.
    Sse { body: String, before: Option<u64> },
    Json {
        status: u16,
        body: Value,
        headers: Vec<(String, String)>,
    },
}

pub type Script = Arc<dyn Fn(&Recorded, usize) -> Reply + Send + Sync>;

struct State {
    port: u16,
    script: Script,
    search: Mutex<Reply>,
    version: Mutex<Value>,
    config: Mutex<Option<Reply>>,
    requests: Mutex<Vec<Recorded>>,
    /// Plain-text files served at other paths (instruction URLs).
    files: Mutex<Vec<(String, String)>>,
}

pub struct FakeGateway {
    pub url: String,
    state: Arc<State>,
    _runtime: tokio::runtime::Runtime,
}

/// The `/config` body the real gateway sends with Auto and economy on.
pub fn default_config(base: &str) -> Value {
    let efforts = json!(["low", "medium", "high", "xhigh", "max"]);
    let economy = json!({"id": "economy", "settings": {"thinking": {"type": "adaptive", "display": "summarized"}, "effort": "low"}, "headers": {"x-anysola-variant": "economy"}});
    json!({
        "model": "anysola/claude-sonnet-5-5",
        "small_model": "anysola/claude-haiku-5-5",
        "agents": {"title": {"model": "anysola/claude-haiku-5-5"}},
        "providers": {"anysola": {"name": "Anysola", "package": "@opencode/ai/providers/anthropic-compatible",
            "settings": {"baseURL": format!("{base}/v1/code"), "apiKey": "{env:ANYSOLA_TOKEN}"}, "headers": {"x-anysola-client": "opencode"},
            "models": {
                "auto": {"name": "Auto", "description": "Picks Haiku or Sonnet per turn", "limit": {"context": 1000000, "output": 32000},
                    "capabilities": {"reasoning": {"supported": true, "effort": efforts}},
                    "compatibility": {"supportsThinkingBlockBinding": true, "supportsEffortUpdates": true}, "variants": [economy]},
                "claude-sonnet-5-5": {"name": "Claude Sonnet 5.5", "description": "Default", "limit": {"context": 1000000, "output": 32000},
                    "capabilities": {"reasoning": {"supported": true, "effort": efforts}}, "variants": [economy], "credits_per_turn_estimate": 6},
                "claude-haiku-5-5": {"name": "Claude Haiku 5.5", "description": "Fastest", "limit": {"context": 200000, "output": 32000}, "credits_per_turn_estimate": 1}
            }}}
    })
}

fn respond(reply: Reply) -> Response<Full<Bytes>> {
    match reply {
        Reply::Sse { body, before } => {
            let mut response = Response::builder()
                .status(200)
                .header("content-type", "text/event-stream");
            if let Some(before) = before {
                response = response.header("x-credits-available-before", before.to_string());
            }
            response
                .body(Full::new(Bytes::from(body)))
                .expect("valid response")
        }
        Reply::Json {
            status,
            body,
            headers,
        } => {
            let mut response = Response::builder()
                .status(status)
                .header("content-type", "application/json");
            for (name, value) in headers {
                response = response.header(name, value);
            }
            response
                .body(Full::new(Bytes::from(body.to_string())))
                .expect("valid response")
        }
    }
}

async fn handle(
    state: Arc<State>,
    request: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = request.method().to_string();
    let path = request.uri().path().to_string();
    let headers = request
        .headers()
        .iter()
        .map(|(n, v)| {
            (
                n.as_str().to_string(),
                v.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let body = request
        .into_body()
        .collect()
        .await
        .map(|b| b.to_bytes())
        .unwrap_or_default();
    let recorded = Recorded {
        method,
        path: path.clone(),
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    };
    let index = {
        let mut requests = state.requests.lock().unwrap();
        requests.push(recorded.clone());
        requests.iter().filter(|r| r.path == path).count() - 1
    };
    let base = format!("http://127.0.0.1:{}", state.port);
    let ok = |body: Value| Reply::Json {
        status: 200,
        body,
        headers: Vec::new(),
    };
    let reply = match path.as_str() {
        "/v1/code/config" => state
            .config
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| ok(default_config(&base))),
        "/v1/code/me" => ok(
            json!({"plan": "team", "access": "paid", "credits": {"available": 731, "renewsAt": "2026-11-23T00:00:00Z"}, "lowBalance": false, "usageToday": {"requests": 3, "credits": 9}}),
        ),
        "/v1/code/version" => ok(state.version.lock().unwrap().clone()),
        "/v1/code/search" => state.search.lock().unwrap().clone(),
        "/v1/code/messages" => (state.script)(&recorded, index),
        other
            if let Some((_, text)) =
                state.files.lock().unwrap().iter().find(|(p, _)| p == other) =>
        {
            return Ok(Response::builder()
                .status(200)
                .header("content-type", "text/plain")
                .body(Full::new(Bytes::from(text.clone())))
                .expect("valid response"));
        }
        _ => Reply::Json {
            status: 404,
            body: json!({"type": "error", "error": {"type": "not_found_error", "message": "fake"}}),
            headers: Vec::new(),
        },
    };
    Ok(respond(reply))
}

impl FakeGateway {
    /// Starts on an ephemeral port; `script` answers the n-th `/messages` request.
    pub fn start(
        script: impl Fn(&Recorded, usize) -> Reply + Send + Sync + 'static,
    ) -> FakeGateway {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("runtime");
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .expect("bind");
        let port = listener.local_addr().expect("address").port();
        let state = Arc::new(State {
            port,
            script: Arc::new(script),
            search: Mutex::new(Reply::Json {
                status: 200,
                body: json!({"results": [{"url": "https://bun.com", "title": "Bun", "content": "Bun 1.4.4"}], "credits": 2}),
                headers: Vec::new(),
            }),
            version: Mutex::new(json!({"latest": "0.5.1", "minimum": "0.2.0"})),
            config: Mutex::new(None),
            requests: Mutex::new(Vec::new()),
            files: Mutex::new(Vec::new()),
        });
        let accept = state.clone();
        runtime.spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let state = accept.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |request| handle(state.clone(), request));
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        FakeGateway {
            url: format!("http://127.0.0.1:{port}"),
            state,
            _runtime: runtime,
        }
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.state.requests.lock().unwrap().clone()
    }

    /// Recorded `/messages` requests.
    pub fn messages(&self) -> Vec<Recorded> {
        self.requests()
            .into_iter()
            .filter(|r| r.path == "/v1/code/messages")
            .collect()
    }

    /// Serves `text` at `path` (for example an instruction URL).
    pub fn serve(&self, path: &str, text: &str) {
        self.state
            .files
            .lock()
            .unwrap()
            .push((path.to_string(), text.to_string()));
    }

    pub fn set_search(&self, reply: Reply) {
        *self.state.search.lock().unwrap() = reply;
    }

    pub fn set_version(&self, body: Value) {
        *self.state.version.lock().unwrap() = body;
    }

    pub fn set_config(&self, body: Value) {
        self.set_config_reply(Reply::Json {
            status: 200,
            body,
            headers: Vec::new(),
        });
    }

    pub fn set_config_reply(&self, reply: Reply) {
        *self.state.config.lock().unwrap() = Some(reply);
    }
}
