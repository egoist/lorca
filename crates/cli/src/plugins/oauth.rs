//! Native service OAuth for the curated integrations. PKCE and the token exchange live on the
//! Runner; the requesting Device receives only the consent URL and sends a sealed callback.
//! Unlike MCP discovery, these provider endpoints do not take a `resource` parameter. With one
//! of Lorca's clients (`lorca_client`), whose secret only the relay holds, the Runner sends its
//! token requests to the relay, which adds the client and passes them on.

use std::collections::BTreeMap;
use std::sync::Arc;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::app::App;
use crate::config::now_secs;

pub struct Pending {
    client_id: String,
    client_secret: Option<String>,
    /// One of Lorca's clients, which the relay swaps the code with.
    lorca_client: Option<String>,
    token_endpoint: String,
    redirect_uri: String,
    state: String,
    verifier: String,
    scopes: Vec<String>,
}

impl Pending {
    pub fn begin(
        authorization_endpoint: &str,
        token_endpoint: &str,
        client_id: &str,
        client_secret: Option<&str>,
        lorca_client: Option<&str>,
        redirect_uri: &str,
        scopes: &[String],
        params: &BTreeMap<String, String>,
    ) -> Result<(Self, String), String> {
        let mut page = reqwest::Url::parse(authorization_endpoint).map_err(|_| "The authorization endpoint does not read.")?;
        let verifier = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        let state = uuid::Uuid::new_v4().simple().to_string();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        page.query_pairs_mut()
            .extend_pairs([
                ("response_type", "code"),
                ("client_id", client_id),
                ("redirect_uri", redirect_uri),
                ("scope", &scopes.join(" ")),
                ("state", &state),
                ("code_challenge", &challenge),
                ("code_challenge_method", "S256"),
            ])
            .extend_pairs(params);
        Ok((
            Self {
                client_id: client_id.into(),
                client_secret: client_secret.map(str::to_string),
                lorca_client: lorca_client.map(str::to_string),
                token_endpoint: token_endpoint.into(),
                redirect_uri: redirect_uri.into(),
                state,
                verifier,
                scopes: scopes.to_vec(),
            },
            page.to_string(),
        ))
    }

    pub async fn finish(self, app: &Arc<App>, callback: &str) -> Result<Value, String> {
        let landed = reqwest::Url::parse(callback).map_err(|_| "The sign-in callback does not read.")?;
        let redirect = reqwest::Url::parse(&self.redirect_uri).map_err(|_| "The sign-in redirect does not read.")?;
        if landed.origin() != redirect.origin()
            || landed.path() != redirect.path()
            || !landed.username().is_empty()
            || landed.password().is_some()
            || landed.fragment().is_some()
        {
            return Err("The sign-in callback is for another Device or redirect.".into());
        }
        let query: Vec<_> = landed.query_pairs().collect();
        let field = |name: &str| {
            let mut matches = query.iter().filter(|(key, _)| key == name);
            let value = matches.next().map(|(_, value)| value.as_ref());
            if matches.next().is_some() {
                None
            } else {
                value
            }
        };
        if field("state") != Some(self.state.as_str()) {
            return Err("The sign-in state did not match. Start it again.".into());
        }
        if field("error").is_some() {
            return Err("The sign-in was denied.".into());
        }
        let code = field("code").filter(|code| !code.is_empty()).ok_or("The sign-in callback has no code.")?;
        let form = [("grant_type", "authorization_code"), ("code", code), ("redirect_uri", &self.redirect_uri), ("code_verifier", &self.verifier)];
        let mut tokens = exchange(app, &self.token_endpoint, &self.client_id, self.client_secret.as_deref(), self.lorca_client.as_deref(), &form).await?;
        if tokens["scope"].as_str().is_none_or(str::is_empty) {
            tokens["scope"] = json!(self.scopes.join(" "));
        }
        Ok(
            json!({ "native_flow": true, "client_id": self.client_id, "client_secret": self.client_secret, "lorca_client": self.lorca_client, "token_endpoint": self.token_endpoint, "tokens": tokens, "signed_in_at": now_secs() }),
        )
    }
}

