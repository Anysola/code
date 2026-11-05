//! The `execute` tool: runs a program against one agent's Code Mode inventory and returns its
//! result as 0.5.1 formatted it, plus the direct form namespace tools take when `execute` is
//! denied.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;

use super::runtime::{self, Finished, HostError, Job, Local, Request};
use super::search::Index;
use super::{Inventory, Namespace, NamespaceTool, direct_name, sanitize};
use crate::embedded_def;
use crate::permission::{PermissionError, PermissionRequest};
use crate::tool::{
    DefinitionEnv, Tool, ToolContent, ToolContext, ToolDefinition, ToolError, ToolOutput,
    ToolResult, parse_input,
};

embedded_def!(execute_def, "execute");

const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub struct ExecuteTool {
    inventory: Inventory,
}

impl ExecuteTool {
    pub fn new(inventory: Inventory) -> Self {
        ExecuteTool { inventory }
    }
}

#[derive(Deserialize)]
struct Input {
    code: String,
}

impl Tool for ExecuteTool {
    fn name(&self) -> &str {
        "execute"
    }

    fn code_mode(&self) -> bool {
        true
    }

    fn definition(&self, _: &DefinitionEnv) -> ToolDefinition {
        let def = execute_def();
        ToolDefinition {
            name: "execute".into(),
            description: def.description.clone(),
            input_schema: def.input_schema.clone(),
        }
    }

    fn execute<'a>(&'a self, input: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolResult> {
        async move {
            let input: Input = parse_input("execute", &execute_def().input_schema, input)?;
            run(&self.inventory, input.code, ctx).await
        }
        .boxed()
    }
}

/// Rows the UI shows for the calls a program made, in start order.
#[derive(Default)]
struct Calls {
    rows: Mutex<Vec<Value>>,
    files: Mutex<Vec<(usize, Vec<ToolContent>)>>,
}

impl Calls {
    fn start(&self, tool: &str, input: Option<Value>) -> usize {
        let mut rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
        let mut row = json!({"tool": tool, "status": "running"});
        if let Some(input) = input {
            row["input"] = input;
        }
        rows.push(row);
        rows.len() - 1
    }

    fn settle(&self, index: usize, ok: bool) {
        let mut rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
        rows[index]["status"] = json!(if ok { "completed" } else { "error" });
    }
}

/// What the call row shows of a tool's input.
fn display_input(input: &Value) -> Option<Value> {
    match input {
        Value::Null => None,
        Value::Object(map) if map.is_empty() => None,
        Value::Object(_) => Some(input.clone()),
        other => Some(json!({"input": other})),
    }
}

pub(crate) async fn run(inventory: &Inventory, code: String, ctx: &ToolContext) -> ToolResult {
    let index = Arc::new(Index::new(inventory));
    let local = Local {
        keys: inventory
            .namespaces()
            .iter()
            .map(|n| {
                (
                    n.name.clone(),
                    n.tools.iter().map(|t| sanitize(t.name())).collect(),
                )
            })
            .collect(),
        index: index.clone(),
    };
    let stop = Arc::new(AtomicBool::new(false));
    let (jobs, mut requests) = mpsc::unbounded_channel::<Job>();
    let (finished, outcome) = oneshot::channel();
    let flag = stop.clone();
    std::thread::Builder::new()
        .name("code-mode".into())
        .stack_size(runtime::THREAD_STACK)
        .spawn(move || {
            let _ = finished.send(runtime::run(&code, local, jobs, flag));
        })
        .map_err(|e| ToolError::execution(e.to_string()))?;

    let calls = Arc::new(Calls::default());
    let mut running = JoinSet::new();
    let mut outcome = std::pin::pin!(outcome);
    let finished = loop {
        tokio::select! {
            result = &mut outcome => match result {
                Ok(finished) => break finished,
                Err(_) => {
                    running.abort_all();
                    return Err(ToolError::execution("Code Mode stopped unexpectedly."));
                }
            },
            Some(job) = requests.recv() => {
                let inventory = inventory.clone();
                let index = index.clone();
                let ctx = ctx.clone();
                let calls = calls.clone();
                running.spawn(async move {
                    let reply = match job.request {
                        Request::Call { path, args } => call(&inventory, &index, &ctx, &calls, path, args).await,
                        Request::Fetch(init) => fetch(&ctx, &calls, init).await,
                    };
                    let _ = job.reply.send(reply);
                });
            }
            _ = ctx.cancel.cancelled() => {
                stop.store(true, Ordering::Relaxed);
                break Finished::Interrupted;
            }
        }
    };
    // Calls the program left pending end with it.
    running.abort_all();
    let Finished::Done { result, logs } = finished else {
        stop.store(true, Ordering::Relaxed);
        return Err(ToolError::interrupted());
    };
    let failed = result.is_err();
    let mut text = match result {
        Ok(text) => text,
        Err(text) => text.trim().to_string(),
    };
    if !logs.is_empty() {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&format!("Logs:\n{}", logs.join("\n")));
    }
    let mut content = vec![ToolContent::text(text)];
    let mut files = std::mem::take(&mut *calls.files.lock().unwrap_or_else(|e| e.into_inner()));
    files.sort_by_key(|(index, _)| *index);
    content.extend(files.into_iter().flat_map(|(_, f)| f));
    let mut metadata = Map::new();
    let rows = std::mem::take(&mut *calls.rows.lock().unwrap_or_else(|e| e.into_inner()));
    metadata.insert("toolCalls".into(), Value::Array(rows));
    if failed {
        metadata.insert("error".into(), Value::Bool(true));
    }
    Ok(ToolOutput { content, metadata })
}

