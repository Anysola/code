//! The configured servers of one process: their lifecycle and statuses, their tools in the Code
//! Mode registry, and what the core asks of them ([`anysola_core::mcp::Mcp`]).

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use anysola_core::api::Core;
use anysola_core::codemode::{Namespace, sanitize};
use anysola_core::config::Config;
use anysola_core::events::Event;
use anysola_core::mcp::{Instructions, Mcp, PromptCommand, Server, SignIn, Status};
use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde_json::{Map, Value, json};
use tokio::sync::watch;

use crate::client::{ConnectError, Connection, Kind, Spec};
use crate::http::Auth;
use crate::rpc::{Error, Handler};

/// Tool lists arrive in bursts; one refresh after they settle.
const TOOLS_DEBOUNCE: Duration = Duration::from_millis(100);

struct Entry {
    config: Value,
    status: Status,
    /// Bumped by every start and stop, so callbacks of an older connection are ignored.
    generation: u64,
    connection: Option<Arc<Connection>>,
    tools: Vec<Value>,
    prompts: Vec<Value>,
    /// False while a start is under way.
    settled: watch::Sender<bool>,
    /// A tool list refresh is already waiting out the debounce.
    refreshing: bool,
}

pub struct Runtime {
    core: Core,
    me: Weak<Runtime>,
    servers: Mutex<BTreeMap<String, Entry>>,
    /// Config order, which the instructions keep.
    order: Mutex<Vec<String>>,
    /// URL elicitations waiting for their completion notice: `server\0id` → form id.
    url_forms: Mutex<HashMap<String, String>>,
}

/// Startups to one URL run one at a time.
fn endpoint_lock(url: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(url.to_string())
        .or_default()
        .clone()
}

