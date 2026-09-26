//! Codex App Server owns the agent loop. Lorca is a JSON-RPC client and projects its
//! events into the same encrypted chat stream used by every Device.

use super::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Lines};
use tokio::process::Child;

use crate::local_store::CodexSession;

const RPC_TIMEOUT: Duration = Duration::from_secs(30);

/// Catalogs are read on the assigned Runner, including its account and project defaults.
/// Only picker metadata leaves this boundary; config secrets and auth never do.
pub(crate) async fn models(app: &Arc<App>, params: &Value) -> Result<Value, String> {
    if params["runner_id"].as_str() != app.this_device_id().as_deref() {
        return Err("This request belongs to another Runner".into());
    }
    let workdir = if let Some(id) = params["bot_id"].as_str() {
        let bot = app.bot(id).ok_or("Unknown bot")?;
        if Some(bot.runner_id.as_str()) != app.this_device_id().as_deref() {
            return Err("This bot belongs to another Runner".into());
        }
        bot.working_directory(&app.config.home)
    } else {
        app.config.home.clone()
    };
    // Opening a picker must not create a bot workspace.
    let workdir = if workdir.is_dir() { workdir } else { app.config.home.clone() };
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut connection = Connection::spawn(&workdir).await?;
        connection.initialize().await?;
        read_models(&mut connection, &workdir).await
    })
    .await
    .map_err(|_| "Codex did not return its model list in time".to_string())?
    .map_err(|error| format!("{error:#}"))
}

async fn read_models(connection: &mut Connection, workdir: &Path) -> anyhow::Result<Value> {
    let config = connection.request("config/read", json!({ "cwd": workdir, "includeLayers": false })).await?;
    let config = &config["config"];
    let mut models = Vec::new();
    let mut cursor = Value::Null;
    let mut seen = HashSet::new();
    let mut default_model = config["model"].as_str().map(str::to_string);
    loop {
        let page = connection.request("model/list", json!({ "limit": 100, "includeHidden": false, "cursor": cursor })).await?;
        for model in page["data"].as_array().context("Codex returned no model catalog")? {
            let Some(id) = model["model"].as_str() else { continue };
            if default_model.is_none() && model["isDefault"] == true {
                default_model = Some(id.into());
            }
            let fast = model["serviceTiers"]
                .as_array()
                .and_then(|tiers| tiers.iter().find(|tier| tier["id"] == "priority" || tier["name"] == "Fast"));
            let legacy_fast = model["additionalSpeedTiers"].as_array().is_some_and(|tiers| tiers.iter().any(|tier| tier == "fast"));
            models.push(json!({
                "id": id, "label": model["displayName"].as_str().unwrap_or(id),
                "levels": model["supportedReasoningEfforts"].as_array().map(|levels| levels.iter().filter_map(|l| l["reasoningEffort"].as_str()).collect::<Vec<_>>()).unwrap_or_default(),
                "default_thinking": model["defaultReasoningEffort"],
                "default_service_tier": model["defaultServiceTier"],
                "fast_tier": fast.and_then(|tier| tier["id"].as_str()).or(legacy_fast.then_some("priority")),
                "fast_description": fast.and_then(|tier| tier["description"].as_str()),
            }));
        }
        cursor = page["nextCursor"].clone();
        let Some(next) = cursor.as_str() else { break };
        if !seen.insert(next.to_string()) || models.len() > 1000 {
            bail!("Codex returned an invalid model catalog cursor");
        }
    }
    if default_model.is_none() {
        default_model = models.first().and_then(|m| m["id"].as_str()).map(str::to_string);
    }
    Ok(json!({ "models": models, "default_model": default_model,
        "default_thinking": config["model_reasoning_effort"], "default_service_tier": config["service_tier"] }))
}

