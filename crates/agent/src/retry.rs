//! Retries and error classes, after pi's `retry`, `provider-retry`, and `overflow`.
//!
//! Two layers: [`send_with_retry`] repeats an HTTP request the way the OpenAI and Anthropic
//! SDKs do (408, 409, 429, 5xx, and transport failures, honoring `retry-after`) with a sleep
//! that cancellation interrupts; [`RetryPolicy`] and [`is_retryable_error`] are for a harness
//! that wants to restart a whole assistant turn after a transient failure.

use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::types::{AssistantMessage, StopReason};

pub const DEFAULT_MAX_RETRY_DELAY_MS: u64 = 60_000;

/// Bounded attempts with exponential backoff: `base_delay_ms * 2^(attempt-1)`, capped.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetryPolicy {
    pub enabled: bool,
    /// The initial call never counts as a retry.
    pub max_retries: u32,
    pub base_delay_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_delay_ms: Option<u64>,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy { enabled: true, max_retries: 3, base_delay_ms: 1_000, max_delay_ms: Some(DEFAULT_MAX_RETRY_DELAY_MS) }
    }
}

impl RetryPolicy {
    /// The wait before retry `attempt` (1-based).
    pub fn delay_ms(&self, attempt: u32) -> u64 {
        let delay = self.base_delay_ms.saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1)));
        delay.min(self.max_delay_ms.unwrap_or(DEFAULT_MAX_RETRY_DELAY_MS))
    }
}

fn pattern(parts: &[&str]) -> Regex {
    Regex::new(&format!("(?i){}", parts.join("|"))).expect("valid pattern")
}

fn non_retryable_limit_pattern() -> &'static Regex {
    static PATTERN: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    PATTERN.get_or_init(|| pattern(&["GoUsageLimitError", "FreeUsageLimitError", "Monthly usage limit reached", "available balance", "insufficient_quota", "out of budget", "quota exceeded", "billing"]))
}

fn retryable_pattern() -> &'static Regex {
    static PATTERN: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    PATTERN.get_or_init(|| {
        pattern(&[
            "overloaded", "rate.?limit", "too many requests", "429", "500", "502", "503", "504", "524", "service.?unavailable", "server.?error", "internal.?error",
            "provider.?returned.?error", "exceeded request buffer limit while retrying upstream",
            "network.?error", "connection.?error", "connection.?refused", "connection.?lost", "other side closed", "fetch failed", "getaddrinfo", "ENOTFOUND", "EAI_AGAIN", "upstream.?connect", "reset before headers", "socket hang up", "socket connection was closed", "timed? out", "timeout", "terminated",
            "websocket.?closed", "websocket.?error",
            "ended without", "stream ended before message_stop", "stream ended before a terminal response event", "http2 request did not get a response",
            "retry delay",
            "you can retry your request", "try your request again", "please retry your request",
            "ResourceExhausted",
        ])
    })
}

/// Whether an error message reads like a transient provider or transport failure, so the
/// turn is worth restarting. Quota and billing exhaustion never is.
pub fn is_retryable_error(message: &str) -> bool {
    if non_retryable_limit_pattern().is_match(message) {
        return false;
    }
    retryable_pattern().is_match(message)
}

/// Whether a failed assistant message is a transient failure worth a retry.
pub fn is_retryable_assistant_error(message: &AssistantMessage) -> bool {
    message.stop_reason == StopReason::Error && message.error_message.as_deref().is_some_and(is_retryable_error)
}

fn overflow_patterns() -> &'static [Regex] {
    static PATTERNS: std::sync::OnceLock<Vec<Regex>> = std::sync::OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            r"prompt is too long",
            r"request_too_large",
            r"input is too long for requested model",
            r"exceeds the context window",
            r"exceeds (?:the )?(?:model'?s )?maximum context length(?: of [\d,]+ tokens?|\s*\([\d,]+\))",
            r"input token count.*exceeds the maximum",
            r"maximum prompt length is \d+",
            r"reduce the length of the messages",
            r"maximum context length is \d+ tokens",
            r"exceeds (?:the )?maximum allowed input length of [\d,]+ tokens?",
            r"input \(\d+ tokens\) is longer than the model'?s context length \(\d+ tokens\)",
            r"exceeds the limit of \d+",
            r"exceeds the available context size",
            r"greater than the context length",
            r"context window exceeds limit",
            r"exceeded model token limit",
            r"too large for model with \d+ maximum context length",
            r"prompt has [\d,]+ tokens?, but the configured context size is [\d,]+ tokens?",
            r"model_context_window_exceeded",
            r"prompt too long; exceeded (?:max )?context length",
            r"range of input length should be",
            r"context[_ ]length[_ ]exceeded",
            r"too many tokens",
            r"token limit exceeded",
            r"^4(?:00|13)\s*(?:status code)?\s*\(no body\)",
        ]
        .iter()
        .map(|p| Regex::new(&format!("(?i){p}")).expect("valid pattern"))
        .collect()
    })
}

