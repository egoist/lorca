//! Servers Lorca answers itself, inside the Runner: Telegram's, and the bot side of Slack. The
//! pool reaches one through an in-memory pipe and speaks MCP to it as to any other server, so
//! its tools go through the bot's Access, Auto-review, the review queue, and the account's call
//! limits like every plugin tool. A server reads the account's token from the plugin store at
//! each call; the token never reaches a process, an environment, or the model.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, WriteHalf};

use crate::app::App;

/// The protocol version the builtin servers answer the handshake with.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// What a builtin server offers: its tools as MCP declares them, and their calls.
#[async_trait::async_trait]
pub trait Service: Send + Sync {
    fn name(&self) -> &'static str;
    fn instructions(&self) -> Option<String>;
    /// The tools as `tools/list` gives them; `plugin_id` is the account they act for.
    fn tools(&self, app: &Arc<App>, plugin_id: &str) -> Vec<Value>;
    /// A call's `CallToolResult`. `context` is the `_meta` Lorca sent with it: the bot and chat
    /// whose turn called, when a turn did.
    async fn call(&self, app: &Arc<App>, plugin_id: &str, tool: &str, args: Value, context: &Value) -> Value;
}

/// The builtin server a manifest names, when this build has it.
pub fn service(name: &str) -> Option<Arc<dyn Service>> {
    match name {
        "telegram" => Some(Arc::new(crate::channels::telegram::Server)),
        "slack" => Some(Arc::new(crate::channels::slack::Server)),
        _ => None,
    }
}

/// A tool call's result: its text, and whether it failed.
pub fn text_result(text: impl Into<String>, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text.into() }], "isError": is_error })
}

/// A tool result with the structured value a script reads, and the same value as text.
pub fn structured_result(value: Value) -> Value {
    json!({ "content": [{ "type": "text", "text": value.to_string() }], "structuredContent": value, "isError": false })
}

/// Starts a builtin server for a plugin and gives the client's end of its pipe. The server ends
/// when the client drops its end.
pub fn start(app: &Arc<App>, plugin_id: &str, service: Arc<dyn Service>) -> DuplexStream {
    let (client, server) = tokio::io::duplex(1 << 20);
    tokio::spawn(run(Arc::downgrade(app), plugin_id.to_string(), service, server));
    client
}

type Writer = Arc<tokio::sync::Mutex<WriteHalf<DuplexStream>>>;

async fn send(writer: &Writer, message: Value) {
    let mut line = message.to_string();
    line.push('\n');
    let _ = writer.lock().await.write_all(line.as_bytes()).await;
}

async fn run(app: Weak<App>, plugin_id: String, service: Arc<dyn Service>, io: DuplexStream) {
    let (read, write) = tokio::io::split(io);
    let writer: Writer = Arc::new(tokio::sync::Mutex::new(write));
    let mut lines = BufReader::new(read).lines();
    // Calls in flight by request id, so a `notifications/cancelled` stops one. A call stops
    // between awaits of its own, never halfway through writing its reply.
    let calls: Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>>> = Arc::default();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(app) = app.upgrade() else { return };
        let method = message["method"].as_str().unwrap_or_default().to_string();
        let Some(id) = message.get("id").cloned() else {
            if method == "notifications/cancelled" {
                if let Some(cancel) = calls.lock().unwrap().remove(&message["params"]["requestId"].to_string()) {
                    let _ = cancel.send(());
                }
            }
            continue;
        };
        let reply = |result: Value| json!({ "jsonrpc": "2.0", "id": id, "result": result });
        match method.as_str() {
            "initialize" => {
                let mut result = json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": format!("lorca-{}", service.name()), "version": crate::config::VERSION },
                });
                if let Some(instructions) = service.instructions() {
                    result["instructions"] = json!(instructions);
                }
                send(&writer, reply(result)).await;
            }
            "ping" => send(&writer, reply(json!({}))).await,
            "tools/list" => send(&writer, reply(json!({ "tools": service.tools(&app, &plugin_id) }))).await,
            "tools/call" => {
                let (writer, service, plugin_id, calls_left) = (writer.clone(), service.clone(), plugin_id.clone(), calls.clone());
                let (cancel, cancelled) = tokio::sync::oneshot::channel();
                calls.lock().unwrap().insert(id.to_string(), cancel);
                let params = message["params"].clone();
                tokio::spawn(async move {
                    let tool = params["name"].as_str().unwrap_or_default();
                    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
                    tokio::select! {
                        result = service.call(&app, &plugin_id, tool, args, &params["_meta"]) => {
                            calls_left.lock().unwrap().remove(&id.to_string());
                            send(&writer, json!({ "jsonrpc": "2.0", "id": id, "result": result })).await;
                        }
                        // The client stopped waiting: no reply.
                        _ = cancelled => {}
                    }
                });
            }
            // `server/discover` and anything else: a server of the initialize era.
            _ => send(&writer, json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("Method not found: {method}") } })).await,
        }
    }
}
