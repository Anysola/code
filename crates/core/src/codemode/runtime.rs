//! One program run on QuickJS. The engine lives on the calling thread (it is not `Send`) with its
//! own single-threaded executor; tool calls and `fetch` go to the host as [`Job`]s and their
//! replies come back as JSON. Interrupting sets a flag the engine polls.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rquickjs::context::EvalOptions;
use rquickjs::function::Async;
use rquickjs::{
    AsyncContext, AsyncRuntime, CatchResultExt, CaughtError, Ctx, Exception, Function, Promise,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use super::search::Index;

const PRELUDE: &str = include_str!("prelude.js");
const MEMORY_LIMIT: usize = 512 << 20;
const STACK_LIMIT: usize = 4 << 20;
/// The engine thread's stack; the engine's own limit stays well below it.
pub const THREAD_STACK: usize = 16 << 20;
const PROGRAM: &str = "<input>";
const SYNTAX_SUFFIX: &str = "Code must be plain JavaScript written as a program body.";
const IMPORTS: &str = "SyntaxError: Imports and exports aren't available. Call tools through `tools` and use the built-in globals.";

/// A failure the program sees as a thrown error.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HostError {
    /// The error class; `Error` when absent.
    pub name: Option<String>,
    pub message: String,
    /// What the model sees if the program does not catch it; `Name: message` by default.
    pub diagnostic: Option<String>,
}

impl HostError {
    /// A plain `Error` whose text is the same inside and outside the program.
    pub fn plain(message: impl Into<String>) -> Self {
        let message = message.into();
        HostError {
            diagnostic: Some(message.clone()),
            message,
            name: None,
        }
    }

    pub fn typed(name: &str, message: impl Into<String>) -> Self {
        HostError {
            name: Some(name.into()),
            message: message.into(),
            diagnostic: None,
        }
    }

    fn to_json(&self) -> Value {
        let mut error = json!({"message": self.message});
        if let Some(name) = &self.name {
            error["name"] = json!(name);
        }
        if let Some(diagnostic) = &self.diagnostic {
            error["diagnostic"] = json!(diagnostic);
        }
        json!({ "error": error })
    }
}

pub enum Request {
    /// `tools.<path>(...args)`.
    Call { path: Vec<String>, args: Vec<Value> },
    /// `fetch(url, init)`, already normalized by the prelude.
    Fetch(Value),
}

pub struct Job {
    pub request: Request,
    pub reply: oneshot::Sender<Result<Value, HostError>>,
}

/// What the engine side needs to answer synchronously.
pub struct Local {
    /// Namespace names with their tool names, in registration order.
    pub keys: Vec<(String, Vec<String>)>,
    pub index: Arc<Index>,
}

#[derive(Debug, PartialEq)]
pub enum Finished {
    /// The formatted result or diagnostic, and the console lines.
    Done {
        result: Result<String, String>,
        logs: Vec<String>,
    },
    Interrupted,
}

#[derive(Deserialize)]
struct Report {
    ok: Option<String>,
    error: Option<String>,
    logs: Vec<String>,
}

/// Runs `code` to completion on this thread.
pub fn run(
    code: &str,
    local: Local,
    jobs: mpsc::UnboundedSender<Job>,
    stop: Arc<AtomicBool>,
) -> Finished {
    if code.trim().is_empty() {
        return done(Err("Code cannot be empty.".into()));
    }
    if let Some((line, column)) = module_syntax(code) {
        return done(Err(format!("{IMPORTS} (line {line}, col {column})")));
    }
    let Ok(executor) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return done(Err("Code Mode could not start.".into()));
    };
    executor.block_on(drive(code, local, jobs, stop))
}

fn done(result: Result<String, String>) -> Finished {
    Finished::Done {
        result,
        logs: Vec::new(),
    }
}

