//! A Claude subscription through the Claude Code CLI on the Runner: each model call is one
//! `claude -p` with stream-json in and out, as its Agent SDK speaks it. Claude Code signs in and
//! sends the request; Lorca keeps the conversation, the tools, and running them. The call replays
//! the conversation as stream-json frames, every user frame but the last marked `shouldQuery:
//! false`, and Claude Code asks the model once (`--max-turns 1`). Its own tools are off
//! (`--tools ""`); the bot's tools reach the model from a tool server that only lists them
//! ([`serve_tools`], named `lorca`, so they arrive as `mcp__lorca__<name>`), and `dontAsk` denies
//! a call before it reaches that server. The reply is read from the raw Messages events Claude
//! Code passes on (`--include-partial-messages`), through the Anthropic adapter's state, and the
//! process ends at `message_stop`, once its plan-limit report is in.

use std::ffi::OsString;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::anthropic::{AnthropicProvider, MessagesState};
use crate::models::ModelInfo;
use crate::provider::{channel_stream, AssistantEvent, AssistantEventStream, ModelRequest, Provider};
use crate::types::ThinkingLevel;

pub const PROVIDER_ID: &str = "claude-code";
/// The tool server's name, so the model calls the bot's tools as `mcp__lorca__<name>`.
const SERVER: &str = "lorca";
const TOOL_PREFIX: &str = "mcp__lorca__";
/// How long the process has after `message_stop` to report the plan's limits.
const AFTER_STOP: Duration = Duration::from_secs(5);
/// How long Claude Code has to record one history frame; it asks nothing for it.
const HISTORY_ACK: Duration = Duration::from_secs(60);

/// Variables that would move Claude Code off the subscription it signed in with: an API key or
/// token, another API root, or a cloud's endpoint. They are left out of its environment, as is a
/// request body of its own.
pub const OVERRIDES: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_FOUNDRY_API_KEY",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_EXTRA_BODY",
];

pub struct ClaudeCodeProvider {
    /// The `claude` to run.
    pub program: PathBuf,
    /// The tool server: a program and its first arguments, the tool list's path going last.
    pub tools_server: (PathBuf, Vec<String>),
    /// Its environment, the overrides taken out ([`OVERRIDES`]).
    pub env: Vec<(OsString, OsString)>,
    /// A reply that Anthropic charged to usage credits instead of the plan stops the turn,
    /// unless this is set.
    pub allow_usage_credits: bool,
    /// Builds the Messages request whose parts `claude -p` is given.
    messages: AnthropicProvider,
}

impl ClaudeCodeProvider {
    pub fn new(program: PathBuf, tools_server: (PathBuf, Vec<String>), env: Vec<(OsString, OsString)>, model: &str) -> Self {
        let mut messages = AnthropicProvider::new(PROVIDER_ID, "", "", model);
        // Claude Code marks the cache, and streams tool arguments as it does for itself.
        messages.cache = false;
        messages.eager_tool_streaming = false;
        messages.max_tokens = 32_000;
        messages.supports_images = messages.info.is_none_or(|info| info.images);
        ClaudeCodeProvider { program, tools_server, env, allow_usage_credits: false, messages }
    }

    pub fn with_thinking(mut self, level: Option<ThinkingLevel>) -> Self {
        self.messages = self.messages.with_thinking(level);
        self
    }