/// Refresh only authorization, before connecting. An effectful MCP call is never replayed.
pub async fn refresh(app: &Arc<App>, token_endpoint: &str, saved: &Value) -> Result<Option<Value>, String> {
    if saved["token_endpoint"].as_str().is_some_and(|endpoint| endpoint != token_endpoint) {
        return Err("The sign-in belongs to another authorization server. Sign in again.".into());
    }
    let expiry = saved["signed_in_at"].as_f64().zip(saved["tokens"]["expires_in"].as_f64());
    if !expiry.is_some_and(|(at, seconds)| at + seconds <= now_secs() + 300.0) {
        return Ok(None);
    }
    let refresh = saved["tokens"]["refresh_token"].as_str().filter(|token| !token.is_empty()).ok_or("The sign-in expired. Sign in again.")?;
    let client_id = saved["client_id"].as_str().ok_or("The saved sign-in has no client id.")?;
    let secret = saved["client_secret"].as_str().filter(|s| !s.is_empty());
    let form = [("grant_type", "refresh_token"), ("refresh_token", refresh)];
    let mut tokens = exchange(app, token_endpoint, client_id, secret, saved["lorca_client"].as_str(), &form).await?;
    for key in ["refresh_token", "scope"] {
        if tokens[key].as_str().is_none_or(str::is_empty) && saved["tokens"].get(key).is_some() {
            tokens[key] = saved["tokens"][key].clone();
        }
    }
    let mut updated = saved.clone();
    updated["tokens"] = tokens;
    updated["signed_in_at"] = json!(now_secs());
    Ok(Some(updated))
}

/// A token request: to the service with the user's own client, or through the relay with one of
/// Lorca's, which adds its id and secret there.
async fn exchange(app: &Arc<App>, endpoint: &str, client_id: &str, client_secret: Option<&str>, lorca_client: Option<&str>, form: &[(&str, &str)]) -> Result<Value, String> {
    let (status, tokens) = match lorca_client {
        Some(client) => through_relay(app, client, form).await?,
        None => {
            let mut form = form.to_vec();
            form.push(("client_id", client_id));
            if let Some(secret) = client_secret {
                form.push(("client_secret", secret));
            }
            let response = app
                .http
                .post(endpoint)
                .header("accept", "application/json")
                .form(&form)
                .timeout(std::time::Duration::from_secs(30))
                .send()
                .await
                .map_err(|_| "The authorization server could not be reached. Try again.")?;
            let status = response.status().as_u16();
            (status, response.json().await.map_err(|_| "The authorization server sent an unreadable token response.")?)
        }
    };
    if !(200..300).contains(&status) || tokens["ok"].as_bool() == Some(false) || tokens["access_token"].as_str().is_none_or(str::is_empty) {
        // Response bodies may echo a code or token. Only fixed recovery messages reach the UI,
        // encrypted machine blob, logs, or model context.
        return Err(match tokens["error"].as_str() {
            Some("invalid_grant" | "invalid_token" | "token_expired" | "bad_refresh_token") => "The sign-in expired. Sign in again.",
            Some("invalid_scope" | "insufficient_scope" | "missing_scope") => "The sign-in needs more access. Sign in again.",
            Some("invalid_client" | "unauthorized_client") => "The OAuth client was refused. Check the plugin's client settings.",
            _ => "The authorization server refused the sign-in. Try signing in again.",
        }
        .into());
    }
    Ok(super::mcp::tidy_tokens(&tokens))
}

