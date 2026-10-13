//! OAuth clients Lorca registered with a service, for the Runners' native sign-ins: Google's, for
//! Gmail, Calendar, and Drive. A client's secret stays here and never reaches a Device. A Runner
//! builds the consent page with the client's id (`/v1/health` lists them), keeps the PKCE
//! verifier, and sends the callback's code, or later a refresh token, to
//! `POST /v1/oauth/{client}/token`. The relay adds the client's id and secret, posts to the
//! service's token endpoint, and answers with the service's status and body. It reads the tokens
//! while it passes them on and keeps none of them.

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Form, Json, Router};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::auth::Auth;
use crate::routes::{ApiError, ApiResult};
use crate::AppState;

pub struct Client {
    pub id: String,
    secret: String,
    /// The service's token endpoint, the only place the secret goes.
    pub token_endpoint: String,
    http: reqwest::Client,
}

impl Client {
    pub fn new(id: &str, secret: &str, token_endpoint: &str) -> anyhow::Result<Client> {
        Ok(Client {
            id: id.into(),
            secret: secret.into(),
            token_endpoint: token_endpoint.into(),
            http: reqwest::Client::builder().user_agent("lorca-relay").timeout(std::time::Duration::from_secs(30)).build()?,
        })
    }
}

#[derive(Default)]
pub struct Clients {
    pub google: Option<Client>,
}

impl Clients {
    fn get(&self, name: &str) -> Option<&Client> {
        match name {
            "google" => self.google.as_ref(),
            _ => None,
        }
    }

    /// Each client's id by name, for `/v1/health`.
    pub fn ids(&self) -> Value {
        let mut ids = Map::new();
        if let Some(google) = &self.google {
            ids.insert("google".into(), json!(google.id));
        }
        Value::Object(ids)
    }

    pub fn describe(&self) -> String {
        if self.google.is_some() { "google".into() } else { "none".into() }
    }
}

pub fn router() -> Router<AppState> {
    Router::new().route("/v1/oauth/{client}/token", post(token))
}

/// A Runner's token request, without the client: a callback's code with the PKCE verifier and
/// the loopback redirect it was issued for, or a refresh token.
#[derive(Debug, Deserialize)]
struct TokenRequest {
    grant_type: String,
    code: Option<String>,
    code_verifier: Option<String>,
    redirect_uri: Option<String>,
    refresh_token: Option<String>,
}

impl TokenRequest {
    /// The fields the service gets, the client's added: only the two grants a native sign-in
    /// uses, each with what it needs.
    fn form<'a>(&'a self, client: &'a Client) -> ApiResult<Vec<(&'a str, &'a str)>> {
        let given = |value: &'a Option<String>| value.as_deref().filter(|value| !value.is_empty() && value.len() <= 4096);
        let mut form = vec![("grant_type", self.grant_type.as_str())];
        match self.grant_type.as_str() {
            "authorization_code" => {
                let (Some(code), Some(verifier), Some(redirect)) = (given(&self.code), given(&self.code_verifier), given(&self.redirect_uri)) else {
                    return Err(ApiError::bad_request("A code needs its verifier and redirect"));
                };
                if !loopback(redirect) {
                    return Err(ApiError::bad_request("The redirect is a loopback address"));
                }
                form.extend([("code", code), ("code_verifier", verifier), ("redirect_uri", redirect)]);
            }
            "refresh_token" => form.push(("refresh_token", given(&self.refresh_token).ok_or_else(|| ApiError::bad_request("No refresh token"))?)),
            _ => return Err(ApiError::bad_request("Only a code or a refresh token")),
        }
        form.extend([("client_id", client.id.as_str()), ("client_secret", client.secret.as_str())]);
        Ok(form)
    }
}

/// A native app's redirect: plain HTTP to its own computer, where a Desktop client takes any port.
fn loopback(redirect: &str) -> bool {
    reqwest::Url::parse(redirect).is_ok_and(|url| {
        url.scheme() == "http" && url.username().is_empty() && url.password().is_none() && matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"))
    })
}

/// Swaps a Runner's code or refresh token at the service with the client's id and secret.
/// Answers `{ status, body }`: what the service answered, its errors included, so any other
/// status is the relay's own.
async fn token(State(state): State<AppState>, _auth: Auth, Path(name): Path<String>, Form(request): Form<TokenRequest>) -> ApiResult<Json<Value>> {
    let client = state.clients.get(&name).ok_or_else(|| ApiError::not_found(&format!("This relay has no {name} client")))?;
    let form = request.form(client)?;
    let response = client.http.post(&client.token_endpoint).header("accept", "application/json").form(&form).send().await.map_err(|error| {
        tracing::warn!(client = %name, error = %error.without_url(), "token endpoint unreachable");
        ApiError::unavailable("The service could not be reached")
    })?;
    let status = response.status().as_u16();
    // The body holds tokens: it goes to the Runner and nowhere else, logs included.
    let body: Value = response.json().await.unwrap_or(Value::Null);
    Ok(Json(json!({ "status": status, "body": body })))
}

#[cfg(test)]
mod tests;
