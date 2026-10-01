//! The Responses API wire shape shared by the subscription adapters and the API-key one:
//! ChatGPT's Codex backend, xAI's `/v1/responses`, and gateways take the same `input` items,
//! apart from where a tool result's images go ([`ToolImages`]), and stream the same events.

use std::collections::{HashMap, HashSet};

use futures::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::models::ModelInfo;
use crate::provider::{AssistantEvent, ToolSpec, WEB_FETCH_TOOL, WEB_SEARCH_TOOL};
use crate::sse::SseParser;
use crate::types::{AssistantPart, ContentPart, LlmMessage, StopReason, Usage};

/// Where the images of a tool result go in the `input`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolImages {
    /// Inside the `function_call_output`: its `output` becomes `input_text` and `input_image`
    /// parts, in the result's order. OpenAI's Responses API takes this encoding, and the Codex
    /// CLI sends its own tool images to the ChatGPT backend this way.
    InOutput,
    /// The `output` is the result's text, and one user message after the run of outputs
    /// carries their images, for a server that documents only a string `output` (xAI) and
    /// gateways that drop the parts.
    UserMessage,
}

/// The `input` items for a transformed transcript, with tool images placed as `tool_images`
/// says.
pub(crate) fn input_items(messages: &[LlmMessage], tool_images: ToolImages) -> Vec<Value> {
    let mut input = Vec::new();
    // The images of the tool results since the last other message, each result's after a line
    // naming its tool, for `ToolImages::UserMessage`.
    let mut images = Vec::new();
    for message in messages {
        if !matches!(message, LlmMessage::ToolResult(_)) {
            push_images(&mut input, &mut images);
        }
        match message {
            LlmMessage::User(user) => {
                let content: Vec<Value> = user.content.iter().map(input_part).collect();
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
                let has_images = result.content.iter().any(|part| matches!(part, ContentPart::Image { .. }));
                let output = match tool_images {
                    _ if !has_images => Value::String(result.text()),
                    ToolImages::InOutput => Value::Array(output_parts(&result.content)),
                    ToolImages::UserMessage => {
                        images.push(json!({ "type": "input_text", "text": format!("Images from the {} tool result:", result.tool_name) }));
                        images.extend(result.content.iter().filter(|part| matches!(part, ContentPart::Image { .. })).map(input_part));
                        let text = result.text();
                        Value::String(if text.is_empty() { "(see attached image)".into() } else { text })
                    }
                };
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": result.tool_call_id,
                    "output": output,
                }));
            }
        }
    }
    push_images(&mut input, &mut images);
    input
}

/// A content part as a Responses input part; an image as a `data:` URL.
fn input_part(part: &ContentPart) -> Value {
    match part {
        ContentPart::Text { text } => json!({ "type": "input_text", "text": text }),
        ContentPart::Image { data, mime_type } => json!({
            "type": "input_image",
            "image_url": format!("data:{mime_type};base64,{data}"),
        }),
    }
}

/// A tool result's content as `function_call_output` parts: each run of text one `input_text`,
/// joined as [`crate::types::ToolResultMessage::text`] joins it, and each image where it was.
/// A blank run is left out.
fn output_parts(content: &[ContentPart]) -> Vec<Value> {
    content
        .chunk_by(|a, b| matches!((a, b), (ContentPart::Text { .. }, ContentPart::Text { .. })))
        .filter_map(|run| match run {
            [image @ ContentPart::Image { .. }] => Some(input_part(image)),
            _ => {
                let text = run.iter().filter_map(ContentPart::as_text).collect::<Vec<_>>().join("\n");
                (!text.trim().is_empty()).then(|| json!({ "type": "input_text", "text": text }))
            }
        })
        .collect()
}