async fn call(
    inventory: &Inventory,
    index: &Index,
    ctx: &ToolContext,
    calls: &Calls,
    path: Vec<String>,
    args: Vec<Value>,
) -> Result<Value, HostError> {
    let segments: Vec<String> = path
        .iter()
        .flat_map(|s| s.split('.'))
        .map(str::to_string)
        .collect();
    let name = segments.join(".");
    let Some((namespace, tool)) = inventory.find(&name) else {
        // Models often write `tools.search(...)` for the bare `search(...)`.
        if name == "search" {
            return match args.as_slice() {
                [] => index.search(&json!({})),
                [input] => index.search(input),
                _ => Err("Tool 'search' expects at most one input object.".into()),
            }
            .map_err(HostError::plain);
        }
        let namespace_exists = inventory.namespaces().iter().any(|n| n.name == segments[0]);
        if segments.len() == 1 && namespace_exists {
            return Err(HostError::plain(format!("Tool '{name}' is not callable.")));
        }
        let message = match index.closest(&segments, namespace_exists) {
            Some(closest) => format!("Unknown tool '{name}'. Did you mean {closest}?"),
            None => format!("Unknown tool '{name}'."),
        };
        return Err(HostError {
            diagnostic: Some(format!("{message}\nUse search to find available tools.")),
            message,
            name: None,
        });
    };
    let input = match args.as_slice() {
        [] => json!({}),
        [input] => input.clone(),
        _ => {
            return Err(HostError::plain(format!(
                "Tool '{name}' expects at most one input object."
            )));
        }
    };
    let row = calls.start(&name, display_input(&input));
    let result = invoke(namespace, tool, input, ctx).await;
    calls.settle(row, result.is_ok());
    let called = result.map_err(|e| HostError::plain(e.message))?;
    let files: Vec<ToolContent> = called
        .content
        .iter()
        .filter(|c| matches!(c, ToolContent::File { uri, .. } if uri.starts_with("data:")))
        .cloned()
        .collect();
    if !files.is_empty() {
        calls
            .files
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((row, files));
    }
    Ok(called.value())
}

/// Validates the input against the tool's schema (named as its direct form) and runs it.
async fn invoke(
    namespace: &Namespace,
    tool: &Arc<dyn NamespaceTool>,
    input: Value,
    ctx: &ToolContext,
) -> Result<super::Called, ToolError> {
    let name = direct_name(&namespace.name, tool.name());
    let input: Value = parse_input(&name, tool.input_schema(), input)?;
    tool.call(input, ctx).await
}

#[derive(Deserialize)]
struct FetchInit {
    url: String,
    method: String,
    headers: Vec<(String, String)>,
    body: Option<Value>,
}

