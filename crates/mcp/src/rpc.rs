//! JSON-RPC 2.0 over an MCP transport: requests with ids and timeouts, the server's requests to
//! the client (`roots/list`, `elicitation/create`, `ping`), and notifications.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

/// Why a request failed. The texts are 0.5.1's (the TypeScript SDK's) messages.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// An HTTP status the transport could not use.
    Http {
        status: u16,
        body: String,
    },
    /// The server could not be reached (fetch's TypeError with a system error code).
    Network(String),
    /// The server wants a sign-in.
    Unauthorized(String),
    /// The transport closed; the last transport error says why.
    Closed(Option<String>),
    Rpc {
        code: i64,
        message: String,
        data: Option<Value>,
    },
    Timeout,
    /// A legacy Streamable HTTP server no longer knows this connection's session.
    SessionExpired,
    Other(String),
}

impl Error {
    /// Overloaded or restarting servers and dropped connections usually recover within a second;
    /// 501 means the server will never support the request.
    pub fn transient(&self) -> bool {
        match self {
            Error::Http { status, .. } => {
                *status == 408 || *status == 429 || (*status >= 500 && *status != 501)
            }
            Error::Network(_) => true,
            _ => false,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Http { status, body } => {
                write!(f, "Error POSTing to endpoint: {body} (HTTP {status})")
            }
            Error::Network(message) | Error::Unauthorized(message) | Error::Other(message) => {
                f.write_str(message)
            }
            Error::Closed(None) => f.write_str("Connection closed"),
            Error::Closed(Some(cause)) => write!(f, "Connection closed: {cause}"),
            Error::Rpc { code, message, .. } => write!(f, "MCP error {code}: {message}"),
            Error::Timeout => f.write_str("MCP error -32001: Request timed out"),
            Error::SessionExpired => f.write_str("MCP server session expired"),
        }
    }
}

/// What a transport hands the peer.
#[derive(Debug)]
pub enum Incoming {
    Message(Value),
    /// A transport problem worth remembering as the reason of a later close.
    Error(String),
    Closed,
}

pub trait Transport: Send + Sync + 'static {
    /// Sends one message; HTTP answers arrive as [`Incoming`] messages.
    fn send(&self, message: Value) -> BoxFuture<'_, Result<(), Error>>;

    fn close(&self) -> BoxFuture<'_, ()>;
}

/// The client side's answers to server requests and its reaction to notifications.
pub trait Handler: Send + Sync + 'static {
    fn request(&self, method: &str, params: Value) -> BoxFuture<'static, Result<Value, Error>>;

    fn notification(&self, method: &str, params: Value);

    /// The connection closed: `Connection closed[: cause]`.
    fn closed(&self, reason: String);
}

type Pending = Mutex<HashMap<u64, oneshot::Sender<Result<Value, Error>>>>;

pub struct Peer {
    transport: Arc<dyn Transport>,
    pending: Arc<Pending>,
    next: AtomicU64,
    /// The last transport error, which explains a close.
    last_error: Arc<Mutex<Option<String>>>,
    /// The 2026-07-28 `_meta` envelope every outgoing message carries once that era is agreed.
    envelope: Mutex<Option<Value>>,
    /// Set with the envelope: that era has no server-to-client requests, so they are dropped.
    modern: Arc<AtomicBool>,
}

impl Peer {
    pub fn start(
        transport: Arc<dyn Transport>,
        mut incoming: mpsc::UnboundedReceiver<Incoming>,
        handler: Arc<dyn Handler>,
    ) -> Arc<Peer> {
        let peer = Arc::new(Peer {
            transport: transport.clone(),
            pending: Arc::default(),
            next: AtomicU64::new(0),
            last_error: Arc::default(),
            envelope: Mutex::default(),
            modern: Arc::default(),
        });
        let (pending, last_error, modern) = (
            peer.pending.clone(),
            peer.last_error.clone(),
            peer.modern.clone(),
        );
        tokio::spawn(async move {
            while let Some(item) = incoming.recv().await {
                match item {
                    Incoming::Message(message) => {
                        if modern.load(Ordering::SeqCst) && is_request(&message) {
                            continue;
                        }
                        dispatch(&transport, &pending, &handler, message);
                    }
                    Incoming::Error(error) => {
                        *last_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(error);
                    }
                    Incoming::Closed => break,
                }
            }
            let cause = last_error.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let waiting: Vec<_> = pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .drain()
                .collect();
            for (_, sender) in waiting {
                let _ = sender.send(Err(Error::Closed(cause.clone())));
            }
            handler.closed(Error::Closed(cause).to_string());
        });
        peer
    }

