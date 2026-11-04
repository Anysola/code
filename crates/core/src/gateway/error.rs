//! Gateway failures: classification (as 0.5.1's provider-error.ts), retry eligibility, the session
//! error shape stored and printed, and the 402 rewrite.

use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const BILLING_URL: &str = "https://account.anysola.com/billing";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    ContextOverflow,
    PayloadTooLarge,
    ContentPolicy,
    Quota,
    Authentication,
    RateLimit,
    ProviderInternal,
    InvalidRequest,
    Unknown,
    /// Network failure; `sent` is false when the request never left.
    Transport {
        sent: bool,
    },
    /// The stream ended without `message_stop`.
    IncompleteStream,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    pub reason: Reason,
    pub message: String,
    pub status: Option<u16>,
    pub body: Option<String>,
    pub retry_after: Option<Duration>,
    /// The gateway's `x-should-retry` verdict, which overrides classification.
    pub should_retry: Option<bool>,
}

/// The error recorded on a message or step and printed by `run`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionError {
    #[serde(rename = "type")]
    pub kind: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<ErrorResponse>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub body: String,
}

impl SessionError {
    pub fn new(kind: &str, message: impl Into<String>) -> Self {
        SessionError {
            kind: kind.into(),
            message: message.into(),
            status: None,
            response: None,
        }
    }
}

impl ApiError {
    pub fn transport(message: impl Into<String>, sent: bool) -> Self {
        ApiError {
            reason: Reason::Transport { sent },
            message: message.into(),
            status: None,
            body: None,
            retry_after: None,
            should_retry: None,
        }
    }

    pub fn incomplete(message: impl Into<String>) -> Self {
        ApiError {
            reason: Reason::IncompleteStream,
            ..ApiError::transport(message, true)
        }
    }

    /// A non-2xx response. 402 bodies are rewritten first (see [`rewrite_402`]).
    pub fn http(
        status: u16,
        body: String,
        retry_after: Option<Duration>,
        should_retry: Option<bool>,
    ) -> Self {
        let body = if status == 402 {
            rewrite_402(&body)
        } else {
            body
        };
        let message = provider_message(&body).unwrap_or_else(|| format!("HTTP {status}"));
        let reason = classify(Some(status), &message, &body);
        ApiError {
            reason,
            message,
            status: Some(status),
            body: Some(body),
            retry_after,
            should_retry,
        }
    }

    /// An `event: error` inside an ok stream.
    pub fn stream(data: &str) -> Self {
        let message = provider_message(data).unwrap_or_else(|| data.to_string());
        let reason = classify(None, &message, data);
        ApiError {
            reason,
            message,
            status: None,
            body: Some(data.to_string()),
            retry_after: None,
            should_retry: None,
        }
    }

    pub fn retryable(&self) -> bool {
        if let Some(verdict) = self.should_retry {
            return verdict;
        }
        matches!(
            self.reason,
            Reason::RateLimit
                | Reason::ProviderInternal
                | Reason::Unknown
                | Reason::IncompleteStream
                | Reason::Transport { .. }
        )
    }

    /// One-shot generations (title) retry only failures where the request did not run.
    pub fn not_run(&self) -> bool {
        if let Some(verdict) = self.should_retry {
            return verdict;
        }
        match self.reason {
            Reason::RateLimit => true,
            Reason::ProviderInternal => matches!(self.status, Some(503 | 529)),
            Reason::Transport { sent } => !sent,
            _ => false,
        }
    }

    pub fn to_session_error(&self) -> SessionError {
        let kind = match self.reason {
            Reason::RateLimit => "provider.rate-limit",
            Reason::Authentication => "provider.auth",
            Reason::Quota => "provider.quota",
            Reason::ContentPolicy => "provider.content-filter",
            Reason::Transport { .. } => "provider.transport",
            Reason::ProviderInternal => "provider.internal",
            Reason::IncompleteStream => "provider.invalid-output",
            Reason::ContextOverflow | Reason::PayloadTooLarge | Reason::InvalidRequest => {
                "provider.invalid-request"
            }
            Reason::Unknown => "provider.unknown",
        };
        SessionError {
            kind: kind.into(),
            message: self.message.clone(),
            status: self.status,
            response: self.body.clone().map(|body| ErrorResponse { body }),
        }
    }
}

