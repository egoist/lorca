//! Local websocket for the app. JSON requests `{ id, method, params }` get `{ id, result }` or
//! `{ id, error }`; events arrive as `{ event, data }`.
//!
//! Only local programs get in. A browser lets any page open a websocket to 127.0.0.1 and leaves
//! the check to the server, so every route refuses a request with an `Origin` (native clients
//! send none) or a `Host` other than this port on loopback (a DNS-rebound page's), and `/ws`
//! takes only `Authorization: Bearer <token>`, the token in `serve-token` in the data directory.

use std::io::Write;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Request as HttpRequest, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use subtle::ConstantTimeEq;
use tokio::sync::mpsc;

use crate::app::App;
use crate::config::Config;
use crate::events::Event;
use crate::api::dispatch;

#[derive(Debug, Deserialize)]
struct Request {
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Clone)]
struct Server {
    app: Arc<App>,
    port: u16,
    token: Arc<str>,
}

pub async fn serve(app: Arc<App>, ready_stdout: bool) -> anyhow::Result<()> {
    let addr: SocketAddr = ([127, 0, 0, 1], app.config.port).into();
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("cannot bind {addr}: {e}. Is another lorca serve running?"))?;
    let port = listener.local_addr()?.port();
    // Read or made once the port is ours, so a second serve that fails to bind never replaces it.
    let token = token(&app.config)?;
    tracing::info!(%addr, "lorca serve");
    if ready_stdout {
        // The parent connects only after this record. Logs go to stderr so stdout is a
        // machine-readable startup channel, independent of tracing filters and formatting.
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{}", json!({ "event": "ready", "port": port }))?;
        stdout.flush()?;
    }
    let server = Server { app, port, token: token.into() };
    let router = Router::new()
        .route("/", get(index))
        .route("/ws", get(upgrade))
        .layer(middleware::from_fn_with_state(server.clone(), local_only))
        .with_state(server);
    axum::serve(listener, router).await?;
    Ok(())
}

/// The token clients send: the one this data directory has, or a new one, kept for later starts.
fn token(config: &Config) -> anyhow::Result<String> {
    if let Some(token) = config.serve_token() {
        return Ok(token);
    }
    let token = crate::keys::b64(&crate::keys::random_32());
    crate::config::write_private(&config.serve_token_path(), token.as_bytes())?;
    Ok(token)
}

async fn local_only(State(server): State<Server>, request: HttpRequest, next: Next) -> Response {
    if let Err(reason) = check_caller(request.headers(), server.port) {
        // Debug: a page can send these as often as it likes.
        tracing::debug!(path = %request.uri().path(), "refused a request: {reason}");
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(request).await
}

/// What a browser's request carries and a local client's does not.
fn check_caller(headers: &HeaderMap, port: u16) -> Result<(), &'static str> {
    if headers.contains_key(header::ORIGIN) {
        return Err("it has an Origin, as a web page's does");
    }
    let host = headers.get(header::HOST).and_then(|host| host.to_str().ok()).unwrap_or_default();
    if ![format!("127.0.0.1:{port}"), format!("localhost:{port}")].iter().any(|local| host.eq_ignore_ascii_case(local)) {
        return Err("its Host is not this port on loopback");
    }
    Ok(())
}

fn check_token(headers: &HeaderMap, token: &str) -> bool {
    let sent = headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok()).and_then(|value| value.strip_prefix("Bearer ")).unwrap_or_default();
    sent.as_bytes().ct_eq(token.as_bytes()).into()
}

async fn index() -> impl IntoResponse {
    "lorca"
}

async fn upgrade(State(server): State<Server>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    if !check_token(&headers, &server.token) {
        tracing::warn!("refused a websocket without this data directory's token");
        return StatusCode::UNAUTHORIZED.into_response();
    }
    ws.on_upgrade(move |socket| connection(server.app, socket))
}

async fn connection(app: Arc<App>, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let (out_tx, mut out_rx) = mpsc::channel::<String>(256);
    let mut events = app.events.subscribe();

    let writer = tokio::spawn(async move {
        while let Some(text) = out_rx.recv().await {
            if sink.send(WsMessage::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    loop {
        tokio::select! {
            incoming = stream.next() => {
                let Some(Ok(message)) = incoming else { break };
                let text = match message {
                    WsMessage::Text(text) => text.to_string(),
                    WsMessage::Close(_) => break,
                    _ => continue,
                };
                let request: Request = match serde_json::from_str(&text) {
                    Ok(request) => request,
                    Err(error) => {
                        let _ = out_tx.send(json!({ "id": null, "error": { "message": format!("bad request: {error}") } }).to_string()).await;
                        continue;
                    }
                };
                let app = app.clone();
                let out = out_tx.clone();
                tokio::spawn(async move {
                    let response = match dispatch(&app, &request.method, request.params).await {
                        Ok(result) => json!({ "id": request.id, "result": result }),
                        Err(message) => json!({ "id": request.id, "error": { "message": message } }),
                    };
                    let _ = out.send(response.to_string()).await;
                });
            }
            event = events.recv() => {
                match event {
                    Ok(event) => {
                        if let Ok(text) = serde_json::to_string(&event) {
                            if out_tx.send(text).await.is_err() { break; }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let _ = out_tx.send(serde_json::to_string(&Event::Snapshot(app.snapshot())).unwrap_or_default()).await;
                    }
                    Err(_) => break,
                }
            }
        }
    }
    writer.abort();
    // The app that said what it was watching is gone.
    app.set_watched_chat(None);
}