fn non_overflow_patterns() -> &'static [Regex] {
    static PATTERNS: std::sync::OnceLock<Vec<Regex>> = std::sync::OnceLock::new();
    PATTERNS.get_or_init(|| {
        [r"^(Throttling error|Service unavailable):", r"rate limit", r"too many requests"]
            .iter()
            .map(|p| Regex::new(&format!("(?i){p}")).expect("valid pattern"))
            .collect()
    })
}

/// Whether the message is the model saying the context no longer fits: an error the
/// providers word in their own ways, or (given the window) a reply whose input already
/// exceeds it or a `Length` stop with nothing generated.
pub fn is_context_overflow(message: &AssistantMessage, context_window: Option<u64>) -> bool {
    if message.stop_reason == StopReason::Error {
        if let Some(error) = &message.error_message {
            let non_overflow = non_overflow_patterns().iter().any(|p| p.is_match(error));
            if !non_overflow && overflow_patterns().iter().any(|p| p.is_match(error)) {
                return true;
            }
        }
    }
    if let Some(window) = context_window.filter(|w| *w > 0) {
        let input = message.usage.input + message.usage.cache_read;
        if message.stop_reason == StopReason::Stop && input > window {
            return true;
        }
        if message.stop_reason == StopReason::Length && message.usage.output == 0 && input as f64 >= window as f64 * 0.99 {
            return true;
        }
    }
    false
}

/// Whether a `Length` stop ended below the output limit asked for, so the cut may come from
/// context pressure and one compact-and-retry is worth it.
pub fn is_recoverable_length(message: &AssistantMessage, desired_max_output: u64) -> bool {
    message.stop_reason == StopReason::Length && desired_max_output > 0 && message.usage.output < desired_max_output
}

/// Why a request could not be sent, after retries.
#[derive(Debug)]
pub enum RequestFailure {
    Aborted,
    /// No response: the transport failed.
    Transport(String),
    /// A response with a failing status; `body` is what the server said.
    Status { status: reqwest::StatusCode, body: String },
}

impl RequestFailure {
    /// The error text for the assistant message, with the server's `error.message` when the
    /// body is JSON that has one.
    pub fn message(&self) -> String {
        match self {
            RequestFailure::Aborted => "Request aborted".into(),
            RequestFailure::Transport(error) => format!("Request failed: {error}"),
            RequestFailure::Status { status, body } => format!("{status}: {}", summarize_error_body(body)),
        }
    }
}

/// The server's `error.message` from a JSON error body, else the body's first 300 characters.
pub fn summarize_error_body(text: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
        if let Some(message) = value["error"]["message"].as_str() {
            return message.to_string();
        }
        if let Some(message) = value["message"].as_str() {
            return message.to_string();
        }
    }
    let trimmed = text.trim();
    if trimmed.chars().count() > 300 {
        format!("{}…", trimmed.chars().take(300).collect::<String>())
    } else {
        trimmed.to_string()
    }
}

fn is_retryable_status(response: &reqwest::Response) -> bool {
    match response.headers().get("x-should-retry").and_then(|v| v.to_str().ok()) {
        Some("true") => return true,
        Some("false") => return false,
        _ => {}
    }
    let status = response.status();
    status == reqwest::StatusCode::REQUEST_TIMEOUT || status == reqwest::StatusCode::CONFLICT || status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

/// The wait the server asked for (`retry-after-ms`, then `retry-after` in seconds), else an
/// exponential backoff with jitter. A server-requested wait above `max_delay_ms` is an error.
fn retry_delay(response: &reqwest::Response, retry_index: u32, max_delay_ms: u64) -> Result<Duration, String> {
    let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<f64>().ok());
    let requested = header("retry-after-ms").or_else(|| header("retry-after").map(|s| s * 1000.0));
    if let Some(ms) = requested {
        if max_delay_ms > 0 && ms > max_delay_ms as f64 {
            return Err(format!("Server requested {}s retry delay (max: {}s)", (ms / 1000.0).ceil(), (max_delay_ms as f64 / 1000.0).ceil()));
        }
        return Ok(Duration::from_millis(ms.max(0.0) as u64));
    }
    let exponential = (0.5 * 2f64.powi(retry_index as i32)).min(8.0) * 1000.0;
    let jitter = 1.0 - rand::random::<f64>() * 0.25;
    Ok(Duration::from_millis((exponential * jitter) as u64))
}

