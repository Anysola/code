//! The ACP agent: sessions mapped onto the core, one core and MCP runtime per directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anysola_core::api::{Core, Target};
use anysola_core::command::Template;
use anysola_core::events::Event;
use anysola_core::model::{Model, ModelRef, PROVIDER};
use anysola_core::storage::{Db, SessionRow};
use anysola_mcp::Runtime;
use serde_json::{Value, json};
use tokio::task::AbortHandle;

use crate::rpc::{Connection, Failure};

pub const AUTH_METHOD: &str = "opencode-login";
const DEFAULT_EFFORT: &str = "default";
const PAGE: u64 = 100;

/// What the client said it supports.
#[derive(Debug, Clone, Copy, Default)]
pub struct Capabilities {
    pub form_elicitation: bool,
}

#[derive(Clone)]
pub struct Location {
    pub core: Core,
    pub mcp: Arc<Runtime>,
}

/// A session the client opened in this connection.
pub struct Attached {
    pub location: Location,
    /// Follows the session's commands once the client knows the session.
    watcher: Option<AbortHandle>,
    pub prompting: bool,
}

pub struct Agent {
    pub connection: Arc<Connection>,
    pub db: Arc<Db>,
    pub capabilities: Mutex<Capabilities>,
    locations: tokio::sync::Mutex<HashMap<PathBuf, Location>>,
    pub sessions: Mutex<HashMap<String, Attached>>,
}

fn string(params: &Value, key: &str) -> Option<String> {
    params.get(key).and_then(Value::as_str).map(str::to_string)
}

pub(crate) fn not_found(id: &str) -> Failure {
    Failure::invalid_params(format!("session not found: {id}"), json!({"sessionId": id}))
}

/// `cwd` must be absolute; the extra directories absolute and free of glob characters.
fn directories(params: &Value) -> Result<(String, Vec<String>), Failure> {
    let cwd = string(params, "cwd").unwrap_or_default();
    if !Path::new(&cwd).is_absolute() {
        return Err(Failure::invalid_field(
            format!("cwd must be an absolute path: {cwd}"),
            "cwd",
        ));
    }
    let mut extra: Vec<String> = Vec::new();
    for directory in params["additionalDirectories"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if !Path::new(directory).is_absolute() || directory.contains(['*', '?']) {
            return Err(Failure::invalid_params(
                format!(
                    "additional directory must be an absolute path without glob characters: {directory}"
                ),
                json!({"additionalDirectory": directory}),
            ));
        }
        if !same_directory(directory, &cwd) && !extra.iter().any(|d| d == directory) {
            extra.push(directory.to_string());
        }
    }
    Ok((cwd, extra))
}

fn same_directory(a: &str, b: &str) -> bool {
    let normal = |p: &str| Path::new(p).components().collect::<PathBuf>();
    normal(a) == normal(b)
}

/// Editor MCP servers as config entries: stdio → local, `http` → remote without OAuth.
fn mcp_servers(params: &Value) -> Result<Vec<(String, Value)>, Failure> {
    let pairs = |list: &Value| -> serde_json::Map<String, Value> {
        list.as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| Some((e["name"].as_str()?.to_string(), e["value"].clone())))
            .collect()
    };
    params["mcpServers"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|server| {
            let name = server["name"].as_str().unwrap_or_default().to_string();
            match server.get("type").and_then(Value::as_str) {
                None => {
                    let mut command = vec![server["command"].clone()];
                    command.extend(server["args"].as_array().cloned().unwrap_or_default());
                    let config = json!({"type": "local", "command": command,
                        "environment": pairs(&server["env"])});
                    Ok((name, config))
                }
                Some("http") => {
                    let config = json!({"type": "remote", "url": server["url"],
                        "headers": pairs(&server["headers"]), "oauth": false});
                    Ok((name, config))
                }
                Some(_) => Err(Failure::invalid_field(
                    "Only stdio and HTTP MCP servers are supported",
                    "mcpServers",
                )),
            }
        })
        .collect()
}

