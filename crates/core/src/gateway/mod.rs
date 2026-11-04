//! Client for the Anysola gateway (`<api>/v1/code/*`): remote config, version check, web search and
//! streamed model calls, with the exact header sets 0.5.1 sent.

pub mod error;
pub mod retry;
pub mod sse;

use std::time::Duration;

use reqwest::header::HeaderMap;
use serde::Deserialize;
use serde_json::Value;

use crate::tool::websearch::SearchResult;
use crate::version::{VERSION, model_user_agent, user_agent};
use error::ApiError;
use sse::{Block, Decoder, SseEvent};

const ACCOUNT_TIMEOUT: Duration = Duration::from_secs(15);
const SEARCH_TIMEOUT: Duration = Duration::from_secs(30);
const SEARCH_MAX_BYTES: usize = 1024 * 1024;

/// `ANYSOLA_API_URL` without trailing slashes, else the production gateway.
pub fn api_url() -> String {
    match std::env::var("ANYSOLA_API_URL") {
        Ok(url) if !url.trim().is_empty() => url.trim().trim_end_matches('/').to_string(),
        _ => "https://api.anysola.com".to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// `ANYSOLA_TOKEN`: `x-api-key` on model calls, `Bearer` elsewhere.
    Env(String),
    /// A device login: always `Bearer`.
    Login(String),
}

impl Token {
    pub fn secret(&self) -> &str {
        match self {
            Token::Env(t) | Token::Login(t) => t,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfigFetch {
    Ok(String),
    Unauthorized,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct VersionInfo {
    pub latest: String,
    #[serde(default)]
    pub minimum: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Gateway {
    base: String,
    http: reqwest::Client,
    token: Option<Token>,
}

/// One item of a model call's stream.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamItem {
    Event(SseEvent),
    /// Credits charged for this call; `remaining` is the balance after it when known.
    Credits {
        charged: Option<u64>,
        remaining: Option<u64>,
    },
}

pub struct MessageStream {
    response: reqwest::Response,
    decoder: Decoder,
    pending: std::collections::VecDeque<StreamItem>,
    /// `x-credits-available-before`, when the credits event is read from the stream.
    before: Option<u64>,
    stream_credits: bool,
    done: bool,
}

fn header_u64(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers
        .get(name)?
        .to_str()
        .ok()?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .map(|n| n.max(0.0) as u64)
}

impl MessageStream {
    pub async fn next(&mut self) -> Option<Result<StreamItem, ApiError>> {
        loop {
            if let Some(item) = self.pending.pop_front() {
                return Some(Ok(item));
            }
            if self.done {
                return None;
            }
            match self.response.chunk().await {
                Ok(Some(bytes)) => {
                    let blocks = self.decoder.push(&bytes);
                    self.queue(blocks);
                }
                Ok(None) => {
                    self.done = true;
                    let blocks = self.decoder.finish();
                    self.queue(blocks);
                }
                Err(error) => {
                    self.done = true;
                    return Some(Err(ApiError::transport(error.to_string(), true)));
                }
            }
        }
    }

    fn queue(&mut self, blocks: Vec<Block>) {
        for block in blocks {
            match block {
                Block::Event(event) => self.pending.push_back(StreamItem::Event(event)),
                Block::Credits(charged) if self.stream_credits => {
                    let charged = charged.unwrap_or(0);
                    let remaining = self.before.map(|before| before.saturating_sub(charged));
                    self.pending.push_back(StreamItem::Credits {
                        charged: Some(charged),
                        remaining,
                    });
                }
                Block::Credits(_) => {}
            }
        }
    }
}

impl Gateway {
    pub fn new(token: Option<Token>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .build()
            .expect("HTTP client with default TLS settings");
        Gateway {
            base: api_url(),
            http,
            token,
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn token(&self) -> Option<&Token> {
        self.token.as_ref()
    }

    fn url(&self, path: &str) -> String {
        format!("{}/v1/code{path}", self.base)
    }

    /// Header set A (account calls and `/config`).
    fn account(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let request = request.header("accept", "application/json");
        let request = match &self.token {
            Some(token) => request.header("authorization", format!("Bearer {}", token.secret())),
            None => request,
        };
        request
            .header("x-anysola-version", VERSION)
            .header("user-agent", user_agent())
            .timeout(ACCOUNT_TIMEOUT)
    }

    /// `GET /config`; the raw body is what gets cached.
    pub async fn config(&self) -> ConfigFetch {
        let Ok(response) = self
            .account(self.http.get(self.url("/config")))
            .send()
            .await
        else {
            return ConfigFetch::Failed;
        };
        match response.status().as_u16() {
            401 => ConfigFetch::Unauthorized,
            200..=299 => response
                .text()
                .await
                .map_or(ConfigFetch::Failed, ConfigFetch::Ok),
            _ => ConfigFetch::Failed,
        }
    }

    /// `GET /version` (no credentials); `None` when offline or malformed.
    pub async fn version(&self, timeout: Duration) -> Option<VersionInfo> {
        let response = self
            .http
            .get(self.url("/version"))
            .header("accept", "application/json")
            .header("user-agent", user_agent())
            .timeout(timeout)
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let mut info: VersionInfo = serde_json::from_slice(&response.bytes().await.ok()?).ok()?;
        info.latest = strip_v(&info.latest);
        info.minimum = info.minimum.as_deref().map(strip_v);
        Some(info)
    }

    /// `POST /search`; errors are the model-visible texts. Returns the results and credits charged.
    pub async fn search(&self, query: &str) -> Result<(Vec<SearchResult>, u64), String> {
        let token = self
            .token
            .as_ref()
            .ok_or("Web search needs an Anysola sign-in. Run `anysola login`.")?;
        let response = self
            .http
            .post(self.url("/search"))
            .header("accept", "application/json")
            .header("authorization", format!("Bearer {}", token.secret()))
            .header("content-type", "application/json")
            .header("x-anysola-client", "opencode")
            .header("x-anysola-version", VERSION)
            .header("user-agent", user_agent())
            .timeout(SEARCH_TIMEOUT)
            .body(crate::json::compact(&serde_json::json!({ "query": query })))
            .send()
            .await
            .map_err(|e| self.search_transport(&e))?;
        let status = response.status().as_u16();
        let mut body = Vec::new();
        let mut response = response;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| self.search_transport(&e))?
        {
            body.extend_from_slice(&chunk);
            if body.len() > SEARCH_MAX_BYTES {
                return Err("Anysola web search returned too much data".into());
            }
        }
        let parsed: Option<Value> = serde_json::from_slice(&body).ok();
        if !(200..300).contains(&status) {
            let detail = parsed
                .as_ref()
                .and_then(error::error_of)
                .unwrap_or_else(|| format!("HTTP {status}"));
            return Err(match status {
                401 => {
                    "Your Anysola sign-in has expired or was revoked. Run `anysola login`.".into()
                }
                402 => format!(
                    "Out of Anysola credits for web search. Add credits at {}",
                    error::BILLING_URL
                ),
                429 => format!("Anysola web search is rate limited: {detail}"),
                _ => format!("Anysola web search failed (HTTP {status}): {detail}"),
            });
        }
        decode_search(parsed)
            .ok_or_else(|| "Anysola web search returned an unexpected response".to_string())
    }

    fn search_transport(&self, error: &reqwest::Error) -> String {
        if error.is_timeout() {
            "Anysola web search timed out".into()
        } else {
            format!("Cannot reach {}", self.base)
        }
    }

    /// `POST /messages` with a serialized body. `headers` are the request-specific ones (betas,
    /// session, variant); identity and credentials are added here.
    pub async fn messages(
        &self,
        body: String,
        headers: &[(String, String)],
    ) -> Result<MessageStream, ApiError> {
        let mut request = self
            .http
            .post(self.url("/messages"))
            .header("content-type", "application/json")
            .header("user-agent", model_user_agent("cli"))
            .header("anthropic-version", "2023-06-01");
        for (name, value) in headers {
            request = request.header(name.as_str(), value.as_str());
        }
        request = request
            .header("x-anysola-client", "opencode")
            .header("x-anysola-credits-event", "1")
            .header("x-anysola-version", VERSION);
        request = match &self.token {
            Some(Token::Env(token)) => request.header("x-api-key", token),
            Some(Token::Login(token)) => request.header("authorization", format!("Bearer {token}")),
            None => request,
        };
        let response = request
            .header("accept", "*/*")
            .body(body)
            .send()
            .await
            .map_err(|e| ApiError::transport(e.to_string(), !e.is_connect()))?;
        let headers = response.headers().clone();
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let retry_after = headers
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<f64>().ok())
                .map(|s| Duration::from_secs_f64(s.max(0.0)));
            let should_retry = match headers.get("x-should-retry").and_then(|v| v.to_str().ok()) {
                Some("true") => Some(true),
                Some("false") => Some(false),
                _ => None,
            };
            let body = response.text().await.unwrap_or_default();
            return Err(ApiError::http(status, body, retry_after, should_retry));
        }
        let charged = header_u64(&headers, "x-credits-charged");
        let remaining = header_u64(&headers, "x-credits-remaining");
        let is_sse = headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("text/event-stream"));
        let mut pending = std::collections::VecDeque::new();
        let header_credits = charged.is_some() || remaining.is_some();
        if header_credits {
            pending.push_back(StreamItem::Credits { charged, remaining });
        }
        Ok(MessageStream {
            response,
            decoder: Decoder::default(),
            pending,
            before: header_u64(&headers, "x-credits-available-before"),
            stream_credits: !header_credits && is_sse,
            done: false,
        })
    }
}

fn strip_v(version: &str) -> String {
    version.strip_prefix('v').unwrap_or(version).to_string()
}

fn decode_search(body: Option<Value>) -> Option<(Vec<SearchResult>, u64)> {
    let body = body?;
    let results = body.get("results")?.as_array()?;
    let mut out = Vec::new();
    for item in results {
        let text = |key: &str| item.get(key).and_then(Value::as_str).map(str::to_string);
        out.push(SearchResult {
            url: text("url")?,
            title: text("title"),
            content: text("content"),
            published: item
                .get("published")
                .and_then(Value::as_f64)
                .map(|n| n as u64),
        });
    }
    let credits = body
        .get("credits")
        .and_then(Value::as_f64)
        .filter(|n| *n > 0.0)
        .map_or(0, |n| n as u64);
    Some((out, credits))
}
