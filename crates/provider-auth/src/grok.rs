//! Grok sign-in: OAuth 2.0 authorization code with PKCE against `auth.x.ai`, the flow xAI's own
//! Grok CLI runs, on a loopback callback with a port chosen at sign-in. A Device runs the flow
//! and stores the tokens in the account's encrypted credentials.

use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// Tokens from a Grok sign-in. Devices carry them in the account's encrypted credentials.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GrokTokens {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default)]
    pub id_token: Option<String>,
    /// The account's `sub`, when the sign-in named it.
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    /// Unix seconds.
    pub expires_at: u64,
}

impl GrokTokens {
    /// Access tokens last about six hours; one within five minutes of its end is refreshed
    /// before it is used.
    pub fn is_expired(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now + 5 * 60 >= self.expires_at
    }
}

/// xAI's public desktop client, the one the Grok CLI and other agents sign in with. It has no
/// secret; PKCE stands in for one.
pub const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
pub const ISSUER: &str = "https://auth.x.ai";
/// `grok-cli:access` and `api:access` are what let the token call `api.x.ai`.
pub const SCOPES: &str = "openid profile email offline_access grok-cli:access api:access";
/// Attribution xAI asks clients to send on the authorize URL.
pub const REFERRER: &str = "lorca";

struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// The authorize and token endpoints, from the issuer (`auth.x.ai`, or a test server).
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub issuer: String,
}

impl Endpoints {
    pub fn xai() -> Self {
        Endpoints { issuer: ISSUER.to_string() }
    }

    pub fn at(issuer: &str) -> Self {
        Endpoints { issuer: issuer.trim_end_matches('/').to_string() }
    }

    pub fn authorize(&self) -> String {
        format!("{}/oauth2/authorize", self.issuer)
    }

    pub fn token(&self) -> String {
        format!("{}/oauth2/token", self.issuer)
    }

    pub fn userinfo(&self) -> String {
        format!("{}/oauth2/userinfo", self.issuer)
    }

    pub fn revoke(&self) -> String {
        format!("{}/oauth2/revoke", self.issuer)
    }
}

/// One sign-in attempt.
pub struct PkceFlow {
    pub verifier: String,
    pub challenge: String,
    pub state: String,
}

impl PkceFlow {
    pub fn new() -> Self {
        let mut verifier_bytes = [0u8; 64];
        rand::thread_rng().fill_bytes(&mut verifier_bytes);
        let verifier = b64url(&verifier_bytes);
        let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
        let mut state_bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut state_bytes);
        PkceFlow { verifier, challenge, state: b64url(&state_bytes) }
    }

    pub fn authorize_url(&self, endpoints: &Endpoints, redirect_uri: &str) -> String {
        let params = [
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", redirect_uri),
            ("scope", SCOPES),
            ("code_challenge", &self.challenge),
            ("code_challenge_method", "S256"),
            ("state", &self.state),
            ("referrer", REFERRER),
        ];
        let query = params
            .iter()
            .map(|(key, value)| format!("{key}={}", urlencode(value)))
            .collect::<Vec<_>>()
            .join("&");
        format!("{}?{query}", endpoints.authorize())
    }
}

impl Default for PkceFlow {
    fn default() -> Self {
        Self::new()
    }
}

fn urlencode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn urldecode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() + 1 => {
                let hex = std::str::from_utf8(&bytes[i + 1..(i + 3).min(bytes.len())]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) if hex.len() == 2 => {
                        out.push(byte);
                        i += 3;
                    }
                    _ => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The page xAI lands the browser on after consent. It calls the loopback callback from
/// JavaScript, cross-origin, so the callback has to answer its CORS preflight and mark its
/// response for this origin; otherwise the page cannot read the answer and falls back to
/// showing a code to paste by hand.
pub const ACCOUNTS_ORIGIN: &str = "https://accounts.x.ai";

/// A loopback listener for the callback, bound before the browser opens so the redirect never
/// races it. The port is whatever was free: xAI's client allows any loopback port (RFC 8252).
pub struct Callback {
    listener: TcpListener,
}

/// One HTTP request head, as much of it as the callback needs.
struct Head {
    method: String,
    target: String,
    origin: Option<String>,
    requested_method: Option<String>,
}

fn parse_head(request: &str) -> Option<Head> {
    let mut lines = request.lines();
    let line = lines.next()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let mut origin = None;
    let mut requested_method = None;
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else { continue };
        match name.trim().to_ascii_lowercase().as_str() {
            "origin" => origin = Some(value.trim().to_string()),
            "access-control-request-method" => requested_method = Some(value.trim().to_string()),
            _ => {}
        }
    }
    Some(Head { method, target, origin, requested_method })
}