/// The first `import`/`export` statement, which a script cannot contain.
fn module_syntax(code: &str) -> Option<(usize, usize)> {
    code.lines().enumerate().find_map(|(index, line)| {
        let trimmed = line.trim_start();
        let keyword = ["import", "export"]
            .into_iter()
            .find(|k| trimmed.starts_with(k))?;
        let rest = &trimmed[keyword.len()..];
        let next = rest.chars().next();
        let statement = match keyword {
            "import" => matches!(next, Some(' ' | '{' | '*' | '"' | '\'')),
            _ => matches!(next, Some(' ' | '{' | '*')),
        };
        statement.then(|| (index + 1, line.len() - trimmed.len() + 1))
    })
}

async fn drive(
    code: &str,
    local: Local,
    jobs: mpsc::UnboundedSender<Job>,
    stop: Arc<AtomicBool>,
) -> Finished {
    let Ok(runtime) = AsyncRuntime::new() else {
        return done(Err("Code Mode could not start.".into()));
    };
    runtime.set_memory_limit(MEMORY_LIMIT).await;
    runtime.set_max_stack_size(STACK_LIMIT).await;
    let flag = stop.clone();
    runtime
        .set_interrupt_handler(Some(Box::new(move || flag.load(Ordering::Relaxed))))
        .await;
    let Ok(context) = AsyncContext::full(&runtime).await else {
        return done(Err("Code Mode could not start.".into()));
    };
    let local = Arc::new(local);
    let source = code.to_string();
    let outcome = context
        .async_with(async |ctx| execute(&ctx, &source, local, jobs).await)
        .await;
    match outcome {
        _ if stop.load(Ordering::Relaxed) => Finished::Interrupted,
        Ok(report) => Finished::Done {
            result: match (report.ok, report.error) {
                (Some(ok), _) => Ok(ok),
                (None, error) => Err(error.unwrap_or_default()),
            },
            logs: report.logs,
        },
        Err(message) => done(Err(message)),
    }
}

async fn execute<'js>(
    ctx: &Ctx<'js>,
    code: &str,
    local: Arc<Local>,
    jobs: mpsc::UnboundedSender<Job>,
) -> Result<Report, String> {
    install(ctx, local, jobs).map_err(|e| describe(ctx, e))?;
    let options = |promise: bool| {
        let mut options = EvalOptions::default();
        options.global = true;
        options.strict = true;
        options.promise = promise;
        options.filename = Some(PROGRAM.into());
        options
    };
    let (completion, wrapped, offset) = match ctx
        .eval_with_options::<Promise, _>(code, options(true))
        .catch(ctx)
    {
        Ok(promise) => (promise, true, 0),
        Err(CaughtError::Exception(e))
            if e.message().as_deref() == Some("return not in a function") =>
        {
            // A top-level `return`: run the program as an async function body instead.
            let body = format!("(async function () {{\n{code}\n}})()");
            match ctx
                .eval_with_options::<Promise, _>(body, options(false))
                .catch(ctx)
            {
                Ok(promise) => (promise, false, 1),
                Err(error) => return Err(compile_error(error, code, 1)),
            }
        }
        Err(error) => return Err(compile_error(error, code, 0)),
    };
    let finish: Function = ctx
        .globals()
        .get("__finish")
        .map_err(|e| describe(ctx, e))?;
    let report: Promise = finish
        .call((completion, wrapped, offset))
        .catch(ctx)
        .map_err(|e| caught(e))?;
    let text: String = report
        .into_future()
        .await
        .catch(ctx)
        .map_err(|e| caught(e))?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

fn install<'js>(
    ctx: &Ctx<'js>,
    local: Arc<Local>,
    jobs: mpsc::UnboundedSender<Job>,
) -> rquickjs::Result<()> {
    let globals = ctx.globals();
    let sync = Function::new(ctx.clone(), move |op: String, payload: String| {
        let reply = match serde_json::from_str::<Value>(&payload) {
            Ok(payload) => answer(&local, &op, payload),
            Err(e) => Err(HostError::plain(e.to_string())),
        };
        reply_text(reply)
    })?;
    globals.set("__sync", sync)?;
    let host = Function::new(
        ctx.clone(),
        Async(move |op: String, payload: String| {
            let jobs = jobs.clone();
            async move {
                let reply = match request(&op, &payload) {
                    Ok(request) => {
                        let (reply, receive) = oneshot::channel();
                        if jobs.send(Job { request, reply }).is_err() {
                            Err(HostError::plain("Code Mode execution ended."))
                        } else {
                            receive.await.unwrap_or_else(|_| {
                                Err(HostError::plain("Code Mode execution ended."))
                            })
                        }
                    }
                    Err(error) => Err(error),
                };
                Ok::<String, rquickjs::Error>(reply_text(reply))
            }
        }),
    )?;
    globals.set("__async", host)?;
    let mut options = EvalOptions::default();
    options.global = true;
    options.strict = true;
    options.filename = Some("<prelude>".into());
    ctx.eval_with_options::<(), _>(PRELUDE, options)
}

