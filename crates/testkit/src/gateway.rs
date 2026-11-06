//! A fake `api.anysola.com/v1/code` on 127.0.0.1: `/config`, the account routes (`/me`,
//! `/limits`, `/limits/redeem`, `/usage`, `/tokens`, revokes, the device flow), `/version`,
//! `/search` and scripted `/messages` streams. Every request is recorded.

use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Instant;

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
    /// The query string without `?`, empty when there is none.
    pub query: String,
    /// When the request arrived.
    pub at: Instant,
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
    /// `text/event-stream`.
    Sse { body: String },
    Json {
        status: u16,
        body: Value,
        headers: Vec<(String, String)>,
    },
}

pub type Script = Arc<dyn Fn(&Recorded, usize) -> Reply + Send + Sync>;

/// Answers every request to one path.
pub type Route = Arc<dyn Fn(&Recorded) -> Reply + Send + Sync>;

/// Answers the n-th `POST /device/token`.
pub type DevicePoll = Arc<dyn Fn(usize) -> Reply + Send + Sync>;

struct State {
    port: u16,
    script: Script,
    search: Mutex<Reply>,
    version: Mutex<Value>,
    config: Mutex<Option<Reply>>,
    /// Banked resets `/limits` reports; `/limits/redeem` spends one.
    bank: Mutex<usize>,
    spent: Mutex<usize>,
    /// `/limits` reports a full 5-hour window until a redeem.
    full: Mutex<bool>,
    /// Fixed answers by path, ahead of the defaults.
    replies: Mutex<Vec<(String, Reply)>>,
    /// Answers computed per request by path (other servers on this port, such as MCP).
    routes: Mutex<Vec<(String, Route)>>,
    device: Mutex<DevicePoll>,
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
            "settings": {"baseURL": format!("{base}/v1/code"), "apiKey": "{env:ANYSOLA_TOKEN}"}, "headers": {"x-anysola-client": "anysola-code"},
            "models": {
                "auto": {"name": "Auto", "description": "Picks Haiku or Sonnet per turn", "limit": {"context": 1000000, "output": 32000},
                    "capabilities": {"reasoning": {"supported": true, "effort": efforts}},
                    "compatibility": {"supportsThinkingBlockBinding": true, "supportsEffortUpdates": true}, "variants": [economy]},
                "claude-sonnet-5-5": {"name": "Claude Sonnet 5.5", "description": "Default", "limit": {"context": 1000000, "output": 32000},
                    "capabilities": {"reasoning": {"supported": true, "effort": efforts}}, "variants": [economy]},
                "claude-haiku-5-5": {"name": "Claude Haiku 5.5", "description": "Fastest", "limit": {"context": 200000, "output": 32000}}
            }}}
    })
}

/// The token every default device login approves.
pub const DEVICE_TOKEN: &str = "asc_FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";

/// Pending on the first poll, approved on the second.
fn default_poll(index: usize) -> Reply {
    if index == 0 {
        Reply::Json {
            status: 400,
            body: json!({"error": "authorization_pending"}),
            headers: Vec::new(),
        }
    } else {
        Reply::Json {
            status: 200,
            body: json!({"access_token": DEVICE_TOKEN, "token_type": "bearer", "expires_at": "2027-11-04T00:00:00Z"}),
            headers: Vec::new(),
        }
    }
}

/// The fake's windows reset 2 h (5-hour) and 3 d 4 h (weekly) after each reply, so texts that
/// count down from the real clock read the same in every run.
pub fn five_hour_reset() -> String {
    from_now(2 * 3_600_000)
}

pub fn weekly_reset() -> String {
    from_now(76 * 3_600_000)
}

fn from_now(ms: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::milliseconds(ms))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// `UsageLimitsWire` with `bank` banked resets; `full` fills the 5-hour window.
pub fn limits(bank: usize, full: bool) -> Value {
    limits_after(0, bank, full)
}

/// [`limits`] once `spent` banked resets were spent (the ones that expired first).
fn limits_after(spent: usize, bank: usize, full: bool) -> Value {
    let bank: Vec<Value> = (spent..spent + bank)
        .map(|i| json!({"id": format!("bank_{}", i + 1), "expiresAt": format!("2026-12-{:02}T00:00:00Z", i + 1), "scope": "five_hour"}))
        .collect();
    json!({
        "plan": "team", "allowed": !full, "blockedBy": if full { json!("five_hour") } else { Value::Null },
        "fiveHour": {"percent": if full { 100 } else { 42 }, "resetsAt": five_hour_reset()},
        "weekly": {"percent": 18, "resetsAt": weekly_reset()},
        "bank": bank
    })
}

/// The `/me` body: a paid Team account, the 5-hour window at 42%.
pub fn default_me() -> Value {
    json!({"plan": "team", "access": "paid", "limits": limits(2, false), "usageToday": {"requests": 3}, "event": null})
}

/// The `/usage` body.
pub fn default_usage() -> Value {
    json!({
        "days": 7,
        "totals": {"requests": 42, "cacheHitRate": 0.61},
        "byDay": [{"date": "2026-11-03", "requests": 30}, {"date": "2026-11-04", "requests": 12}],
        "byModel": [{"model": "claude-sonnet-5-5", "requests": 40}, {"model": "claude-haiku-5-5", "requests": 2}],
        "today": {"requests": 12}, "plan": "team",
        "limits": limits(2, false)
    })
}

