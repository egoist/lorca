//! MCP's HTTP client with a response-header observer: rmcp's reqwest client, with the service's
//! rate guidance read off every response. OAuth remains in rmcp's AuthClient; the guidance
//! delays subsequent calls and never causes this client to replay a POST.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use futures::{stream::BoxStream, StreamExt};
use mcp_http::header::{HeaderName, HeaderValue};
use rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};
use rmcp::transport::streamable_http_client::{
    AuthRequiredError, InsufficientScopeError, SseError, StreamableHttpClient, StreamableHttpError,
    StreamableHttpPostResponse,
};
use sse_stream::{Sse, SseStream};

use crate::app::App;

#[derive(Clone)]
pub struct LimitedHttpClient {
    inner: mcp_http::Client,
    app: Weak<App>,
    plugin_id: String,
}

impl LimitedHttpClient {
    pub fn new(app: &Arc<App>, inner: mcp_http::Client, plugin_id: &str) -> Self {
        Self {
            inner,
            app: Arc::downgrade(app),
            plugin_id: plugin_id.into(),
        }
    }

    fn observe(&self, response: &mcp_http::Response) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let number = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|h| h.to_str().ok())
                .and_then(|h| h.trim().parse::<f64>().ok())
        };
        let seconds = number("retry-after-ms")
            .map(|ms| ms / 1000.0)
            .or_else(|| {
                let text = response.headers().get("retry-after")?.to_str().ok()?;
                text.trim().parse::<f64>().ok().or_else(|| {
                    httpdate::parse_http_date(text).ok().map(|date| {
                        date.duration_since(std::time::SystemTime::now())
                            .unwrap_or_default()
                            .as_secs_f64()
                    })
                })
            })
            .or_else(|| {
                // Services send the reset as a Unix time or as seconds from now.
                let reset = number("x-ratelimit-reset").filter(|_| number("x-ratelimit-remaining") == Some(0.0))?;
                let now = crate::config::now_secs();
                Some(if reset > now { reset - now } else { reset })
            });
        if let Some(seconds) = seconds.filter(|seconds| seconds.is_finite() && *seconds >= 0.0) {
            app.connector_limits
                .cooldown(&app, &self.plugin_id, seconds);
        } else if response.status().as_u16() == 429 {
            app.connector_limits.cooldown(&app, &self.plugin_id, 1.0);
        }
    }
}