    /// What one call gives `claude -p`, from the Messages request: the frames to replay, the
    /// system prompt, the tool list, the request fields Claude Code takes as its extra body, and
    /// the output cap.
    fn plan(&self, request: &ModelRequest) -> Result<Plan, String> {
        let body = self.messages.body(request);
        let mut frames: Vec<Value> = Vec::new();
        for message in body["messages"].as_array().into_iter().flatten() {
            let role = message["role"].as_str().unwrap_or("user");
            let mut content = message["content"].as_array().cloned().unwrap_or_default();
            for block in content.iter_mut().filter(|block| block["type"] == "tool_use") {
                let name = block["name"].as_str().unwrap_or_default();
                block["name"] = Value::String(format!("{TOOL_PREFIX}{name}"));
            }
            frames.push(json!({ "type": role, "message": { "role": role, "content": content } }));
        }
        if frames.last().is_none_or(|frame| frame["type"] != "user") {
            return Err("Claude Code needs a conversation that ends with the user or a tool result".into());
        }
        // Every user frame before the last is history: Claude Code records it and asks nothing.
        let last = frames.len() - 1;
        for frame in &mut frames[..last] {
            if frame["type"] == "user" {
                frame["shouldQuery"] = Value::Bool(false);
            }
        }
        let tools = body["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|tool| json!({ "name": tool["name"], "description": tool["description"], "inputSchema": input_schema(&tool["input_schema"]) }))
            .collect();
        let mut extra = serde_json::Map::new();
        for field in ["thinking", "output_config"] {
            if !body[field].is_null() {
                extra.insert(field.into(), body[field].clone());
            }
        }
        // Claude Code clears old thinking by default, which the API refuses with thinking off.
        if body["thinking"]["type"] == "disabled" {
            extra.insert("context_management".into(), json!({ "edits": [] }));
        }
        Ok(Plan {
            frames,
            system: body["system"].as_str().unwrap_or_default().to_string(),
            tools,
            extra: Value::Object(extra),
            max_tokens: body["max_tokens"].as_u64(),
        })
    }
}

/// What one call hands `claude -p`.
struct Plan {
    frames: Vec<Value>,
    system: String,
    tools: Vec<Value>,
    extra: Value,
    max_tokens: Option<u64>,
}

/// A tool's parameters as a tool server lists them: an object schema, with its properties.
fn input_schema(schema: &Value) -> Value {
    let mut schema = if schema.is_object() { schema.clone() } else { json!({}) };
    if schema["type"].is_null() {
        schema["type"] = json!("object");
    }
    if schema["type"] == "object" && !schema["properties"].is_object() {
        schema["properties"] = json!({});
    }
    schema
}

/// Why Anthropic would not take the request on the plan, said so the user can act; `None` for
/// any other error.
fn usage_credits_refusal(message: &str) -> Option<String> {
    let lower = message.to_lowercase();
    let refused = ["third-party apps now draw from your extra usage", "out of extra usage", "draws from your usage credits", "draw from your usage credits"]
        .iter()
        .any(|phrase| lower.contains(phrase));
    refused.then(|| format!("Claude Code billing: Anthropic did not take this request on your Claude plan's limits and wants to charge it to usage credits. Lorca does not ask again. {message}"))
}

#[async_trait]
impl Provider for ClaudeCodeProvider {
    fn default_max_output_tokens(&self) -> Option<u64> {
        self.messages.default_max_output_tokens()
    }

    fn provider_id(&self) -> &str {
        PROVIDER_ID
    }

    fn model_id(&self) -> &str {
        &self.messages.model
    }

    fn supports_images(&self) -> bool {
        self.messages.supports_images
    }

    fn model_info(&self) -> Option<&'static ModelInfo> {
        self.messages.info
    }

    async fn stream(&self, request: ModelRequest, cancel: CancellationToken) -> AssistantEventStream {
        let (tx, rx) = mpsc::channel(64);
        let plan = self.plan(&request);
        let call = Call {
            program: self.program.clone(),
            tools_server: self.tools_server.clone(),
            env: self.env.clone(),
            model: self.messages.model.clone(),
            info: self.messages.info,
            allow_usage_credits: self.allow_usage_credits,
        };
        tokio::spawn(async move {
            let terminal = match plan {
                Ok(plan) => call.run(plan, &cancel, &tx).await,
                Err(message) => AssistantEvent::Error { message, aborted: false },
            };
            let _ = tx.send(terminal).await;
        });
        channel_stream(rx)
    }
}