fn title_case(variant: &str) -> String {
    variant
        .split(['_', '-'])
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The session's model as the client sees it: its own, else the configured default, else
/// the first model.
pub(crate) fn current_model(core: &Core, session: &SessionRow) -> Option<(ModelRef, Model)> {
    let loaded = core.loaded();
    let reference = session
        .model
        .as_ref()
        .and_then(ModelRef::from_json)
        .or_else(|| loaded.config.model().and_then(ModelRef::parse))
        .or_else(|| {
            loaded.models.first().map(|m| ModelRef {
                provider: PROVIDER.into(),
                id: m.id.clone(),
                variant: None,
            })
        })?;
    let model = loaded.models.iter().find(|m| m.id == reference.id)?.clone();
    Some((reference, model))
}

fn current_mode(core: &Core, session: &SessionRow) -> String {
    session
        .agent
        .clone()
        .or_else(|| core.loaded().agents.default_agent().map(|a| a.id.clone()))
        .unwrap_or_default()
}

pub fn config_options(core: &Core, session: &SessionRow) -> Value {
    let loaded = core.loaded();
    let current = current_model(core, session);
    let mut models: Vec<&Model> = loaded.models.iter().collect();
    models.sort_by(|a, b| a.name.cmp(&b.name));
    let mut options = vec![json!({
        "id": "model",
        "name": "Model",
        "category": "model",
        "type": "select",
        "currentValue": current.as_ref().map(|(r, _)| format!("{PROVIDER}/{}", r.id)),
        "options": models.iter().map(|m| json!({
            "value": format!("{PROVIDER}/{}", m.id),
            "name": format!("{PROVIDER}/{}", m.name),
        })).collect::<Vec<_>>(),
    })];
    if let Some((reference, model)) = &current
        && !model.variants.is_empty()
    {
        let mut values: Vec<&str> = model.variants.iter().map(|v| v.id.as_str()).collect();
        if !values.contains(&DEFAULT_EFFORT) {
            values.push(DEFAULT_EFFORT);
        }
        let selected = reference
            .variant
            .as_deref()
            .filter(|v| values.contains(v))
            .unwrap_or(DEFAULT_EFFORT);
        options.push(json!({
            "id": "effort",
            "name": "Effort",
            "description": "Available effort levels for this model",
            "category": "thought_level",
            "type": "select",
            "currentValue": selected,
            "options": values.iter().map(|v| json!({"value": v, "name": title_case(v)})).collect::<Vec<_>>(),
        }));
    }
    let modes: Vec<Value> = loaded
        .agents
        .list
        .iter()
        .filter(|a| a.selectable())
        .map(|a| {
            let mut mode = json!({"value": a.id, "name": a.name});
            if let Some(description) = &a.description {
                mode["description"] = json!(description);
            }
            mode
        })
        .collect();
    options.push(json!({
        "id": "mode",
        "name": "Session Mode",
        "category": "mode",
        "type": "select",
        "currentValue": current_mode(core, session),
        "options": modes,
    }));
    Value::Array(options)
}

/// Config commands and MCP prompts, then `goal` and the built-in `compact`.
pub fn available_commands(core: &Core) -> Value {
    let commands = core.commands();
    let (goal, rest): (Vec<_>, Vec<_>) = commands
        .list()
        .iter()
        .partition(|c| matches!(c.template, Template::Goal));
    let mut list: Vec<Value> = rest
        .into_iter()
        .chain(goal)
        .map(|c| json!({"name": c.name, "description": c.description.clone().unwrap_or_default()}))
        .collect();
    list.push(json!({"name": "compact", "description": "Compact the session"}));
    Value::Array(list)
}

impl Agent {
    pub fn new(connection: Arc<Connection>, db: Arc<Db>) -> Agent {
        Agent {
            connection,
            db,
            capabilities: Mutex::default(),
            locations: Default::default(),
            sessions: Mutex::default(),
        }
    }

    pub fn sessions(&self) -> std::sync::MutexGuard<'_, HashMap<String, Attached>> {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn initialize(&self, params: &Value) -> Value {
        let client = &params["clientCapabilities"];
        *self.capabilities.lock().unwrap_or_else(|e| e.into_inner()) = Capabilities {
            form_elicitation: !client["elicitation"]["form"].is_null(),
        };
        let mut method = json!({
            "description": "Run `anysola login` in the terminal",
            "name": "Login with Anysola",
            "id": AUTH_METHOD,
        });
        if client["auth"]["terminal"] == json!(true) {
            method["type"] = json!("terminal");
            method["args"] = json!(["--login"]);
        }
        if client["_meta"]["terminal-auth"] == json!(true) {
            method["_meta"] = json!({"terminal-auth": {"command": "anysola", "args": ["login"], "label": "Anysola Login"}});
        }
        json!({
            "protocolVersion": 1,
            "agentCapabilities": {
                "loadSession": true,
                "mcpCapabilities": {"http": true, "sse": false},
                "promptCapabilities": {"embeddedContext": true, "image": true},
                "sessionCapabilities": {"additionalDirectories": {}, "close": {}, "delete": {}, "fork": {}, "list": {}, "resume": {}},
                "_meta": {"opencode/child-session-updates": true},
            },
            "authMethods": [method],
            "agentInfo": {"name": "Anysola Code", "version": anysola_core::version::VERSION},
        })
    }

    pub fn authenticate(&self, params: &Value) -> Result<Value, Failure> {
        let method = string(params, "methodId").unwrap_or_default();
        if method == AUTH_METHOD {
            return Ok(json!({}));
        }
        Err(Failure::invalid_params(
            format!("unknown auth method: {method}"),
            json!({"methodId": method}),
        ))
    }

    /// The directory's core and MCP runtime, started on first use and kept for the process.
    async fn location(&self, cwd: &str) -> Result<Location, Failure> {
        let mut locations = self.locations.lock().await;
        if let Some(location) = locations.get(Path::new(cwd)) {
            return Ok(location.clone());
        }
        let core = Core::start(Path::new(cwd))
            .await
            .map_err(|e| Failure::internal(e, json!({"service": "session"})))?;
        let mcp = Runtime::attach(&core);
        let location = Location { core, mcp };
        locations.insert(PathBuf::from(cwd), location.clone());
        Ok(location)
    }

    /// The stored session, which must live in `cwd`.
    fn stored(&self, id: &str, cwd: &str) -> Result<SessionRow, Failure> {
        if !id.starts_with("ses_") {
            return Err(Failure::invalid_field("Invalid session ID", "sessionID"));
        }
        let session = self
            .db
            .session(id)
            .map_err(|e| Failure::internal(e, json!({"service": "session"})))?
            .ok_or_else(|| not_found(id))?;
        if !same_directory(&session.directory, cwd) {
            return Err(Failure::invalid_params(
                format!("session {id} does not belong to cwd: {cwd}"),
                json!({"sessionId": id, "cwd": cwd}),
            ));
        }
        Ok(session)
    }

    /// Grants the directories, adds the editor's MCP servers and starts following the session.
    fn attach(
        self: &Arc<Self>,
        session: &SessionRow,
        location: Location,
        extra: &[String],
        servers: Vec<(String, Value)>,
    ) -> Result<Value, Failure> {
        location
            .core
            .grant_directories(&session.id, extra)
            .map_err(|e| Failure::internal(e, json!({"service": "session"})))?;
        for (name, server) in servers {
            location.mcp.add(&name, server);
        }
        let options = config_options(&location.core, session);
        let attached = Attached {
            location,
            watcher: None,
            prompting: false,
        };
        if let Some(previous) = self.sessions().insert(session.id.clone(), attached)
            && let Some(watcher) = previous.watcher
        {
            watcher.abort();
        }
        Ok(options)
    }

    /// Sends the session's commands and keeps them current.
    pub fn announce(self: &Arc<Self>, id: &str) {
        let mut sessions = self.sessions();
        let Some(attached) = sessions.get_mut(id) else {
            return;
        };
        let watcher = tokio::spawn(follow(
            Arc::downgrade(self),
            id.to_string(),
            attached.location.clone(),
        ));
        if let Some(previous) = attached.watcher.replace(watcher.abort_handle()) {
            previous.abort();
        }
    }

    pub async fn new_session(self: &Arc<Self>, params: &Value) -> Result<Value, Failure> {
        let (cwd, extra) = directories(params)?;
        let servers = mcp_servers(params)?;
        let location = self.location(&cwd).await?;
        let (session, _) = location
            .core
            .open(&Target::New, false)
            .map_err(|e| Failure::internal(e, json!({"service": "session"})))?;
        let options = self.attach(&session, location, &extra, servers)?;
        Ok(json!({"sessionId": session.id, "configOptions": options}))
    }

    /// `session/load` (replaying the history first) and `session/resume`.
    pub async fn load_session(
        self: &Arc<Self>,
        params: &Value,
        replay: bool,
    ) -> Result<Value, Failure> {
        let (cwd, extra) = directories(params)?;
        let servers = mcp_servers(params)?;
        let id = string(params, "sessionId").unwrap_or_default();
        let session = self.stored(&id, &cwd)?;
        let location = self.location(&session.directory).await?;
        if replay {
            crate::turn::replay(&self.connection, &location.core, &session);
        }
        let options = self.attach(&session, location, &extra, servers)?;
        Ok(json!({"configOptions": options}))
    }

    pub async fn fork_session(self: &Arc<Self>, params: &Value) -> Result<Value, Failure> {
        let (cwd, extra) = directories(params)?;
        let servers = mcp_servers(params)?;
        let id = string(params, "sessionId").unwrap_or_default();
        let source = self.stored(&id, &cwd)?;
        let location = self.location(&source.directory).await?;
        let (session, _) = location
            .core
            .open(&Target::Id(source.id), true)
            .map_err(|e| Failure::internal(e, json!({"service": "session"})))?;
        let options = self.attach(&session, location, &extra, servers)?;
        Ok(json!({"sessionId": session.id, "configOptions": options}))
    }

    pub fn list_sessions(&self, params: &Value) -> Result<Value, Failure> {
        let cwd = string(params, "cwd");
        if let Some(cwd) = &cwd
            && !Path::new(cwd).is_absolute()
        {
            return Err(Failure::invalid_field(
                format!("cwd must be an absolute path: {cwd}"),
                "cwd",
            ));
        }
        let cursor = string(params, "cursor");
        let after = cursor.as_deref().and_then(|c| {
            let (time, id) = c.split_once(':')?;
            Some((time.parse().ok()?, id))
        });
        let rows = self
            .db
            .list_sessions(cwd.as_deref(), after, PAGE)
            .map_err(|e| Failure::internal(e, json!({"service": "session"})))?;
        let next = (rows.len() as u64 == PAGE)
            .then(|| rows.last().map(|r| format!("{}:{}", r.time_updated, r.id)))
            .flatten();
        let sessions: Vec<Value> = rows
            .iter()
            .map(|row| {
                let mut entry = json!({"sessionId": row.id, "cwd": row.directory});
                let granted = self
                    .sessions()
                    .get(&row.id)
                    .map(|a| a.location.core.granted_directories(&row.id))
                    .transpose()
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                if !granted.is_empty() {
                    entry["additionalDirectories"] = json!(granted);
                }
                entry["title"] = json!(row.title);
                entry["updatedAt"] = json!(anysola_core::time::iso(row.time_updated));
                entry
            })
            .collect();
        let mut result = json!({"sessions": sessions});
        if let Some(next) = next {
            result["nextCursor"] = json!(next);
        }
        Ok(result)
    }

    /// Stops following the session (and its running turn).
    pub fn close(&self, id: &str) {
        if let Some(attached) = self.sessions().remove(id) {
            if let Some(watcher) = attached.watcher {
                watcher.abort();
            }
            attached.location.core.interrupt(id);
        }
    }

    pub fn delete_session(&self, params: &Value) -> Result<Value, Failure> {
        let id = string(params, "sessionId").unwrap_or_default();
        self.close(&id);
        match self.db.delete_session(&id) {
            Ok(true) => Ok(json!({})),
            Ok(false) => Err(not_found(&id)),
            Err(e) => Err(Failure::internal(e, json!({"service": "session"}))),
        }
    }

    pub fn require(&self, id: &str) -> Result<(Location, SessionRow), Failure> {
        let location = self
            .sessions()
            .get(id)
            .map(|a| a.location.clone())
            .ok_or_else(|| not_found(id))?;
        let session = self
            .db
            .session(id)
            .ok()
            .flatten()
            .ok_or_else(|| not_found(id))?;
        Ok((location, session))
    }

    pub fn set_config_option(&self, params: &Value) -> Result<Value, Failure> {
        let id = string(params, "sessionId").unwrap_or_default();
        let (location, session) = self.require(&id)?;
        let config = string(params, "configId").unwrap_or_default();
        let Some(value) = params["value"].as_str() else {
            return Err(unknown_option(&config));
        };
        select(&location.core, &session, &config, value)?;
        let session = self.require(&id)?.1;
        Ok(json!({"configOptions": config_options(&location.core, &session)}))
    }

    pub fn set_mode(&self, params: &Value) -> Result<Value, Failure> {
        let id = string(params, "sessionId").unwrap_or_default();
        let (location, session) = self.require(&id)?;
        let mode = string(params, "modeId").unwrap_or_default();
        select(&location.core, &session, "mode", &mode)?;
        Ok(json!({}))
    }
}

