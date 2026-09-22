//! Local websocket for the app. JSON requests `{ id, method, params }` get `{ id, result }` or
//! `{ id, error }`; events arrive as `{ event, data }`.

use std::io::Write;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::app::App;
use crate::events::Event;
use crate::api::dispatch;

#[derive(Debug, Deserialize)]
struct Request {
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

pub async fn serve(app: Arc<App>, ready_stdout: bool) -> anyhow::Result<()> {
    let addr: SocketAddr = ([127, 0, 0, 1], app.config.port).into();
    let router = Router::new()
        .route("/", get(index))
        .route("/ws", get(upgrade))
        .with_state(app.clone());
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("cannot bind {addr}: {e}. Is another lorca serve running?"))?;
    tracing::info!(%addr, "lorca serve");
    if ready_stdout {
        // The parent connects only after this record. Logs go to stderr so stdout is a
        // machine-readable startup channel, independent of tracing filters and formatting.
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{}", json!({ "event": "ready", "port": listener.local_addr()?.port() }))?;
        stdout.flush()?;
    }
    axum::serve(listener, router).await?;
    Ok(())
}

async fn index() -> impl IntoResponse {
    "lorca"
}

async fn upgrade(State(app): State<Arc<App>>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| connection(app, socket))
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