/// Sends the request `build` makes, retrying a retryable failure up to `max_retries` times.
/// Returns the successful response, or why the last attempt failed.
pub async fn send_with_retry(
    build: impl Fn() -> reqwest::RequestBuilder,
    max_retries: u32,
    max_delay_ms: u64,
    cancel: &CancellationToken,
) -> Result<reqwest::Response, RequestFailure> {
    let mut retries_left = max_retries;
    loop {
        if cancel.is_cancelled() {
            return Err(RequestFailure::Aborted);
        }
        let sent = tokio::select! {
            _ = cancel.cancelled() => return Err(RequestFailure::Aborted),
            sent = build().send() => sent,
        };
        let (failure, delay) = match sent {
            Ok(response) if response.status().is_success() => return Ok(response),
            Ok(response) => {
                let retryable = retries_left > 0 && is_retryable_status(&response);
                let delay = if retryable { retry_delay(&response, max_retries - retries_left, max_delay_ms) } else { Ok(Duration::ZERO) };
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                match (retryable, delay) {
                    (true, Ok(delay)) => (RequestFailure::Status { status, body }, Some(delay)),
                    (true, Err(reason)) => return Err(RequestFailure::Status { status, body: format!("{reason}. {}", summarize_error_body(&body)) }),
                    (false, _) => return Err(RequestFailure::Status { status, body }),
                }
            }
            Err(error) => {
                let failure = RequestFailure::Transport(error.to_string());
                if retries_left == 0 {
                    return Err(failure);
                }
                let exponential = (0.5 * 2f64.powi((max_retries - retries_left) as i32)).min(8.0) * 1000.0;
                (failure, Some(Duration::from_millis((exponential * (1.0 - rand::random::<f64>() * 0.25)) as u64)))
            }
        };
        let Some(delay) = delay else { return Err(failure) };
        retries_left -= 1;
        tracing::debug!(?delay, retries_left, error = %failure.message(), "retrying provider request");
        tokio::select! {
            _ = cancel.cancelled() => return Err(RequestFailure::Aborted),
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_transient_and_final_errors() {
        assert!(is_retryable_error("529: Overloaded"));
        assert!(is_retryable_error("Request failed: connection reset before headers"));
        assert!(is_retryable_error("Anthropic stream ended before message_stop"));
        assert!(!is_retryable_error("429: insufficient_quota, check your billing"));
        assert!(!is_retryable_error("400: messages.1: tool_use ids must be unique"));
    }

    #[test]
    fn spots_context_overflow_in_the_providers_words() {
        let mut message = AssistantMessage::empty("p", "m");
        message.stop_reason = StopReason::Error;
        message.error_message = Some("400: prompt is too long: 213462 tokens > 200000 maximum".into());
        assert!(is_context_overflow(&message, None));
        message.error_message = Some("Throttling error: Too many tokens, please wait".into());
        assert!(!is_context_overflow(&message, None));

        let mut silent = AssistantMessage::empty("p", "m");
        silent.usage.input = 130_000;
        assert!(is_context_overflow(&silent, Some(128_000)));
        assert!(!is_context_overflow(&silent, None));
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let policy = RetryPolicy::default();
        assert_eq!((policy.delay_ms(1), policy.delay_ms(2), policy.delay_ms(3)), (1_000, 2_000, 4_000));
        assert_eq!(RetryPolicy { max_delay_ms: Some(2_500), ..RetryPolicy::default() }.delay_ms(3), 2_500);
        assert_eq!(policy.delay_ms(40), DEFAULT_MAX_RETRY_DELAY_MS);
    }

    #[tokio::test]
    async fn retries_a_server_error_then_gives_the_last_answer() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = hits.clone();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = [0u8; 4096];
                    let _ = socket.read(&mut buf).await;
                    let body = if n < 2 { "{\"error\":{\"message\":\"Overloaded\"}}" } else { "{\"ok\":true}" };
                    let status = if n < 2 { "503 Service Unavailable" } else { "200 OK" };
                    let response = format!("HTTP/1.1 {status}\r\nretry-after-ms: 1\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        let client = reqwest::Client::new();
        let url = format!("http://{address}/");
        let response = send_with_retry(|| client.get(&url), 3, DEFAULT_MAX_RETRY_DELAY_MS, &CancellationToken::new()).await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 3);

        // Out of retries: the failure carries the server's message.
        let failing = send_with_retry(|| client.get(&url), 0, DEFAULT_MAX_RETRY_DELAY_MS, &CancellationToken::new()).await;
        match failing {
            Ok(response) => assert_eq!(response.status(), 200, "the fourth hit succeeds"),
            Err(failure) => panic!("unexpected {}", failure.message()),
        }
    }
}