impl Runtime {
    /// Attaches a runtime to the core, which hands it the config and its changes.
    pub fn attach(core: &Core) -> Arc<Runtime> {
        let runtime = Arc::new_cyclic(|me| Runtime {
            core: core.clone(),
            me: me.clone(),
            servers: Mutex::default(),
            order: Mutex::default(),
            url_forms: Mutex::default(),
        });
        core.attach_mcp(runtime.clone());
        runtime
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Entry>> {
        self.servers.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn directory(&self) -> PathBuf {
        self.core.directory().to_path_buf()
    }

    /// The configured entry of a server.
    pub fn config(&self, name: &str) -> Option<Value> {
        self.lock().get(name).map(|e| e.config.clone())
    }

    fn set_status(&self, name: &str, entry: &mut Entry, status: Status) {
        entry.status = status.clone();
        self.core.emit(Event::McpStatusChanged {
            server: Server {
                name: name.to_string(),
                status,
                integration: crate::oauth::capable(&entry.config).then(|| {
                    crate::oauth::integration_id(
                        name,
                        entry.config["url"].as_str().unwrap_or_default(),
                    )
                }),
            },
        });
    }

    fn publish_tools(&self, name: &str, entry: &Entry) {
        let Some(me) = self.me.upgrade() else {
            return;
        };
        self.core.set_namespace(Arc::new(Namespace {
            name: sanitize(name),
            description: None,
            direct: entry.config["codemode"] == json!(false),
            tools: crate::tool::tools(&me, name, &entry.tools),
        }));
    }

    /// Closes the server's connection and takes its tools away.
    fn stop(&self, name: &str, entry: &mut Entry) {
        entry.generation += 1;
        entry.tools.clear();
        entry.prompts.clear();
        if let Some(connection) = entry.connection.take() {
            self.core.remove_namespace(&sanitize(name));
            tokio::spawn(async move { connection.close().await });
        }
    }

    fn start(&self, name: &str) {
        let generation = {
            let mut servers = self.lock();
            let Some(entry) = servers.get_mut(name) else {
                return;
            };
            self.stop(name, entry);
            entry.settled.send_replace(false);
            self.set_status(name, entry, Status::Pending);
            entry.generation
        };
        let (me, name) = (self.me.clone(), name.to_string());
        tokio::spawn(async move {
            if let Some(runtime) = me.upgrade() {
                runtime.connect(&name, generation).await;
            }
        });
    }

    async fn connect(self: Arc<Self>, name: &str, generation: u64) {
        let Some(config) = self.config(name) else {
            return;
        };
        let spec = Spec::from_config(&config, &self.directory());
        let auth: Option<Arc<dyn Auth>> = crate::oauth::capable(&config).then(|| {
            Arc::new(crate::oauth::Provider::new(
                self.core.db().clone(),
                name,
                &config,
            )) as Arc<dyn Auth>
        });
        let handler = Arc::new(ServerHandler {
            runtime: self.me.clone(),
            server: name.to_string(),
            generation,
        });
        let serialized = match &spec.transport {
            Kind::Remote { url, .. } => Some(endpoint_lock(url)),
            Kind::Local { .. } => None,
        };
        let _held = match &serialized {
            Some(lock) => Some(lock.lock().await),
            None => None,
        };
        // Tools are listed as part of connect, so a listing failure marks the server failed.
        let result = match Connection::connect(name, &spec, handler, auth).await {
            Ok(connection) => match connection.tools().await {
                Ok(tools) => Ok((connection, tools)),
                Err(error) => {
                    connection.close().await;
                    Err(ConnectError::Failed(error.to_string()))
                }
            },
            Err(error) => Err(error),
        };
        let connected = {
            let mut servers = self.lock();
            let Some(entry) = servers.get_mut(name).filter(|e| e.generation == generation) else {
                if let Ok((connection, _)) = result {
                    tokio::spawn(async move { connection.close().await });
                }
                return;
            };
            let status = match result {
                Ok((connection, tools)) => {
                    entry.connection = Some(Arc::new(connection));
                    entry.tools = tools;
                    self.publish_tools(name, entry);
                    Status::Connected
                }
                Err(ConnectError::NeedsAuth(error)) => Status::NeedsAuth { error },
                Err(ConnectError::Failed(error)) => Status::Failed { error },
            };
            let connected = status == Status::Connected;
            self.set_status(name, entry, status);
            connected
        };
        // Prompts are known before the start counts as finished, so commands and `settled` see
        // them.
        if connected {
            self.refresh_prompts(name, generation).await;
        }
        if let Some(entry) = self
            .lock()
            .get_mut(name)
            .filter(|e| e.generation == generation)
        {
            entry.settled.send_replace(true);
        }
    }

    fn live(&self, name: &str, generation: u64) -> Option<Arc<Connection>> {
        self.lock()
            .get(name)
            .filter(|e| e.generation == generation)
            .and_then(|e| e.connection.clone())
    }

    async fn refresh_prompts(&self, name: &str, generation: u64) {
        let Some(connection) = self.live(name, generation) else {
            return;
        };
        let prompts = connection.prompts().await.unwrap_or_default();
        if let Some(entry) = self
            .lock()
            .get_mut(name)
            .filter(|e| e.generation == generation)
        {
            entry.prompts = prompts;
        }
    }

    async fn refresh_tools(&self, name: &str, generation: u64) {
        tokio::time::sleep(TOOLS_DEBOUNCE).await;
        if let Some(entry) = self.lock().get_mut(name) {
            entry.refreshing = false;
        }
        let Some(connection) = self.live(name, generation) else {
            return;
        };
        let Ok(tools) = connection.tools().await else {
            return;
        };
        if let Some(entry) = self
            .lock()
            .get_mut(name)
            .filter(|e| e.generation == generation)
        {
            entry.tools = tools;
            self.publish_tools(name, entry);
        }
    }

    fn closed(&self, name: &str, generation: u64, reason: String) {
        let mut servers = self.lock();
        let Some(entry) = servers
            .get_mut(name)
            .filter(|e| e.generation == generation && e.connection.is_some())
        else {
            return;
        };
        self.stop(name, entry);
        self.set_status(name, entry, Status::Failed { error: reason });
    }

    /// Waits until no server is starting, at most `limit`.
    pub async fn settled(&self, limit: Duration) {
        let receivers: Vec<watch::Receiver<bool>> = self
            .lock()
            .values()
            .map(|e| e.settled.subscribe())
            .collect();
        let _ = tokio::time::timeout(limit, async {
            for mut receiver in receivers {
                let _ = receiver.wait_for(|settled| *settled).await;
            }
        })
        .await;
    }

    /// Waits for the server's current start to finish.
    async fn started(&self, name: &str) {
        let receiver = self.lock().get(name).map(|e| e.settled.subscribe());
        if let Some(mut receiver) = receiver {
            let _ = receiver.wait_for(|settled| *settled).await;
        }
    }

    /// Why a server's tools cannot be called now.
    fn unavailable(name: &str, status: &Status) -> String {
        match status {
            Status::Failed { error } => {
                format!("MCP server \"{name}\" is not connected: {error}. Reconnect it from /mcps.")
            }
            Status::NeedsAuth { error } => {
                format!("MCP server \"{name}\" needs authentication: {error}. Sign in from /mcps.")
            }
            Status::Disabled => format!("MCP server \"{name}\" is disabled."),
            _ => format!("MCP server \"{name}\" is not connected."),
        }
    }

    /// The connection of a server once its start finished, or why there is none.
    async fn connection(&self, name: &str) -> Result<(Arc<Connection>, u64), String> {
        if !self.lock().contains_key(name) {
            return Err(format!("MCP server not found: {name}"));
        }
        self.started(name).await;
        let servers = self.lock();
        let entry = servers
            .get(name)
            .ok_or_else(|| format!("MCP server not found: {name}"))?;
        match &entry.connection {
            Some(connection) => Ok((connection.clone(), entry.generation)),
            None => Err(Self::unavailable(name, &entry.status)),
        }
    }

    /// Runs a request; when the server dropped the HTTP session, reconnects and runs it once
    /// more.
    async fn recovering<T, F>(&self, name: &str, run: F) -> Result<T, String>
    where
        F: Fn(Arc<Connection>) -> BoxFuture<'static, Result<T, Error>>,
    {
        let (connection, generation) = self.connection(name).await?;
        match run(connection).await {
            Err(Error::SessionExpired) => {
                if self.live(name, generation).is_some() {
                    self.start(name);
                }
                let (connection, _) = self.connection(name).await?;
                run(connection).await.map_err(|e| e.to_string())
            }
            other => other.map_err(|e| e.to_string()),
        }
    }

    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        input: Value,
        session_id: &str,
    ) -> Result<Value, String> {
        if !self.lock().contains_key(server) {
            return Err(format!("MCP server \"{server}\" is not available"));
        }
        let (tool_name, session) = (tool.to_string(), session_id.to_string());
        // Unavailable servers are reported as they are, not as a failed call.
        self.connection(server).await?;
        self.recovering(server, move |connection| {
            let (tool, input, session) = (tool_name.clone(), input.clone(), session.clone());
            async move { connection.call_tool(&tool, input, &session).await }.boxed()
        })
        .await
        .map_err(|e| format!("MCP tool \"{tool}\" on server \"{server}\" failed: {e}"))
    }

    async fn catalog(&self, name: &str) -> Result<Value, String> {
        let server = name.to_string();
        self.recovering(name, move |connection| {
            let server = server.clone();
            async move {
                let resources = connection.resources().await?;
                // Some servers declare resources without implementing template listing.
                let templates = connection.resource_templates().await.unwrap_or_default();
                let pick = |item: &Value, keys: &[&str]| {
                    let mut out = Map::new();
                    out.insert("server".into(), json!(server));
                    for key in keys {
                        if let Some(v) = item.get(*key).filter(|v| !v.is_null()) {
                            out.insert((*key).into(), v.clone());
                        }
                    }
                    Value::Object(out)
                };
                Ok(json!({
                    "resources": resources.iter().map(|r| pick(r, &["name", "uri", "description", "mimeType"])).collect::<Vec<_>>(),
                    "templates": templates.iter().map(|t| pick(t, &["name", "uriTemplate", "description", "mimeType"])).collect::<Vec<_>>(),
                }))
            }
            .boxed()
        })
        .await
    }

    /// Closes every connection (the process is ending).
    pub async fn shutdown(&self) {
        let connections: Vec<Arc<Connection>> = self
            .lock()
            .values_mut()
            .filter_map(|e| {
                e.generation += 1;
                e.connection.take()
            })
            .collect();
        futures_util::future::join_all(connections.iter().map(|c| c.close())).await;
    }
}

impl Mcp for Runtime {
    fn servers(&self) -> Vec<Server> {
        self.lock()
            .iter()
            .map(|(name, entry)| Server {
                name: name.clone(),
                status: entry.status.clone(),
                integration: crate::oauth::capable(&entry.config).then(|| {
                    crate::oauth::integration_id(
                        name,
                        entry.config["url"].as_str().unwrap_or_default(),
                    )
                }),
            })
            .collect()
    }

