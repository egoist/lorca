//! Local websocket for the app. JSON requests `{ id, method, params }` get `{ id, result }` or
//! `{ id, error }`; events arrive as `{ event, data }`.

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
use crate::model::*;
use crate::{identity, pairing, providers, runtime};

#[derive(Debug, Deserialize)]
struct Request {
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

pub async fn serve(app: Arc<App>) -> anyhow::Result<()> {
    let addr: SocketAddr = ([127, 0, 0, 1], app.config.port).into();
    let router = Router::new()
        .route("/", get(index))
        .route("/ws", get(upgrade))
        .with_state(app.clone());
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("cannot bind {addr}: {e}. Is another tinybot serve running?"))?;
    tracing::info!(%addr, "tinybot serve");
    axum::serve(listener, router).await?;
    Ok(())
}

async fn index() -> impl IntoResponse {
    "tinybot"
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
}

fn string(params: &Value, key: &str) -> Result<String, String> {
    params[key].as_str().map(str::to_string).filter(|s| !s.is_empty()).ok_or_else(|| format!("missing {key}"))
}

fn opt_string(params: &Value, key: &str) -> Option<String> {
    params[key].as_str().map(str::to_string).filter(|s| !s.is_empty())
}

pub async fn dispatch(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    match method {
        "hello" => Ok(json!({
            "version": crate::config::VERSION,
            "has_identity": app.has_identity(),
            "is_identity_device": app.is_identity_device(),
            "device_id": app.this_device_id(),
            "relay_url": app.relay_url(),
            "relay_connected": app.relay_connected.load(std::sync::atomic::Ordering::Relaxed),
        })),
        "bootstrap" => {
            runtime::prime_names(app);
            Ok(app.snapshot())
        }

        "identity.create" => {
            let phrase = identity::create(app, opt_string(&params, "device_name")).map_err(|e| e.to_string())?;
            Ok(json!({ "phrase": phrase, "device_id": app.this_device_id() }))
        }
        "identity.restore" => {
            let phrase = string(&params, "phrase")?;
            identity::restore(app, &phrase, opt_string(&params, "device_name")).await.map_err(|e| e.to_string())?;
            Ok(json!({ "device_id": app.this_device_id() }))
        }

        "pair.start" => {
            let (nonce, pairing_string) = pairing::start(app.clone()).await.map_err(|e| e.to_string())?;
            Ok(json!({ "nonce": nonce, "pairing_string": pairing_string }))
        }
        "pair.status" => Ok(pairing::status(app, &string(&params, "nonce")?)),
        "pair.cancel" => {
            pairing::cancel(app, &string(&params, "nonce")?);
            Ok(Value::Null)
        }
        "pair.accept" => {
            let text = string(&params, "pairing_string")?;
            pairing::accept(app.clone(), &text, opt_string(&params, "device_name")).await.map_err(|e| e.to_string())
        }

        "config.set" => {
            if params.get("relay_url").is_some() {
                let url = params["relay_url"].as_str().map(str::to_string);
                app.set_relay_url(url).map_err(|e| e.to_string())?;
            }
            Ok(json!({ "relay_url": app.relay_url() }))
        }

        "bots.create" => {
            let bot = Bot {
                id: opt_string(&params, "id").unwrap_or_default(),
                name: string(&params, "name")?,
                label: opt_string(&params, "label").unwrap_or_else(|| "New bot".into()),
                description: opt_string(&params, "description").unwrap_or_default(),
                symbol_name: opt_string(&params, "symbol_name").unwrap_or_else(|| "sparkles".into()),
                accent: opt_string(&params, "accent").unwrap_or_else(|| "indigo".into()),
                runner_id: string(&params, "runner_id")?,
                provider: opt_string(&params, "provider").unwrap_or_else(|| "deepseek".into()),
                model: opt_string(&params, "model"),
                instructions: opt_string(&params, "instructions").unwrap_or_default(),
                workdir: opt_string(&params, "workdir"),
                created_at: 0.0,
            };
            // Every bot has one direct chat; both land in a single roster change.
            let (bot, chat) = app.create_bot_with_dm(bot, opt_string(&params, "chat_id")).map_err(|e| e.to_string())?;
            runtime::prime_names(app);
            Ok(json!({ "bot": bot, "chat_id": chat.meta.id }))
        }
        "bots.update" => {
            let id = string(&params, "id")?;
            app.update_bot(&id, |bot| {
                if let Some(v) = opt_string(&params, "name") { bot.name = v; }
                if let Some(v) = opt_string(&params, "label") { bot.label = v; }
                if let Some(v) = params["description"].as_str() { bot.description = v.trim().to_string(); }
                if let Some(v) = params["instructions"].as_str() { bot.instructions = v.to_string(); }
                if let Some(v) = opt_string(&params, "provider") { bot.provider = v; }
                if let Some(v) = params["model"].as_str() { bot.model = Some(v.trim().to_string()).filter(|m| !m.is_empty()); }
                if let Some(v) = opt_string(&params, "runner_id") { bot.runner_id = v; }
                if let Some(v) = params["workdir"].as_str() { bot.workdir = Some(v.to_string()).filter(|w| !w.trim().is_empty()); }
            })
            .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }

        "chats.create" => {
            let bot_ids: Vec<String> = serde_json::from_value(params["bot_ids"].clone()).unwrap_or_default();
            let kind = opt_string(&params, "kind").unwrap_or_else(|| "group".into());
            let chat = if kind == "dm" {
                let bot_id = bot_ids.first().cloned().ok_or("missing bot_ids")?;
                app.dm_with(&bot_id, opt_string(&params, "id")).map_err(|e| e.to_string())?
            } else {
                app.create_chat(ChatMeta {
                    id: opt_string(&params, "id").unwrap_or_default(),
                    kind,
                    title: opt_string(&params, "title"),
                    owner_bot_id: opt_string(&params, "owner_bot_id").or_else(|| bot_ids.first().cloned()),
                    bot_ids,
                    is_pinned: false,
                    created_at: 0.0,
                })
                .map_err(|e| e.to_string())?
            };
            Ok(json!({ "chat": chat }))
        }
        "chats.dm" => {
            let chat = app.dm_with(&string(&params, "bot_id")?, opt_string(&params, "id")).map_err(|e| e.to_string())?;
            Ok(json!({ "chat": chat }))
        }
        "chats.send" => {
            runtime::prime_names(app);
            let files: Vec<crate::files::OutgoingFile> = serde_json::from_value(params["attachments"].clone()).unwrap_or_default();
            let mut attachments = Vec::new();
            for file in &files {
                attachments.push(crate::files::store(app, file).map_err(|e| e.to_string())?);
            }
            let text = opt_string(&params, "text").unwrap_or_default();
            let message = runtime::send_user_message(app.clone(), &string(&params, "chat_id")?, &text, opt_string(&params, "message_id"), attachments)
                .map_err(|e| e.to_string())?;
            Ok(json!({ "message": message }))
        }
        "files.path" => {
            // Where the attachment's bytes are on this machine, fetched from the relay first
            // when another Device sent it.
            let attachment: Attachment = serde_json::from_value(params["attachment"].clone()).map_err(|e| e.to_string())?;
            let path = crate::files::ensure_local(app, &attachment).await.map_err(|e| e.to_string())?;
            Ok(json!({ "path": path }))
        }
        "chats.stop" => {
            app.cancel_chat(&string(&params, "chat_id")?);
            Ok(Value::Null)
        }
        "chats.delete" => {
            app.delete_chat(&string(&params, "chat_id")?);
            Ok(Value::Null)
        }
        "chats.rename" => {
            let title = params["title"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            app.update_chat_meta(&string(&params, "chat_id")?, |meta| meta.title = title).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "chats.pin" => {
            let pinned = params["pinned"].as_bool();
            app.update_chat_meta(&string(&params, "chat_id")?, |meta| meta.is_pinned = pinned.unwrap_or(!meta.is_pinned)).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "chats.add_bot" => {
            let chat_id = string(&params, "chat_id")?;
            let bot_id = string(&params, "bot_id")?;
            let bot = app.bot(&bot_id).ok_or("Unknown bot")?;
            app.update_chat_meta(&chat_id, |meta| {
                if meta.is_group() && meta.bot_ids.len() < MAX_GROUP_BOTS && !meta.bot_ids.contains(&bot_id) {
                    meta.bot_ids.push(bot_id.clone());
                }
            })
            .map_err(|e| e.to_string())?;
            app.notice(&chat_id, format!("{} joined the chat.", bot.name));
            Ok(Value::Null)
        }
        "chats.remove_bot" => {
            let chat_id = string(&params, "chat_id")?;
            let bot_id = string(&params, "bot_id")?;
            app.update_chat_meta(&chat_id, |meta| {
                if meta.is_group() && meta.bot_ids.len() > 1 {
                    meta.bot_ids.retain(|id| id != &bot_id);
                    if meta.owner_bot_id.as_deref() == Some(bot_id.as_str()) {
                        meta.owner_bot_id = meta.bot_ids.first().cloned();
                    }
                }
            })
            .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "chats.set_owner" => {
            let chat_id = string(&params, "chat_id")?;
            let bot_id = string(&params, "bot_id")?;
            app.update_chat_meta(&chat_id, |meta| {
                if meta.bot_ids.contains(&bot_id) {
                    meta.owner_bot_id = Some(bot_id.clone());
                }
            })
            .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "chats.mark_read" => {
            app.mark_read(&string(&params, "chat_id")?);
            Ok(Value::Null)
        }

        "providers.connect_deepseek" => {
            providers::connect_deepseek(app, &string(&params, "api_key")?).await?;
            Ok(json!({ "providers": app.credentials.lock().unwrap().statuses() }))
        }
        "providers.connect_chatgpt" => {
            let tokens = providers::connect_chatgpt(app).await?;
            Ok(json!({ "email": tokens.email, "providers": app.credentials.lock().unwrap().statuses() }))
        }
        "providers.disconnect" => {
            providers::disconnect(app, &string(&params, "kind")?)?;
            Ok(json!({ "providers": app.credentials.lock().unwrap().statuses() }))
        }

        other => Err(format!("unknown method {other}")),
    }
}
