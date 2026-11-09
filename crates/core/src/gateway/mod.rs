//! Client for the Anysola gateway (`<api>/v1/code/*`): remote config, version check, web search and
//! streamed model calls, with the exact header sets 0.5.1 sent.

pub mod account;
pub mod config;
pub mod error;
pub mod format;
pub mod limits;
pub mod retry;
pub mod sse;

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::tool::websearch::SearchResult;
use crate::version::{VERSION, model_user_agent, user_agent};
use error::ApiError;
use sse::{Block, Decoder, SseEvent};

/// Asks the gateway to close each model stream with an `anysola_usage` event.
pub const USAGE_EVENT_HEADER: &str = "x-anysola-usage-event";
/// On a 429 for a full window: `five_hour` or `weekly`.
pub const LIMIT_HEADER: &str = "x-anysola-limit";

const ACCOUNT_TIMEOUT: Duration = Duration::from_secs(15);
/// A model stream that sends nothing (not even a `: keepalive` comment) for this long is cut
/// and the step retries; a dead connection otherwise waits for the kernel's TCP timeout.
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
/// `/messages` must answer with headers within this long.
const STREAM_HEADERS_TIMEOUT: Duration = Duration::from_secs(120);
const SEARCH_TIMEOUT: Duration = Duration::from_secs(30);
const SEARCH_MAX_BYTES: usize = 1024 * 1024;

/// `ANYSOLA_API_URL` without trailing slashes, else the production gateway.
pub fn api_url() -> String {
    match std::env::var("ANYSOLA_API_URL") {
        Ok(url) if !url.trim().is_empty() => url.trim().trim_end_matches('/').to_string(),
        _ => "https://api.anysola.com".to_string(),
    }
}

/// `ANYSOLA_TOKEN`, trimmed, when set and not blank.
pub fn env_token() -> Option<String> {
    std::env::var("ANYSOLA_TOKEN")
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// `ANYSOLA_TOKEN`: `x-api-key` on model calls, `Bearer` elsewhere.
    Env(String),
    /// A device login: always `Bearer`.
    Login(String),
}

impl Token {
    /// `ANYSOLA_TOKEN` wins over a stored device login everywhere.
    pub fn resolve(db: &crate::storage::Db) -> crate::storage::Result<Option<Token>> {
        Ok(match env_token() {
            Some(token) => Some(Token::Env(token)),
            None => db.anysola_login()?.map(Token::Login),
        })
    }

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
    /// Another status, or `None` when the request did not complete.
    Failed(Option<u16>),
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
    /// The gateway's closing usage event: what this call took of the 5-hour window and both
    /// windows after it.
    Usage(limits::UsageEvent),
}

pub struct MessageStream {
    response: reqwest::Response,
    decoder: Decoder,
    pending: std::collections::VecDeque<StreamItem>,
    done: bool,
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
            let chunk = match tokio::time::timeout(STREAM_IDLE_TIMEOUT, self.response.chunk()).await
            {
                Ok(chunk) => chunk,
                Err(_) => {
                    self.done = true;
                    return Some(Err(ApiError::transport(
                        format!(
                            "The model's stream sent nothing for {} seconds",
                            STREAM_IDLE_TIMEOUT.as_secs()
                        ),
                        true,
                    )));
                }
            };
            match chunk {
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
                Block::Usage(Some(usage)) => self.pending.push_back(StreamItem::Usage(usage)),
                Block::Usage(None) => {}
            }
        }
    }
}

impl Gateway {
    pub fn new(token: Option<Token>) -> Self {
        Self::with_base(api_url(), token)
    }

    pub fn with_base(base: String, token: Option<Token>) -> Self {
        let builder = || {
            reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(30))
                .tcp_keepalive(Duration::from_secs(30))
                .http2_keep_alive_interval(Duration::from_secs(30))
                .http2_keep_alive_timeout(Duration::from_secs(20))
                .http2_keep_alive_while_idle(true)
        };
        // An unreadable system certificate store fails the build; without its roots the program
        // still starts and each request reports the TLS failure.
        let http = builder().build().unwrap_or_else(|error| {
            crate::log::error(
                "cannot build the HTTP client with the system certificates",
                &[("error", &format!("{error:?}"))],
            );
            builder()
                .tls_built_in_root_certs(false)
                .build()
                .expect("HTTP client without built-in root certificates")
        });
        Gateway { base, http, token }
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
            return ConfigFetch::Failed(None);
        };
        match response.status().as_u16() {
            401 => ConfigFetch::Unauthorized,
            200..=299 => response
                .text()
                .await
                .map_or(ConfigFetch::Failed(None), ConfigFetch::Ok),
            status => ConfigFetch::Failed(Some(status)),
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

    /// A plain `GET` of a user-configured URL (instruction sources); `None` unless it answers 2xx
    /// within `timeout`.
    pub async fn fetch_text(&self, url: &str, timeout: Duration) -> Option<String> {
        let response = self
            .http
            .get(url)
            .header("user-agent", user_agent())
            .timeout(timeout)
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.text().await.ok()
    }

    /// `POST /search`; errors are the model-visible texts.
    pub async fn search(&self, query: &str) -> Result<Vec<SearchResult>, String> {
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
            .header("x-anysola-client", "anysola-code")
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
                429 => format!("Anysola web search is limited: {detail}"),
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
            .header("user-agent", model_user_agent())
            .header("anthropic-version", "2023-06-01");
        for (name, value) in headers {
            request = request.header(name.as_str(), value.as_str());
        }
        request = request
            .header("x-anysola-client", "anysola-code")
            .header(USAGE_EVENT_HEADER, "1")
            .header("x-anysola-version", VERSION);
        request = match &self.token {
            Some(Token::Env(token)) => request.header("x-api-key", token),
            Some(Token::Login(token)) => request.header("authorization", format!("Bearer {token}")),
            None => request,
        };
        let sent = request.header("accept", "*/*").body(body).send();
        let response = match tokio::time::timeout(STREAM_HEADERS_TIMEOUT, sent).await {
            Ok(result) => {
                result.map_err(|e| ApiError::transport(e.to_string(), !e.is_connect()))?
            }
            Err(_) => {
                return Err(ApiError::transport(
                    format!(
                        "The model did not answer within {} seconds",
                        STREAM_HEADERS_TIMEOUT.as_secs()
                    ),
                    true,
                ));
            }
        };
        let headers = response.headers().clone();
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let retry_after = headers
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<f64>().ok())
                .filter(|s| s.is_finite())
                .map(|s| Duration::from_secs_f64(s.clamp(0.0, 1e9)));
            let should_retry = match headers.get("x-should-retry").and_then(|v| v.to_str().ok()) {
                Some("true") => Some(true),
                Some("false") => Some(false),
                _ => None,
            };
            let limit = headers
                .get(LIMIT_HEADER)
                .and_then(|v| v.to_str().ok())
                .and_then(limits::WindowId::parse);
            let body = response.text().await.unwrap_or_default();
            let error = ApiError::http(status, body, retry_after, should_retry);
            return Err(match limit {
                Some(window) => error.usage_limit(window),
                None => error,
            });
        }
        Ok(MessageStream {
            response,
            decoder: Decoder::default(),
            pending: std::collections::VecDeque::new(),
            done: false,
        })
    }
}

fn strip_v(version: &str) -> String {
    version.strip_prefix('v').unwrap_or(version).to_string()
}

fn decode_search(body: Option<Value>) -> Option<Vec<SearchResult>> {
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
    Some(out)
}
