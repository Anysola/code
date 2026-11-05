//! One connection to an MCP server: the initialize handshake over stdio or Streamable HTTP, the
//! listings and calls, and the connect retries 0.5.1 makes.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};
use tokio::sync::mpsc;

use crate::http::{Auth, Http, HttpTransport};
use crate::rpc::{Error, Handler, Peer, Transport};
use crate::stdio::Stdio;

const DEFAULT_STARTUP: u64 = 30_000;
const DEFAULT_CATALOG: u64 = 30_000;
const DEFAULT_EXECUTION: u64 = 12 * 60 * 60 * 1_000;
/// The revision a plain initialize offers.
const LEGACY_VERSION: &str = "2025-11-25";
const SUPPORTED: [&str; 5] = [
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
    "2024-10-07",
];
const MODERN_VERSION: &str = "2026-07-28";

/// A configured server, as the runtime reads it from `mcp.servers.<name>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub transport: Kind,
    pub codemode: bool,
    pub startup: Duration,
    pub catalog: Duration,
    pub execution: Duration,
    /// `legacy` (absent), `auto` or a pinned revision.
    pub protocol: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Local {
        command: Vec<String>,
        cwd: PathBuf,
        environment: Vec<(String, String)>,
    },
    Remote {
        url: String,
        headers: Vec<(String, String)>,
    },
}

impl Spec {
    /// `cwd` is relative to the location directory, which is also the default.
    pub fn from_config(server: &Value, directory: &Path) -> Spec {
        let millis = |key: &str, default: u64| {
            Duration::from_millis(server["timeout"][key].as_u64().unwrap_or(default))
        };
        let pairs = |value: &Value| -> Vec<(String, String)> {
            value
                .as_object()
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                        .collect()
                })
                .unwrap_or_default()
        };
        let transport = if server["type"] == "local" {
            let command: Vec<String> = server["command"]
                .as_array()
                .map(|c| {
                    c.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let mut environment = pairs(&server["environment"]);
            if command.first().map(String::as_str) == Some("opencode") {
                environment.insert(0, ("BUN_BE_BUN".into(), "1".into()));
            }
            Kind::Local {
                command,
                cwd: server["cwd"]
                    .as_str()
                    .map_or_else(|| directory.to_path_buf(), |c| directory.join(c)),
                environment,
            }
        } else {
            Kind::Remote {
                url: server["url"].as_str().unwrap_or_default().to_string(),
                headers: pairs(&server["headers"]),
            }
        };
        Spec {
            transport,
            codemode: server["codemode"] != json!(false),
            startup: millis("startup", DEFAULT_STARTUP),
            catalog: millis("catalog", DEFAULT_CATALOG),
            execution: millis("execution", DEFAULT_EXECUTION),
            protocol: server["protocol"]
                .as_str()
                .filter(|p| *p != "legacy")
                .map(str::to_string),
        }
    }
}

/// Why a server did not connect.
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectError {
    NeedsAuth(String),
    Failed(String),
}

pub struct Connection {
    peer: Arc<Peer>,
    capabilities: Value,
    pub instructions: Option<String>,
    catalog: Duration,
    execution: Duration,
}

/// These servers reject unknown query parameters and have no Code Mode of their own.
fn rejects_unknown_query(url: &url::Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    let zoho = host.split('.').collect::<Vec<_>>();
    host.ends_with(".googleapis.com")
        || zoho
            .iter()
            .position(|label| *label == "zohomcp")
            .is_some_and(|i| i + 1 < zoho.len())
        || host == "localhost"
        || host == "[::1]"
        || (host.starts_with("127.")
            && host.split('.').count() == 4
            && host.split('.').all(|p| p.parse::<u8>().is_ok()))
}

/// A tool `tools/list` may carry; others are dropped one by one.
fn valid_tool(tool: &Value) -> bool {
    let object_schema =
        |schema: Option<&Value>| schema.is_some_and(|s| s.is_object() && s["type"] == "object");
    tool["name"].is_string()
        && object_schema(tool.get("inputSchema"))
        && tool
            .get("outputSchema")
            .is_none_or(|s| object_schema(Some(s)))
        && tool.get("description").is_none_or(Value::is_string)
}