fn runtime_params(bot: &Bot, catalog: &Value) -> anyhow::Result<Value> {
    let model = bot.model.as_deref().or(catalog["default_model"].as_str()).context("Codex returned no default model; select a model")?;
    let entry = catalog["models"].as_array().and_then(|models| models.iter().find(|m| m["id"] == model));
    let levels = entry.and_then(|m| m["levels"].as_array());
    let supports = |effort: &str| levels.is_none_or(|levels| levels.iter().any(|l| l == effort));
    let effort = match bot.thinking.as_deref() {
        Some(effort) => {
            let effort = if effort == "off" { "none" } else { effort };
            if !supports(effort) {
                bail!("Codex model {model} does not support thinking level {effort}; choose a supported level");
            }
            Some(effort)
        }
        None => catalog["default_thinking"].as_str().filter(|e| supports(e)).or_else(|| entry.and_then(|m| m["default_thinking"].as_str())),
    };
    let tier = match bot.codex_options.speed {
        CodexSpeed::Default => catalog["default_service_tier"]
            .as_str()
            .filter(|tier| *tier != "priority" || entry.is_none_or(|m| m["fast_tier"].is_string()))
            .or_else(|| entry.and_then(|m| m["default_service_tier"].as_str()))
            .unwrap_or("default"),
        CodexSpeed::Standard => "default",
        CodexSpeed::Fast => entry.and_then(|m| m["fast_tier"].as_str()).context("This Codex model does not offer Fast mode")?,
    };
    Ok(json!({ "model": model, "effort": effort, "serviceTierForTurn": tier,
        "approvalPolicy": "on-request",
        "approvalsReviewer": match bot.codex_options.approvals { CodexApprovals::AutoReview => "auto_review", CodexApprovals::User => "user" },
    }))
}

/// One transport per active Lorca turn. Durable Codex threads resume across processes.
/// The process group also bounds cleanup if a server exits while a tool is running.
struct Connection {
    child: Option<Child>,
    writer: Box<dyn AsyncWrite + Unpin + Send>,
    reader: Lines<BufReader<Box<dyn AsyncRead + Unpin + Send>>>,
    pending: VecDeque<Value>,
    next_id: u64,
    active_turn: Option<(String, String)>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            #[cfg(unix)]
            if let Some(pid) = child.id() {
                // This is the dedicated group created by spawn(), never Lorca's group.
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
            let _ = child.start_kill();
        }
    }
}

impl Connection {
    async fn spawn(workdir: &Path) -> anyhow::Result<Self> {
        let executable = std::env::var_os("LORCA_CODEX_BIN").unwrap_or_else(|| "codex".into());
        let mut command = lorca_agent::login_shell::command(executable).await;
        command
            .args(["app-server", "--listen", "stdio://"])
            .current_dir(workdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let mut child =
            command.spawn().context("Cannot start Codex. Install Codex on this Runner, or set LORCA_CODEX_BIN to its executable")?;
        let writer = Box::new(child.stdin.take().context("Codex stdin is unavailable")?);
        let reader: Box<dyn AsyncRead + Unpin + Send> = Box::new(child.stdout.take().context("Codex stdout is unavailable")?);
        Ok(Self {
            child: Some(child),
            writer,
            reader: BufReader::new(reader).lines(),
            pending: VecDeque::new(),
            next_id: 0,
            active_turn: None,
        })
    }

    async fn send(&mut self, value: Value) -> anyhow::Result<()> {
        let mut bytes = serde_json::to_vec(&value)?;
        bytes.push(b'\n');
        self.writer.write_all(&bytes).await?;
        self.writer.flush().await?;
        Ok(())
    }

    async fn read(&mut self) -> anyhow::Result<Value> {
        let line = self.reader.next_line().await?.context("Codex App Server closed the connection")?;
        serde_json::from_str(&line).context("Codex returned an invalid JSON-RPC message")
    }

    async fn next(&mut self) -> anyhow::Result<Value> {
        match self.pending.pop_front() {
            Some(value) => Ok(value),
            None => self.read().await,
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({ "id": id, "method": method, "params": params })).await?;
        tokio::time::timeout(RPC_TIMEOUT, async {
            loop {
                let value = self.read().await?;
                if value.get("method").is_none() && value["id"] == id {
                    if let Some(error) = value.get("error") {
                        bail!("Codex {method}: {}", error["message"].as_str().unwrap_or("request failed"));
                    }
                    return value.get("result").cloned().context("Codex response has no result");
                }
                self.pending.push_back(value);
            }
        })
        .await
        .context("Codex App Server did not answer within 30 seconds")?
    }

    async fn initialize(&mut self) -> anyhow::Result<()> {
        self.request(
            "initialize",
            json!({
                "clientInfo": { "name": "lorca", "title": "Lorca", "version": crate::config::VERSION },
                "capabilities": { "experimentalApi": true }
            }),
        )
        .await?;
        self.send(json!({ "method": "initialized", "params": {} })).await
    }

    async fn interrupt(&mut self) {
        let Some((thread, turn)) = self.active_turn.clone() else { return };
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            self.request("turn/interrupt", json!({ "threadId": thread, "turnId": turn })).await?;
            loop {
                let value = self.next().await?;
                if value["method"] == "turn/completed" && value["params"]["turn"]["id"] == turn {
                    return Ok::<_, anyhow::Error>(());
                }
            }
        })
        .await;
    }
}

