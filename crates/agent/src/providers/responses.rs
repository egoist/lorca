//! The Responses API wire shape shared by the subscription adapters: ChatGPT's Codex backend
//! and xAI's `/v1/responses` both take the same `input` items and stream the same events.

use std::collections::{HashMap, HashSet};

use futures::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::models::ModelInfo;
use crate::provider::{AssistantEvent, ToolSpec, WEB_FETCH_TOOL, WEB_SEARCH_TOOL};
use crate::sse::SseParser;
use crate::types::{AssistantPart, ContentPart, LlmMessage, StopReason, Usage};

/// The `input` items for a transformed transcript.
pub(crate) fn input_items(messages: &[LlmMessage]) -> Vec<Value> {
    let mut input = Vec::new();
    for message in messages {
        match message {
            LlmMessage::User(user) => {
                let content: Vec<Value> = user
                    .content
                    .iter()
                    .map(|part| match part {
                        ContentPart::Text { text } => json!({ "type": "input_text", "text": text }),
                        ContentPart::Image { data, mime_type } => json!({
                            "type": "input_image",
                            "image_url": format!("data:{mime_type};base64,{data}"),
                        }),
                    })
                    .collect();
                input.push(json!({ "type": "message", "role": "user", "content": content }));
            }
            LlmMessage::Assistant(assistant) => {
                let text = assistant.text();
                if !text.is_empty() {
                    input.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": text }],
                    }));
                }
                for part in &assistant.content {
                    if let AssistantPart::ToolCall(call) = part {
                        input.push(json!({
                            "type": "function_call",
                            "call_id": call.id,
                            "name": call.name,
                            "arguments": call.arguments.to_string(),
                        }));
                    }
                }
            }
            LlmMessage::ToolResult(result) => {
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": result.tool_call_id,
                    "output": result.text(),
                }));
            }
        }
    }
    input
}

/// Function tools in the Responses shape.
pub(crate) fn function_tools(tools: &[ToolSpec]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
                "strict": false,
            })
        })
        .collect()
}

#[derive(Default)]
pub(crate) struct ResponsesState {
    next_index: usize,
    items: HashMap<String, usize>,
    /// Server-side search item ids in flight, reported as server tools rather than blocks.
    web_calls: HashSet<String>,
    text_item: Option<usize>,
    thinking_item: Option<usize>,
    tool_deltas_seen: HashMap<usize, bool>,
    pub(crate) stop_reason: StopReason,
    pub(crate) usage: Usage,
}

