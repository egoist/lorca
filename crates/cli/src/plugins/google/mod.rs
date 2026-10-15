//! Gmail, Google Calendar, and Google Drive as builtin servers over Google's REST APIs. Their
//! tools are the ones Google's hosted MCP servers offer, by the same names and arguments
//! (`schemas.json`), so tool hints, message drafts, calendar routines, and skills read them as
//! before; the hosted servers need the Workspace Developer Preview, the REST APIs do not. Each
//! call reads the account's token through `service_token`, which refreshes it, through the relay
//! for Lorca's client, before it expires.

mod calendar;
mod drive;
mod gmail;

use std::sync::{Arc, OnceLock};

use serde_json::{json, Value};

use super::builtin::{structured_result, text_result, Service};
use crate::app::App;

/// The tools of one service as `schemas.json` declares them, in `tools/list`'s shape.
fn tools(service: &str) -> Vec<Value> {
    static SCHEMAS: OnceLock<Value> = OnceLock::new();
    let schemas = SCHEMAS.get_or_init(|| serde_json::from_str(include_str!("schemas.json")).expect("google/schemas.json reads"));
    schemas[service]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|tool| tool["inputSchema"].is_object())
        .map(|tool| {
            json!({
                "name": tool["name"],
                "description": tool["description"],
                "inputSchema": tool["inputSchema"],
                "annotations": { "readOnlyHint": tool["readOnly"].as_bool().unwrap_or(false) },
            })
        })
        .collect()
}

/// Where Gmail's API answers. `LORCA_GOOGLE_API_URL` points all three services at one place,
/// a stub in tests: Google serves Gmail's paths there too.
fn gmail_api() -> String {
    std::env::var("LORCA_GOOGLE_API_URL").unwrap_or_else(|_| "https://gmail.googleapis.com".into())
}

/// Where Calendar's and Drive's APIs answer.
fn google_api() -> String {
    std::env::var("LORCA_GOOGLE_API_URL").unwrap_or_else(|_| "https://www.googleapis.com".into())
}

/// One account's calls to a Google API, with its sign-in on `server`.
pub(crate) struct Api<'a> {
    app: &'a Arc<App>,
    plugin_id: &'a str,
    server: &'a str,
}

/// A request to one of Google's APIs that did not succeed, in words a bot can act on. The ones
/// that mean the sign-in is gone or too narrow carry the markers the pool recognizes, so the
/// account reads Sign in or Needs more access.
pub(crate) type Failure = String;

impl<'a> Api<'a> {
    pub fn new(app: &'a Arc<App>, plugin_id: &'a str, server: &'a str) -> Self {
        Api { app, plugin_id, server }
    }

    pub async fn get(&self, url: &str, query: &[(&str, String)]) -> Result<Value, Failure> {
        self.send(reqwest::Method::GET, url, query, None).await
    }

    pub async fn post(&self, url: &str, query: &[(&str, String)], body: Value) -> Result<Value, Failure> {
        self.send(reqwest::Method::POST, url, query, Some(body)).await
    }

    pub async fn patch(&self, url: &str, query: &[(&str, String)], body: Value) -> Result<Value, Failure> {
        self.send(reqwest::Method::PATCH, url, query, Some(body)).await
    }

    pub async fn delete(&self, url: &str, query: &[(&str, String)]) -> Result<Value, Failure> {
        self.send(reqwest::Method::DELETE, url, query, None).await
    }

    /// A request's raw answer: its bytes and content type, for a file's content.
    pub async fn bytes(&self, url: &str, query: &[(&str, String)]) -> Result<(Vec<u8>, String), Failure> {
        let response = self.request(reqwest::Method::GET, url, query, None).await?;
        let kind = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("application/octet-stream").to_string();
        let bytes = response.bytes().await.map_err(|_| "Google's answer was cut off. Try again.".to_string())?;
        Ok((bytes.to_vec(), kind))
    }