    fn reconcile(&self, config: Arc<Config>) {
        let desired = config.mcp_servers();
        *self.order.lock().unwrap_or_else(|e| e.into_inner()) =
            desired.iter().map(|(n, _)| n.clone()).collect();
        let mut starting = Vec::new();
        {
            let mut servers = self.lock();
            let removed: Vec<String> = servers
                .keys()
                .filter(|name| !desired.iter().any(|(n, _)| n == *name))
                .cloned()
                .collect();
            for name in removed {
                if let Some(mut entry) = servers.remove(&name) {
                    self.stop(&name, &mut entry);
                    entry.settled.send_replace(true);
                }
            }
            for (name, server) in desired {
                if servers.get(&name).is_some_and(|e| e.config == server) {
                    continue;
                }
                if let Some(mut previous) = servers.remove(&name) {
                    self.stop(&name, &mut previous);
                    previous.settled.send_replace(true);
                }
                let disabled = server["disabled"] == json!(true);
                let mut entry = Entry {
                    config: server,
                    status: Status::Disabled,
                    generation: 0,
                    connection: None,
                    tools: Vec::new(),
                    prompts: Vec::new(),
                    settled: watch::Sender::new(true),
                    refreshing: false,
                };
                if disabled {
                    self.set_status(&name, &mut entry, Status::Disabled);
                } else {
                    starting.push(name.clone());
                }
                servers.insert(name, entry);
            }
        }
        for name in starting {
            self.start(&name);
        }
    }