fn is_callback_path(target: &str) -> bool {
    target == "/callback" || target.starts_with("/callback?")
}

fn html_response(status: &str, body: &str, cors: bool) -> String {
    let cors = if cors { format!("Access-Control-Allow-Origin: {ACCOUNTS_ORIGIN}\r\nVary: Origin\r\n") } else { String::new() };
    format!(
        "HTTP/1.1 {status}\r\n{cors}Content-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn preflight_response() -> String {
    format!(
        "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: {ACCOUNTS_ORIGIN}\r\nAccess-Control-Allow-Methods: GET\r\n\
         Access-Control-Allow-Private-Network: true\r\n\
         Vary: Origin, Access-Control-Request-Method, Access-Control-Request-Private-Network\r\n\
         Content-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

const SUCCESS_PAGE: &str = "<html><body style=\"font-family:-apple-system\"><h2>Signed in to Grok</h2><p>You can close this window and return to Lorca.</p></body></html>";
const FAILURE_PAGE: &str = "<html><body style=\"font-family:-apple-system\"><h2>Sign-in failed</h2><p>Go back to Lorca and try again.</p></body></html>";
const NOT_FOUND: &str = "<!doctype html><title>Not found</title>Not found.";

/// Serves one connection: the preflight, the callback itself, or something unrelated (a
/// speculative preconnect, a favicon). Sends the callback's outcome, if it was one.
async fn serve(mut socket: tokio::net::TcpStream, expected_state: String, results: mpsc::Sender<Result<String, String>>) {
    // A browser may open a connection and never speak on it; give it a bounded wait.
    let mut buffer = vec![0u8; 16 * 1024];
    let mut read = 0;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let n = match tokio::time::timeout_at(deadline, socket.read(&mut buffer[read..])).await {
            Ok(Ok(n)) => n,
            _ => return,
        };
        if n == 0 {
            return;
        }
        read += n;
        if buffer[..read].windows(4).any(|w| w == b"\r\n\r\n") || read == buffer.len() {
            break;
        }
    }
    let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
    let Some(head) = parse_head(&request) else {
        let _ = socket.write_all(html_response("400 Bad Request", NOT_FOUND, false).as_bytes()).await;
        return;
    };
    let origin_allowed = head.origin.as_deref() == Some(ACCOUNTS_ORIGIN);

    if head.method == "OPTIONS" {
        let valid = origin_allowed && head.requested_method.as_deref().is_some_and(|m| m.eq_ignore_ascii_case("GET")) && is_callback_path(&head.target);
        let response = if valid { preflight_response() } else { html_response("404 Not Found", NOT_FOUND, false) };
        let _ = socket.write_all(response.as_bytes()).await;
        let _ = socket.shutdown().await;
        return;
    }
    if head.method != "GET" || !is_callback_path(&head.target) || (head.origin.is_some() && !origin_allowed) {
        let _ = socket.write_all(html_response("404 Not Found", NOT_FOUND, false).as_bytes()).await;
        let _ = socket.shutdown().await;
        return;
    }

    let query = head.target.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "code" => code = Some(urldecode(value)),
            "state" => state = Some(urldecode(value)),
            "error_description" => error = Some(urldecode(value)),
            "error" if error.is_none() => error = Some(urldecode(value)),
            _ => {}
        }
    }

    let outcome = if let Some(error) = error {
        Err(format!("Sign-in was denied: {error}"))
    } else if state.as_deref() != Some(expected_state.as_str()) {
        Err("Sign-in state mismatch".to_string())
    } else if let Some(code) = code {
        Ok(code)
    } else {
        Err("The callback carried no authorization code".to_string())
    };
    let response = match &outcome {
        Ok(_) => html_response("200 OK", SUCCESS_PAGE, origin_allowed),
        Err(_) => html_response("400 Bad Request", FAILURE_PAGE, origin_allowed),
    };
    let _ = socket.write_all(response.as_bytes()).await;
    let _ = socket.shutdown().await;
    let _ = results.send(outcome).await;
}