impl Connection {
    pub async fn connect(
        server: &str,
        spec: &Spec,
        handler: Arc<dyn Handler>,
        auth: Option<Arc<dyn Auth>>,
    ) -> Result<Connection, ConnectError> {
        let failed = |e: Error| match e {
            Error::Unauthorized(message) => ConnectError::NeedsAuth(message),
            other => ConnectError::Failed(other.to_string()),
        };
        match &spec.transport {
            Kind::Local {
                command,
                cwd,
                environment,
            } => {
                let (sender, receiver) = mpsc::unbounded_channel();
                let stdio =
                    Stdio::spawn(server, command, cwd, environment, sender).map_err(failed)?;
                let transport: Arc<dyn Transport> = Arc::new(stdio);
                let peer = Peer::start(transport, receiver, handler);
                Self::initialize(peer, None, spec).await.map_err(failed)
            }
            Kind::Remote { url, headers } => {
                let configured = url::Url::parse(url).map_err(|_| {
                    ConnectError::Failed(format!("Invalid MCP URL for \"{server}\""))
                })?;
                let mut dialed = configured.clone();
                let added = spec.codemode
                    && !configured.query_pairs().any(|(k, _)| k == "codemode")
                    && !rejects_unknown_query(&configured);
                if added {
                    dialed.query_pairs_mut().append_pair("codemode", "false");
                }
                let open = |url: url::Url| {
                    let (handler, auth) = (handler.clone(), auth.clone());
                    async move {
                        let (sender, receiver) = mpsc::unbounded_channel();
                        let http = Http::new(url, headers.clone(), auth, sender);
                        let transport: Arc<dyn Transport> = Arc::new(HttpTransport(http.clone()));
                        let peer = Peer::start(transport, receiver, handler);
                        Self::initialize(peer, Some(http), spec).await
                    }
                };
                // Two more attempts, 250 ms and 1 s later, while the failure is transient and the
                // startup deadline has not passed.
                let deadline = Instant::now() + spec.startup;
                let mut delays = [Duration::from_millis(250), Duration::from_secs(1)].into_iter();
                loop {
                    let result = match open(dialed.clone()).await {
                        Err(Error::Http { status, .. })
                            if added && (status == 400 || status == 404) =>
                        {
                            open(configured.clone()).await
                        }
                        other => other,
                    };
                    match result {
                        Ok(connection) => return Ok(connection),
                        Err(error) => match delays.next() {
                            Some(delay)
                                if error.transient() && Instant::now() + delay < deadline =>
                            {
                                tokio::time::sleep(delay).await;
                            }
                            _ => return Err(failed(error)),
                        },
                    }
                }
            }
        }
    }

    async fn initialize(
        peer: Arc<Peer>,
        http: Option<Arc<Http>>,
        spec: &Spec,
    ) -> Result<Connection, Error> {
        let offered = match spec.protocol.as_deref() {
            None => LEGACY_VERSION,
            Some(_) => MODERN_VERSION,
        };
        let params = json!({
            "protocolVersion": offered,
            "capabilities": {"elicitation": {"form": {}, "url": {}}, "roots": {}},
            "clientInfo": {"name": "opencode", "version": anysola_core::version::session_version()},
        });
        let result = match peer.request("initialize", params, spec.startup).await {
            Ok(result) => result,
            Err(error) => {
                peer.close().await;
                return Err(error);
            }
        };
        let version = result["protocolVersion"].as_str().unwrap_or_default();
        let accepted = match spec.protocol.as_deref() {
            None => SUPPORTED.contains(&version),
            Some("auto") => version == MODERN_VERSION || SUPPORTED.contains(&version),
            Some(pinned) => version == pinned,
        };
        if !accepted {
            peer.close().await;
            return Err(Error::Other(match spec.protocol.as_deref() {
                Some(pinned) if pinned != "auto" => format!(
                    "Server does not support protocol version {pinned}; the server supports {version}. Set \"protocol\" for this server to one of those or to \"legacy\"."
                ),
                _ => format!("Server's protocol version is not supported: {version}"),
            }));
        }
        if let Some(http) = &http {
            http.set_protocol(version);
        }
        peer.notify("notifications/initialized", Value::Null)
            .await?;
        Ok(Connection {
            peer,
            capabilities: result["capabilities"].clone(),
            instructions: result["instructions"]
                .as_str()
                .map(str::trim)
                .filter(|i| !i.is_empty())
                .map(str::to_string),
            catalog: spec.catalog,
            execution: spec.execution,
        })
    }