pub(super) async fn run(
    app: &Arc<App>,
    job: &Job,
    bot: &Bot,
    routine: Option<&Routine>,
    trigger: &Trigger,
    cancel: CancellationToken,
) -> TurnOutcome {
    let Some(chat) = app.chat(&job.chat_id) else { return TurnOutcome::Skipped };
    let mut output = Output::new(app, job, bot);
    let workdir = bot.working_directory(&app.config.home);
    let result = async {
        std::fs::create_dir_all(&workdir).context("Cannot create the bot's workspace")?;
        let workdir = std::fs::canonicalize(workdir)?;
        let mut connection = tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            result = Connection::spawn(&workdir) => result?,
        };
        let result = tokio::select! {
            _ = cancel.cancelled() => Ok(()),
            result = run_connected(&mut connection, app, job, bot, &chat, routine, trigger, &workdir, &cancel, &mut output) => result,
        };
        if cancel.is_cancelled() {
            close_permissions(app, &chat.meta.id);
            connection.interrupt().await;
        }
        result
    }
    .await;
    output.finish();
    if let Err(error) = result {
        output.fail(&format!("{error:#}"));
    }
    if let Some(error) = &output.error {
        crate::push::failed(app, &chat, bot, error);
    } else if !cancel.is_cancelled() {
        if let Some(text) = &output.last_text {
            crate::push::reply(app, &chat, bot, text);
        }
    }
    if let Some(line) = turn_log_line(output.last_text.as_deref(), &output.tools_used, output.error.is_some()) {
        let _ =
            MemoryStore::for_bot(&app.config.home, bot).append_log(&line, Some(&format!("in {}", chat_source(&chat))), now_secs() as i64);
    }
    if output.error.is_some() {
        TurnOutcome::Skipped
    } else if output.last_text.is_some() {
        TurnOutcome::Sent
    } else {
        TurnOutcome::Pass
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_connected(
    connection: &mut Connection,
    app: &Arc<App>,
    job: &Job,
    bot: &Bot,
    chat: &Chat,
    routine: Option<&Routine>,
    trigger: &Trigger,
    workdir: &Path,
    cancel: &CancellationToken,
    output: &mut Output,
) -> anyhow::Result<()> {
    connection.initialize().await?;
    let account = connection.request("account/read", json!({ "refreshToken": false })).await?;
    if account["requiresOpenaiAuth"] == true && account["account"].is_null() {
        bail!("Codex is not signed in on this Runner. Run codex login there, then send the message again.");
    }
    let catalog = read_models(connection, workdir).await?;
    let settings = runtime_params(bot, &catalog)?;
    let workdir_key = workdir.to_string_lossy();
    let stored = app.store.codex_session(&chat.meta.id, &bot.id, &workdir_key)?;
    let store = MemoryStore::for_bot(&app.config.home, bot);
    let (plugin_tools, briefs) = crate::plugins::mcp::turn_tools(app, &chat.meta.id, trigger, bot, routine.is_some());
    let mut tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(ListTeammates { app: app.clone(), chat_id: chat.meta.id.clone() }),
        Arc::new(MessageBot { app: app.clone(), chat_id: chat.meta.id.clone(), bot: bot.clone(), hops: job.hops }),
        Arc::new(CreateBot { app: app.clone(), chat_id: chat.meta.id.clone(), bot: bot.clone() }),
        Arc::new(EditBot { app: app.clone(), bot: bot.clone() }),
        Arc::new(Routines { app: app.clone(), bot: bot.clone() }),
        Arc::new(SearchPlugins { app: app.clone() }),
        Arc::new(InstallPlugin { app: app.clone(), chat_id: chat.meta.id.clone(), bot: bot.clone(), unattended: routine.is_some() }),
        Arc::new(ConnectPlugin { app: app.clone(), chat_id: chat.meta.id.clone(), bot: bot.clone() }),
        Arc::new(Recall { app: app.clone(), store: store.clone(), bot: bot.clone() }),
    ];
    tools.extend(memory_tools(&store, chat));
    tools.extend(plugin_tools.discovery_tools());
    let mut instructions = system_prompt(app, chat, bot, job, &store, routine, &briefs);
    instructions.push_str("\nLorca tool catalog (call through lorca_call; use tool=catalog to refresh after plugin discovery):\n");
    instructions.push_str(&serde_json::to_string(&tools.iter().map(|t| t.spec()).collect::<Vec<_>>())?);
    let mut params = json!({ "cwd": workdir, "developerInstructions": instructions,
        "model": settings["model"], "approvalPolicy": settings["approvalPolicy"], "approvalsReviewer": settings["approvalsReviewer"] });
    let response = if let Some(session) = &stored {
        params["threadId"] = json!(session.thread_id);
        connection.request("thread/resume", params).await?
    } else {
        params["dynamicTools"] = json!([{
            "type": "function", "name": "lorca_call",
            "description": "Call a Lorca team, memory, routine, or discovered plugin tool. tool=catalog returns complete schemas for available tools.",
            "inputSchema": { "type": "object", "properties": {
                "tool": { "type": "string" }, "arguments": { "type": "object" }
            }, "required": ["tool", "arguments"], "additionalProperties": false }
        }]);
        connection.request("thread/start", params).await?
    };
    let thread_id = response["thread"]["id"].as_str().context("Codex did not return a thread id")?.to_string();
    let mut session =
        CodexSession { thread_id: thread_id.clone(), after_message_id: stored.as_ref().and_then(|s| s.after_message_id.clone()) };
    app.store.save_codex_session(&chat.meta.id, &bot.id, &workdir_key, &session)?;
    let (messages, found) = app.store.context(&chat.meta.id, session.after_message_id.as_deref(), Some(MAX_CONTEXT_MESSAGES))?;
    if session.after_message_id.is_some() && !found {
        bail!("The Lorca message used to resume this Codex chat is unavailable. Create a new chat to continue with fresh context.");
    }
    let mut input = input_for(app, bot, workdir, &messages, stored.is_none()).await;
    if let Some(routine) = routine {
        input.push(json!({ "type": "text", "text": format!("Routine {}: {}", routine.name, routine.prompt) }));
    }
    if job.kind == "room_turn" {
        input.push(json!({ "type": "text", "text": room_turn_cue(app, chat, bot, job) }));
    }
    if let Some(setup) = &job.setup {
        input.push(json!({ "type": "text", "text": setup_cue(app, setup) }));
    }
    if input.is_empty() {
        input.push(json!({ "type": "text", "text": "Continue." }));
    }
    // Send effective values every turn so switching back to Default also clears a
    // previous thread override. A missing field would keep that old override alive.
    let mut params = settings;
    params["threadId"] = json!(thread_id);
    params["input"] = json!(input);
    let response = connection.request("turn/start", params).await?;
    let turn_id = response["turn"]["id"].as_str().context("Codex did not return a turn id")?.to_string();
    connection.active_turn = Some((thread_id.clone(), turn_id.clone()));
    // Advance only after Codex accepted the input; output stays in Codex's own history.
    session.after_message_id = messages.last().map(|m| m.id.clone()).or(session.after_message_id);
    app.store.save_codex_session(&chat.meta.id, &bot.id, &workdir_key, &session)?;
    if !chat.meta.is_group() {
        if let Some(index) = messages.iter().position(|m| m.id == job.trigger_message_id) {
            for message in &messages[index + 1..] {
                if message.author == Author::You {
                    app.claim_steering_message(&chat.meta.id, &message.id);
                }
            }
        }
    }
    let mut steering = tokio::time::interval(Duration::from_millis(250));
    steering.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let value = tokio::select! {
            value = connection.next() => value?,
            _ = steering.tick(), if !chat.meta.is_group() => {
                if let Some(after) = &session.after_message_id {
                    let fresh: Vec<_> = app.store.messages_after(&chat.meta.id, after)?.into_iter()
                        .filter(|m| m.author == Author::You && m.is_complete()).collect();
                    if !fresh.is_empty() {
                        let input = input_for(app, bot, workdir, &fresh, false).await;
                        // A completed turn can race a steer. Leave its admitted Lorca Job
                        // pending unless Codex acknowledges that this turn took the input.
                        if connection.request("turn/steer", json!({ "threadId": thread_id, "expectedTurnId": turn_id, "input": input })).await.is_ok() {
                            for message in &fresh { app.claim_steering_message(&chat.meta.id, &message.id); }
                            session.after_message_id = fresh.last().map(|m| m.id.clone());
                            app.store.save_codex_session(&chat.meta.id, &bot.id, &workdir_key, &session)?;
                        }
                    }
                }
                continue;
            }
        };
        let Some(method) = value["method"].as_str() else { continue };
        let params = &value["params"];
        // Child-agent output stays in Codex. Requests still need an answer: a child
        // waiting for approval must not leave the root turn hanging indefinitely.
        if value.get("id").is_none() {
            if params["threadId"].as_str().is_some_and(|id| id != thread_id) {
                continue;
            }
            if params["turnId"].as_str().is_some_and(|id| id != turn_id) {
                continue;
            }
        }
        if let Some(id) = value.get("id") {
            let response = match method {
                "item/tool/call" if params["tool"] == "lorca_call" => call_tool(params, &tools, &plugin_tools, cancel).await,
                "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                    let summary = output.approval_summary(method, params);
                    let decision = if routine.is_some() {
                        crate::plugins::mcp::Decision::Denied
                    } else {
                        crate::plugins::mcp::ask_with_rule(
                            app,
                            &chat.meta.id,
                            &bot.id,
                            "codex",
                            "Codex",
                            "approval",
                            &summary,
                            params.clone(),
                            params["reason"].as_str().map(str::to_string),
                            None,
                            cancel,
                        )
                        .await
                    };
                    json!({ "decision": if matches!(decision, crate::plugins::mcp::Decision::Allowed | crate::plugins::mcp::Decision::Always) { "accept" } else { "decline" } })
                }
                // Permission-profile grants need a richer UI; grant no extra access.
                "item/permissions/requestApproval" => json!({ "permissions": {}, "scope": "turn" }),
                "item/tool/requestUserInput" => {
                    let questions = params["questions"]
                        .as_array()
                        .map(|q| q.iter().filter_map(|q| q["question"].as_str()).collect::<Vec<_>>().join("\n"))
                        .unwrap_or_default();
                    app.notice(&chat.meta.id, format!("Codex asks:\n{questions}\nReply in the chat to continue."));
                    connection.send(json!({ "id": id, "result": { "answers": {} } })).await?;
                    connection.request("turn/interrupt", json!({ "threadId": thread_id, "turnId": turn_id })).await?;
                    continue;
                }
                _ => {
                    connection
                        .send(json!({ "id": id, "error": { "code": -32601, "message": "This Codex request is not supported by Lorca" } }))
                        .await?;
                    continue;
                }
            };
            connection.send(json!({ "id": id, "result": response })).await?;
        } else if method == "turn/completed" && params["turn"]["id"] == turn_id {
            if params["turn"]["status"] == "failed" {
                bail!("{}", params["turn"]["error"]["message"].as_str().unwrap_or("Codex turn failed"));
            }
            return Ok(());
        } else {
            output.event(method, params);
        }
    }
}