/// Replaces a 402 body with a billing error that says where to top up. The URL is appended only
/// when the gateway's message does not already name it.
pub fn rewrite_402(body: &str) -> String {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| error_of(&value))
        .filter(|m| m != "HTTP 402")
        .unwrap_or_else(|| "Out of Anysola credits.".to_string());
    let message = if message.contains("account.anysola.com/billing") {
        message
    } else {
        format!("{message}\nUpgrade: {BILLING_URL}")
    };
    crate::json::compact(
        &json!({"type": "error", "error": {"type": "billing_error", "message": message}}),
    )
}

/// `error_description`, else `error.message`, else `message`, else the error code.
pub fn error_of(body: &Value) -> Option<String> {
    let text = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_string);
    let error = body.get("error");
    text(body.get("error_description"))
        .or_else(|| text(error.and_then(|e| e.get("message"))))
        .or_else(|| text(body.get("message")))
        .or_else(|| text(error.filter(|e| e.is_string())))
        .or_else(|| text(error.and_then(|e| e.get("type"))))
}

/// The human-readable message of a provider error body.
fn provider_message(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    let non_blank = |v: Option<&Value>| {
        v.and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
    };
    let error = value.get("error");
    non_blank(error.and_then(|e| e.get("message")))
        .or_else(|| non_blank(error))
        .or_else(|| non_blank(value.get("message")))
        .or_else(|| non_blank(value.get("Message")))
        .or_else(|| non_blank(value.get("detail")))
        .or_else(|| {
            non_blank(
                value
                    .get("errors")
                    .and_then(|e| e.get(0))
                    .and_then(|e| e.get("message")),
            )
        })
}

fn codes(body: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut push = |v: Option<&Value>| {
        if let Some(code) = v.and_then(Value::as_str) {
            out.push(code.to_lowercase());
        }
    };
    let error = value.get("error");
    push(error.and_then(|e| e.get("type")));
    push(error.and_then(|e| e.get("code")));
    push(error.filter(|e| e.is_string()));
    push(value.get("code"));
    push(value.get("type").filter(|t| t.as_str() != Some("error")));
    out
}

fn regexes(sources: &[&str]) -> Vec<Regex> {
    sources
        .iter()
        .map(|s| Regex::new(&format!("(?i){s}")).expect("static regex"))
        .collect()
}

fn overflow(text: &str) -> bool {
    static PATTERNS: OnceLock<(Vec<Regex>, Vec<Regex>)> = OnceLock::new();
    let (patterns, exclusions) = PATTERNS.get_or_init(|| {
        (
            regexes(&[
                r"prompt is too long",
                r"input is too long for (?:the )?(?:requested|selected) model",
                r"too large for the model'?s configured input limit",
                r"exceeded this model context window limit",
                r"exceeds the context window",
                r"exceeds (?:the )?(?:model'?s )?maximum context length(?: of [\d,]+ tokens?|\s*\([\d,]+\))",
                r"input token count.*exceeds the maximum",
                r"number of input tokens exceeds maximum length",
                r"tokens in request more than max tokens allowed",
                r"maximum prompt length is \d+",
                r"reduce the length of the messages",
                r"requested input length \d+ exceeds maximum input length",
                r"maximum context length is \d+ tokens",
                r"exceeds (?:the )?maximum allowed input length of [\d,]+ tokens?",
                r"input(?: \(\d+ tokens\))? is longer than the model'?s context length",
                r"exceeds the limit of \d+",
                r"exceeds the available context size",
                r"greater than the context length",
                r"`inputs` tokens \+ `max_new_tokens` must be <= \d+",
                r"context window exceeds limit",
                r"context window exceeded",
                r"exceeded model token limit",
                r"context[_ ]length[_ ]exceeded",
                r"context length is only \d+ tokens",
                r"input length.*exceeds.*context length",
                r"prompt (?:too long|exceeds max length|超长)",
                r"too large for model with \d+ maximum context length",
                r"prompt has [\d,]+ tokens?, but the configured context size is [\d,]+ tokens?",
                r"model_context_window_exceeded",
                r"range of input length should be",
                r"too many tokens",
                r"token limit exceeded",
                r"request_too_large",
                r"^4(?:00|13)\s*(status code)?\s*\(no body\)",
            ]),
            regexes(&[
                r"^(throttling error|service unavailable):",
                r"rate limit",
                r"too many requests",
                r"max[_ ]tokens must be less than",
                r"too many tokens(?: per day)?, please wait",
            ]),
        )
    });
    !exclusions.iter().any(|r| r.is_match(text)) && patterns.iter().any(|r| r.is_match(text))
}