fn reply_text(reply: Result<Value, HostError>) -> String {
    let value = match reply {
        Ok(value) => json!({ "ok": value }),
        Err(error) => error.to_json(),
    };
    crate::json::compact(&value)
}

fn request(op: &str, payload: &str) -> Result<Request, HostError> {
    let mut value: Value =
        serde_json::from_str(payload).map_err(|e| HostError::plain(e.to_string()))?;
    match op {
        "call" => {
            let path = value["path"]
                .as_array()
                .map(|p| {
                    p.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let args = value["args"]
                .as_array_mut()
                .map(std::mem::take)
                .unwrap_or_default();
            Ok(Request::Call { path, args })
        }
        "fetch" => Ok(Request::Fetch(value)),
        _ => Err(HostError::plain(format!("Unknown host operation {op}"))),
    }
}

/// The synchronous host operations: namespace keys, `search`, URL parsing and randomness.
fn answer(local: &Local, op: &str, payload: Value) -> Result<Value, HostError> {
    match op {
        "keys" => {
            let segments: Vec<String> = payload
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .flat_map(|s| s.split('.'))
                .map(str::to_string)
                .collect();
            keys(local, &segments).map(|k| json!(k))
        }
        "search" => {
            let args = payload.as_array().cloned().unwrap_or_default();
            match args.len() {
                0 => local.index.search(&json!({})),
                1 => local.index.search(&args[0]),
                _ => Err("Tool 'search' expects at most one input object.".into()),
            }
            .map_err(HostError::plain)
        }
        "url" => parse_url(&payload),
        "uuid" => {
            let bytes: [u8; 16] = rand::random();
            let mut b = bytes;
            b[6] = (b[6] & 0x0f) | 0x40;
            b[8] = (b[8] & 0x3f) | 0x80;
            let hex = hex::encode(b);
            Ok(json!(format!(
                "{}-{}-{}-{}-{}",
                &hex[..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..]
            )))
        }
        "random" => {
            let count = payload.as_u64().unwrap_or(0).min(65_536) as usize;
            Ok(json!(
                (0..count).map(|_| rand::random::<u8>()).collect::<Vec<_>>()
            ))
        }
        _ => Err(HostError::plain(format!("Unknown host operation {op}"))),
    }
}

fn keys(local: &Local, segments: &[String]) -> Result<Vec<String>, HostError> {
    match segments {
        [] => Ok(local.keys.iter().map(|(n, _)| n.clone()).collect()),
        [namespace, rest @ ..] => {
            let found = local.keys.iter().find(|(n, _)| n == namespace);
            match (found, rest) {
                (Some((_, tools)), []) => Ok(tools.clone()),
                (Some((_, tools)), [tool]) if tools.contains(tool) => Ok(Vec::new()),
                _ => Err(HostError::plain(format!(
                    "Unknown tool namespace '{}'.",
                    segments.join(".")
                ))),
            }
        }
    }
}

fn parse_url(payload: &Value) -> Result<Value, HostError> {
    let input = payload[0].as_str().unwrap_or_default();
    let parsed = match payload[1].as_str() {
        Some(base) => url::Url::parse(base).and_then(|b| b.join(input)),
        None => url::Url::parse(input),
    }
    .map_err(|_| HostError::typed("TypeError", "Invalid URL"))?;
    let prefixed = |prefix: &str, value: Option<&str>| {
        value
            .filter(|v| !v.is_empty())
            .map(|v| format!("{prefix}{v}"))
            .unwrap_or_default()
    };
    let hostname = parsed.host_str().unwrap_or_default().to_string();
    let port = parsed.port().map(|p| p.to_string()).unwrap_or_default();
    let host = if port.is_empty() {
        hostname.clone()
    } else {
        format!("{hostname}:{port}")
    };
    Ok(json!({
        "href": parsed.as_str(),
        "origin": parsed.origin().ascii_serialization(),
        "protocol": format!("{}:", parsed.scheme()),
        "username": parsed.username(),
        "password": parsed.password().unwrap_or_default(),
        "host": host,
        "hostname": hostname,
        "port": port,
        "pathname": parsed.path(),
        "search": prefixed("?", parsed.query()),
        "hash": prefixed("#", parsed.fragment()),
    }))
}

fn caught(error: CaughtError<'_>) -> String {
    match error {
        CaughtError::Exception(e) => exception_text(&e),
        CaughtError::Value(v) => format!("Uncaught: {v:?}"),
        CaughtError::Error(e) => e.to_string(),
    }
}

fn describe(ctx: &Ctx<'_>, error: rquickjs::Error) -> String {
    caught(CaughtError::from_error(ctx, error))
}

fn exception_text(e: &Exception<'_>) -> String {
    let name: String = e.get("name").unwrap_or_else(|_| "Error".into());
    let message = e.message().unwrap_or_default();
    if message == "out of memory" {
        return "Execution exceeded the memory limit.".into();
    }
    format!("{name}: {message}")
}

/// A compile error in 0.5.1's shape: `SyntaxError: <Reason>. Code must be ... (line L, col C)`,
/// located in the submitted source.
fn compile_error(error: CaughtError<'_>, code: &str, offset: usize) -> String {
    let CaughtError::Exception(e) = error else {
        return caught(error);
    };
    let name: String = e.get("name").unwrap_or_default();
    let message = e.message().unwrap_or_default();
    if name != "SyntaxError" {
        return exception_text(&e);
    }
    // QuickJS names the token it stopped at; acorn's wording does not.
    let mut reason = if message.starts_with("unexpected token") {
        "Unexpected token".to_string()
    } else {
        message
    };
    if let Some(first) = reason.get(..1) {
        reason = format!("{}{}", first.to_uppercase(), &reason[1..]);
    }
    if !reason.ends_with('.') {
        reason.push('.');
    }
    let location = e
        .stack()
        .and_then(|stack| {
            let at = stack.find(&format!("{PROGRAM}:"))? + PROGRAM.len() + 1;
            let mut parts = stack[at..].split(|c: char| !c.is_ascii_digit());
            let line: usize = parts.next()?.parse().ok()?;
            let column: usize = parts.next()?.parse().ok()?;
            Some((line.saturating_sub(offset).max(1), column))
        })
        .map(|(line, column)| {
            let lines: Vec<&str> = code.lines().collect();
            if line > lines.len() {
                let last = lines.last().copied().unwrap_or_default();
                (lines.len().max(1), last.chars().count() + 1)
            } else {
                (line, column)
            }
        })
        .map(|(line, column)| format!(" (line {line}, col {column})"))
        .unwrap_or_default();
    format!("SyntaxError: {reason} {SYNTAX_SUFFIX}{location}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_statements_are_found() {
        assert_eq!(module_syntax("import fs from 'fs'"), Some((1, 1)));
        assert_eq!(module_syntax("const a = 1\n  export { a }"), Some((2, 3)));
        assert_eq!(module_syntax("await import('x')"), None);
        assert_eq!(module_syntax("const important = 1"), None);
    }
}