/// One `claude -p`.
struct Call {
    program: PathBuf,
    tools_server: (PathBuf, Vec<String>),
    env: Vec<(OsString, OsString)>,
    model: String,
    info: Option<&'static ModelInfo>,
    allow_usage_credits: bool,
}

impl Call {
    /// Runs it in a folder of its own, removed after, and answers the stream's last event.
    async fn run(&self, plan: Plan, cancel: &CancellationToken, tx: &mpsc::Sender<AssistantEvent>) -> AssistantEvent {
        let folder = std::env::temp_dir().join(format!("lorca-claude-code-{}", uuid::Uuid::new_v4()));
        let terminal = match self.prepare(&folder, &plan) {
            Ok(args) => self.talk(&folder, args, &plan.frames, plan.max_tokens, cancel, tx).await,
            Err(error) => AssistantEvent::Error { message: format!("Could not prepare Claude Code: {error}"), aborted: false },
        };
        let _ = std::fs::remove_dir_all(&folder);
        terminal
    }

    /// Writes the system prompt, the tool list, and the settings, and answers the arguments.
    fn prepare(&self, folder: &Path, plan: &Plan) -> std::io::Result<Vec<OsString>> {
        std::fs::create_dir_all(folder)?;
        let (system, tools, settings) = (folder.join("system.md"), folder.join("tools.json"), folder.join("settings.json"));
        std::fs::write(&system, &plan.system)?;
        std::fs::write(&tools, serde_json::to_vec(&plan.tools)?)?;
        std::fs::write(&settings, serde_json::to_vec(&json!({ "env": { "CLAUDE_CODE_EXTRA_BODY": plan.extra.to_string() } }))?)?;
        let (server, server_args) = &self.tools_server;
        let mut server_args: Vec<String> = server_args.clone();
        server_args.push(tools.to_string_lossy().into_owned());
        let mcp = json!({ "mcpServers": { SERVER: { "command": server, "args": server_args } } });
        let mut args: Vec<OsString> = [
            "-p",
            "--model",
            &self.model,
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--tools",
            "",
            "--setting-sources",
            "",
            "--strict-mcp-config",
            "--disable-slash-commands",
            "--max-turns",
            "1",
            "--permission-mode",
            "dontAsk",
            "--no-session-persistence",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        args.extend(["--system-prompt-file".into(), system.into_os_string(), "--settings".into(), settings.into_os_string(), "--mcp-config".into(), mcp.to_string().into()]);
        Ok(args)
    }

    async fn talk(&self, folder: &Path, args: Vec<OsString>, frames: &[Value], max_tokens: Option<u64>, cancel: &CancellationToken, tx: &mpsc::Sender<AssistantEvent>) -> AssistantEvent {
        let mut command = tokio::process::Command::new(&self.program);
        command.args(&args).current_dir(folder).env_clear().envs(self.env.iter().filter(|(name, _)| !OVERRIDES.iter().any(|o| name.to_str() == Some(o))).map(|(n, v)| (n, v)));
        // Nothing but the one request: no searching for tools, no compacting, no retries (the
        // loop retries), no reports.
        for (name, value) in [
            ("ENABLE_TOOL_SEARCH", "false"),
            ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1"),
            ("CLAUDE_CODE_MAX_RETRIES", "0"),
            ("DISABLE_AUTO_COMPACT", "1"),
            ("DISABLE_COMPACT", "1"),
            ("DISABLE_TELEMETRY", "1"),
            ("DISABLE_ERROR_REPORTING", "1"),
        ] {
            command.env(name, value);
        }
        if let Some(max_tokens) = max_tokens {
            command.env("CLAUDE_CODE_MAX_OUTPUT_TOKENS", max_tokens.to_string());
        }
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => return AssistantEvent::Error { message: format!("Could not start Claude Code ({}): {error}", self.program.display()), aborted: false },
        };
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let mut stderr = child.stderr.take().expect("piped");
        let errors = tokio::spawn(async move {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text).await;
            text
        });
        let _ = tx.send(AssistantEvent::Start).await;
        let outcome = tokio::select! {
            _ = cancel.cancelled() => Err(AssistantEvent::Error { message: "Request aborted".into(), aborted: true }),
            outcome = self.converse(stdin, stdout, frames, tx) => outcome,
        };
        let _ = child.start_kill();
        let _ = child.wait().await;
        match outcome {
            Ok(terminal) => terminal,
            Err(AssistantEvent::Error { message, aborted }) if message.is_empty() => {
                let detail = errors.await.unwrap_or_default();
                let detail = detail.trim();
                let tail: String = detail.chars().rev().take(500).collect::<Vec<_>>().into_iter().rev().collect();
                let message = if tail.is_empty() { "Claude Code ended before it replied".to_string() } else { format!("Claude Code ended before it replied: {tail}") };
                AssistantEvent::Error { message, aborted }
            }
            Err(terminal) => terminal,
        }
    }

    /// Replays the frames and reads the reply. Claude Code records a history frame only once it
    /// has answered the one before: each `shouldQuery: false` frame waits for its zero-turn
    /// result, or the model never sees the turns it replays. Stdin closes after the last frame,
    /// so Claude Code asks once and ends. `Err` with an empty message is an end before any
    /// reply, which stderr says.
    async fn converse(&self, mut stdin: tokio::process::ChildStdin, stdout: tokio::process::ChildStdout, frames: &[Value], tx: &mpsc::Sender<AssistantEvent>) -> Result<AssistantEvent, AssistantEvent> {
        let fail = |message: String| Err(AssistantEvent::Error { message, aborted: false });
        let mut lines = BufReader::new(stdout).lines();
        for frame in frames {
            if stdin.write_all((frame.to_string() + "\n").as_bytes()).await.is_err() {
                return fail(String::new());
            }
            if frame["shouldQuery"] != false {
                continue;
            }
            loop {
                let line = match tokio::time::timeout(HISTORY_ACK, lines.next_line()).await {
                    Ok(line) => line,
                    Err(_) => return fail("Claude Code did not take the conversation in time".into()),
                };
                let Ok(Some(line)) = line else { return fail(String::new()) };
                let Ok(event) = serde_json::from_str::<Value>(&line) else { continue };
                if event["type"] != "result" {
                    continue;
                }
                if event["is_error"] == true || event["num_turns"].as_u64().is_some_and(|turns| turns > 0) {
                    let text = event["result"].as_str().filter(|text| !text.is_empty()).unwrap_or("Claude Code did not take the conversation");
                    return fail(usage_credits_refusal(text).unwrap_or_else(|| text.to_string()));
                }
                break;
            }
        }
        let _ = stdin.shutdown().await;
        drop(stdin);
        self.read(lines, tx).await
    }

    /// Reads Claude Code's events into the stream until the reply is whole and its plan-limit
    /// report is in.
    async fn read(&self, mut lines: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>, tx: &mpsc::Sender<AssistantEvent>) -> Result<AssistantEvent, AssistantEvent> {
        let fail = |message: String| Err(AssistantEvent::Error { message, aborted: false });
        let mut state = MessagesState::new();
        let mut terminal: Option<AssistantEvent> = None;
        let mut deadline: Option<tokio::time::Instant> = None;
        loop {
            let line = match deadline {
                Some(deadline) => match tokio::time::timeout_at(deadline, lines.next_line()).await {
                    Ok(line) => line,
                    Err(_) => break,
                },
                None => lines.next_line().await,
            };
            let Ok(Some(line)) = line else { break };
            let Ok(event) = serde_json::from_str::<Value>(&line) else { continue };
            match event["type"].as_str().unwrap_or_default() {
                // The model's own events, the one request's; a subagent's are not this call's.
                "stream_event" if event["parent_tool_use_id"].is_null() && terminal.is_none() => {
                    let mut event = event["event"].clone();
                    let kind = event["type"].as_str().unwrap_or_default().to_string();
                    if kind == "content_block_start" && event["content_block"]["type"] == "tool_use" {
                        let name = event["content_block"]["name"].as_str().unwrap_or_default().to_string();
                        let Some(tool) = name.strip_prefix(TOOL_PREFIX) else { return fail(format!("Claude Code called a tool of its own, {name}")) };
                        event["content_block"]["name"] = Value::String(tool.to_string());
                    }
                    match state.apply(&kind, &event, tx).await {
                        Ok(true) => {
                            terminal = Some(state.finish(true, self.info, "Claude Code", tx).await);
                            deadline = Some(tokio::time::Instant::now() + AFTER_STOP);
                        }
                        Ok(false) => {}
                        Err(message) => return fail(usage_credits_refusal(&message).unwrap_or(message)),
                    }
                }
                // Claude Code's word on a request that failed: an API error as a message.
                "assistant" if terminal.is_none() && (event["error"].is_string() || event["message"]["error"].is_string() || event["is_api_error_message"] == true) => {
                    let text: String = event["message"]["content"].as_array().into_iter().flatten().filter_map(|block| block["text"].as_str()).collect::<Vec<_>>().join("\n");
                    let text = if text.is_empty() { "Claude Code's request failed".to_string() } else { text };
                    return fail(usage_credits_refusal(&text).unwrap_or(text));
                }
                "rate_limit_event" => {
                    let info = &event["rate_limit_info"];
                    if info["isUsingOverage"] == true && !self.allow_usage_credits {
                        let status = info["status"].as_str().unwrap_or("unknown");
                        return fail(format!("Claude Code billing: this reply was charged to Claude usage credits instead of your plan's limits ({status}). Lorca stopped so no more are. Turn usage credits off at claude.ai/settings/usage, or set LORCA_CLAUDE_CODE_ALLOW_USAGE_CREDITS=1 to allow them."));
                    }
                    if terminal.is_some() {
                        break;
                    }
                }
                "result" if terminal.is_some() => break,
                // A result before any reply is a failure.
                "result" if event["is_error"] == true => {
                    let text = event["result"].as_str().filter(|text| !text.is_empty()).map(str::to_string).unwrap_or_else(|| format!("Claude Code ended with {}", event["subtype"].as_str().unwrap_or("an error")));
                    return fail(usage_credits_refusal(&text).unwrap_or(text));
                }
                _ => {}
            }
        }
        match terminal {
            Some(terminal) => Ok(terminal),
            None => fail(String::new()),
        }
    }
}