fn matches_any(
    text: &str,
    sources: &'static [&'static str],
    cache: &'static OnceLock<Vec<Regex>>,
) -> bool {
    cache
        .get_or_init(|| regexes(sources))
        .iter()
        .any(|r| r.is_match(text))
}

const QUOTA_CODES: &[&str] = &[
    "insufficient_quota",
    "usage_not_included",
    "billing_error",
    "gousagelimiterror",
    "budgetlimiterror",
    "freeusagelimiterror",
    "creditlimitexceeded",
];
const AUTH_CODES: &[&str] = &[
    "authentication_error",
    "permission_error",
    "permission_denied",
    "api_key_invalid",
];
const SERVER_CODES: &[&str] = &[
    "api_error",
    "internal_error",
    "internalserverexception",
    "modelstreamerrorexception",
    "overloaded_error",
    "server_error",
    "server_is_overloaded",
    "slow_down",
    "serviceunavailableexception",
];
const INVALID_REQUEST_CODES: &[&str] = &[
    "model_not_found",
    "invalid_prompt",
    "invalid_request",
    "invalid_request_error",
    "validationexception",
];
const CONTENT_POLICY_CODES: &[&str] = &[
    "content_filter",
    "content_moderation",
    "responsibleaipolicyviolation",
    "content_policy_violation",
    "image_content_policy_violation",
    "refusal",
    "datainspectionfailed",
    "moderation_blocked",
];