/// A cancelled future must leave neither an unanswered card nor a dropped sender behind.
fn close_permissions(app: &App, chat_id: &str) {
    let pending: Vec<_> = app.pending_permissions.lock().unwrap().extract_if(|_, (chat, _)| chat == chat_id).collect();
    for (id, (_, sender)) in pending {
        let _ = sender.send(crate::plugins::mcp::Decision::Denied);
        if let Some(mut row) = app.message(chat_id, &id) {
            if let Body::Permission { decision, .. } = &mut row.body {
                *decision = "denied".into();
            }
            app.upsert_message(row, true);
        }
    }
}

async fn input_for(app: &Arc<App>, bot: &Bot, workdir: &Path, messages: &[Message], include_own: bool) -> Vec<Value> {
    let mut input = Vec::new();
    for message in messages.iter().filter(|m| m.is_complete()) {
        match (&message.author, &message.body) {
            (Author::Bot { bot_id }, Body::Handoff { to, reason, .. }) if bot_id != &bot.id => {
                let from = app.bot(bot_id).map(|b| b.name).unwrap_or_else(|| bot_id.clone());
                let text = if to == &bot.id {
                    format!("[Message from {from}]: {reason}")
                } else {
                    let to = app.bot(to).map(|b| b.name).unwrap_or_else(|| to.clone());
                    format!("[{from} → {to}]: {reason}")
                };
                input.push(json!({ "type": "text", "text": text }));
            }
            (Author::System, Body::Notice { text, routine_id: Some(id) }) => {
                let text = match app.routine(id) {
                    Some(routine) => format!("[Routine {}]: {}", routine.name, routine.prompt),
                    None => format!("[{text} ran on its schedule]"),
                };
                input.push(json!({ "type": "text", "text": text }));
            }
            _ => {}
        }
        if let Body::Text { text, attachments, mentions } = &message.body {
            if !include_own && matches!(&message.author, Author::Bot { bot_id } if bot_id == &bot.id) {
                continue;
            }
            let label = match &message.author {
                Author::You => "User".into(),
                Author::Bot { bot_id } => app.bot(bot_id).map(|b| b.name).unwrap_or_else(|| bot_id.clone()),
                Author::System => "Context".into(),
            };
            input.push(json!({ "type": "text", "text": format!("[{label}] {}", with_mention_ids(app, text, mentions)) }));
            crate::files::prefetch(app, attachments).await;
            for attachment in attachments {
                if let Some(path) = crate::files::materialize(app, attachment, workdir) {
                    input.push(json!({ "type": "text", "text": format!("Attachment {}: {}", attachment.name, path.display()) }));
                    if attachment.is_image() {
                        input.push(json!({ "type": "localImage", "path": path }));
                    }
                } else {
                    input.push(json!({ "type": "text", "text": format!("Attachment {} is unavailable on this Runner", attachment.name) }));
                }
            }
        }
    }
    input
}