async fn fetch(ctx: &ToolContext, calls: &Calls, init: Value) -> Result<Value, HostError> {
    let init: FetchInit =
        serde_json::from_value(init).map_err(|e| HostError::typed("TypeError", e.to_string()))?;
    let row = calls.start(
        "fetch",
        Some(json!({"method": init.method, "url": init.url})),
    );
    let result = send(ctx, init).await;
    calls.settle(row, result.is_ok());
    result
}

async fn send(ctx: &ToolContext, init: FetchInit) -> Result<Value, HostError> {
    let url = url::Url::parse(&init.url)
        .map_err(|_| HostError::typed("TypeError", "fetch() URL is invalid"))?;
    if url.scheme() == "file" {
        return Err(HostError::typed(
            "Error",
            "fetch cannot be used for filesystem access.",
        ));
    }
    // The same rules as the webfetch tool, so a URL denied there is denied here.
    ctx.permissions
        .ask(PermissionRequest {
            action: "webfetch".into(),
            resources: vec![init.url.clone()],
            save: vec!["*".into()],
            metadata: json!({"method": init.method, "url": init.url}),
        })
        .await
        .map_err(|error| match error {
            PermissionError::Corrected(feedback) => HostError::typed(
                "Error",
                format!("The user rejected fetching {}: {feedback}", init.url),
            ),
            other => HostError::typed("Error", other.to_string()),
        })?;
    let method = reqwest::Method::from_bytes(init.method.as_bytes())
        .map_err(|_| HostError::typed("TypeError", "fetch() method is invalid"))?;
    let mut request = reqwest::Client::new()
        .request(method, url.clone())
        .timeout(FETCH_TIMEOUT)
        .header(reqwest::header::USER_AGENT, crate::version::user_agent());
    for (name, value) in &init.headers {
        request = request.header(name, value);
    }
    request = match init.body {
        Some(Value::String(text)) => request.body(text),
        Some(Value::Object(map)) => request.body(
            map.get("bytes")
                .and_then(Value::as_array)
                .map(|b| {
                    b.iter()
                        .filter_map(|v| v.as_u64().map(|n| n as u8))
                        .collect::<Vec<u8>>()
                })
                .unwrap_or_default(),
        ),
        _ => request,
    };
    let response = request.send().await.map_err(|e| {
        let message = if e.is_timeout() {
            "The operation timed out.".to_string()
        } else if e.is_connect() {
            "ECONNREFUSED: Unable to connect. Is the computer able to access the url?".to_string()
        } else {
            e.to_string()
        };
        HostError::typed("TypeError", message)
    })?;
    let status = response.status();
    let final_url = response.url().to_string();
    let headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect();
    let body = response
        .bytes()
        .await
        .map_err(|e| HostError::typed("TypeError", e.to_string()))?;
    Ok(json!({
        "url": final_url,
        "status": status.as_u16(),
        "statusText": status.canonical_reason().unwrap_or_default(),
        "ok": status.is_success(),
        "redirected": final_url != url.as_str(),
        "headers": headers,
        "body": body.to_vec(),
    }))
}

/// A namespace tool exposed as a direct tool `<namespace>_<tool>`, for agents that cannot run
/// `execute`.
pub struct DirectTool {
    name: String,
    namespace: Arc<Namespace>,
    tool: Arc<dyn NamespaceTool>,
}

impl DirectTool {
    pub fn new(namespace: Arc<Namespace>, tool: Arc<dyn NamespaceTool>) -> Self {
        DirectTool {
            name: direct_name(&namespace.name, tool.name()),
            namespace,
            tool,
        }
    }
}

impl Tool for DirectTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn permission(&self) -> &str {
        self.tool.permission().unwrap_or(&self.name)
    }

    fn definition(&self, _: &DefinitionEnv) -> ToolDefinition {
        ToolDefinition {
            name: self.name.clone(),
            description: self.tool.description().to_string(),
            input_schema: self.tool.input_schema().clone(),
        }
    }

    fn execute<'a>(&'a self, input: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolResult> {
        async move {
            let called = invoke(&self.namespace, &self.tool, input, ctx).await?;
            Ok(ToolOutput {
                content: called.content(),
                metadata: serde_json::Map::new(),
            })
        }
        .boxed()
    }
}