    /// The id the next request gets (also its progress token).
    pub fn next_id(&self) -> u64 {
        self.next.load(Ordering::SeqCst)
    }

    /// Switches to the 2026-07-28 era: `envelope` joins every later message's `params._meta`.
    pub fn set_envelope(&self, envelope: Value) {
        *self.envelope.lock().unwrap_or_else(|e| e.into_inner()) = Some(envelope);
        self.modern.store(true, Ordering::SeqCst);
    }

    fn enveloped(&self, params: Value) -> Value {
        let Some(envelope) = self
            .envelope
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        else {
            return params;
        };
        with_meta(params, envelope)
    }

    pub async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, Error> {
        let params = self.enveloped(params);
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, sender);
        let message = json!({"method": method, "params": params, "jsonrpc": "2.0", "id": id});
        let exchange = async {
            self.transport.send(message).await?;
            receiver.await.unwrap_or(Err(Error::Closed(None)))
        };
        let result = tokio::time::timeout(timeout, exchange).await;
        let forget = || {
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id)
        };
        match result {
            Ok(Err(error)) => {
                forget();
                Err(self.explain(error))
            }
            Ok(Ok(value)) => Ok(value),
            Err(_) => {
                forget();
                let _ = self
                    .notify(
                        "notifications/cancelled",
                        json!({"requestId": id, "reason": "MCP error -32001: Request timed out"}),
                    )
                    .await;
                Err(Error::Timeout)
            }
        }
    }

    /// A bare close carries the transport error before it.
    fn explain(&self, error: Error) -> Error {
        match error {
            Error::Closed(None) => Error::Closed(
                self.last_error
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone(),
            ),
            other => other,
        }
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<(), Error> {
        let params = self.enveloped(params);
        let mut message = json!({"method": method, "jsonrpc": "2.0"});
        if !params.is_null() {
            message["params"] = params;
        }
        self.transport.send(message).await
    }

    pub async fn close(&self) {
        self.transport.close().await;
    }
}

/// Adds `envelope`'s keys to `params._meta`; keys the caller set win.
pub fn with_meta(params: Value, envelope: Value) -> Value {
    let mut params = match params {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    let mut meta = envelope.as_object().cloned().unwrap_or_default();
    if let Some(Value::Object(own)) = params.remove("_meta") {
        meta.extend(own);
    }
    params.insert("_meta".into(), Value::Object(meta));
    Value::Object(params)
}

fn is_request(message: &Value) -> bool {
    message.get("method").is_some() && message.get("id").is_some_and(|id| !id.is_null())
}

fn dispatch(
    transport: &Arc<dyn Transport>,
    pending: &Pending,
    handler: &Arc<dyn Handler>,
    message: Value,
) {
    let method = message.get("method").and_then(Value::as_str);
    let id = message.get("id").filter(|id| !id.is_null()).cloned();
    match (method, id) {
        (None, Some(id)) => {
            let Some(sender) = id.as_u64().and_then(|id| {
                pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id)
            }) else {
                return;
            };
            let result = match message.get("error") {
                Some(error) => Err(Error::Rpc {
                    code: error["code"].as_i64().unwrap_or(-32603),
                    message: error["message"].as_str().unwrap_or_default().to_string(),
                    data: error.get("data").cloned(),
                }),
                None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
            };
            let _ = sender.send(result);
        }
        (Some(method), Some(id)) => {
            let answer = handler.request(method, message["params"].clone());
            let transport = transport.clone();
            tokio::spawn(async move {
                let reply = match answer.await {
                    Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                    Err(Error::Rpc { code, message, .. }) => {
                        json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
                    }
                    Err(error) => {
                        json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": error.to_string()}})
                    }
                };
                let _ = transport.send(reply).await;
            });
        }
        (Some(method), None) => handler.notification(method, message["params"].clone()),
        (None, None) => {}
    }
}