async fn call_tool(params: &Value, base: &[Arc<dyn Tool>], plugins: &crate::plugins::mcp::TurnTools, cancel: &CancellationToken) -> Value {
    let name = params["arguments"]["tool"].as_str().unwrap_or("");
    let tools = plugins.tools_with_selected(base);
    let result = async {
        if name == "catalog" {
            return Ok(ToolResult::text(serde_json::to_string(&tools.iter().map(|t| t.spec()).collect::<Vec<_>>())?));
        }
        let tool = tools
            .iter()
            .find(|t| t.name() == name)
            .ok_or_else(|| ToolError(format!("Unknown Lorca tool {name}; search plugins or call catalog first")))?;
        let args = tool.prepare_arguments(params["arguments"]["arguments"].clone());
        let args = lorca_agent::schema::validate_tool_arguments(name, &tool.parameters(), &args).map_err(ToolError)?;
        tool.execute(params["callId"].as_str().unwrap_or("codex"), args, cancel.clone(), Arc::new(|_| {})).await
    }
    .await;
    match result {
        Ok(result) => {
            let content: Vec<Value> = result
                .content
                .iter()
                .map(|part| match part {
                    ContentPart::Text { text } => json!({ "type": "inputText", "text": text }),
                    ContentPart::Image { data, mime_type } => {
                        json!({ "type": "inputImage", "imageUrl": format!("data:{mime_type};base64,{data}") })
                    }
                })
                .collect();
            json!({ "success": true, "contentItems": content })
        }
        Err(error) => json!({ "success": false, "contentItems": [{ "type": "inputText", "text": error.to_string() }] }),
    }
}