/// The 429 a full 5-hour window answers a model request with.
pub fn limit_reply(bank: usize) -> Reply {
    let plural = if bank == 1 { "" } else { "s" };
    let banked = if bank > 0 {
        format!(" You have {bank} banked reset{plural}.")
    } else {
        String::new()
    };
    Reply::Json {
        status: 429,
        body: json!({"type": "error", "error": {"type": "rate_limit_error", "message": format!("5-hour limit reached. Resets in 2 h.{banked}")}}),
        headers: vec![
            ("x-should-retry".into(), "false".into()),
            ("x-anysola-limit".into(), "five_hour".into()),
        ],
    }
}

/// The `/tokens` body of the 0.5.1 capture.
pub fn default_tokens() -> Value {
    json!({"tokens": [
        {"id": "tok_1", "name": "laptop", "client": "anysola-code/0.5.1", "kind": "device", "createdAt": "2026-11-01T10:00:00Z",
         "lastUsedAt": "2026-11-04T10:00:00Z", "expiresAt": "2027-11-04T10:00:00Z", "current": true},
        {"id": "tok_2", "name": "", "client": "ci", "kind": "ci", "createdAt": "2026-11-01T10:00:00Z",
         "lastUsedAt": null, "expiresAt": null, "current": false}
    ]})
}

fn respond(reply: Reply) -> Response<Full<Bytes>> {
    match reply {
        Reply::Sse { body } => Response::builder()
            .status(200)
            .header("content-type", "text/event-stream")
            .body(Full::new(Bytes::from(body)))
            .expect("valid response"),
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
    let at = Instant::now();
    let method = request.method().to_string();
    let path = request.uri().path().to_string();
    let query = request.uri().query().unwrap_or_default().to_string();
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
        query,
        at,
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
    let fixed = state
        .replies
        .lock()
        .unwrap()
        .iter()
        .find(|(p, _)| *p == path)
        .map(|(_, reply)| reply.clone());
    let route = state
        .routes
        .lock()
        .unwrap()
        .iter()
        .find(|(p, _)| *p == path)
        .map(|(_, route)| route.clone());
    let reply = match path.as_str() {
        _ if let Some(route) = route => route(&recorded),
        _ if let Some(reply) = fixed => reply,
        "/v1/code/config" => state
            .config
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| ok(default_config(&base))),
        "/v1/code/me" => ok(default_me()),
        "/v1/code/limits" => ok(limits_after(
            *state.spent.lock().unwrap(),
            *state.bank.lock().unwrap(),
            *state.full.lock().unwrap(),
        )),
        "/v1/code/limits/redeem" if recorded.method == "POST" => {
            let mut bank = state.bank.lock().unwrap();
            if *bank == 0 {
                Reply::Json {
                    status: 409,
                    body: json!({"type": "error", "error": {"type": "invalid_request_error", "message": "No banked resets to spend."}}),
                    headers: Vec::new(),
                }
            } else {
                *bank -= 1;
                *state.full.lock().unwrap() = false;
                let mut spent = state.spent.lock().unwrap();
                *spent += 1;
                let mut limits = limits_after(*spent, *bank, false);
                limits["fiveHour"] = json!({"percent": 0, "resetsAt": null});
                ok(limits)
            }
        }
        "/v1/code/usage" => ok(default_usage()),
        "/v1/code/tokens" => ok(default_tokens()),
        "/v1/code/device/code" => ok(
            json!({"device_code": "D".repeat(43), "user_code": "WXYZ-1234", "verification_uri": "https://account.anysola.com/device", "interval": 1, "expires_in": 600}),
        ),
        "/v1/code/device/token" => {
            let poll = state.device.lock().unwrap().clone();
            poll(index)
        }
        revoke if revoke.starts_with("/v1/code/tokens/") && revoke.ends_with("/revoke") => {
            ok(json!({"revoked": true}))
        }
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
                body: json!({"results": [{"url": "https://bun.com", "title": "Bun", "content": "Bun 1.4.4"}]}),
                headers: Vec::new(),
            }),
            version: Mutex::new(json!({"latest": "0.5.1", "minimum": "0.2.0"})),
            config: Mutex::new(None),
            bank: Mutex::new(2),
            spent: Mutex::new(0),
            full: Mutex::new(false),
            replies: Mutex::new(Vec::new()),
            routes: Mutex::new(Vec::new()),
            device: Mutex::new(Arc::new(default_poll)),
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

    /// How many banked resets `/limits` reports, and whether the 5-hour window is full.
    pub fn set_limits(&self, bank: usize, full: bool) {
        *self.state.bank.lock().unwrap() = bank;
        *self.state.full.lock().unwrap() = full;
    }

    /// Banked resets left.
    pub fn bank(&self) -> usize {
        *self.state.bank.lock().unwrap()
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

    /// Answers every later request to `path` (no query) with `reply`.
    pub fn set_reply(&self, path: &str, reply: Reply) {
        let mut replies = self.state.replies.lock().unwrap();
        replies.retain(|(p, _)| p != path);
        replies.push((path.to_string(), reply));
    }

    /// Answers every later request to `path` (no query) with what `route` computes.
    pub fn route(&self, path: &str, route: impl Fn(&Recorded) -> Reply + Send + Sync + 'static) {
        let mut routes = self.state.routes.lock().unwrap();
        routes.retain(|(p, _)| p != path);
        routes.push((path.to_string(), Arc::new(route)));
    }

    pub fn set_device_poll(&self, poll: impl Fn(usize) -> Reply + Send + Sync + 'static) {
        *self.state.device.lock().unwrap() = Arc::new(poll);
    }

    /// Recorded requests to `path`.
    pub fn requests_to(&self, path: &str) -> Vec<Recorded> {
        self.requests()
            .into_iter()
            .filter(|r| r.path == path)
            .collect()
    }
}