    fn instructions(&self) -> Vec<Instructions> {
        let order = self.order.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let servers = self.lock();
        order
            .iter()
            .filter_map(|name| {
                let entry = servers.get(name)?;
                let text = entry.connection.as_ref()?.instructions.clone()?;
                let namespace = sanitize(name);
                Some(Instructions {
                    server: name.clone(),
                    text,
                    codemode: entry.config["codemode"] != json!(false),
                    tools: entry
                        .tools
                        .iter()
                        .map(|t| {
                            anysola_core::codemode::direct_name(
                                &namespace,
                                t["name"].as_str().unwrap_or_default(),
                            )
                        })
                        .collect(),
                })
            })
            .collect()
    }

    fn list_resources<'a>(
        &'a self,
        server: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Value, String>> {
        async move {
            if let Some(server) = server {
                return self.catalog(server).await;
            }
            let names: Vec<String> = self.lock().keys().cloned().collect();
            let mut resources = Vec::new();
            let mut templates = Vec::new();
            for name in names {
                if let Ok(catalog) = self.catalog(&name).await {
                    resources.extend(catalog["resources"].as_array().cloned().unwrap_or_default());
                    templates.extend(catalog["templates"].as_array().cloned().unwrap_or_default());
                }
            }
            let key = |v: &Value, k: &str| v[k].as_str().unwrap_or_default().to_string();
            resources.sort_by_key(|r| (key(r, "server"), key(r, "name"), key(r, "uri")));
            templates.sort_by_key(|t| (key(t, "server"), key(t, "name"), key(t, "uriTemplate")));
            Ok(json!({"resources": resources, "templates": templates}))
        }
        .boxed()
    }

    fn read_resource<'a>(
        &'a self,
        server: &'a str,
        uri: &'a str,
    ) -> BoxFuture<'a, Result<Option<Value>, String>> {
        async move {
            let target = uri.to_string();
            let result = self
                .recovering(server, move |connection| {
                    let uri = target.clone();
                    async move { connection.read_resource(&uri).await }.boxed()
                })
                .await
                .map_err(|e| {
                    if e.starts_with("MCP server not found") {
                        e
                    } else {
                        format!("Unable to read MCP resource {server}:{uri}: {e}")
                    }
                })?;
            Ok(result.map(|read| {
                let contents: Vec<Value> = read["contents"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|part| {
                        let mut out = match part.get("text") {
                            Some(text) => json!({"type": "text", "uri": part["uri"], "text": text}),
                            None => {
                                json!({"type": "blob", "uri": part["uri"], "blob": part["blob"]})
                            }
                        };
                        if let Some(mime) = part.get("mimeType") {
                            out["mimeType"] = mime.clone();
                        }
                        out
                    })
                    .collect();
                json!({"server": server, "uri": uri, "contents": contents})
            }))
        }
        .boxed()
    }

    fn prompts(&self) -> Vec<PromptCommand> {
        self.lock()
            .iter()
            .flat_map(|(server, entry)| {
                entry.prompts.iter().filter_map(move |prompt| {
                    let name = prompt["name"].as_str()?;
                    Some(PromptCommand {
                        name: format!("{}:{}", sanitize(server), sanitize(name)),
                        description: prompt["description"].as_str().map(str::to_string),
                        server: server.clone(),
                        prompt: name.to_string(),
                        arguments: prompt["arguments"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|a| a["name"].as_str().map(str::to_string))
                            .collect(),
                    })
                })
            })
            .collect()
    }

    fn expand_prompt<'a>(
        &'a self,
        command: &'a PromptCommand,
        arguments: Vec<String>,
    ) -> BoxFuture<'a, Result<String, String>> {
        async move {
            let values: Map<String, Value> = command
                .arguments
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    (
                        name.clone(),
                        json!(arguments.get(i).cloned().unwrap_or_default()),
                    )
                })
                .collect();
            let prompt = command.prompt.clone();
            let result = self
                .recovering(&command.server, move |connection| {
                    let (prompt, values) = (prompt.clone(), values.clone());
                    async move { connection.prompt(&prompt, values).await }.boxed()
                })
                .await?;
            let text: Vec<&str> = result["messages"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|m| match &m["content"] {
                    Value::String(text) => text.as_str(),
                    content if content["type"] == "text" => {
                        content["text"].as_str().unwrap_or_default()
                    }
                    _ => "",
                })
                .collect();
            Ok(text.join("\n").trim().to_string())
        }
        .boxed()
    }

    fn connect(&self, name: &str) {
        self.start(name);
    }

    fn authorize<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<SignIn, String>> {
        async move {
            let config = self
                .config(name)
                .ok_or_else(|| format!("MCP server not found: {name}"))?;
            let mut attempt =
                crate::oauth::authorize(self.core.db().clone(), name, &config).await?;
            let (sender, done) = tokio::sync::oneshot::channel();
            let inner = std::mem::replace(&mut attempt.done, done);
            let (me, name) = (self.me.clone(), name.to_string());
            tokio::spawn(async move {
                let result = inner
                    .await
                    .unwrap_or_else(|_| Err("Authorization expired".into()));
                if result.is_ok()
                    && let Some(runtime) = me.upgrade()
                {
                    runtime.start(&name);
                }
                let _ = sender.send(result);
            });
            Ok(attempt)
        }
        .boxed()
    }

    fn disconnect(&self, name: &str) {
        let mut servers = self.lock();
        if let Some(entry) = servers.get_mut(name) {
            self.stop(name, entry);
            entry.settled.send_replace(true);
            self.set_status(name, entry, Status::Disabled);
        }
    }
}