struct Output {
    app: Arc<App>,
    job: Job,
    bot: Bot,
    rows: HashMap<String, Message>,
    texts: HashMap<String, String>,
    completed: HashSet<String>,
    flushed: HashMap<String, (usize, std::time::Instant)>,
    last_text: Option<String>,
    error: Option<String>,
    tools_used: Vec<String>,
}

impl Output {
    fn new(app: &Arc<App>, job: &Job, bot: &Bot) -> Self {
        Self {
            app: app.clone(),
            job: job.clone(),
            bot: bot.clone(),
            rows: HashMap::new(),
            texts: HashMap::new(),
            completed: HashSet::new(),
            flushed: HashMap::new(),
            last_text: None,
            error: None,
            tools_used: Vec::new(),
        }
    }

    fn text(&mut self, id: &str, text: &str, complete: bool) {
        if self.completed.contains(id) {
            return;
        }
        if complete {
            self.completed.insert(id.into());
        }
        self.texts.insert(id.to_string(), text.to_string());
        let (shown, last) = self.flushed.entry(id.into()).or_insert((0, std::time::Instant::now()));
        let cut = if complete { Some(text.len()) } else { chunk_boundary(text, *shown, last.elapsed()) };
        let Some(cut) = cut else { return };
        if text.trim().is_empty() || is_pass(text) {
            return;
        }
        *shown = cut;
        *last = std::time::Instant::now();
        let row = self
            .rows
            .entry(id.into())
            .or_insert_with(|| Message::new(&self.job.chat_id, Author::Bot { bot_id: self.bot.id.clone() }, Body::text("")));
        row.body = Body::text(&text[..cut]);
        row.state = if complete { MessageState::Complete } else { MessageState::Streaming };
        self.app.upsert_message(row.clone(), true);
        if complete {
            self.last_text = Some(text.to_string());
        }
    }

