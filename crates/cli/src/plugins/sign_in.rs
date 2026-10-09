//! A plugin's browser sign-in, finished on whichever Device the user is at. The Runner holds the
//! authorization (`mcp::connect_oauth_for_card`): the client, PKCE, and the token exchange. The
//! Device that asked listens on its own loopback, opens the page there (a phone hands it to its
//! in-app browser, which keeps the core alive), and sends the Runner where the browser landed,
//! sealed like every request.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::app::App;
use crate::events::Event;

/// How long the sign-in page may take, here and on the Runner.
pub const TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// How long the Runner gives a server's discovery and client registration.
pub const SETUP_TIMEOUT: Duration = Duration::from_secs(45);
/// How long a Device waits for the Runner to start a sign-in: the setup and the relay both ways.
pub const START_TIMEOUT: Duration = Duration::from_secs(75);

/// A loopback listener for the browser's redirect, bound before the page opens so the redirect
/// never races it. The port is whatever was free, and the client is registered with it, unless a
/// preregistered client names its own (`bind_fixed`).
pub struct Callback {
    listener: TcpListener,
    /// The path the browser comes back to.
    path: String,
    /// A preregistered client's redirect, sent as written.
    fixed: Option<String>,
}

impl Callback {
    pub async fn bind() -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| format!("Cannot listen for the sign-in callback: {e}"))?;
        Ok(Callback { listener, path: "/callback".into(), fixed: None })
    }

    /// A listener at the redirect a preregistered client was registered with: `url`, plain http to
    /// this computer, or else `http://127.0.0.1:<port>/callback`. A URL without a port takes `port`,
    /// or a free one, as RFC 8252 lets a loopback redirect.
    pub async fn bind_fixed(port: Option<u16>, url: Option<&str>) -> Result<Self, String> {
        let parsed = url.map(reqwest::Url::parse).transpose().map_err(|e| format!("The sign-in's callbackUrl does not read: {e}"))?;
        let port = parsed.as_ref().and_then(reqwest::Url::port).or(port).unwrap_or(0);
        let host = if parsed.as_ref().and_then(|url| url.host_str()) == Some("[::1]") { "::1" } else { "127.0.0.1" };
        let listener = TcpListener::bind((host, port)).await.map_err(|e| format!("Cannot listen for the sign-in callback on port {port}: {e}"))?;
        let bound = listener.local_addr().map(|a| a.port()).unwrap_or(port);
        let (path, fixed) = match (parsed, url) {
            (Some(parsed), Some(written)) if parsed.port().is_some() => (parsed.path().to_string(), Some(written.to_string())),
            (Some(mut parsed), _) => {
                let _ = parsed.set_port(Some(bound));
                (parsed.path().to_string(), Some(parsed.to_string()))
            }
            (None, _) => ("/callback".to_string(), None),
        };
        Ok(Callback { listener, path, fixed })
    }

    pub fn redirect_uri(&self) -> String {
        self.fixed.clone().unwrap_or_else(|| format!("http://127.0.0.1:{}/callback", self.port()))
    }

    fn port(&self) -> u16 {
        self.listener.local_addr().map(|a| a.port()).unwrap_or(0)
    }

    /// Waits for the browser to land on the callback, answers it with a page that says how it
    /// went, and returns the whole URL, a denial too: whoever finishes the sign-in reads the
    /// code from it. Every connection is served on its own, so a browser's idle preconnect never
    /// holds up the redirect.
    pub async fn wait(self, name: &str, timeout: Duration) -> Result<String, String> {
        let origin = reqwest::Url::parse(&self.redirect_uri()).map_err(|_| "The callback URL does not read.")?.origin().ascii_serialization();
        let (listener, path) = (self.listener, self.path);
        let (landed, mut arrivals) = tokio::sync::mpsc::channel::<String>(1);
        let name = escape(name);
        let accepting = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else { return };
                tokio::spawn(serve(socket, origin.clone(), path.clone(), name.clone(), landed.clone()));
            }
        });
        let arrived = tokio::time::timeout(timeout, arrivals.recv()).await;
        accepting.abort();
        match arrived {
            Ok(Some(url)) => Ok(url),
            Ok(None) => Err("The sign-in callback closed".into()),
            Err(_) => Err("Timed out waiting for the browser".into()),
        }
    }
}

/// True for the redirect a Device's `Callback` listens on: plain http to a loopback host on a port of
/// its own, at `/callback`, with no user, query, or fragment, so the code goes nowhere else.
pub fn is_loopback_redirect(uri: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(uri) else { return false };
    url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "localhost"))
        && url.port().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/callback"
        && url.query().is_none()
        && url.fragment().is_none()
}

