//! The Streamable HTTP transport: every message is a POST whose answer is JSON or an SSE stream,
//! a GET stream carries the server's own messages, and DELETE ends the session.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use futures_util::{FutureExt, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::rpc::{Error, Incoming, Transport};

const TERMINATE_TIMEOUT: Duration = Duration::from_secs(1);
const SESSION_HEADER: &str = "mcp-session-id";

/// Bearer tokens for a server that signs in with OAuth.
pub trait Auth: Send + Sync {
    /// The access token to send, refreshed when it has expired; `None` without credentials.
    fn token(&self) -> BoxFuture<'_, Option<String>>;

    /// The server refused this token: refresh it (true: retry) or give up with the reason.
    fn rejected<'a>(&'a self, token: Option<&'a str>) -> BoxFuture<'a, Result<(), String>>;
}

pub struct Http {
    client: reqwest::Client,
    url: url::Url,
    headers: Vec<(String, String)>,
    auth: Option<Arc<dyn Auth>>,
    session: Mutex<Option<String>>,
    protocol: Mutex<Option<String>>,
    incoming: mpsc::UnboundedSender<Incoming>,
    cancel: CancellationToken,
}

/// The `reqwest` client every MCP HTTP request uses (and OAuth's), with 0.5.1's User-Agent.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(anysola_core::version::user_agent())
        .connect_timeout(std::time::Duration::from_secs(30))
        .tcp_keepalive(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default()
}

/// fetch's wording for a request that never got an answer.
pub fn network_error(error: &reqwest::Error) -> Error {
    if error.is_connect() {
        Error::Network(
            "ECONNREFUSED: Unable to connect. Is the computer able to access the url?".into(),
        )
    } else {
        Error::Network(error.to_string())
    }
}

impl Http {
    pub fn new(
        url: url::Url,
        headers: Vec<(String, String)>,
        auth: Option<Arc<dyn Auth>>,
        incoming: mpsc::UnboundedSender<Incoming>,
    ) -> Arc<Http> {
        Arc::new(Http {
            client: client(),
            url,
            headers,
            auth,
            session: Mutex::default(),
            protocol: Mutex::default(),
            incoming,
            cancel: CancellationToken::new(),
        })
    }

    fn session(&self) -> Option<String> {
        self.session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The protocol revision the server agreed to, sent on every later request.
    pub fn set_protocol(&self, version: &str) {
        *self.protocol.lock().unwrap_or_else(|e| e.into_inner()) = Some(version.to_string());
    }

    async fn request(
        &self,
        method: reqwest::Method,
        body: Option<&Value>,
    ) -> Result<reqwest::Response, Error> {
        let accept = if method == reqwest::Method::GET {
            "text/event-stream"
        } else {
            "application/json, text/event-stream"
        };
        let mut retried = false;
        loop {
            let token = match &self.auth {
                Some(auth) => auth.token().await,
                None => None,
            };
            let mut request = self
                .client
                .request(method.clone(), self.url.clone())
                .header("accept", accept);
            for (key, value) in &self.headers {
                request = request.header(key, value);
            }
            if let Some(session) = self.session() {
                request = request.header(SESSION_HEADER, session);
            }
            let claimed = body.and_then(claimed_headers);
            let protocol = match &claimed {
                Some((version, _, _)) => Some(version.clone()),
                None => self
                    .protocol
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone(),
            };
            if let Some(protocol) = protocol {
                request = request.header("mcp-protocol-version", protocol);
            }
            if let Some((_, method, name)) = claimed {
                request = request.header("mcp-method", method);
                if let Some(name) = name {
                    request = request.header("mcp-name", name);
                }
            }
            if let Some(token) = &token {
                request = request.bearer_auth(token);
            }
            if let Some(body) = body {
                request = request
                    .header("content-type", "application/json")
                    .body(body.to_string());
            }
            let response = request.send().await.map_err(|e| network_error(&e))?;
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                let Some(auth) = &self.auth else {
                    return Err(Error::Http {
                        status: 401,
                        body: response.text().await.unwrap_or_default(),
                    });
                };
                if retried {
                    return Err(Error::Unauthorized(
                        auth.rejected(None).await.err().unwrap_or_default(),
                    ));
                }
                auth.rejected(token.as_deref())
                    .await
                    .map_err(Error::Unauthorized)?;
                retried = true;
                continue;
            }
            if let Some(session) = response.headers().get(SESSION_HEADER) {
                *self.session.lock().unwrap_or_else(|e| e.into_inner()) =
                    session.to_str().ok().map(str::to_string);
            }
            return Ok(response);
        }
    }

