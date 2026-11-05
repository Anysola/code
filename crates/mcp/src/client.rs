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
const PROTOCOL_KEY: &str = "io.modelcontextprotocol/protocolVersion";
const CLIENT_INFO_KEY: &str = "io.modelcontextprotocol/clientInfo";
const CLIENT_CAPABILITIES_KEY: &str = "io.modelcontextprotocol/clientCapabilities";
/// The JSON-RPC error a server answers an unsupported revision with.
const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

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

fn instructions(result: &Value) -> Option<String> {
    result["instructions"]
        .as_str()
        .map(str::trim)
        .filter(|i| !i.is_empty())
        .map(str::to_string)
}

/// What one `server/discover` answer means for the connection.
enum Verdict {
    Modern(Value),
    /// The server named a revision we offer in its rejection: ask again with it, once.
    Corrective(String, Error),
    Legacy,
    Failed(Error),
}

/// 0.5.1's message for a revision the server does not support (`client.ts:458-463`).
fn unsupported(message: String, supported: &[String]) -> Error {
    Error::Other(format!(
        "{message}; the server supports {}. Set \"protocol\" for this server to one of those or to \"legacy\".",
        supported.join(", ")
    ))
}

fn negotiation_failed(text: &str) -> Error {
    Error::Other(format!("Version negotiation failed: {text}"))
}

fn is_modern(version: &str) -> bool {
    version >= MODERN_VERSION
}

fn strings(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|v| v.as_str().map(str::to_string))
        .collect()
}

fn classify_rpc(code: i64, message: String, data: Option<&Value>, fallback: bool) -> Verdict {
    if code != UNSUPPORTED_PROTOCOL_VERSION {
        return Verdict::Legacy;
    }
    let Some(supported) = data
        .and_then(|d| strings(&d["supported"]))
        .filter(|s| !s.is_empty())
    else {
        return Verdict::Legacy;
    };
    let error = unsupported(message, &supported);
    let modern: Vec<&String> = supported.iter().filter(|v| is_modern(v)).collect();
    if modern.iter().any(|v| *v == MODERN_VERSION) {
        return Verdict::Corrective(MODERN_VERSION.to_string(), error);
    }
    if !modern.is_empty() || !fallback {
        return Verdict::Failed(error);
    }
    Verdict::Legacy
}