    fn approval_summary(&self, method: &str, params: &Value) -> String {
        let mut parts = Vec::new();
        if let Some(command) = params["command"].as_str() {
            parts.push(command.to_string());
        }
        if let Some(cwd) = params["cwd"].as_str() {
            parts.push(format!("Working directory: {cwd}"));
        }
        if let Some(network) = params.get("networkApprovalContext").filter(|v| !v.is_null()) {
            parts.push(format!("Network access: {network}"));
        }
        if let Some(permissions) = params.get("additionalPermissions").filter(|v| !v.is_null()) {
            parts.push(format!("Additional access: {permissions}"));
        }
        if method == "item/fileChange/requestApproval" {
            if let Some(Body::Tool { arguments, .. }) = params["itemId"].as_str().and_then(|id| self.rows.get(id)).map(|r| &r.body) {
                if let Some(changes) = arguments["changes"].as_array() {
                    for change in changes {
                        parts.push(format!(
                            "{}\n{}",
                            change["path"].as_str().unwrap_or("File change"),
                            change["diff"].as_str().unwrap_or("")
                        ));
                    }
                }
            }
            if let Some(root) = params["grantRoot"].as_str() {
                parts.push(format!("Requested write root: {root}"));
            }
        }
        if parts.is_empty() {
            parts.push(format!("Codex approval: {params}"));
        }
        parts.join("\n\n")
    }

