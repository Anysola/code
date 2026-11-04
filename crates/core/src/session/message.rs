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
// Parts mirror the stored JSON and live briefly; boxing the large variant buys nothing.
#[allow(clippy::large_enum_variant)]
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

/// `model-switched`: the session's model or variant changed before the next prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelSwitchedData {
    pub time: Created,
    pub model: StoredModel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<StoredModel>,
}

/// `agent-switched`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSwitchedData {
    pub time: Created,
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub directory: String,
}

/// `location-switched`: the session moved to another directory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocationSwitchedData {
    pub time: Created,
    pub location: Location,
}

/// `skill`: a skill the user activated, with its rendered text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillData {
    pub time: Created,
    pub skill: String,
    pub name: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShellOutput {
    #[serde(default)]
    pub output: String,
}

/// `shell`: a command the user ran (`!cmd` in the TUI).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShellData {
    pub time: Created,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<ShellOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

impl ShellData {
    pub fn background(&self) -> bool {
        self.metadata
            .as_ref()
            .and_then(|m| m.get("background"))
            .and_then(Value::as_bool)
            == Some(true)
    }
}

/// `compaction`: a summary that replaces the history before it once `completed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompactionData {
    pub time: Created,
    pub status: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<StoredModel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<SessionError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Tokens>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

impl CompactionData {
    pub fn completed(&self) -> bool {
        self.status == "completed"
    }

    /// The first message the summary kept verbatim (`metadata.keepID`).
    pub fn keep_id(&self) -> Option<&str> {
        self.metadata.as_ref()?.get("keepID")?.as_str()
    }
}

/// A decoded transcript entry.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Message {
    User(UserData),
    Synthetic(SyntheticData),
    /// An instruction update (`system`).
    System(SyntheticData),
    Assistant(AssistantData),
    ModelSwitched(ModelSwitchedData),
    AgentSwitched(AgentSwitchedData),
    LocationSwitched(LocationSwitchedData),
    Skill(SkillData),
    Shell(ShellData),
    Compaction(CompactionData),
    /// Kinds that never reach the model (`idle`) and rows that do not decode.
    Other,
}

impl Message {
    pub fn decode(kind: &str, data: &Value) -> Message {
        fn read<T: serde::de::DeserializeOwned>(data: &Value, wrap: fn(T) -> Message) -> Message {
            serde_json::from_value(data.clone()).map_or(Message::Other, wrap)
        }
        match kind {
            "user" => read(data, Message::User),
            "synthetic" => read(data, Message::Synthetic),
            "system" => read(data, Message::System),
            "assistant" => read(data, Message::Assistant),
            "model-switched" => read(data, Message::ModelSwitched),
            "agent-switched" => read(data, Message::AgentSwitched),
            "location-switched" => read(data, Message::LocationSwitched),
            "skill" => read(data, Message::Skill),
            "shell" => read(data, Message::Shell),
            "compaction" => read(data, Message::Compaction),
            _ => Message::Other,
        }
    }
}

pub fn encode<T: Serialize>(data: &T) -> Value {
    serde_json::to_value(data).unwrap_or(Value::Null)
}