    /// A multipart upload (Drive's `uploadType=multipart`): the metadata, then the content.
    pub async fn upload(&self, url: &str, metadata: Value, content: &[u8], content_type: &str) -> Result<Value, Failure> {
        let boundary = format!("lorca-{}", uuid::Uuid::new_v4().simple());
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n--{boundary}\r\nContent-Type: {content_type}\r\n\r\n").as_bytes());
        body.extend_from_slice(content);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let token = self.token().await?;
        let response = self
            .app
            .http
            .post(url)
            .query(&[("uploadType", "multipart")])
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, format!("multipart/related; boundary={boundary}"))
            .body(body)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|_| "Google could not be reached. Try again.".to_string())?;
        read_json(checked(response).await?).await
    }

    async fn send(&self, method: reqwest::Method, url: &str, query: &[(&str, String)], body: Option<Value>) -> Result<Value, Failure> {
        read_json(self.request(method, url, query, body).await?).await
    }

    async fn token(&self) -> Result<String, Failure> {
        // `service_token`'s own words already say to sign in again where that is the way out.
        super::mcp::service_token(self.app, self.plugin_id, self.server).await
    }

    async fn request(&self, method: reqwest::Method, url: &str, query: &[(&str, String)], body: Option<Value>) -> Result<reqwest::Response, Failure> {
        let token = self.token().await?;
        let mut request = self.app.http.request(method, url).query(query).bearer_auth(token).header(reqwest::header::ACCEPT, "application/json").timeout(std::time::Duration::from_secs(60));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|_| "Google could not be reached. Try again.".to_string())?;
        checked(response).await
    }
}

async fn read_json(response: reqwest::Response) -> Result<Value, Failure> {
    let bytes = response.bytes().await.map_err(|_| "Google's answer was cut off. Try again.".to_string())?;
    if bytes.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(&bytes).map_err(|_| "Google sent an answer that does not read.".to_string())
}

/// The response when it succeeded, else what went wrong. A 401 reads as an expired sign-in and a
/// 403 for scopes as an insufficient one, with Google's challenge, which names the scope.
async fn checked(response: reqwest::Response) -> Result<reqwest::Response, Failure> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let challenge = response.headers().get(reqwest::header::WWW_AUTHENTICATE).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    let message = body["error"]["message"].as_str().unwrap_or_default();
    let reason = body["error"]["errors"][0]["reason"].as_str().or(body["error"]["details"][0]["reason"].as_str()).unwrap_or_default();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("Google refused the sign-in (401 Unauthorized, invalid_token). Sign in to the account again.".into());
    }
    if status == reqwest::StatusCode::FORBIDDEN && (challenge.contains("insufficient_scope") || reason.contains("SCOPE_INSUFFICIENT") || reason == "insufficientPermissions") {
        let challenge = if challenge.contains("insufficient_scope") { challenge } else { "Bearer error=\"insufficient_scope\"".into() };
        // The challenge first: the pool reads the scope from the first line that names it.
        return Err(format!("{challenge}\nThe sign-in needs more access for this. Sign in to the account again to grant it."));
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS || reason.contains("RateLimit") || reason == "rateLimitExceeded" || reason == "userRateLimitExceeded" {
        return Err(format!("Google is limiting this account's requests right now ({}). Wait a minute, then try again.", status.as_u16()));
    }
    let said = if message.is_empty() { status.canonical_reason().unwrap_or("an error").to_string() } else { message.to_string() };
    Err(format!("Google answered {}: {said}", status.as_u16()))
}

/// A tool's result from its call: the value, or the failure as an error result.
fn result(outcome: Result<Value, Failure>) -> Value {
    match outcome {
        Ok(value) => structured_result(value),
        Err(failure) => text_result(failure, true),
    }
}

/// A string argument, trimmed, when it is given.
fn text(args: &Value, key: &str) -> Option<String> {
    args[key].as_str().map(str::trim).filter(|value| !value.is_empty()).map(str::to_string)
}

/// A required string argument.
fn required(args: &Value, key: &str) -> Result<String, Failure> {
    text(args, key).ok_or_else(|| format!("{key} is required."))
}

/// A list argument: an array of strings, or one string of comma-separated values.
fn list(args: &Value, key: &str) -> Vec<String> {
    match &args[key] {
        Value::Array(values) => values.iter().filter_map(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string).collect(),
        Value::String(value) => value.split(',').map(str::trim).filter(|v| !v.is_empty()).map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

/// A query parameter from an argument, when it is given.
fn param(query: &mut Vec<(&'static str, String)>, name: &'static str, value: Option<String>) {
    if let Some(value) = value {
        query.push((name, value));
    }
}

/// A number argument as a query value.
fn number(args: &Value, key: &str) -> Option<String> {
    args[key].as_u64().map(|n| n.to_string()).or_else(|| text(args, key))
}

/// The builtin servers of the three services.
pub fn service(name: &str) -> Option<Arc<dyn Service>> {
    match name {
        "gmail" => Some(Arc::new(gmail::Server)),
        "google-calendar" => Some(Arc::new(calendar::Server)),
        "google-drive" => Some(Arc::new(drive::Server)),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