    /// Feeds the messages of a JSON or SSE answer to the peer.
    async fn deliver(&self, response: reqwest::Response) {
        let sse = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        if !sse {
            if let Ok(body) = response.bytes().await {
                match serde_json::from_slice::<Value>(&body) {
                    Ok(Value::Array(messages)) => {
                        for message in messages {
                            let _ = self.incoming.send(Incoming::Message(message));
                        }
                    }
                    Ok(message) if message.is_object() => {
                        let _ = self.incoming.send(Incoming::Message(message));
                    }
                    _ => {}
                }
            }
            return;
        }
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut data = String::new();
        loop {
            let chunk = tokio::select! {
                chunk = stream.next() => chunk,
                _ = self.cancel.cancelled() => return,
            };
            let Some(Ok(chunk)) = chunk else {
                return;
            };
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(end) = buffer.find('\n') {
                let line = buffer[..end].trim_end_matches('\r').to_string();
                buffer.drain(..=end);
                if line.is_empty() {
                    if let Ok(message) = serde_json::from_str::<Value>(&data) {
                        let _ = self.incoming.send(Incoming::Message(message));
                    }
                    data.clear();
                } else if let Some(value) = line.strip_prefix("data:") {
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(value.strip_prefix(' ').unwrap_or(value));
                }
            }
        }
    }

    /// The standalone GET stream for messages the server sends on its own; servers without one
    /// answer 405.
    fn listen(self: &Arc<Self>) {
        let http = self.clone();
        tokio::spawn(async move {
            if let Ok(response) = http.request(reqwest::Method::GET, None).await
                && response.status().is_success()
            {
                http.deliver(response).await;
            }
        });
    }

    async fn post(self: &Arc<Self>, message: Value) -> Result<(), Error> {
        let response = self.request(reqwest::Method::POST, Some(&message)).await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let expired = self.session().is_some()
                && (status == reqwest::StatusCode::NOT_FOUND
                    || (status == reqwest::StatusCode::BAD_REQUEST
                        && body.contains("Bad Request: Server not initialized")));
            if expired {
                // The server forgot the session, so there is nothing to end with DELETE.
                *self.session.lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
            return Err(if expired {
                Error::SessionExpired
            } else {
                Error::Http {
                    status: status.as_u16(),
                    body,
                }
            });
        }
        if message["method"] == "notifications/initialized" {
            self.listen();
        }
        if status == reqwest::StatusCode::ACCEPTED {
            return Ok(());
        }
        let http = self.clone();
        tokio::spawn(async move { http.deliver(response).await });
        Ok(())
    }

    /// DELETE ends the server's session (1 s at most).
    pub async fn terminate(&self) {
        if self.session().is_none() {
            return;
        }
        let _ = tokio::time::timeout(
            TERMINATE_TIMEOUT,
            self.request(reqwest::Method::DELETE, None),
        )
        .await;
    }
}

/// [`Http`] as a peer transport.
pub struct HttpTransport(pub Arc<Http>);

impl Transport for HttpTransport {
    fn send(&self, message: Value) -> BoxFuture<'_, Result<(), Error>> {
        self.0.post(message).boxed()
    }

    fn close(&self) -> BoxFuture<'_, ()> {
        async move {
            self.0.terminate().await;
            self.0.cancel.cancel();
            let _ = self.0.incoming.send(Incoming::Closed);
        }
        .boxed()
    }
}

/// A request whose `_meta` claims a protocol revision (the 2026-07-28 era) also names it, its
/// method and its target in headers: `(version, method, name)`.
fn claimed_headers(body: &Value) -> Option<(String, String, Option<String>)> {
    let method = body.get("method")?.as_str()?;
    body.get("id").filter(|id| !id.is_null())?;
    let version = body["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"].as_str()?;
    let field = match method {
        "tools/call" | "prompts/get" => "name",
        "resources/read" => "uri",
        "tasks/get" | "tasks/update" | "tasks/cancel" => "taskId",
        _ => return Some((version.into(), method.into(), None)),
    };
    let name = body["params"][field].as_str().map(header_value);
    Some((version.into(), method.into(), name))
}

/// Values a header cannot carry verbatim go as `=?base64?…?=`.
fn header_value(value: &str) -> String {
    use base64::Engine;
    let plain = !value.is_empty()
        && value == value.trim()
        && !(value.starts_with("=?base64?") && value.ends_with("?="))
        && value.chars().all(|c| c == '\t' || (' '..='~').contains(&c));
    if plain {
        return value.to_string();
    }
    format!(
        "=?base64?{}?=",
        base64::engine::general_purpose::STANDARD.encode(value)
    )
}
