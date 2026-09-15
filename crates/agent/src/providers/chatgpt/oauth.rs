//! ChatGPT sign-in: OAuth authorization code with PKCE and a localhost callback, the same flow
//! the Codex CLI uses. Runs on the Runner; tokens never leave it.

use base64::Engine;
use rand::RngCore;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::ChatGptTokens;

pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const ISSUER: &str = "https://auth.openai.com";
pub const CALLBACK_PORT: u16 = 1455;
pub const SCOPES: &str = "openid profile email offline_access";

fn redirect_uri() -> String {
    format!("http://localhost:{CALLBACK_PORT}/auth/callback")
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
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

    pub fn authorize_url(&self) -> String {
        let params = [
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", &redirect_uri()),
            ("scope", SCOPES),
            ("code_challenge", &self.challenge),
            ("code_challenge_method", "S256"),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("state", &self.state),
        ];
        let query = params
            .iter()
            .map(|(key, value)| format!("{key}={}", urlencode(value)))
            .collect::<Vec<_>>()
            .join("&");
        format!("{ISSUER}/oauth/authorize?{query}")
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
            b'%' if i + 2 < bytes.len() + 1 && i + 2 <= bytes.len() - 1 + 1 => {
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

/// Waits for the browser to hit the localhost callback and returns the authorization code.
pub async fn wait_for_callback(expected_state: &str, timeout: std::time::Duration) -> Result<String, String> {
    let listener = TcpListener::bind(("127.0.0.1", CALLBACK_PORT))
        .await
        .map_err(|e| format!("Cannot listen on port {CALLBACK_PORT}: {e}"))?;

    let accept = async {
        loop {
            let (mut socket, _) = listener.accept().await.map_err(|e| e.to_string())?;
            let mut buffer = vec![0u8; 8192];
            let read = socket.read(&mut buffer).await.map_err(|e| e.to_string())?;
            let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
            let path = request.lines().next().and_then(|line| line.split_whitespace().nth(1)).unwrap_or("/");

            if !path.starts_with("/auth/callback") {
                let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").await;
                continue;
            }

            let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
            let mut code = None;
            let mut state = None;
            let mut error = None;
            for pair in query.split('&') {
                let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                match key {
                    "code" => code = Some(urldecode(value)),
                    "state" => state = Some(urldecode(value)),
                    "error" => error = Some(urldecode(value)),
                    _ => {}
                }
            }

            let (status, body) = if error.is_some() || state.as_deref() != Some(expected_state) || code.is_none() {
                ("400 Bad Request", "<html><body style=\"font-family:-apple-system\"><h2>Sign-in failed</h2><p>Go back to Tinybot and try again.</p></body></html>")
            } else {
                ("200 OK", "<html><body style=\"font-family:-apple-system\"><h2>Signed in to ChatGPT</h2><p>You can close this window and return to Tinybot.</p></body></html>")
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;

            if let Some(error) = error {
                return Err(format!("Sign-in was denied: {error}"));
            }
            if state.as_deref() != Some(expected_state) {
                return Err("Sign-in state mismatch".to_string());
            }
            if let Some(code) = code {
                return Ok(code);
            }
        }
    };

    tokio::time::timeout(timeout, accept).await.map_err(|_| "Timed out waiting for the browser".to_string())?
}

/// Swap an authorization code for tokens.
pub async fn exchange_code(client: &reqwest::Client, code: &str, verifier: &str) -> Result<ChatGptTokens, String> {
    let response = client
        .post(format!("{ISSUER}/oauth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &redirect_uri()),
            ("client_id", CLIENT_ID),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .map_err(|e| format!("Token request failed: {e}"))?;
    tokens_from_response(response).await
}

/// Refresh an expired access token.
pub async fn refresh(client: &reqwest::Client, refresh_token: &str) -> Result<ChatGptTokens, String> {
    let response = client
        .post(format!("{ISSUER}/oauth/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
        ])
        .send()
        .await
        .map_err(|e| format!("Token refresh failed: {e}"))?;
    tokens_from_response(response).await
}

async fn tokens_from_response(response: reqwest::Response) -> Result<ChatGptTokens, String> {
    let status = response.status();
    let body: Value = response.json().await.map_err(|e| format!("Token response unreadable: {e}"))?;
    if !status.is_success() {
        let message = body["error_description"].as_str().or(body["error"].as_str()).unwrap_or("Token request rejected");
        return Err(format!("{status}: {message}"));
    }
    let access_token = body["access_token"].as_str().ok_or("Token response has no access_token")?.to_string();
    let refresh_token = body["refresh_token"].as_str().ok_or("Token response has no refresh_token")?.to_string();
    let id_token = body["id_token"].as_str().map(str::to_string);
    let expires_in = body["expires_in"].as_u64().unwrap_or(3600);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

    let claims = id_token.as_deref().and_then(jwt_claims).or_else(|| jwt_claims(&access_token)).unwrap_or(Value::Null);
    let account_id = claims["https://api.openai.com/auth"]["chatgpt_account_id"]
        .as_str()
        .or(claims["chatgpt_account_id"].as_str())
        .ok_or("Sign-in token carries no ChatGPT account id")?
        .to_string();
    let email = claims["email"].as_str().map(str::to_string);

    Ok(ChatGptTokens { access_token, refresh_token, id_token, account_id, email, expires_at: now + expires_in })
}

/// Decodes the payload of a JWT without verifying it. The relay never sees these; the only
/// consumer is this Runner picking its account id out of a token it just received over TLS.
pub fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Full interactive sign-in: opens the browser (through `open_url`), waits for the callback,
/// exchanges the code.
pub async fn login(
    client: &reqwest::Client,
    open_url: impl FnOnce(&str) -> Result<(), String>,
    timeout: std::time::Duration,
) -> Result<ChatGptTokens, String> {
    let flow = PkceFlow::new();
    let url = flow.authorize_url();
    let state = flow.state.clone();
    // Bind before the browser opens so the redirect never races the listener.
    let callback = tokio::spawn(async move { wait_for_callback(&state, timeout).await });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    open_url(&url)?;
    let code = callback.await.map_err(|e| e.to_string())??;
    exchange_code(client, &code, &flow.verifier).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_carries_pkce() {
        let flow = PkceFlow::new();
        let url = flow.authorize_url();
        assert!(url.starts_with("https://auth.openai.com/oauth/authorize?"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains(&format!("code_challenge={}", flow.challenge)));
    }

    #[test]
    fn decodes_query() {
        assert_eq!(urldecode("a%20b+c"), "a b c");
    }
}