/// One connection's answers to its server and its reactions to the server's notices.
struct ServerHandler {
    runtime: Weak<Runtime>,
    server: String,
    generation: u64,
}

impl Handler for ServerHandler {
    fn request(&self, method: &str, params: Value) -> BoxFuture<'static, Result<Value, Error>> {
        let runtime = self.runtime.upgrade();
        let server = self.server.clone();
        let method = method.to_string();
        async move {
            let Some(runtime) = runtime else {
                return Err(Error::Closed(None));
            };
            match method.as_str() {
                "roots/list" => {
                    let uri = url::Url::from_file_path(runtime.directory())
                        .map(|u| u.to_string())
                        .unwrap_or_default();
                    Ok(json!({"roots": [{"uri": uri}]}))
                }
                "ping" => Ok(json!({})),
                "elicitation/create" => {
                    let id = anysola_core::ids::form();
                    let key = params["elicitationId"]
                        .as_str()
                        .filter(|_| params["mode"] == "url")
                        .map(|e| format!("{server}\0{e}"));
                    if let Some(key) = &key {
                        runtime
                            .url_forms
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(key.clone(), id.clone());
                    }
                    let result = crate::elicit::ask(&runtime.core, &server, &params, id).await;
                    if let Some(key) = key {
                        runtime
                            .url_forms
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .remove(&key);
                    }
                    Ok(result)
                }
                _ => Err(Error::Rpc {
                    code: -32601,
                    message: "Method not found".into(),
                }),
            }
        }
        .boxed()
    }

    fn notification(&self, method: &str, params: Value) {
        let Some(runtime) = self.runtime.upgrade() else {
            return;
        };
        let (server, generation) = (self.server.clone(), self.generation);
        match method {
            "notifications/tools/list_changed" => {
                let first = runtime
                    .lock()
                    .get_mut(&server)
                    .is_some_and(|e| !std::mem::replace(&mut e.refreshing, true));
                if first {
                    tokio::spawn(async move { runtime.refresh_tools(&server, generation).await });
                }
            }
            "notifications/prompts/list_changed" => {
                tokio::spawn(async move { runtime.refresh_prompts(&server, generation).await });
            }
            "notifications/elicitation/complete" => {
                let key = format!(
                    "{server}\0{}",
                    params["elicitationId"].as_str().unwrap_or_default()
                );
                let form = runtime
                    .url_forms
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&key)
                    .cloned();
                if let Some(form) = form {
                    crate::elicit::complete(&runtime.core, &form);
                }
            }
            _ => {}
        }
    }

    fn closed(&self, reason: String) {
        if let Some(runtime) = self.runtime.upgrade() {
            runtime.closed(&self.server, self.generation, reason);
        }
    }
}