    fn event(&mut self, method: &str, params: &Value) {
        match method {
            "item/agentMessage/delta" => {
                if let (Some(id), Some(delta)) = (params["itemId"].as_str(), params["delta"].as_str()) {
                    let text = self.texts.entry(id.into()).or_default();
                    text.push_str(delta);
                    let text = text.clone();
                    self.text(id, &text, false);
                }
            }
            "item/started" | "item/completed" => {
                let item = &params["item"];
                let Some(id) = item["id"].as_str() else { return };
                let kind = item["type"].as_str().unwrap_or("");
                let complete = method == "item/completed";
                if kind == "agentMessage" {
                    if complete {
                        self.text(id, item["text"].as_str().unwrap_or(""), true);
                    }
                } else if kind == "reasoning" {
                    if !complete {
                        crate::runtime::report_activity(&self.app, &self.job, JobActivity::Thinking);
                    }
                } else if matches!(
                    kind,
                    "commandExecution" | "fileChange" | "mcpToolCall" | "dynamicToolCall" | "webSearch" | "contextCompaction"
                ) {
                    let (name, summary) = match kind {
                        "commandExecution" => ("codex_command", "Running command…"),
                        "fileChange" => ("codex_edit", "Editing files…"),
                        "webSearch" => ("codex_search", "Searching the web…"),
                        "contextCompaction" => ("codex_compact", "Compacting context…"),
                        _ => ("codex_tool", "Using tools…"),
                    };
                    if !self.tools_used.iter().any(|n| n == name) {
                        self.tools_used.push(name.into());
                    }
                    let error = item["status"] == "failed" || item["status"] == "declined";
                    let detail = item["command"].as_str().or(item["tool"].as_str()).unwrap_or(summary).to_string();
                    let row = self
                        .rows
                        .entry(id.into())
                        .or_insert_with(|| Message::new(&self.job.chat_id, Author::Bot { bot_id: self.bot.id.clone() }, Body::text("")));
                    row.body = Body::Tool {
                        name: name.into(),
                        summary: if complete {
                            if error {
                                "Failed"
                            } else {
                                "Finished"
                            }
                        } else {
                            summary
                        }
                        .into(),
                        detail,
                        is_running: !complete,
                        call_id: id.into(),
                        arguments: item.clone(),
                        result: complete.then(|| item.to_string()),
                        is_error: error,
                        description: None,
                        target_bot_id: None,
                        run: None,
                    };
                    row.state = if complete { MessageState::Complete } else { MessageState::Streaming };
                    self.app.upsert_message(row.clone(), true);
                }
            }
            _ => {}
        }
    }

    fn fail(&mut self, error: &str) {
        self.error = Some(error.into());
        let mut row = Message::new(&self.job.chat_id, Author::Bot { bot_id: self.bot.id.clone() }, Body::text(error));
        row.state = MessageState::Failed { error: error.into() };
        self.app.upsert_message(row, true);
    }

    fn finish(&mut self) {
        for (id, text) in self.texts.clone() {
            self.text(&id, &text, true);
        }
        for row in self.rows.values_mut() {
            if let Body::Tool { is_running, summary, .. } = &mut row.body {
                if *is_running {
                    *is_running = false;
                    *summary = "Stopped".into();
                    row.state = MessageState::Complete;
                    self.app.upsert_message(row.clone(), true);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