impl StreamableHttpClient for LimitedHttpClient {
    type Error = mcp_http::Error;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        self.post_message_with_max_sse_event_size(
            uri,
            message,
            session_id,
            auth_header,
            custom_headers,
            8 * 1024 * 1024,
        )
        .await
    }

    async fn post_message_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        if let Some(app) = self.app.upgrade() {
            crate::connector_limits::ConnectorLimits::wait_cooldown(&app, &self.plugin_id)
                .await
                .map_err(|error| StreamableHttpError::UnexpectedServerResponse(error.into()))?;
        }
        let mut request = self
            .inner
            .post(uri.as_ref())
            .header("accept", "text/event-stream, application/json");
        if let Some(token) = auth_header {
            request = request.bearer_auth(token);
        }
        for (name, value) in custom_headers {
            if matches!(name.as_str(), "accept" | "mcp-session-id" | "last-event-id") {
                return Err(StreamableHttpError::ReservedHeaderConflict(
                    name.to_string(),
                ));
            }
            request = request.header(name, value);
        }
        let session_was_attached = session_id.is_some();
        if let Some(session_id) = &session_id {
            request = request.header("mcp-session-id", session_id.as_ref());
        }
        let response = request
            .json(&message)
            .send()
            .await
            .map_err(StreamableHttpError::Client)?;
        self.observe(&response);
        let status = response.status();
        if let Some(challenge) = response
            .headers()
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok())
        {
            if status.as_u16() == 401 {
                return Err(StreamableHttpError::AuthRequired(AuthRequiredError::new(
                    challenge.into(),
                )));
            }
            if status.as_u16() == 403 {
                let scope = challenge.split("scope=").nth(1).and_then(|v| {
                    if let Some(v) = v.strip_prefix('"') {
                        v.split('"').next().map(str::to_string)
                    } else {
                        v.split([',', ';', ' '])
                            .next()
                            .filter(|v| !v.is_empty())
                            .map(str::to_string)
                    }
                });
                return Err(StreamableHttpError::InsufficientScope(
                    InsufficientScopeError::new(challenge.into(), scope),
                ));
            }
        }
        let session = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        if status.as_u16() == 202 || status.as_u16() == 204 {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        // The server ended the session and did not run the request; rmcp starts a new session
        // and sends it again.
        if status.as_u16() == 404 && session_was_attached {
            return Err(StreamableHttpError::SessionExpired);
        }
        // Some servers answer a notification or response with an empty 200 instead of 202.
        let awaits_reply = matches!(message, ClientJsonRpcMessage::Request(_));
        if status.is_success() && response.content_length() == Some(0) && !awaits_reply {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        if !status.is_success() {
            let body = response.text().await.map_err(StreamableHttpError::Client)?;
            if let Ok(parsed @ ServerJsonRpcMessage::Error(_)) = serde_json::from_str(&body) {
                return Ok(StreamableHttpPostResponse::Json(parsed, session));
            }
            return Err(StreamableHttpError::UnexpectedServerResponse(
                format!(
                    "HTTP {status}: {}",
                    body.chars().take(400).collect::<String>()
                )
                .into(),
            ));
        }
        match content_type.as_deref() {
            Some(ct) if ct.starts_with("text/event-stream") => {
                let mut event_bytes = 0usize;
                let mut line_bytes = 0usize;
                let mut previous_cr = false;
                let raw = response.bytes_stream().map(move |chunk| {
                    let chunk = chunk.map_err(std::io::Error::other)?;
                    for byte in &chunk {
                        if *byte == b'\n' && previous_cr {
                            previous_cr = false;
                            continue;
                        }
                        event_bytes += 1;
                        if event_bytes > max_sse_event_size {
                            return Err(std::io::Error::other(
                                "MCP SSE event exceeds its configured size limit",
                            ));
                        }
                        if *byte == b'\r' || *byte == b'\n' {
                            if line_bytes == 0 {
                                event_bytes = 0;
                            }
                            line_bytes = 0;
                            previous_cr = *byte == b'\r';
                        } else {
                            line_bytes += 1;
                            previous_cr = false;
                        }
                    }
                    Ok(chunk)
                });
                Ok(StreamableHttpPostResponse::Sse(
                    Box::pin(SseStream::from_bytes_stream(raw)),
                    session,
                ))
            }
            Some(ct) if ct.starts_with("application/json") => {
                let body = response
                    .bytes()
                    .await
                    .map_err(StreamableHttpError::Client)?;
                match serde_json::from_slice::<ServerJsonRpcMessage>(&body) {
                    Ok(parsed) => Ok(StreamableHttpPostResponse::Json(parsed, session)),
                    Err(_) if !awaits_reply => {
                        Ok(StreamableHttpPostResponse::Accepted)
                    }
                    Err(error) => Err(StreamableHttpError::UnexpectedServerResponse(
                        format!("Invalid MCP JSON response: {error}").into(),
                    )),
                }
            }
            _ => Err(StreamableHttpError::UnexpectedContentType(content_type)),
        }
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        self.inner
            .delete_session(uri, session_id, auth_header, custom_headers)
            .await
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        self.inner
            .get_stream(uri, session_id, last_event_id, auth_header, custom_headers)
            .await
    }

    async fn get_stream_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max: usize,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        self.inner
            .get_stream_with_max_sse_event_size(
                uri,
                session_id,
                last_event_id,
                auth_header,
                custom_headers,
                max,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn http_retry_guidance_is_shared_and_an_effectful_post_is_sent_once() {
        let scratch = crate::connector_limits::tests::scratch();
        let app = &scratch.0;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url: Arc<str> = format!("http://{}/mcp", listener.local_addr().unwrap()).into();
        let hits = Arc::new(AtomicUsize::new(0));
        let count = hits.clone();
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                count.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 8192];
                let _ = socket.read(&mut buf).await;
                socket.write_all(b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 3\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
            }
        });
        let client = LimitedHttpClient::new(
            app,
            mcp_http::Client::builder()
                .retry(mcp_http::retry::never())
                .build()
                .unwrap(),
            "work",
        );
        let request = serde_json::from_value::<ClientJsonRpcMessage>(serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"send_message","arguments":{}}})).unwrap();
        assert!(client
            .post_message(url, request, None, None, HashMap::new())
            .await
            .is_err());
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "a service cooldown never replays the effectful call"
        );
        let held = app.connector_limits.state.lock().unwrap();
        assert!(
            held.as_ref().unwrap().buckets[&crate::connector_limits::account_key("work")]
                .cooldown_until
                > crate::config::now_secs() + 1.0
        );
        server.abort();
    }
}
