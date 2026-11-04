//! Stored message shapes. Each row's `data` is JSON-compatible with 0.5.1's `session_message.data`
//! for the same `type`, so imported history replays unchanged.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::gateway::error::SessionError;
use crate::tool::ToolContent;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Created {
    pub created: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileAttachment {
    /// Base64 bytes.
    pub data: String,
    pub mime: String,
    pub source: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserData {
    pub time: Created,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FileAttachment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyntheticData {
    pub time: Created,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AssistantTime {
    pub created: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streamed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    pub read: u64,
    pub write: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cache: Cache,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredModel {
    pub id: String,
    #[serde(rename = "providerID")]
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantData {
    pub time: AssistantTime,
    pub agent: String,
    pub model: StoredModel,
    pub content: Vec<Part>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish: Option<String>,
    #[serde(default, rename = "rawFinish", skip_serializing_if = "Option::is_none")]
    pub raw_finish: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Tokens>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<SessionError>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PartTime {
    pub created: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ran: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum ToolState {
    Streaming {
        input: String,
    },
    Running {
        input: Value,
        #[serde(default)]
        metadata: Map<String, Value>,
    },
    Completed {
        input: Value,
        content: Vec<ToolContent>,
        #[serde(default, skip_serializing_if = "Map::is_empty")]
        metadata: Map<String, Value>,
    },
    Error {
        input: Value,
        error: SessionError,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        content: Vec<ToolContent>,
        #[serde(default, skip_serializing_if = "Map::is_empty")]
        metadata: Map<String, Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Part {
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        native: Option<Value>,
    },
    Reasoning {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        native: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        time: Option<PartTime>,
    },
    Tool {
        id: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        executed: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        native: Option<Value>,
        state: ToolState,
        time: PartTime,
    },
}

/// A decoded transcript entry.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    User(UserData),
    Synthetic(SyntheticData),
    Assistant(AssistantData),
    /// Kinds M1a stores but does not send (`agent-switched`, `model-switched`, `idle`, …).
    Other,
}

impl Message {
    pub fn decode(kind: &str, data: &Value) -> Message {
        let decoded = match kind {
            "user" => serde_json::from_value(data.clone()).map(Message::User),
            "synthetic" => serde_json::from_value(data.clone()).map(Message::Synthetic),
            "assistant" => serde_json::from_value(data.clone()).map(Message::Assistant),
            _ => return Message::Other,
        };
        decoded.unwrap_or(Message::Other)
    }
}

pub fn encode<T: Serialize>(data: &T) -> Value {
    serde_json::to_value(data).unwrap_or(Value::Null)
}