/// The 2026-07-28 revision's `server/discover` probe, sent before any initialize (the SDK's
/// `negotiateEra`): `Some` with the agreed revision and the discover result, `None` to fall back to
/// the plain initialize. `pinned` has no fallback. 0.5.1's own stdio transport is not the SDK's,
/// so a probe that closes the process is a connect error rather than a retry on a sibling.
async fn negotiate(
    peer: &Peer,
    stdio: bool,
    pinned: Option<&str>,
    client_info: &Value,
    capabilities: &Value,
    timeout: Duration,
) -> Result<Option<(String, Value)>, Error> {
    let fallback = pinned.is_none();
    let mut requested = pinned.unwrap_or(MODERN_VERSION).to_string();
    let mut corrected = false;
    loop {
        let envelope = json!({
            PROTOCOL_KEY: requested,
            CLIENT_INFO_KEY: client_info,
            CLIENT_CAPABILITIES_KEY: capabilities,
        });
        let reply = peer
            .request("server/discover", json!({"_meta": envelope}), timeout)
            .await;
        let mut closed = false;
        let verdict = match reply {
            Ok(result) => {
                match strings(&result["supportedVersions"]) {
                    Some(supported) if result["capabilities"].is_object() => {
                        if supported.contains(&requested) {
                            Verdict::Modern(result)
                        } else if fallback {
                            Verdict::Legacy
                        } else {
                            Verdict::Failed(unsupported(
                                format!("Unsupported protocol version: {requested}"),
                                &supported,
                            ))
                        }
                    }
                    _ => Verdict::Legacy,
                }
            }
            Err(Error::Rpc { code, message, data }) => {
                classify_rpc(code, message, data.as_ref(), fallback)
            }
            Err(Error::Unauthorized(message)) => Verdict::Failed(Error::Unauthorized(message)),
            Err(Error::Http { status, body }) => match status {
                401 => Verdict::Failed(negotiation_failed(
                    "the server requires authorization (HTTP 401)",
                )),
                403 => Verdict::Failed(negotiation_failed("the server denied access (HTTP 403)")),
                500.. => Verdict::Failed(negotiation_failed(&format!(
                    "the server answered the probe with HTTP {status}"
                ))),
                _ => match serde_json::from_str::<Value>(&body)
                    .ok()
                    .filter(|b| b["error"]["code"].is_i64())
                {
                    Some(body) => classify_rpc(
                        body["error"]["code"].as_i64().unwrap_or_default(),
                        body["error"]["message"].as_str().unwrap_or_default().to_string(),
                        body["error"].get("data"),
                        fallback,
                    ),
                    None => Verdict::Legacy,
                },
            },
            Err(Error::Closed(_)) if stdio => {
                closed = true;
                Verdict::Legacy
            }
            Err(Error::Closed(_)) => Verdict::Failed(Error::Other(
                "Version negotiation probe failed: Connection closed during the version negotiation probe"
                    .into(),
            )),
            Err(Error::Timeout) if stdio => Verdict::Legacy,
            Err(Error::Timeout) => Verdict::Failed(Error::Other(format!(
                "Version negotiation probe timed out after {}ms",
                timeout.as_millis()
            ))),
            Err(error) => Verdict::Failed(Error::Other(format!(
                "Version negotiation probe failed: {error}"
            ))),
        };
        match verdict {
            Verdict::Modern(result) => return Ok(Some((requested, result))),
            Verdict::Corrective(version, error) => {
                if corrected {
                    return Err(error);
                }
                corrected = true;
                requested = version;
            }
            Verdict::Failed(error) => return Err(error),
            Verdict::Legacy => {
                const CLOSED: &str = "the connection closed during the server/discover probe";
                if let Some(pinned) = pinned {
                    return Err(negotiation_failed(&if closed {
                        format!(
                            "{CLOSED} before the server offered pinned protocol version {pinned} (no fallback in pin mode)"
                        )
                    } else {
                        format!(
                            "the server did not offer pinned protocol version {pinned} via server/discover (no fallback in pin mode)"
                        )
                    }));
                }
                if closed {
                    return Err(negotiation_failed(&format!(
                        "{CLOSED} (this transport probed in place — the disposable sibling probe requires the SDK's base StdioClientTransport)"
                    )));
                }
                return Ok(None);
            }
        }
    }
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
        let capabilities = json!({"elicitation": {"form": {}, "url": {}}, "roots": {}});
        let client_info =
            json!({"name": "opencode", "version": anysola_core::version::session_version()});
        let handshake = match spec.protocol.as_deref() {
            None => Ok(None),
            Some(protocol) => {
                let pinned = (protocol != "auto").then_some(protocol);
                negotiate(
                    &peer,
                    http.is_none(),
                    pinned,
                    &client_info,
                    &capabilities,
                    spec.startup,
                )
                .await
            }
        };
        let modern = match handshake {
            Ok(modern) => modern,
            Err(error) => {
                peer.close().await;
                return Err(error);
            }
        };
        if let Some((version, discover)) = modern {
            peer.set_envelope(json!({
                PROTOCOL_KEY: version,
                CLIENT_INFO_KEY: client_info,
                CLIENT_CAPABILITIES_KEY: capabilities,
            }));
            if let Some(http) = &http {
                http.set_protocol(&version);
            }
            return Ok(Connection {
                peer,
                capabilities: discover["capabilities"].clone(),
                instructions: instructions(&discover),
                catalog: spec.catalog,
                execution: spec.execution,
            });
        }
        let params = json!({
            "protocolVersion": LEGACY_VERSION,
            "capabilities": capabilities,
            "clientInfo": client_info,
        });
        let result = match peer.request("initialize", params, spec.startup).await {
            Ok(result) => result,
            Err(error) => {
                peer.close().await;
                return Err(error);
            }
        };
        let version = result["protocolVersion"].as_str().unwrap_or_default();
        if !SUPPORTED.contains(&version) {
            peer.close().await;
            return Err(Error::Other(format!(
                "Server's protocol version is not supported: {version}"
            )));
        }
        if let Some(http) = &http {
            http.set_protocol(version);
        }
        peer.notify("notifications/initialized", Value::Null)
            .await?;
        Ok(Connection {
            peer,
            capabilities: result["capabilities"].clone(),
            instructions: instructions(&result),
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
    fn specs_from_config() {
        let dir = Path::new("/w");
        let local = Spec::from_config(
            &json!({"type": "local", "command": ["opencode", "mcp"], "cwd": "sub", "environment": {"A": "1"}}),
            dir,
        );
        assert_eq!(
            local.transport,
            Kind::Local {
                command: vec!["opencode".into(), "mcp".into()],
                cwd: dir.join("sub"),
                environment: vec![("BUN_BE_BUN".into(), "1".into()), ("A".into(), "1".into())],
            }
        );
        assert_eq!(
            (local.startup, local.catalog, local.execution),
            (
                Duration::from_secs(30),
                Duration::from_secs(30),
                Duration::from_secs(12 * 60 * 60)
            )
        );
        let remote = Spec::from_config(
            &json!({"type": "remote", "url": "https://h/mcp", "codemode": false, "timeout": {"startup": 5}, "protocol": "legacy"}),
            dir,
        );
        assert_eq!(
            (remote.codemode, remote.startup, remote.protocol),
            (false, Duration::from_millis(5), None)
        );
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