/// True when the browser came back with an error instead of a code: the user said no.
pub fn denied(callback: &str) -> bool {
    callback.split_once('?').is_some_and(|(_, query)| query.split('&').any(|pair| pair.split('=').next() == Some("error")))
}

/// Serves one connection: the redirect to `expected`, or anything else the browser asks for (a
/// favicon).
async fn serve(mut socket: tokio::net::TcpStream, origin: String, expected: String, name: String, landed: tokio::sync::mpsc::Sender<String>) {
    let mut buffer = vec![0u8; 16 * 1024];
    let mut read = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while read < buffer.len() && !buffer[..read].windows(4).any(|w| w == b"\r\n\r\n") {
        match tokio::time::timeout_at(deadline, socket.read(&mut buffer[read..])).await {
            Ok(Ok(n)) if n > 0 => read += n,
            _ => return,
        }
    }
    let head = String::from_utf8_lossy(&buffer[..read]);
    let path = head.lines().next().and_then(|line| line.split_whitespace().nth(1)).unwrap_or("/").to_string();
    if path != expected && !path.starts_with(&format!("{expected}?")) {
        let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        return;
    }
    let (status, page) = if denied(&path) {
        ("400 Bad Request", "<h2>Sign-in failed</h2><p>Go back to Lorca and try again.</p>".to_string())
    } else {
        ("200 OK", format!("<h2>Signed in to {name}</h2><p>You can close this window and return to Lorca.</p>"))
    };
    let body = format!("<html><body style=\"font-family:-apple-system\">{page}</body></html>");
    let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = socket.write_all(response.as_bytes()).await;
    let _ = socket.shutdown().await;
    let _ = landed.send(format!("{origin}{path}")).await;
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Starts a sign-in on another Runner for the user here: Sign in on a card
/// (`permission.answer`) or on a plugin's sheet (`plugins.connect`). The Runner registers this
/// Device's loopback as the redirect and answers with the page; this Device opens it and hands
/// the Runner where the browser landed (`plugins.sign_in.finish`), or that the page was closed
/// (`plugins.sign_in.cancel`). A Runner that signs in another way, with a device code on the
/// card, answers with no page.
pub async fn from_here(app: &Arc<App>, runner_id: &str, verb: &str, mut body: Value, plugin_id: &str, name: &str) -> Result<Value, String> {
    // A native client may register one fixed callback port (Slack). Bind it on the Device
    // that opens the page, so the Runner still holds the tokens and needs no local browser.
    let detail = crate::requests::ask(app, runner_id, "plugins.detail", json!({ "plugin_id": plugin_id })).await?;
    let auth = detail["servers"].as_array().into_iter().flatten()
        .find(|server| body["server"].as_str().is_none_or(|name| server["name"].as_str() == Some(name)) && server["auth"]["oauth"].as_bool() == Some(true))
        .map(|server| &server["auth"]);
    let port = auth.and_then(|auth| auth["callback_port"].as_u64()).and_then(|port| u16::try_from(port).ok());
    let url = auth.and_then(|auth| auth["callback_url"].as_str()).filter(|url| is_loopback_redirect(url));
    let callback = if port.is_some() || url.is_some() { Callback::bind_fixed(port, url).await? } else { Callback::bind().await? };
    body["redirect_uri"] = json!(callback.redirect_uri());
    let answer = crate::requests::ask_within(app, runner_id, verb, body, START_TIMEOUT).await?;
    let (Some(page), Some(id)) = (answer["url"].as_str().map(str::to_string), answer["sign_in"].as_str().map(str::to_string)) else { return Ok(answer) };
    let cancel = app.begin_plugin_sign_in(&id);
    let (app, runner_id, plugin_id, name) = (app.clone(), runner_id.to_string(), plugin_id.to_string(), name.to_string());
    let opened = open_page(&app, &plugin_id, &id, &page);
    let open = opened.is_ok();
    tokio::spawn(async move {
        let landed = if open {
            tokio::select! {
                landed = callback.wait(&name, TIMEOUT) => Some(landed),
                _ = cancel.cancelled() => None,
            }
        } else {
            None
        };
        app.emit(Event::PluginAuthDone { plugin_id: plugin_id.clone(), sign_in: id.clone() });
        let (verb, body) = match landed {
            Some(Ok(url)) => ("plugins.sign_in.finish", json!({ "plugin_id": plugin_id, "sign_in": id, "url": url })),
            // The Runner stops waiting at the same time.
            Some(Err(_)) => return,
            None => ("plugins.sign_in.cancel", json!({ "plugin_id": plugin_id, "sign_in": id })),
        };
        if let Err(error) = crate::requests::ask(&app, &runner_id, verb, body).await {
            tracing::warn!(%error, plugin = %plugin_id, "finishing a sign-in on its Runner");
        }
    });
    opened?;
    Ok(answer)
}

/// Opens sign-in `id`'s page on this Device: a computer's browser, or a phone's in-app browser
/// through its app.
fn open_page(app: &Arc<App>, plugin_id: &str, id: &str, url: &str) -> Result<(), String> {
    #[cfg(feature = "runner")]
    {
        let _ = (plugin_id, id);
        crate::plugins::mcp::open_browser(app, url)
    }
    #[cfg(not(feature = "runner"))]
    {
        app.emit(Event::PluginAuth { plugin_id: plugin_id.to_string(), sign_in: id.to_string(), url: url.to_string() });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_preregistered_redirect_is_served_where_it_was_registered() {
        // A port of its own: the redirect a client was registered with.
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let callback = Callback::bind_fixed(Some(free), None).await.unwrap();
        assert_eq!(callback.redirect_uri(), format!("http://127.0.0.1:{free}/callback"));
        drop(callback);
        // A whole URL, with a path of its own; one without a port takes a free one.
        let callback = Callback::bind_fixed(None, Some("http://localhost/oauth/done")).await.unwrap();
        let redirect = callback.redirect_uri();
        assert!(redirect.starts_with("http://localhost:") && redirect.ends_with("/oauth/done"), "{redirect}");
        let port = reqwest::Url::parse(&redirect).unwrap().port().unwrap();
        let waiting = tokio::spawn(callback.wait("Docs", Duration::from_secs(5)));
        assert_eq!(reqwest::get(format!("http://127.0.0.1:{port}/callback?code=x")).await.unwrap().status(), 404, "only its own path is the redirect");
        assert_eq!(reqwest::get(format!("http://127.0.0.1:{port}/oauth/done?code=abc&state=s")).await.unwrap().status(), 200);
        assert_eq!(waiting.await.unwrap().unwrap(), format!("{redirect}?code=abc&state=s"), "the registered host and path survive the callback");
        // One written with its port is sent as written.
        let callback = Callback::bind_fixed(None, Some(&format!("http://127.0.0.1:{free}/cb"))).await.unwrap();
        assert_eq!(callback.redirect_uri(), format!("http://127.0.0.1:{free}/cb"));
    }

    #[tokio::test]
    async fn the_callback_hands_back_where_the_browser_landed() {
        let callback = Callback::bind().await.unwrap();
        let redirect = callback.redirect_uri();
        let waiting = tokio::spawn(callback.wait("Docs <Team>", Duration::from_secs(5)));
        // A preconnect that never speaks holds nothing up.
        let idle = tokio::net::TcpStream::connect(redirect.trim_start_matches("http://").trim_end_matches("/callback")).await.unwrap();
        let page = reqwest::get(format!("{redirect}?code=abc&state=xyz")).await.unwrap();
        assert_eq!(page.status(), 200);
        assert!(page.text().await.unwrap().contains("Signed in to Docs &lt;Team&gt;"));
        assert_eq!(waiting.await.unwrap().unwrap(), format!("{redirect}?code=abc&state=xyz"));
        drop(idle);

        assert!(is_loopback_redirect(&redirect));
        assert!(is_loopback_redirect("http://localhost:5555/callback"));
        for elsewhere in [
            "http://127.0.0.1:80@evil.example/callback",
            "http://user@127.0.0.1:5555/callback",
            "http://127.0.0.1.evil.example:5555/callback",
            "https://127.0.0.1:5555/callback",
            "http://127.0.0.1/callback",
            "http://127.0.0.1:5555/elsewhere",
            "http://127.0.0.1:5555/callback?next=https://evil.example",
        ] {
            assert!(!is_loopback_redirect(elsewhere), "{elsewhere}");
        }

        assert!(denied("http://127.0.0.1:1/callback?error=access_denied&state=xyz"));
        assert!(!denied("http://127.0.0.1:1/callback?code=abc&state=error"));
        let callback = Callback::bind().await.unwrap();
        assert_eq!(callback.wait("Docs", Duration::from_millis(50)).await.unwrap_err(), "Timed out waiting for the browser");
    }
}