/// A token request for one of Lorca's clients, sent to the relay with this machine's token.
async fn through_relay(app: &Arc<App>, client: &str, form: &[(&str, &str)]) -> Result<(u16, Value), String> {
    let unreachable = "The relay could not be reached. Try again.";
    let relay = app.relay_url().ok_or("Lorca's client signs in through the relay, and this Device has none.")?;
    let machine = app.machine_file().and_then(|m| m.machine().ok()).ok_or("This Device isn't paired.")?;
    let token = crate::sync::token_or_register(app, &relay, &machine).await.map_err(|_| unreachable)?;
    app.relay.oauth_token(&relay, &token, client, form).await.map_err(|error| match error.status {
        Some(404) => "This relay does not hold Lorca's client. Set up a client of your own in the account's settings.".into(),
        _ => unreachable.into(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// One real HTTP token response and the exact request it received.
    pub(crate) async fn token_server(body: Value) -> (String, tokio::sync::oneshot::Receiver<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let (send, recv) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0; 4096];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n").map(|at| at + 4) {
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                    let length = headers.lines().find_map(|line| line.strip_prefix("content-length:")?.trim().parse::<usize>().ok()).unwrap_or(0);
                    if bytes.len() >= end + length {
                        break;
                    }
                }
            }
            let body = body.to_string();
            socket
                .write_all(
                    format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                        .as_bytes(),
                )
                .await
                .unwrap();
            let _ = send.send(String::from_utf8(bytes).unwrap());
        });
        (endpoint, recv)
    }

    /// An App in a home of its own, removed when it drops.
    struct Scratch(Arc<App>, std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }
    fn scratch() -> Scratch {
        let home = std::env::temp_dir().join(format!("lorca-oauth-{}", uuid::Uuid::new_v4()));
        Scratch(App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap(), home)
    }

    fn fields(request: &str) -> BTreeMap<String, String> {
        let form = request.split_once("\r\n\r\n").unwrap().1;
        let url = reqwest::Url::parse(&format!("http://localhost/?{form}")).unwrap();
        url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
    }

    #[tokio::test]
    async fn native_sign_in_uses_pkce_offline_consent_and_only_the_bound_callback() {
        let scratch = scratch();
        let app = &scratch.0;
        let (endpoint, request) = token_server(json!({ "access_token": "a", "refresh_token": "r", "token_type": "Bearer", "expires_in": 3600 })).await;
        let params = BTreeMap::from([("access_type".into(), "offline".into()), ("prompt".into(), "consent select_account".into())]);
        let redirect = "http://127.0.0.1:54321/callback";
        let (flow, page) =
            Pending::begin("https://accounts.google.test/auth", &endpoint, "registered", Some("client-secret"), None, redirect, &["mail.read".into()], &params)
                .unwrap();
        let page = reqwest::Url::parse(&page).unwrap();
        let query: BTreeMap<_, _> = page.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
        assert_eq!(query["code_challenge_method"], "S256");
        assert_eq!(query["access_type"], "offline");
        assert_eq!(query["prompt"], "consent select_account");
        assert!(!query.contains_key("client_secret") && !query.contains_key("resource"));
        let verifier = flow.verifier.clone();
        let saved = flow.finish(app, &format!("{redirect}?state={}&code=code", query["state"])).await.unwrap();
        let form = fields(&request.await.unwrap());
        assert_eq!(form["code_verifier"], verifier);
        assert_eq!(URL_SAFE_NO_PAD.encode(Sha256::digest(form["code_verifier"].as_bytes())), query["code_challenge"]);
        assert_eq!(form["client_secret"], "client-secret");
        assert!(!form.contains_key("resource"));
        assert_eq!(saved["tokens"]["refresh_token"], "r");
        for callback in [
            format!("{redirect}?state=wrong&code=x"),
            format!("http://127.0.0.1:54322/callback?state={}&code=x", query["state"]),
            format!("{redirect}?state={}&state={}&code=x", query["state"], query["state"]),
        ] {
            let (flow, _) = Pending::begin("https://auth.test", &endpoint, "c", None, None, redirect, &[], &BTreeMap::new()).unwrap();
            assert!(flow.finish(app, &callback).await.is_err());
        }
    }

    #[tokio::test]
    async fn refresh_rotates_and_preserves_refresh_tokens_and_reports_revocation_safely() {
        let scratch = scratch();
        let app = &scratch.0;
        let saved = json!({ "native_flow": true, "client_id": "c", "client_secret": "secret", "signed_in_at": now_secs() - 7200.0, "tokens": { "access_token": "old", "refresh_token": "old-refresh", "scope": "read", "expires_in": 3600 } });
        let (endpoint, request) = token_server(json!({ "access_token": "new", "refresh_token": "rotated", "token_type": "Bearer", "expires_in": 3600 })).await;
        let updated = refresh(app, &endpoint, &saved).await.unwrap().unwrap();
        let form = fields(&request.await.unwrap());
        assert_eq!(form["refresh_token"], "old-refresh");
        assert_eq!(form["client_secret"], "secret");
        assert!(!form.contains_key("scope") && !form.contains_key("resource"));
        assert_eq!(updated["tokens"]["refresh_token"], "rotated");
        assert_eq!(updated["tokens"]["scope"], "read");
        assert!(refresh(app, "http://127.0.0.1:1", &updated).await.unwrap().is_none(), "a fresh token makes no network request");
        let (endpoint, _) = token_server(json!({ "access_token": "new", "token_type": "Bearer", "expires_in": 3600 })).await;
        assert_eq!(refresh(app, &endpoint, &saved).await.unwrap().unwrap()["tokens"]["refresh_token"], "old-refresh");
        let (endpoint, _) = token_server(json!({ "error": "invalid_grant", "error_description": "secret old-refresh" })).await;
        assert_eq!(refresh(app, &endpoint, &saved).await.unwrap_err(), "The sign-in expired. Sign in again.");
    }
}