pub fn classify(status: Option<u16>, message: &str, body: &str) -> Reason {
    static PAYLOAD: OnceLock<Vec<Regex>> = OnceLock::new();
    static POLICY: OnceLock<Vec<Regex>> = OnceLock::new();
    static QUOTA: OnceLock<Vec<Regex>> = OnceLock::new();
    static RATE: OnceLock<Vec<Regex>> = OnceLock::new();
    static SERVER: OnceLock<Vec<Regex>> = OnceLock::new();
    let codes: Vec<String> = codes(body).into_iter().chain(codes(message)).collect();
    let has = |set: &[&str]| codes.iter().any(|c| set.contains(&c.as_str()));
    let text = [message, body]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    let client_scoped = status.is_none_or(|s| (400..500).contains(&s));

    if client_scoped
        && (codes.iter().any(|c| {
            matches!(
                c.as_str(),
                "context_length_exceeded" | "model_context_window_exceeded" | "request_too_large"
            )
        }) || (status != Some(429)
            && !codes.iter().any(|c| c == "throttlingexception")
            && overflow(&text)))
    {
        return Reason::ContextOverflow;
    }
    if status == Some(413)
        || matches_any(
            &text,
            &[
                "request entity too large",
                "payload too large",
                "request too large",
            ],
            &PAYLOAD,
        )
    {
        return Reason::PayloadTooLarge;
    }
    if has(CONTENT_POLICY_CODES)
        || (client_scoped
            && matches_any(
                message,
                &[
                    r"violating our usage policy|blocked by content filtering policy|content[-_\s]?policy|rejected (?:as a result of our|by the) safety system|detected potentially unsafe or sensitive content",
                ],
                &POLICY,
            ))
    {
        return Reason::ContentPolicy;
    }
    // `(?<!token_)` from 0.5.1's pattern is approximated by excluding the token-quota wording.
    let quota_text = status == Some(429)
        && !text.to_lowercase().contains("token_quota_exceeded")
        && matches_any(
            &text,
            &[
                r"insufficient[-_\s]?(?:quota|balance)|quota[-_\s]?exceeded|budget exceeded|usage limit|limit exhausted|package has expired|plan does not yet include",
            ],
            &QUOTA,
        );
    if status == Some(402) || has(QUOTA_CODES) || quota_text {
        return Reason::Quota;
    }
    if matches!(status, Some(401 | 403))
        || has(AUTH_CODES)
        || (status == Some(400) && text.to_lowercase().contains("incorrect api key provided"))
    {
        return Reason::Authentication;
    }
    if status == Some(429)
        || codes.iter().any(|c| {
            c.contains("rate_limit") || c == "too_many_requests" || c == "throttlingexception"
        })
        || matches_any(
            &text,
            &[r"rate increased too quickly|rate[-_\s]?limit|too[_\s]?many[_\s]?requests"],
            &RATE,
        )
    {
        return Reason::RateLimit;
    }
    let server_text = || {
        matches_any(
            &text,
            &[
                r"\b(?:try again|(?:please |you can )?retry (?:the |this |your )?request|try (?:the |this |your )?request again|(?:currently |temporarily )?at capacity|overloaded|temporarily unavailable|service[-_\s]?unavailable|(?:server|internal)[-_\s]?error|server (?:is )?busy|provider returned (?:an )?error|resource[-_\s]?exhausted|upstream (?:connect|connection|request)|request buffer limit while retrying upstream)\b",
            ],
            &SERVER,
        )
    };
    if matches!(status, Some(408 | 409))
        || status.is_some_and(|s| s >= 500)
        || (status.is_none_or(|s| s < 400)
            && ((!has(INVALID_REQUEST_CODES) && server_text())
                || codes.iter().any(|c| {
                    SERVER_CODES.contains(&c.as_str())
                        || c.contains("exhausted")
                        || c.contains("unavailable")
                })))
    {
        return Reason::ProviderInternal;
    }
    if has(INVALID_REQUEST_CODES) || status.is_some_and(|s| (400..500).contains(&s)) {
        return Reason::InvalidRequest;
    }
    Reason::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_dedupes_billing_url() {
        let gateway = r#"{"type":"error","error":{"type":"insufficient_credits","message":"You've used this cycle's Anysola credits. They renew on 23 Nov. account.anysola.com/billing"}}"#;
        let rewritten: Value = serde_json::from_str(&rewrite_402(gateway)).unwrap();
        assert_eq!(
            rewritten["error"]["message"],
            "You've used this cycle's Anysola credits. They renew on 23 Nov. account.anysola.com/billing"
        );
        let plain: Value =
            serde_json::from_str(&rewrite_402(r#"{"error":{"message":"No credits."}}"#)).unwrap();
        assert_eq!(
            plain["error"]["message"],
            "No credits.\nUpgrade: https://account.anysola.com/billing"
        );
        let empty: Value = serde_json::from_str(&rewrite_402("<html>")).unwrap();
        assert_eq!(
            empty["error"]["message"],
            "Out of Anysola credits.\nUpgrade: https://account.anysola.com/billing"
        );
    }

    #[test]
    fn classification() {
        let http = |status, body: &str| ApiError::http(status, body.to_string(), None, None).reason;
        assert_eq!(http(402, "{}"), Reason::Quota);
        assert_eq!(
            http(
                401,
                r#"{"error":{"type":"authentication_error","message":"bad"}}"#
            ),
            Reason::Authentication
        );
        assert_eq!(
            http(
                429,
                r#"{"error":{"type":"rate_limit_error","message":"slow down"}}"#
            ),
            Reason::RateLimit
        );
        assert_eq!(
            http(
                529,
                r#"{"error":{"type":"overloaded_error","message":"Overloaded"}}"#
            ),
            Reason::ProviderInternal
        );
        assert_eq!(
            http(
                400,
                r#"{"error":{"type":"invalid_request_error","message":"prompt is too long: 1200000 tokens > 1000000 maximum"}}"#
            ),
            Reason::ContextOverflow
        );
        assert_eq!(
            http(
                404,
                r#"{"error":{"type":"not_found_error","message":"model: x"}}"#
            ),
            Reason::InvalidRequest
        );
        assert_eq!(http(413, "{}"), Reason::PayloadTooLarge);
        assert!(!ApiError::http(500, "{}".into(), None, Some(false)).retryable());
        assert!(ApiError::http(400, "{}".into(), None, Some(true)).retryable());
    }

    #[test]
    fn session_error_shape() {
        let error = ApiError::http(
            402,
            r#"{"error":{"message":"x account.anysola.com/billing"}}"#.into(),
            None,
            None,
        );
        let json = serde_json::to_value(error.to_session_error()).unwrap();
        assert_eq!(json["type"], "provider.quota");
        assert_eq!(json["status"], 402);
        assert!(
            json["response"]["body"]
                .as_str()
                .unwrap()
                .contains("billing_error")
        );
    }
}