fn unknown_option(config: &str) -> Failure {
    Failure::invalid_params(
        format!("unknown config option: {config}"),
        json!({"configId": config}),
    )
}

fn effort_not_found(effort: &str) -> Failure {
    Failure::invalid_params(
        format!("effort not found: {effort}"),
        json!({"effort": effort}),
    )
}

fn select(core: &Core, session: &SessionRow, config: &str, value: &str) -> Result<(), Failure> {
    let internal =
        |e: anysola_core::api::CoreError| Failure::internal(e, json!({"service": "session"}));
    let loaded = core.loaded();
    let current = current_model(core, session);
    match config {
        "model" => {
            let find = |advertised: &str| {
                let id = advertised.strip_prefix(PROVIDER)?.strip_prefix('/')?;
                loaded.models.iter().find(|m| m.id == id)
            };
            let (model, variant) = match find(value) {
                Some(model) => (model, None),
                None => {
                    let (base, variant) = value.rsplit_once('/').unwrap_or((value, ""));
                    match find(base) {
                        Some(model) if model.variant(variant).is_some() => {
                            (model, Some(variant.to_string()))
                        }
                        Some(_) => return Err(effort_not_found(variant)),
                        None => {
                            let provider = value.split('/').next().unwrap_or(value);
                            return Err(Failure::invalid_params(
                                format!("model not found: {value}"),
                                json!({"providerId": provider, "modelId": value}),
                            ));
                        }
                    }
                }
            };
            // The effort carries over to the same model.
            let variant = variant.or_else(|| {
                current
                    .as_ref()
                    .filter(|(r, _)| r.id == model.id)
                    .and_then(|(r, _)| r.variant.clone())
                    .filter(|v| model.variant(v).is_some())
            });
            let reference = ModelRef {
                provider: PROVIDER.into(),
                id: model.id.clone(),
                variant,
            };
            core.select_model(&session.id, &reference).map_err(internal)
        }
        "effort" => {
            let Some((reference, model)) = current else {
                return Err(effort_not_found(value));
            };
            if value != DEFAULT_EFFORT && model.variant(value).is_none() {
                return Err(effort_not_found(value));
            }
            let reference = ModelRef {
                variant: (value != DEFAULT_EFFORT).then(|| value.to_string()),
                ..reference
            };
            core.select_model(&session.id, &reference).map_err(internal)
        }
        "mode" => {
            let found = loaded
                .agents
                .get(value)
                .filter(|a| a.selectable())
                .is_some();
            if !found {
                return Err(Failure::invalid_params(
                    format!("mode not found: {value}"),
                    json!({"mode": value}),
                ));
            }
            core.select_agent(&session.id, value).map_err(internal)
        }
        _ => Err(unknown_option(config)),
    }
}

/// Pushes the session's commands now and whenever they (or its config options) change: MCP
/// servers connecting, config reloads seen at the end of a turn.
async fn follow(agent: std::sync::Weak<Agent>, session_id: String, location: Location) {
    let mut events = location.core.subscribe();
    let mut commands = Value::Null;
    let mut options = Value::Null;
    loop {
        let Some(agent) = agent.upgrade() else {
            return;
        };
        let next = available_commands(&location.core);
        if next != commands {
            agent.connection.update(
                &session_id,
                json!({"sessionUpdate": "available_commands_update", "availableCommands": next}),
            );
            commands = next;
        }
        if let Ok(Some(session)) = agent.db.session(&session_id) {
            let next = config_options(&location.core, &session);
            if !options.is_null() && next != options {
                agent.connection.update(
                    &session_id,
                    json!({"sessionUpdate": "config_option_update", "configOptions": next}),
                );
            }
            options = next;
        }
        drop(agent);
        loop {
            match events.recv().await {
                Ok(Event::McpStatusChanged { .. }) => break,
                Ok(Event::Succeeded { session_id: s } | Event::Failed { session_id: s, .. })
                    if s == session_id =>
                {
                    break;
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    }
}