impl Callback {
    pub async fn bind() -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| format!("Cannot listen for the sign-in callback: {e}"))?;
        Ok(Callback { listener })
    }

    pub fn redirect_uri(&self) -> String {
        let port = self.listener.local_addr().map(|a| a.port()).unwrap_or(0);
        format!("http://127.0.0.1:{port}/callback")
    }

    /// Waits for the browser to hit the callback and returns the authorization code. Every
    /// connection is served on its own, so an idle one never holds up the real redirect.
    pub async fn wait(self, expected_state: &str, timeout: std::time::Duration) -> Result<String, String> {
        let listener = self.listener;
        let (results, mut outcomes) = mpsc::channel::<Result<String, String>>(4);
        let expected = expected_state.to_string();
        let accepting = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else { return };
                tokio::spawn(serve(socket, expected.clone(), results.clone()));
            }
        });
        let outcome = tokio::time::timeout(timeout, outcomes.recv()).await;
        accepting.abort();
        match outcome {
            Ok(Some(result)) => result,
            Ok(None) => Err("The sign-in callback closed".to_string()),
            Err(_) => Err("Timed out waiting for the browser".to_string()),
        }
    }
}

/// Swap an authorization code for tokens.
pub async fn exchange_code(client: &reqwest::Client, endpoints: &Endpoints, code: &str, verifier: &str, redirect_uri: &str) -> Result<GrokTokens, String> {
    let response = client
        .post(endpoints.token())
        .header("Accept", "application/json")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", CLIENT_ID),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .map_err(|e| format!("Token request failed: {e}"))?;
    let mut tokens = tokens_from_response(response, None).await?;
    fill_identity(client, endpoints, &mut tokens).await;
    Ok(tokens)
}

/// Refresh an expired access token. xAI rotates the refresh token: the answer carries a new
/// one to store, or none, in which case the old one stays good.
pub async fn refresh(client: &reqwest::Client, endpoints: &Endpoints, refresh_token: &str) -> Result<GrokTokens, String> {
    let response = client
        .post(endpoints.token())
        .header("Accept", "application/json")
        .form(&[("grant_type", "refresh_token"), ("refresh_token", refresh_token), ("client_id", CLIENT_ID)])
        .send()
        .await
        .map_err(|e| format!("Token refresh failed: {e}"))?;
    tokens_from_response(response, Some(refresh_token)).await
}

/// Tells xAI the sign-in is over. Best effort: a failure changes nothing locally.
pub async fn revoke(client: &reqwest::Client, endpoints: &Endpoints, refresh_token: &str) -> Result<(), String> {
    let response = client
        .post(endpoints.revoke())
        .form(&[("token", refresh_token), ("client_id", CLIENT_ID)])
        .send()
        .await
        .map_err(|e| format!("Revoke request failed: {e}"))?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("Revoke answered {}", response.status()))
    }
}