/// A tool server that lists the tools in `manifest` (a JSON array of `{ name, description,
/// inputSchema }`) over MCP on stdin and stdout, and refuses every call: Claude Code shows the
/// model the bot's tools, and Lorca runs them. Ends when stdin does.
pub fn serve_tools(manifest: &Path) -> std::io::Result<()> {
    let tools: Value = serde_json::from_slice(&std::fs::read(manifest)?)?;
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(request) = serde_json::from_str::<Value>(&line?) else { continue };
        let result = match request["method"].as_str().unwrap_or_default() {
            "initialize" => json!({ "protocolVersion": "2024-11-05", "capabilities": { "tools": {} }, "serverInfo": { "name": SERVER, "version": "1" } }),
            "tools/list" => json!({ "tools": tools }),
            "tools/call" => json!({ "isError": true, "content": [{ "type": "text", "text": "Lorca runs this tool, not Claude Code." }] }),
            _ => json!({}),
        };
        if !request["id"].is_null() {
            writeln!(stdout, "{}", json!({ "jsonrpc": "2.0", "id": request["id"], "result": result }))?;
            stdout.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{AssistantAccumulator, ToolSpec};
    use crate::types::{AssistantMessage, AssistantPart, ContentPart, LlmMessage, StopReason, ToolCall, ToolResultMessage, UserMessage};
    use futures::StreamExt;

    fn provider(program: &Path, model: &str) -> ClaudeCodeProvider {
        ClaudeCodeProvider::new(program.to_path_buf(), (PathBuf::from("lorca"), vec!["claude-code-tools".into()]), std::env::vars_os().collect(), model)
    }

    fn request() -> ModelRequest {
        let mut assistant = AssistantMessage::empty(PROVIDER_ID, "claude-haiku-5-5");
        assistant.content.push(AssistantPart::ToolCall(ToolCall { id: "toolu_1".into(), name: "get_weather".into(), arguments: json!({ "city": "Paris" }) }));
        assistant.stop_reason = StopReason::ToolUse;
        ModelRequest {
            system_prompt: "You are Scout.".into(),
            messages: vec![
                LlmMessage::User(UserMessage::text("Weather in Paris?")),
                LlmMessage::Assistant(assistant),
                LlmMessage::ToolResult(ToolResultMessage {
                    tool_call_id: "toolu_1".into(),
                    tool_name: "get_weather".into(),
                    content: vec![ContentPart::text("Sunny, 21°C")],
                    details: Value::Null,
                    is_error: false,
                    timestamp: 0,
                }),
            ],
            tools: vec![ToolSpec { name: "get_weather".into(), description: "Weather for a city".into(), parameters: json!({ "type": "object", "properties": { "city": { "type": "string" } } }) }],
            cache_points: Vec::new(),
            max_tokens: None,
            options: Default::default(),
        }
    }

    #[test]
    fn the_conversation_replays_as_frames_and_the_tools_as_the_servers() {
        let plan = provider(Path::new("claude"), "claude-haiku-5-5").with_thinking(Some(ThinkingLevel::Off)).plan(&request()).unwrap();
        assert_eq!(plan.frames.len(), 3);
        assert_eq!(plan.frames[0]["shouldQuery"], false, "history asks nothing");
        assert_eq!(plan.frames[1]["message"]["content"][0]["name"], "mcp__lorca__get_weather");
        assert!(plan.frames[2].get("shouldQuery").is_none(), "the last frame asks");
        assert_eq!(plan.frames[2]["message"]["content"][0]["type"], "tool_result");
        assert_eq!(plan.system, "You are Scout.");
        assert_eq!(plan.tools, vec![json!({ "name": "get_weather", "description": "Weather for a city", "inputSchema": { "type": "object", "properties": { "city": { "type": "string" } } } })]);
        assert_eq!(plan.extra, json!({ "thinking": { "type": "disabled" }, "context_management": { "edits": [] } }));

        let plan = provider(Path::new("claude"), "claude-opus-5-5").with_thinking(Some(ThinkingLevel::High)).plan(&request()).unwrap();
        assert_eq!(plan.extra, json!({ "thinking": { "type": "adaptive" }, "output_config": { "effort": "high" } }));
        assert!(plan.max_tokens.is_some());

        let mut ends_with_reply = request();
        let mut reply = AssistantMessage::empty(PROVIDER_ID, "claude-haiku-5-5");
        reply.content.push(AssistantPart::Text { text: "Sunny.".into() });
        ends_with_reply.messages = vec![LlmMessage::User(UserMessage::text("Weather?")), LlmMessage::Assistant(reply)];
        assert!(provider(Path::new("claude"), "claude-haiku-5-5").plan(&ends_with_reply).is_err());
    }

    /// A stand-in for `claude` that acknowledges each history frame, prints `output` once stdin
    /// ends, as Claude Code would, and records its arguments, input, and environment beside it.
    #[cfg(unix)]
    fn fake_claude(folder: &Path, output: &[Value]) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(folder).unwrap();
        let lines: String = output.iter().map(|line| line.to_string() + "\n").collect();
        std::fs::write(folder.join("output.jsonl"), lines).unwrap();
        let script = folder.join("claude");
        let body = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{dir}/args'\nenv > '{dir}/env'\nwhile IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> '{dir}/stdin'\n  case \"$line\" in *'\"shouldQuery\":false'*) echo '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"num_turns\":0}}';; esac\ndone\ncat '{dir}/output.jsonl'\nsleep 30\n",
            dir = folder.display()
        );
        std::fs::write(&script, body).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    fn stream_event(event: Value) -> Value {
        json!({ "type": "stream_event", "event": event, "parent_tool_use_id": null })
    }

    async fn run(provider: &ClaudeCodeProvider) -> AssistantMessage {
        let mut stream = provider.stream(request(), CancellationToken::new()).await;
        let mut accumulator = AssistantAccumulator::new(PROVIDER_ID, provider.model_id());
        while let Some(event) = stream.next().await {
            accumulator.apply(&event);
        }
        accumulator.finish(false)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_tool_call_streams_back_under_the_bots_name_and_the_process_ends_at_the_stop() {
        let folder = std::env::temp_dir().join(format!("lorca-fake-claude-{}", uuid::Uuid::new_v4()));
        let output = [
            json!({ "type": "system", "subtype": "init" }),
            json!({ "type": "result", "subtype": "success", "is_error": false, "num_turns": 0 }),
            stream_event(json!({ "type": "message_start", "message": { "usage": { "input_tokens": 3, "cache_read_input_tokens": 900 } } })),
            stream_event(json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } })),
            stream_event(json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "Checking." } })),
            stream_event(json!({ "type": "content_block_stop", "index": 0 })),
            stream_event(json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "tool_use", "id": "toolu_2", "name": "mcp__lorca__get_weather", "input": {} } })),
            stream_event(json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "input_json_delta", "partial_json": "{\"city\":\"Lyon\"}" } })),
            stream_event(json!({ "type": "content_block_stop", "index": 1 })),
            stream_event(json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" }, "usage": { "output_tokens": 12 } })),
            stream_event(json!({ "type": "message_stop" })),
            json!({ "type": "rate_limit_event", "rate_limit_info": { "status": "allowed", "isUsingOverage": false } }),
        ];
        let program = fake_claude(&folder, &output);
        let mut provider = provider(&program, "claude-haiku-5-5");
        provider.env.push(("ANTHROPIC_API_KEY".into(), "sk-ant-should-not-pass".into()));
        let started = std::time::Instant::now();
        let message = run(&provider).await;
        assert!(started.elapsed() < Duration::from_secs(10), "it ends at the stop, not when the process does");
        assert_eq!(message.stop_reason, StopReason::ToolUse, "{:?}", message.error_message);
        assert_eq!(message.text(), "Checking.");
        let calls = message.tool_calls();
        assert_eq!((calls[0].name.as_str(), &calls[0].arguments), ("get_weather", &json!({ "city": "Lyon" })));
        assert_eq!((message.usage.input, message.usage.cache_read, message.usage.output), (3, 900, 12));

        let args = std::fs::read_to_string(folder.join("args")).unwrap();
        for arg in ["--max-turns", "dontAsk", "--include-partial-messages", "claude-haiku-5-5", "\"lorca\""] {
            assert!(args.contains(arg), "{arg} in {args}");
        }
        let env = std::fs::read_to_string(folder.join("env")).unwrap();
        assert!(!env.contains("sk-ant-should-not-pass") && env.contains("CLAUDE_CODE_MAX_RETRIES=0"));
        let stdin: Vec<Value> = std::fs::read_to_string(folder.join("stdin")).unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(stdin.len(), 3);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn api_errors_overage_and_foreign_tools_fail_the_call() {
        let cases = [
            (
                vec![json!({ "type": "assistant", "is_api_error_message": true, "message": { "content": [{ "type": "text", "text": "API Error: 400 bad request" }] } })],
                "API Error: 400 bad request",
            ),
            (
                vec![
                    stream_event(json!({ "type": "message_start", "message": { "usage": {} } })),
                    stream_event(json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "tool_use", "id": "t", "name": "Bash", "input": {} } })),
                ],
                "Claude Code called a tool of its own, Bash",
            ),
            (
                vec![
                    stream_event(json!({ "type": "message_start", "message": { "usage": {} } })),
                    stream_event(json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn" }, "usage": { "output_tokens": 1 } })),
                    stream_event(json!({ "type": "message_stop" })),
                    json!({ "type": "rate_limit_event", "rate_limit_info": { "status": "allowed", "isUsingOverage": true } }),
                ],
                "Claude Code billing: this reply was charged to Claude usage credits",
            ),
            (vec![json!({ "type": "result", "subtype": "success", "is_error": true, "result": "Not logged in · Please run /login" })], "Not logged in"),
        ];
        for (output, expected) in cases {
            let folder = std::env::temp_dir().join(format!("lorca-fake-claude-{}", uuid::Uuid::new_v4()));
            let message = run(&provider(&fake_claude(&folder, &output), "claude-haiku-5-5")).await;
            assert_eq!(message.stop_reason, StopReason::Error);
            assert!(message.error_message.as_deref().unwrap_or_default().contains(expected), "{:?} for {expected}", message.error_message);
            let _ = std::fs::remove_dir_all(&folder);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_process_that_ends_without_a_reply_says_what_it_printed() {
        use std::os::unix::fs::PermissionsExt;
        let folder = std::env::temp_dir().join(format!("lorca-fake-claude-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&folder).unwrap();
        let script = folder.join("claude");
        std::fs::write(&script, "#!/bin/sh\nhead -n 1 > /dev/null\necho 'error: unknown option' >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let message = run(&provider(&script, "claude-haiku-5-5")).await;
        assert_eq!(message.error_message.as_deref(), Some("Claude Code ended before it replied: error: unknown option"));
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// The real `claude`, signed in, with `lorca claude-code-tools` from a build of the CLI:
    /// `cargo build -p lorca && cargo test -p lorca-agent live_claude_code -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_claude_code_calls_a_tool_and_reads_its_result() {
        let lorca = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/lorca");
        let mut provider = ClaudeCodeProvider::new(PathBuf::from("claude"), (lorca, vec!["claude-code-tools".into()]), std::env::vars_os().collect(), "claude-haiku-5-5").with_thinking(Some(ThinkingLevel::Low));
        provider.allow_usage_credits = false;
        let mut asking = request();
        asking.messages.truncate(1);
        asking.messages[0] = LlmMessage::User(UserMessage::text("What's the weather in Paris? Call get_weather."));
        let collect = |request: ModelRequest| {
            let provider = &provider;
            async move {
                let mut stream = provider.stream(request, CancellationToken::new()).await;
                let mut accumulator = AssistantAccumulator::new(PROVIDER_ID, provider.model_id());
                while let Some(event) = stream.next().await {
                    accumulator.apply(&event);
                }
                accumulator.finish(false)
            }
        };
        let call = collect(asking).await;
        assert_eq!(call.stop_reason, StopReason::ToolUse, "{:?}", call.error_message);
        assert_eq!(call.tool_calls()[0].name, "get_weather");
        assert!(call.usage.output > 0);

        let mut answered = request();
        answered.messages[1] = LlmMessage::Assistant(call.clone());
        if let LlmMessage::ToolResult(result) = &mut answered.messages[2] {
            result.tool_call_id = call.tool_calls()[0].id.clone();
        }
        let reply = collect(answered).await;
        assert_eq!(reply.stop_reason, StopReason::Stop, "{:?}", reply.error_message);
        assert!(reply.text().contains("21"), "{}", reply.text());
    }

    #[test]
    fn usage_credit_refusals_say_what_to_do() {
        assert!(usage_credits_refusal("400: Third-party apps now draw from your extra usage").unwrap().starts_with("Claude Code billing"));
        assert!(usage_credits_refusal("overloaded").is_none());
    }
}