impl ResponsesState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) async fn apply(&mut self, kind: &str, value: &Value, tx: &mpsc::Sender<AssistantEvent>) -> Result<bool, String> {
        match kind {
            "response.output_item.added" => {
                let item = &value["item"];
                let item_id = item["id"].as_str().unwrap_or("").to_string();
                match item["type"].as_str().unwrap_or("") {
                    "function_call" => {
                        let index = self.next_index;
                        self.next_index += 1;
                        self.items.insert(item_id, index);
                        let id = item["call_id"].as_str().unwrap_or("").to_string();
                        let name = item["name"].as_str().unwrap_or("").to_string();
                        let _ = tx.send(AssistantEvent::ToolCallStart { index, id, name }).await;
                        self.stop_reason = StopReason::ToolUse;
                    }
                    "message" => {
                        let index = self.next_index;
                        self.next_index += 1;
                        self.items.insert(item_id, index);
                        self.text_item = Some(index);
                        let _ = tx.send(AssistantEvent::TextStart { index }).await;
                    }
                    "reasoning" => {
                        let index = self.next_index;
                        self.next_index += 1;
                        self.items.insert(item_id, index);
                        self.thinking_item = Some(index);
                        let _ = tx.send(AssistantEvent::ThinkingStart { index }).await;
                    }
                    kind @ ("web_search_call" | "x_search_call") => {
                        self.web_calls.insert(item_id.clone());
                        let (name, detail, _) = search_call_action(kind, &item["action"]);
                        let _ = tx.send(AssistantEvent::ServerToolStart { id: item_id, name, detail }).await;
                    }
                    _ => {}
                }
            }
            "response.output_text.delta" => {
                let index = self.index_for(&value["item_id"]).or(self.text_item);
                if let (Some(index), Some(delta)) = (index, value["delta"].as_str()) {
                    let _ = tx.send(AssistantEvent::TextDelta { index, delta: delta.to_string() }).await;
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                let index = self.index_for(&value["item_id"]).or(self.thinking_item);
                if let (Some(index), Some(delta)) = (index, value["delta"].as_str()) {
                    let _ = tx.send(AssistantEvent::ThinkingDelta { index, delta: delta.to_string() }).await;
                }
            }
            "response.function_call_arguments.delta" => {
                if let (Some(index), Some(delta)) = (self.index_for(&value["item_id"]), value["delta"].as_str()) {
                    self.tool_deltas_seen.insert(index, true);
                    let _ = tx.send(AssistantEvent::ToolCallDelta { index, delta: delta.to_string() }).await;
                }
            }
            "response.output_item.done" => {
                let item = &value["item"];
                if let Some(kind @ ("web_search_call" | "x_search_call")) = item["type"].as_str() {
                    let item_id = item["id"].as_str().unwrap_or("").to_string();
                    if self.web_calls.remove(&item_id) {
                        let (name, detail, summary) = search_call_action(kind, &item["action"]);
                        let _ = tx.send(AssistantEvent::ServerToolEnd { id: item_id, name, detail, summary }).await;
                    }
                    return Ok(false);
                }
                let Some(index) = self.index_for(&item["id"]) else { return Ok(false) };
                match item["type"].as_str().unwrap_or("") {
                    "function_call" => {
                        if !self.tool_deltas_seen.contains_key(&index) {
                            if let Some(arguments) = item["arguments"].as_str() {
                                let _ = tx.send(AssistantEvent::ToolCallDelta { index, delta: arguments.to_string() }).await;
                            }
                        }
                        let _ = tx.send(AssistantEvent::ToolCallEnd { index }).await;
                    }
                    "message" => {
                        let _ = tx.send(AssistantEvent::TextEnd { index }).await;
                        self.text_item = None;
                    }
                    "reasoning" => {
                        let _ = tx.send(AssistantEvent::ThinkingEnd { index }).await;
                        self.thinking_item = None;
                    }
                    _ => {}
                }
            }
            "response.completed" | "response.done" => {
                let usage = &value["response"]["usage"];
                // `input_tokens` counts the cached tokens too; `input` is the part read fresh.
                let cache_read = usage["input_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0);
                self.usage = Usage {
                    reasoning: usage["output_tokens_details"]["reasoning_tokens"].as_u64(),
                    cost: Default::default(),
                    input: usage["input_tokens"].as_u64().unwrap_or(0).saturating_sub(cache_read),
                    output: usage["output_tokens"].as_u64().unwrap_or(0),
                    cache_read,
                    cache_write: 0,
                    total_tokens: usage["total_tokens"].as_u64().unwrap_or(0),
                };
                if value["response"]["status"].as_str() == Some("incomplete") {
                    self.stop_reason = StopReason::Length;
                }
                return Ok(true);
            }
            "response.incomplete" => {
                self.stop_reason = StopReason::Length;
                return Ok(true);
            }
            "response.failed" => {
                let message = value["response"]["error"]["message"].as_str().unwrap_or("Response failed").to_string();
                return Err(message);
            }
            "error" => {
                let message = value["message"].as_str().or(value["error"]["message"].as_str()).unwrap_or("Provider error");
                return Err(message.to_string());
            }
            _ => {}
        }
        Ok(false)
    }

    fn index_for(&self, item_id: &Value) -> Option<usize> {
        item_id.as_str().and_then(|id| self.items.get(id).copied())
    }
}

/// What a server-side search item did, from its `action`: the tool name it counts as, the
/// detail to keep (query or URL), and a one-line summary for the finished row. A `search` (or
/// `find` within a page) is a search; `open_page` is a read of one page. An `x_search_call`
/// searched X.
pub(crate) fn search_call_action(kind: &str, action: &Value) -> (String, String, String) {
    let query = action["query"].as_str().unwrap_or("").trim();
    let url = action["url"].as_str().unwrap_or("").trim();
    let on_x = kind == "x_search_call";
    match action["type"].as_str().unwrap_or("search") {
        "open_page" if !url.is_empty() => (WEB_FETCH_TOOL.into(), url.into(), format!("Read {url}")),
        "find" if !url.is_empty() => {
            let pattern = action["pattern"].as_str().unwrap_or("").trim();
            let detail = if pattern.is_empty() { url.to_string() } else { format!("{pattern} in {url}") };
            (WEB_FETCH_TOOL.into(), detail, format!("Searched {url}"))
        }
        _ if !query.is_empty() && on_x => (WEB_SEARCH_TOOL.into(), query.into(), format!("Searched X for “{query}”")),
        _ if !query.is_empty() => (WEB_SEARCH_TOOL.into(), query.into(), format!("Searched the web for “{query}”")),
        _ if on_x => (WEB_SEARCH_TOOL.into(), String::new(), "Searched X".into()),
        _ => (WEB_SEARCH_TOOL.into(), String::new(), "Searched the web".into()),
    }
}

/// Reads the SSE stream of a started response into assistant events, ending with `Done`.
/// `provider` names the adapter in the log line for a stream that ends early.
pub(crate) async fn pump(
    response: reqwest::Response,
    tx: mpsc::Sender<AssistantEvent>,
    cancel: CancellationToken,
    info: Option<&'static ModelInfo>,
    provider: &str,
) {
    let _ = tx.send(AssistantEvent::Start).await;
    let mut parser = SseParser::new();
    let mut state = ResponsesState::new();
    let mut bytes = response.bytes_stream();
    let mut completed = false;

    'outer: loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => {
                let _ = tx.send(AssistantEvent::Error { message: "Request aborted".into(), aborted: true }).await;
                return;
            }
            chunk = bytes.next() => chunk,
        };
        let Some(chunk) = chunk else { break };
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                let _ = tx.send(AssistantEvent::Error { message: format!("Stream failed: {error}"), aborted: false }).await;
                return;
            }
        };
        for event in parser.push(&chunk) {
            let Ok(value) = serde_json::from_str::<Value>(&event.data) else { continue };
            let kind = value["type"].as_str().map(str::to_string).or(event.event.clone()).unwrap_or_default();
            match state.apply(&kind, &value, &tx).await {
                Ok(true) => {
                    completed = true;
                    break 'outer;
                }
                Ok(false) => {}
                Err(message) => {
                    let _ = tx.send(AssistantEvent::Error { message, aborted: false }).await;
                    return;
                }
            }
        }
    }

    if !completed {
        tracing::debug!("{provider} stream ended without response.completed");
    }
    let mut usage = state.usage.clone();
    if let Some(info) = info {
        usage.cost = info.cost_of(&usage);
    }
    let _ = tx.send(AssistantEvent::Done { stop_reason: state.stop_reason, usage }).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn web_search_calls_stream_as_server_tools_and_never_as_blocks() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut state = ResponsesState::new();
        let added = json!({ "item": { "type": "web_search_call", "id": "ws_1", "status": "in_progress" } });
        assert_eq!(state.apply("response.output_item.added", &added, &tx).await, Ok(false));
        let done = json!({ "item": {
            "type": "web_search_call", "id": "ws_1", "status": "completed",
            "action": { "type": "search", "query": "lorca relay" },
        } });
        assert_eq!(state.apply("response.output_item.done", &done, &tx).await, Ok(false));
        let page = json!({ "item": { "type": "web_search_call", "id": "ws_2", "action": { "type": "open_page", "url": "https://example.com/a" } } });
        assert_eq!(state.apply("response.output_item.added", &page, &tx).await, Ok(false));
        assert_eq!(state.apply("response.output_item.done", &page, &tx).await, Ok(false));
        // A message after the searches is still block 0: the searches took no index.
        let message = json!({ "item": { "type": "message", "id": "msg_1" } });
        assert_eq!(state.apply("response.output_item.added", &message, &tx).await, Ok(false));
        drop(tx);

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }
        assert!(matches!(&events[0], AssistantEvent::ServerToolStart { id, name, detail } if id == "ws_1" && name == WEB_SEARCH_TOOL && detail.is_empty()));
        assert!(matches!(&events[1], AssistantEvent::ServerToolEnd { id, name, detail, summary }
            if id == "ws_1" && name == WEB_SEARCH_TOOL && detail == "lorca relay" && summary == "Searched the web for “lorca relay”"));
        assert!(matches!(&events[2], AssistantEvent::ServerToolStart { name, detail, .. } if name == WEB_FETCH_TOOL && detail == "https://example.com/a"));
        assert!(matches!(&events[3], AssistantEvent::ServerToolEnd { name, summary, .. } if name == WEB_FETCH_TOOL && summary == "Read https://example.com/a"));
        assert!(matches!(&events[4], AssistantEvent::TextStart { index: 0 }));
        assert_eq!(state.stop_reason, StopReason::Stop);
    }

    #[tokio::test]
    async fn an_x_search_reads_as_a_search_of_x() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut state = ResponsesState::new();
        let item = json!({ "item": { "type": "x_search_call", "id": "xs_1", "action": { "type": "search", "query": "lorca" } } });
        assert_eq!(state.apply("response.output_item.added", &item, &tx).await, Ok(false));
        assert_eq!(state.apply("response.output_item.done", &item, &tx).await, Ok(false));
        drop(tx);
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }
        assert!(matches!(&events[1], AssistantEvent::ServerToolEnd { name, detail, summary, .. }
            if name == WEB_SEARCH_TOOL && detail == "lorca" && summary == "Searched X for “lorca”"));
    }

    #[test]
    fn a_find_inside_a_page_counts_as_a_read() {
        let (name, detail, summary) = search_call_action("web_search_call", &json!({ "type": "find", "url": "https://x.dev", "pattern": "pricing" }));
        assert_eq!((name.as_str(), detail.as_str(), summary.as_str()), (WEB_FETCH_TOOL, "pricing in https://x.dev", "Searched https://x.dev"));
        let (name, _, summary) = search_call_action("web_search_call", &json!({}));
        assert_eq!((name.as_str(), summary.as_str()), (WEB_SEARCH_TOOL, "Searched the web"));
    }

    #[tokio::test]
    async fn usage_counts_cached_input_once_and_keeps_reasoning_tokens() {
        let (tx, _rx) = mpsc::channel(16);
        let mut state = ResponsesState::new();
        let completed = json!({ "response": { "status": "completed", "usage": {
            "input_tokens": 10, "output_tokens": 30, "total_tokens": 40,
            "input_tokens_details": { "cached_tokens": 4 }, "output_tokens_details": { "reasoning_tokens": 12 },
        } } });
        assert_eq!(state.apply("response.completed", &completed, &tx).await, Ok(true));
        assert_eq!((state.usage.input, state.usage.output, state.usage.cache_read, state.usage.reasoning), (6, 30, 4, Some(12)));
        assert_eq!(crate::estimate::context_tokens(&state.usage), 40);
    }
}