async fn tokens_from_response(response: reqwest::Response, previous_refresh: Option<&str>) -> Result<GrokTokens, String> {
    let status = response.status();
    let text = response.text().await.map_err(|e| format!("Token response unreadable: {e}"))?;
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !status.is_success() {
        let message = body["error_description"].as_str().or(body["error"].as_str()).unwrap_or("Token request rejected");
        if status == reqwest::StatusCode::FORBIDDEN || message.contains("subscription") || message.contains("not eligible") {
            return Err(format!("{status}: {message}. Grok sign-in needs a SuperGrok or X Premium+ subscription."));
        }
        return Err(format!("{status}: {message}"));
    }
    let access_token = body["access_token"].as_str().ok_or("Token response has no access_token")?.to_string();
    let refresh_token = body["refresh_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| previous_refresh.map(str::to_string))
        .ok_or("Token response has no refresh_token")?;
    let id_token = body["id_token"].as_str().map(str::to_string);
    let expires_in = body["expires_in"].as_u64().unwrap_or(6 * 3600);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

    let claims = id_token.as_deref().and_then(jwt_claims).or_else(|| jwt_claims(&access_token)).unwrap_or(Value::Null);
    let account_id = claims["sub"].as_str().map(str::to_string);
    let email = claims["email"].as_str().map(str::to_string);

    Ok(GrokTokens { access_token, refresh_token, id_token, account_id, email, expires_at: now + expires_in })
}

/// Fills the account id and email from the userinfo endpoint when the tokens did not carry
/// them. Best effort: the sign-in stands without a name to show.
async fn fill_identity(client: &reqwest::Client, endpoints: &Endpoints, tokens: &mut GrokTokens) {
    if tokens.account_id.is_some() && tokens.email.is_some() {
        return;
    }
    let Ok(response) = client.get(endpoints.userinfo()).bearer_auth(&tokens.access_token).header("Accept", "application/json").send().await else { return };
    if !response.status().is_success() {
        return;
    }
    let Ok(info) = response.json::<Value>().await else { return };
    if tokens.account_id.is_none() {
        tokens.account_id = info["sub"].as_str().map(str::to_string);
    }
    if tokens.email.is_none() {
        tokens.email = info["email"].as_str().or(info["preferred_username"].as_str()).or(info["name"].as_str()).map(str::to_string);
    }
}

/// Decodes the payload of a JWT without verifying it. The Device is picking the account's id
/// and email out of a token it just received over TLS.
pub fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Full interactive sign-in: binds the callback, opens the browser (through `open_url`), waits
/// for the redirect, exchanges the code.
pub async fn login(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    open_url: impl FnOnce(&str) -> Result<(), String>,
    timeout: std::time::Duration,
) -> Result<GrokTokens, String> {
    let flow = PkceFlow::new();
    let callback = Callback::bind().await?;
    let redirect_uri = callback.redirect_uri();
    let url = flow.authorize_url(endpoints, &redirect_uri);
    let state = flow.state.clone();
    let waiter = tokio::spawn(async move { callback.wait(&state, timeout).await });
    let _abort_on_drop = AbortOnDrop(waiter.abort_handle());
    open_url(&url)?;
    let code = waiter.await.map_err(|e| e.to_string())??;
    exchange_code(client, endpoints, &code, &flow.verifier, &redirect_uri).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_carries_pkce_and_the_loopback_redirect() {
        let flow = PkceFlow::new();
        let url = flow.authorize_url(&Endpoints::xai(), "http://127.0.0.1:53211/callback");
        assert!(url.starts_with("https://auth.x.ai/oauth2/authorize?"));
        assert!(url.contains(&format!("client_id={CLIENT_ID}")));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A53211%2Fcallback"));
        assert!(url.contains("scope=openid%20profile%20email%20offline_access%20grok-cli%3Aaccess%20api%3Aaccess"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains(&format!("code_challenge={}", flow.challenge)));
        assert!(url.contains("referrer=lorca"));
    }

    #[test]
    fn decodes_query() {
        assert_eq!(urldecode("a%20b+c"), "a b c");
    }

    #[tokio::test]
    async fn the_callback_binds_a_free_port() {
        let callback = Callback::bind().await.unwrap();
        let uri = callback.redirect_uri();
        assert!(uri.starts_with("http://127.0.0.1:"));
        assert!(uri.ends_with("/callback"));
        assert_ne!(uri, "http://127.0.0.1:0/callback");
    }
}

#[cfg(test)]
mod flow_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A one-endpoint HTTP server: answers every request with `body` as JSON and records the
    /// request lines and bodies it saw.
    async fn fake_issuer(body: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                let log = log.clone();
                tokio::spawn(async move {
                    let mut buffer = vec![0u8; 16384];
                    let mut read = 0;
                    loop {
                        let n = socket.read(&mut buffer[read..]).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        read += n;
                        let text = String::from_utf8_lossy(&buffer[..read]);
                        if let Some((head, tail)) = text.split_once("\r\n\r\n") {
                            let length = head
                                .lines()
                                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                                .unwrap_or(0);
                            if tail.len() >= length {
                                break;
                            }
                        }
                    }
                    log.lock().unwrap().push(String::from_utf8_lossy(&buffer[..read]).into_owned());
                    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        (issuer, seen)
    }

    #[tokio::test]
    async fn login_opens_the_browser_takes_the_callback_and_exchanges_the_code() {
        let (issuer, seen) = fake_issuer(r#"{"access_token":"at","refresh_token":"rt","expires_in":21600,"id_token":"x.eyJzdWIiOiJ1c2VyXzEiLCJlbWFpbCI6Im1lQHguYWkifQ.y"}"#).await;
        let endpoints = Endpoints::at(&issuer);
        let client = reqwest::Client::new();
        let opened = Arc::new(Mutex::new(String::new()));
        let record = opened.clone();
        let tokens = login(
            &client,
            &endpoints,
            move |url| {
                *record.lock().unwrap() = url.to_string();
                // The "browser": follow the redirect straight back to the callback.
                let redirect = url.split("redirect_uri=").nth(1).unwrap().split('&').next().unwrap();
                let redirect = urldecode(redirect);
                let state = url.split("state=").nth(1).unwrap().split('&').next().unwrap().to_string();
                tokio::spawn(async move {
                    let _ = reqwest::get(format!("{redirect}?code=abc123&state={state}")).await;
                });
                Ok(())
            },
            std::time::Duration::from_secs(10),
        )
        .await
        .unwrap();

        assert_eq!((tokens.access_token.as_str(), tokens.refresh_token.as_str()), ("at", "rt"));
        assert_eq!(tokens.account_id.as_deref(), Some("user_1"));
        assert_eq!(tokens.email.as_deref(), Some("me@x.ai"));
        assert!(opened.lock().unwrap().starts_with(&format!("{issuer}/oauth2/authorize?")));
        let requests = seen.lock().unwrap().clone();
        let exchange = requests.iter().find(|r| r.starts_with("POST /oauth2/token")).expect("a token exchange");
        assert!(exchange.contains("grant_type=authorization_code"));
        assert!(exchange.contains("code=abc123"));
        assert!(exchange.contains("code_verifier="));
        assert!(exchange.contains(&format!("client_id={CLIENT_ID}")));
        assert!(exchange.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A"));
    }

    /// Sends raw bytes to the callback and returns the whole response.
    async fn raw(uri: &str, request: String) -> String {
        let addr = uri.trim_start_matches("http://").split('/').next().unwrap().to_string();
        let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
        socket.write_all(request.as_bytes()).await.unwrap();
        let mut out = Vec::new();
        socket.read_to_end(&mut out).await.unwrap();
        String::from_utf8_lossy(&out).into_owned()
    }

    #[tokio::test]
    async fn the_callback_answers_the_accounts_page_across_origins_and_survives_an_idle_preconnect() {
        let callback = Callback::bind().await.unwrap();
        let uri = callback.redirect_uri();
        let waiter = tokio::spawn(async move { callback.wait("s1", std::time::Duration::from_secs(10)).await });
        // Chrome's speculative connection: opened, never spoken on. It must not block the rest.
        let addr = uri.trim_start_matches("http://").split('/').next().unwrap().to_string();
        let _idle = tokio::net::TcpStream::connect(addr).await.unwrap();
        // The page's preflight.
        let preflight = raw(&uri, "OPTIONS /callback?code=c&state=s1 HTTP/1.1\r\nHost: x\r\nOrigin: https://accounts.x.ai\r\nAccess-Control-Request-Method: GET\r\nAccess-Control-Request-Private-Network: true\r\n\r\n".into()).await;
        assert!(preflight.starts_with("HTTP/1.1 204"), "{preflight}");
        assert!(preflight.contains("Access-Control-Allow-Origin: https://accounts.x.ai\r\n"));
        assert!(preflight.contains("Access-Control-Allow-Private-Network: true\r\n"));
        // An unrelated request from somewhere else.
        let other = raw(&uri, "GET /favicon.ico HTTP/1.1\r\nHost: x\r\n\r\n".into()).await;
        assert!(other.starts_with("HTTP/1.1 404"));
        // The callback itself, fetched by the page.
        let done = raw(&uri, "GET /callback?code=c&state=s1 HTTP/1.1\r\nHost: x\r\nOrigin: https://accounts.x.ai\r\n\r\n".into()).await;
        assert!(done.starts_with("HTTP/1.1 200"), "{done}");
        assert!(done.contains("Access-Control-Allow-Origin: https://accounts.x.ai\r\n"));
        assert!(done.contains("Signed in to Grok"));
        assert_eq!(waiter.await.unwrap().unwrap(), "c");
    }

    #[tokio::test]
    async fn a_denied_sign_in_fails_the_wait() {
        let callback = Callback::bind().await.unwrap();
        let uri = callback.redirect_uri();
        let waiter = tokio::spawn(async move { callback.wait("s1", std::time::Duration::from_secs(10)).await });
        let denied = raw(&uri, "GET /callback?error=access_denied&error_description=nope&state=s1 HTTP/1.1\r\nHost: x\r\n\r\n".into()).await;
        assert!(denied.starts_with("HTTP/1.1 400"));
        assert!(!denied.contains("Access-Control-Allow-Origin"));
        assert_eq!(waiter.await.unwrap().unwrap_err(), "Sign-in was denied: nope");
    }

    #[tokio::test]
    async fn refresh_keeps_the_old_refresh_token_when_none_comes_back() {
        let (issuer, seen) = fake_issuer(r#"{"access_token":"at2","expires_in":21600}"#).await;
        let tokens = refresh(&reqwest::Client::new(), &Endpoints::at(&issuer), "rt-old").await.unwrap();
        assert_eq!((tokens.access_token.as_str(), tokens.refresh_token.as_str()), ("at2", "rt-old"));
        let request = seen.lock().unwrap()[0].clone();
        assert!(request.contains("grant_type=refresh_token"));
        assert!(request.contains("refresh_token=rt-old"));
    }
}
