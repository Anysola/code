//! The `webfetch` tool: an http(s) GET with manual redirects (each target re-checked against deny
//! rules), a 5 MiB body cap and a timeout, returning the body as text, markdown or raw HTML.

mod entities;
mod html;
mod js;
pub mod markdown;
#[cfg(test)]
mod tests;

use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;

use super::{
    DefinitionEnv, Tool, ToolContext, ToolDefinition, ToolError, ToolOutput, ToolResult,
    parse_input,
};
use crate::embedded_def;
use crate::permission::{PermissionError, PermissionRequest};

embedded_def!(def, "webfetch");

const MAX_RESPONSE_BYTES: usize = markdown::MAX_MARKDOWN_BYTES;
const DEFAULT_TIMEOUT_SECONDS: f64 = 30.0;
const MAX_REDIRECTS: usize = 10;
const USER_AGENT: &str = "Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; AnysolaCode-User/1.0; +https://www.anysola.com/code";
/// Retried once with this agent when Cloudflare answers with a challenge.
const CHALLENGE_USER_AGENT: &str = "opencode";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
enum Format {
    Text,
    #[default]
    Markdown,
    Html,
}

impl Format {
    fn accept(self) -> &'static str {
        match self {
            Format::Markdown => {
                "text/markdown;q=1.0, text/x-markdown;q=0.9, text/plain;q=0.8, text/html;q=0.7, */*;q=0.1"
            }
            Format::Text => "text/plain;q=1.0, text/markdown;q=0.9, text/html;q=0.8, */*;q=0.1",
            Format::Html => {
                "text/html;q=1.0, application/xhtml+xml;q=0.9, text/plain;q=0.8, text/markdown;q=0.7, */*;q=0.1"
            }
        }
    }
}

#[derive(Deserialize)]
struct Input {
    url: String,
    #[serde(default)]
    format: Format,
    timeout: Option<f64>,
}

enum FetchError {
    Failed(String),
    /// A redirect target is denied by a rule.
    Denied,
}

fn failed(message: impl Into<String>) -> FetchError {
    FetchError::Failed(message.into())
}

/// Parses (relative to `base` for redirects) and accepts only http(s).
fn parse_url(text: &str, base: Option<&Url>) -> Result<Url, String> {
    let parsed = match base {
        Some(base) => base.join(text),
        None => Url::parse(text),
    };
    let url = parsed.map_err(|_| "Invalid URL".to_string())?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        _ => Err("URL must use http:// or https://".into()),
    }
}

/// `text/*`, JSON, XML and JavaScript types, and an empty type.
fn textual(mime: &str) -> bool {
    mime.is_empty()
        || mime.starts_with("text/")
        || mime == "application/json"
        || mime.ends_with("+json")
        || mime == "application/xml"
        || mime.ends_with("+xml")
        || mime == "application/javascript"
        || mime == "application/x-javascript"
}

struct Fetched {
    content_type: String,
    body: Vec<u8>,
}

async fn request(
    http: &reqwest::Client,
    ctx: &ToolContext,
    start: &Url,
    format: Format,
    agent: &str,
) -> Result<(reqwest::Response, Url), FetchError> {
    let mut url = start.clone();
    let mut redirects = 0;
    loop {
        let response = http
            .get(url.clone())
            .header("user-agent", agent)
            .header("accept", format.accept())
            .header("accept-language", "en-US,en;q=0.9")
            .send()
            .await
            .map_err(|_| failed(format!("Transport error (GET {url})")))?;
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        match location {
            Some(location) if matches!(status, 301 | 302 | 303 | 307 | 308) => {
                if redirects >= MAX_REDIRECTS {
                    return Err(failed(format!(
                        "Too many redirects (more than {MAX_REDIRECTS})"
                    )));
                }
                let next = parse_url(&location, Some(&url)).map_err(FetchError::Failed)?;
                if ctx
                    .permissions
                    .denies("webfetch", &[next.as_str().to_string()])
                {
                    return Err(FetchError::Denied);
                }
                url = next;
                redirects += 1;
            }
            _ => return Ok((response, url)),
        }
    }
}

async fn collect(mut response: reqwest::Response, url: &Url) -> Result<Vec<u8>, FetchError> {
    let too_large = || {
        failed(format!(
            "Response too large (exceeds {MAX_RESPONSE_BYTES} byte limit)"
        ))
    };
    if response
        .content_length()
        .is_some_and(|n| n as usize > MAX_RESPONSE_BYTES)
    {
        return Err(too_large());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| failed(format!("Transport error (GET {url})")))?
    {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn fetch(ctx: &ToolContext, url: &Url, format: Format) -> Result<Fetched, FetchError> {
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| failed(e.to_string()))?;
    let mut answer = request(&http, ctx, url, format, USER_AGENT).await?;
    let challenge = |response: &reqwest::Response| {
        response.status() == 403
            && response
                .headers()
                .get("cf-mitigated")
                .is_some_and(|v| v == "challenge")
    };
    if challenge(&answer.0) {
        answer = request(&http, ctx, url, format, CHALLENGE_USER_AGENT).await?;
    }
    let (response, last) = answer;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(failed(format!(
            "StatusCode: non 2xx status code ({status} GET {last})"
        )));
    }
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if mime.starts_with("image/") && mime != "image/svg+xml" && mime != "image/vnd.fastbidsheet" {
        return Err(failed(format!(
            "Unsupported fetched image content type: {mime}"
        )));
    }
    if !textual(&mime) {
        return Err(failed(format!(
            "Unsupported fetched file content type: {mime}"
        )));
    }
    Ok(Fetched {
        body: collect(response, &last).await?,
        content_type,
    })
}

fn convert(content: String, content_type: &str, format: Format) -> String {
    if !content_type.contains("text/html") {
        return content;
    }
    match format {
        Format::Markdown => markdown::to_markdown(&content),
        Format::Text => markdown::to_text(&content),
        Format::Html => content,
    }
}

pub struct WebfetchTool;

impl Tool for WebfetchTool {
    fn name(&self) -> &'static str {
        "webfetch"
    }

    fn definition(&self, _: &DefinitionEnv) -> ToolDefinition {
        let def = def();
        ToolDefinition {
            name: self.name().into(),
            description: def.description.clone(),
            input_schema: def.input_schema.clone(),
        }
    }

    fn execute<'a>(&'a self, input: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolResult> {
        async move {
            let raw = input.clone();
            let input: Input = parse_input(self.name(), &def().input_schema, input)?;
            let url = parse_url(&input.url, None).map_err(ToolError::unknown)?;
            ctx.ask(PermissionRequest {
                action: self.name().into(),
                resources: vec![input.url.clone()],
                save: vec!["*".into()],
                metadata: raw,
            })
            .await?;
            let timeout = Duration::from_secs_f64(input.timeout.unwrap_or(DEFAULT_TIMEOUT_SECONDS));
            let fetched = match tokio::time::timeout(timeout, fetch(ctx, &url, input.format)).await
            {
                Err(_) => return Err(ToolError::unknown("Request timed out")),
                Ok(Err(FetchError::Denied)) => {
                    return Err(PermissionError::Blocked {
                        action: self.name().into(),
                        reason: None,
                    }
                    .into());
                }
                Ok(Err(FetchError::Failed(message))) => return Err(ToolError::unknown(message)),
                Ok(Ok(fetched)) => fetched,
            };
            let text = String::from_utf8_lossy(&fetched.body);
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text).to_string();
            let output = convert(text, &fetched.content_type, input.format);
            Ok(ToolOutput::text(output).with_metadata("contentType", fetched.content_type))
        }
        .boxed()
    }
}