    fn advertised(&self, capability: &str) -> bool {
        self.capabilities
            .get(capability)
            .is_some_and(|c| !c.is_null())
    }

    /// Every page of a listing; an unadvertised capability lists nothing without asking.
    async fn list(&self, capability: &str, method: &str, key: &str) -> Result<Vec<Value>, Error> {
        if !self.advertised(capability) {
            return Ok(Vec::new());
        }
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = match &cursor {
                Some(cursor) => json!({"cursor": cursor}),
                None => json!({}),
            };
            let page = self.peer.request(method, params, self.catalog).await?;
            items.extend(page[key].as_array().cloned().unwrap_or_default());
            match page["nextCursor"].as_str() {
                Some(next) => cursor = Some(next.to_string()),
                None => return Ok(items),
            }
        }
    }

    pub async fn tools(&self) -> Result<Vec<Value>, Error> {
        let tools = self.list("tools", "tools/list", "tools").await?;
        Ok(tools
            .into_iter()
            .filter(|tool| {
                let valid = valid_tool(tool);
                if !valid {
                    anysola_core::log::warn(
                        "skipping invalid MCP tool",
                        &[("tool", tool["name"].as_str().unwrap_or_default())],
                    );
                }
                valid
            })
            .collect())
    }

    pub async fn prompts(&self) -> Result<Vec<Value>, Error> {
        self.list("prompts", "prompts/list", "prompts").await
    }

    pub async fn resources(&self) -> Result<Vec<Value>, Error> {
        self.list("resources", "resources/list", "resources").await
    }

    pub async fn resource_templates(&self) -> Result<Vec<Value>, Error> {
        self.list("resources", "resources/templates/list", "resourceTemplates")
            .await
    }

    /// `None` when the server does not advertise resources.
    pub async fn read_resource(&self, uri: &str) -> Result<Option<Value>, Error> {
        if !self.advertised("resources") {
            return Ok(None);
        }
        self.peer
            .request("resources/read", json!({"uri": uri}), self.execution)
            .await
            .map(Some)
    }

    pub async fn prompt(&self, name: &str, arguments: Map<String, Value>) -> Result<Value, Error> {
        self.peer
            .request(
                "prompts/get",
                json!({"name": name, "arguments": arguments}),
                self.execution,
            )
            .await
    }

    /// Requests progress so long calls are kept alive; the execution timeout is the hard wall.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
        session_id: &str,
    ) -> Result<Value, Error> {
        let token = self.peer.next_id();
        let params = json!({
            "name": name,
            "arguments": arguments,
            "_meta": {"ai.opencode/sessionID": session_id, "progressToken": token},
        });
        self.peer
            .request("tools/call", params, self.execution)
            .await
    }

    /// Ends the session (HTTP DELETE) or the process.
    pub async fn close(&self) {
        self.peer.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_that_reject_unknown_query() {
        for (url, rejects) in [
            ("https://gmailmcp.googleapis.com/mcp", true),
            ("https://x.zohomcp.eu/mcp", true),
            ("http://127.0.0.1:9/mcp", true),
            ("http://localhost/mcp", true),
            ("https://mcp.example.com/mcp", false),
        ] {
            assert_eq!(
                rejects_unknown_query(&url::Url::parse(url).unwrap()),
                rejects,
                "{url}"
            );
        }
    }

    #[test]
    fn malformed_tools() {
        assert!(valid_tool(
            &json!({"name": "a", "inputSchema": {"type": "object"}})
        ));
        assert!(!valid_tool(
            &json!({"name": "a", "inputSchema": {"type": "string"}})
        ));
        assert!(!valid_tool(&json!({"inputSchema": {"type": "object"}})));
    }
}
