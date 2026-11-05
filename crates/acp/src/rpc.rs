//! JSON-RPC 2.0 over newline-delimited JSON: what the agent writes and the client's answers to
//! the agent's own requests.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

/// A JSON-RPC error with the TypeScript SDK's texts.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl Failure {
    pub fn invalid_params(message: impl std::fmt::Display, data: Value) -> Failure {
        Failure {
            code: -32602,
            message: format!("Invalid params: {message}"),
            data: Some(data),
        }
    }

    /// An invalid request field, which the SDK reports as invalid params naming the field.
    pub fn invalid_field(message: impl std::fmt::Display, field: &str) -> Failure {
        Failure::invalid_params(message, json!({"field": field}))
    }

    pub fn internal(message: impl std::fmt::Display, data: Value) -> Failure {
        Failure {
            code: -32603,
            message: format!("Internal error: {message}"),
            data: Some(data),
        }
    }

    pub fn auth_required() -> Failure {
        Failure {
            code: -32000,
            message: "Authentication required: provider authentication required".into(),
            data: Some(json!({})),
        }
    }

    pub fn method_not_found(method: &str) -> Failure {
        Failure {
            code: -32601,
            message: format!("\"Method not found\": {method}"),
            data: Some(json!({"method": method})),
        }
    }

    pub fn parse_error() -> Failure {
        Failure {
            code: -32700,
            message: "Parse error".into(),
            data: None,
        }
    }

    pub fn invalid_request(data: Value) -> Failure {
        Failure {
            code: -32600,
            message: "Invalid request".into(),
            data: Some(data),
        }
    }

    pub fn to_value(&self) -> Value {
        let mut error = json!({"code": self.code, "message": self.message});
        if let Some(data) = &self.data {
            error["data"] = data.clone();
        }
        error
    }
}

/// The agent's side of the connection. Lines go to the writer task in order.
pub struct Connection {
    out: mpsc::UnboundedSender<String>,
    next: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<Value, Value>>>>,
}

impl Connection {
    pub fn new(out: mpsc::UnboundedSender<String>) -> Connection {
        Connection {
            out,
            next: AtomicU64::new(0),
            pending: Mutex::default(),
        }
    }

    fn send(&self, message: Value) {
        let _ = self.out.send(message.to_string());
    }

    pub fn respond(&self, id: Value, result: Result<Value, Failure>) {
        match result {
            Ok(result) => self.send(json!({"jsonrpc": "2.0", "id": id, "result": result})),
            Err(failure) => {
                self.send(json!({"jsonrpc": "2.0", "id": id, "error": failure.to_value()}))
            }
        }
    }

    pub fn notify(&self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    pub fn update(&self, session_id: &str, update: Value) {
        self.notify(
            "session/update",
            json!({"sessionId": session_id, "update": update}),
        );
    }

    /// Sends a request to the client; `Err` carries its error object, or null once the
    /// connection closed.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, Value> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.lock().insert(id, sender);
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        receiver.await.unwrap_or(Err(Value::Null))
    }

    /// Settles a request the agent sent; unknown ids are ignored.
    pub fn resolve(&self, id: &Value, outcome: Result<Value, Value>) {
        if let Some(sender) = id.as_u64().and_then(|id| self.lock().remove(&id)) {
            let _ = sender.send(outcome);
        }
    }

    /// Fails every request still waiting for the client.
    pub fn close(&self) {
        self.lock().clear();
    }

    fn lock(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<u64, oneshot::Sender<Result<Value, Value>>>> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }
}