/// Sends the tool images gathered for [`ToolImages::UserMessage`] as one user message.
fn push_images(input: &mut Vec<Value>, images: &mut Vec<Value>) {
    if !images.is_empty() {
        input.push(json!({ "type": "message", "role": "user", "content": std::mem::take(images) }));
    }
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

/// What the Responses adapters' tests share: a turn whose tool returned a screenshot, and a
/// live probe that asks a model to read one.
#[cfg(test)]
pub(crate) mod testing {
    use base64::Engine;
    use futures::StreamExt;
    use serde_json::{json, Value};
    use tokio_util::sync::CancellationToken;

    use crate::provider::{AssistantAccumulator, ModelRequest, Provider, ToolSpec};
    use crate::request::RequestOptions;
    use crate::types::{AssistantMessage, AssistantPart, ContentPart, LlmMessage, StopReason, ToolCall, ToolResultMessage, UserMessage};

    /// The model's `take_screenshot` call and its result: a line of text, then the image.
    pub(crate) fn screenshot_turn(provider: &str, model: &str, png: &str) -> Vec<LlmMessage> {
        let mut call = AssistantMessage::empty(provider, model);
        call.content = vec![AssistantPart::ToolCall(ToolCall { id: "call_1".into(), name: "take_screenshot".into(), arguments: json!({}) })];
        call.stop_reason = StopReason::ToolUse;
        vec![LlmMessage::Assistant(call), screenshot("call_1", png)]
    }

    fn screenshot(call_id: &str, png: &str) -> LlmMessage {
        LlmMessage::ToolResult(ToolResultMessage {
            tool_call_id: call_id.into(),
            tool_name: "take_screenshot".into(),
            content: vec![ContentPart::text("Took a screenshot of the page."), ContentPart::Image { data: png.into(), mime_type: "image/png".into() }],
            details: Value::Null,
            is_error: false,
            timestamp: 0,
        })
    }

    /// One provider's entry in the `credentials.json` that `LORCA_CREDENTIALS` names (a Lorca
    /// data directory's), or `None` to skip the probe.
    pub(crate) fn credential<T: serde::de::DeserializeOwned>(key: &str) -> Option<T> {
        let path = std::env::var("LORCA_CREDENTIALS").ok()?;
        let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("reading {path}: {error}"));
        let credentials: Value = serde_json::from_str(&text).expect("credentials.json is JSON");
        match credentials.get(key).filter(|entry| !entry.is_null()) {
            Some(entry) => Some(serde_json::from_value(entry.clone()).unwrap_or_else(|error| panic!("{key} in {path}: {error}"))),
            None => {
                eprintln!("{key} is not connected in {path}; skipped");
                None
            }
        }
    }

    /// Asks the model to call `take_screenshot`, returns a screenshot of a random six-digit
    /// code, and answers the reply when it names the code. The code is only in the pixels.
    pub(crate) async fn reads_a_screenshot(provider: &dyn Provider, options: RequestOptions) -> Result<String, String> {
        let code = format!("{:06}", rand::random::<u32>() % 1_000_000);
        let mut request = ModelRequest {
            system_prompt: "You are under test. Do what the user asks, with no other tool.".into(),
            messages: vec![LlmMessage::User(UserMessage::text("Call take_screenshot. The screenshot shows a six-digit code: reply with the code alone."))],
            tools: vec![ToolSpec {
                name: "take_screenshot".into(),
                description: "Takes a screenshot of the test page.".into(),
                parameters: json!({ "type": "object", "properties": {} }),
            }],
            cache_points: Vec::new(),
            max_tokens: None,
            options,
        };
        let first = complete(provider, &request).await;
        let calls = first.tool_calls();
        let [call] = calls.as_slice() else {
            return Err(format!("expected one take_screenshot call: {:?} {:?}", first.content, first.error_message));
        };
        let png = base64::engine::general_purpose::STANDARD.encode(digits_png(&code));
        let result = screenshot(&call.id, &png);
        request.messages.extend([LlmMessage::Assistant(first), result]);
        let reply = complete(provider, &request).await;
        eprintln!("{} {}: {:?} {:?} usage {:?}", provider.provider_id(), provider.model_id(), reply.stop_reason, reply.text(), reply.usage);
        match reply.text() {
            text if text.contains(&code) => Ok(text),
            text => Err(format!("the reply does not name {code}: {text:?} {:?}", reply.error_message)),
        }
    }

    async fn complete(provider: &dyn Provider, request: &ModelRequest) -> AssistantMessage {
        let mut stream = provider.stream(request.clone(), CancellationToken::new()).await;
        let mut message = AssistantAccumulator::new(provider.provider_id(), provider.model_id());
        while let Some(event) = stream.next().await {
            message.apply(&event);
        }
        message.finish(false)
    }

    /// A PNG of `digits` in a 5×7 dot font, eight pixels a dot, black on white.
    fn digits_png(digits: &str) -> Vec<u8> {
        // Each row's five dots, the leftmost in bit 4.
        const GLYPHS: [[u8; 7]; 10] = [
            [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
            [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
            [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
            [0x0E, 0x11, 0x01, 0x06, 0x01, 0x11, 0x0E],
            [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
            [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
            [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
            [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
            [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
            [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        ];
        const DOT: usize = 8;
        const MARGIN: usize = 3;
        let glyphs: Vec<&[u8; 7]> = digits.bytes().map(|digit| &GLYPHS[usize::from(digit - b'0')]).collect();
        let (width, height) = ((glyphs.len() * 6 - 1 + 2 * MARGIN) * DOT, (7 + 2 * MARGIN) * DOT);
        // Rows of 8-bit gray, each after the byte that says it is not filtered.
        let mut pixels = Vec::with_capacity((width + 1) * height);
        for y in 0..height {
            pixels.push(0);
            let row = (y / DOT).wrapping_sub(MARGIN);
            for x in 0..width {
                let column = (x / DOT).wrapping_sub(MARGIN);
                let ink = row < 7 && column % 6 < 5 && glyphs.get(column / 6).is_some_and(|glyph| glyph[row] & (0x10 >> (column % 6)) != 0);
                pixels.push(if ink { 0 } else { 255 });
            }
        }
        // A zlib stream of stored deflate blocks.
        let mut zlib = vec![0x78, 0x01];
        let blocks: Vec<&[u8]> = pixels.chunks(usize::from(u16::MAX)).collect();
        for (index, block) in blocks.iter().enumerate() {
            let length = block.len() as u16;
            zlib.push(u8::from(index + 1 == blocks.len()));
            zlib.extend(length.to_le_bytes());
            zlib.extend((!length).to_le_bytes());
            zlib.extend_from_slice(block);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &byte in &pixels {
            a = (a + u32::from(byte)) % 65_521;
            b = (b + a) % 65_521;
        }
        zlib.extend(((b << 16) | a).to_be_bytes());

        let mut header = Vec::new();
        header.extend((width as u32).to_be_bytes());
        header.extend((height as u32).to_be_bytes());
        header.extend([8, 0, 0, 0, 0]);
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        for (kind, data) in [(b"IHDR", header.as_slice()), (b"IDAT", zlib.as_slice()), (b"IEND", &[])] {
            png.extend((data.len() as u32).to_be_bytes());
            let start = png.len();
            png.extend(kind);
            png.extend_from_slice(data);
            let mut crc = !0u32;
            for &byte in &png[start..] {
                crc ^= u32::from(byte);
                for _ in 0..8 {
                    crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
                }
            }
            png.extend((!crc).to_be_bytes());
        }
        png
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AssistantMessage, ToolCall, ToolResultMessage, UserMessage};

    fn result(id: &str, tool: &str, content: Vec<ContentPart>) -> LlmMessage {
        LlmMessage::ToolResult(ToolResultMessage { tool_call_id: id.into(), tool_name: tool.into(), content, details: Value::Null, is_error: false, timestamp: 0 })
    }

    fn png(data: &str) -> ContentPart {
        ContentPart::Image { data: data.into(), mime_type: "image/png".into() }
    }

    /// Three tools run at once, two of them returning images, then the user's next message.
    fn parallel_turn() -> Vec<LlmMessage> {
        let mut calls = AssistantMessage::empty("p", "m");
        calls.content = ["browser", "bash", "read"]
            .iter()
            .enumerate()
            .map(|(index, tool)| AssistantPart::ToolCall(ToolCall { id: format!("call_{index}"), name: tool.to_string(), arguments: json!({}) }))
            .collect();
        vec![
            LlmMessage::Assistant(calls),
            result("call_0", "browser", vec![ContentPart::text("Took a screenshot"), png("AAAA")]),
            result("call_1", "bash", vec![ContentPart::text("ok")]),
            result("call_2", "read", vec![png("BBBB")]),
            LlmMessage::User(UserMessage::text("next")),
        ]
    }

    #[test]
    fn tool_images_go_inside_the_output_in_order() {
        let mixed = result("call_0", "codemode", vec![ContentPart::text("a"), ContentPart::text("b"), png("AAAA"), ContentPart::text(" "), png("BBBB"), ContentPart::text("c")]);
        let input = input_items(&[mixed, result("call_1", "bash", vec![ContentPart::text("ok")])], ToolImages::InOutput);
        // Text runs join as a text-only result's do; a blank run is left out.
        assert_eq!(
            input[0]["output"],
            json!([
                { "type": "input_text", "text": "a\nb" },
                { "type": "input_image", "image_url": "data:image/png;base64,AAAA" },
                { "type": "input_image", "image_url": "data:image/png;base64,BBBB" },
                { "type": "input_text", "text": "c" },
            ])
        );
        assert_eq!(input[1]["output"], "ok");

        let input = input_items(&parallel_turn(), ToolImages::InOutput);
        assert_eq!(input[5]["output"], json!([{ "type": "input_image", "image_url": "data:image/png;base64,BBBB" }]));
        assert_eq!(input.len(), 7, "no message for the images: {input:?}");
    }

    #[test]
    fn tool_images_follow_the_run_of_outputs_in_one_user_message() {
        let turn = parallel_turn();
        let input = input_items(&turn, ToolImages::UserMessage);
        let kinds: Vec<&str> = input.iter().map(|item| item["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["function_call", "function_call", "function_call", "function_call_output", "function_call_output", "function_call_output", "message", "message"]);
        assert_eq!(input[3]["output"], "Took a screenshot");
        assert_eq!(input[4]["output"], "ok");
        assert_eq!(input[5]["output"], "(see attached image)");
        assert_eq!(
            input[6],
            json!({ "type": "message", "role": "user", "content": [
                { "type": "input_text", "text": "Images from the browser tool result:" },
                { "type": "input_image", "image_url": "data:image/png;base64,AAAA" },
                { "type": "input_text", "text": "Images from the read tool result:" },
                { "type": "input_image", "image_url": "data:image/png;base64,BBBB" },
            ] })
        );
        assert_eq!(input[7]["content"], json!([{ "type": "input_text", "text": "next" }]));

        // A turn that ends on its results still sends their images.
        let ending = input_items(&turn[..4], ToolImages::UserMessage);
        assert_eq!(ending.last().unwrap()["content"][1]["image_url"], "data:image/png;base64,AAAA");
    }

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
