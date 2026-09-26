use super::*;
use std::path::PathBuf;
use std::sync::Mutex;

struct Fixture {
    app: Arc<App>,
    home: PathBuf,
    bot: Bot,
    chat: Chat,
    job: Job,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}
impl Fixture {
    fn new() -> Self {
        let home = std::env::temp_dir().join(format!("lorca-codex-test-{}", uuid::Uuid::new_v4()));
        let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        let bot: Bot = serde_json::from_value(json!({ "id": "bot", "name": "Coder", "description": "Use Codex", "symbol_name": "sparkles", "accent": "blue", "runner_id": "runner", "harness": "codex", "provider": "deepseek", "created_at": 1.0 })).unwrap();
        let chat: Chat = serde_json::from_value(
            json!({ "id": "chat", "kind": "dm", "bot_ids": ["bot"], "is_pinned": false, "created_at": 1.0, "unread_count": 0 }),
        )
        .unwrap();
        let message = Message::new("chat", Author::You, Body::text("first request"));
        let job: Job = serde_json::from_value(json!({ "id": "job", "chat_id": "chat", "bot_id": "bot", "kind": "turn", "trigger_message_id": message.id, "requested_by": "runner", "created_at": 1.0 })).unwrap();
        {
            let mut state = app.state.lock().unwrap();
            state.bots.push(bot.clone());
            state.chats.push(chat.clone());
        }
        app.upsert_message(message, false);
        Self { app, home, bot, chat, job }
    }
    async fn run(&self, connection: &mut Connection) -> anyhow::Result<Output> {
        let mut output = Output::new(&self.app, &self.job, &self.bot);
        run_connected(
            connection,
            &self.app,
            &self.job,
            &self.bot,
            &self.chat,
            None,
            &Trigger::default(),
            &self.home,
            &CancellationToken::new(),
            &mut output,
        )
        .await?;
        output.finish();
        Ok(output)
    }
}

fn io_connection(io: tokio::io::DuplexStream) -> Connection {
    let (reader, writer) = tokio::io::split(io);
    Connection {
        child: None,
        writer: Box::new(writer),
        reader: BufReader::new(Box::new(reader) as Box<dyn AsyncRead + Unpin + Send>).lines(),
        pending: VecDeque::new(),
        next_id: 0,
        active_turn: None,
    }
}

/// Exercises the actual JSON-RPC client against a server, including notifications that
/// arrive before the reply to turn/start. No provider credential or network is involved.
fn server(mode: &str, app: Arc<App>) -> (Connection, Arc<Mutex<Vec<Value>>>, tokio::task::JoinHandle<()>) {
    let (client, server) = tokio::io::duplex(1024 * 1024);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let mode = mode.to_string();
    let task = tokio::spawn(async move {
        let mut io = io_connection(server);
        loop {
            let value = io.next().await.unwrap();
            captured.lock().unwrap().push(value.clone());
            let method = value["method"].as_str().unwrap_or("");
            let result = match method {
                "initialize" => json!({}),
                "initialized" => continue,
                "config/read" => {
                    json!({ "config": { "model": "native-default", "model_reasoning_effort": "medium", "service_tier": "priority", "secret": "must-not-leak" } })
                }
                "model/list" => json!({ "data": [
                    { "model": "native-default", "displayName": "Native Default", "isDefault": true,
                      "supportedReasoningEfforts": [{ "reasoningEffort": "low" }, { "reasoningEffort": "medium" }],
                      "defaultReasoningEffort": "medium", "serviceTiers": [{ "id": "priority", "name": "Fast" }] },
                    { "model": "native-other", "displayName": "Native Other", "isDefault": false,
                      "supportedReasoningEfforts": [{ "reasoningEffort": "high" }], "defaultReasoningEffort": "high", "serviceTiers": [] }
                ], "nextCursor": null }),
                "account/read" => {
                    json!({ "requiresOpenaiAuth": true, "account": if mode == "unsigned" { Value::Null } else { json!({ "type": "chatgpt" }) } })
                }
                "thread/start" => {
                    assert_eq!(value["params"]["dynamicTools"][0]["type"], "function");
                    assert!(value["params"].get("baseInstructions").is_none());
                    assert!(value["params"].get("sandbox").is_none());
                    json!({ "thread": { "id": "thread-1" } })
                }
                "thread/resume" => {
                    assert_eq!(value["params"]["threadId"], "thread-1");
                    json!({ "thread": { "id": "thread-1" } })
                }
                "turn/start" => {
                    assert_eq!(value["params"]["threadId"], "thread-1");
                    if mode == "disconnect" {
                        return;
                    }
                    // Subagent output must not become a bubble in the parent chat.
                    io.send(json!({ "method": "item/completed", "params": { "threadId": "other-thread", "turnId": "turn-1", "item": { "type": "agentMessage", "id": "other", "text": "hidden" } } })).await.unwrap();
                    io.send(json!({ "method": "item/agentMessage/delta", "params": { "threadId": "thread-1", "turnId": "turn-1", "itemId": "reply", "delta": "Working.\n\n" } })).await.unwrap();
                    io.send(json!({ "id": value["id"], "result": { "turn": { "id": "turn-1" } } })).await.unwrap();
                    match mode.as_str() {
                        "steer" => {
                            app.upsert_message(Message::new("chat", Author::You, Body::text("new direction")), false);
                            continue;
                        }
                        "approval" => {
                            io.send(json!({ "id": "approval-1", "method": "item/commandExecution/requestApproval", "params": { "threadId": "thread-1", "turnId": "turn-1", "itemId": "command", "command": "touch blocked-file", "reason": "outside workspace" } })).await.unwrap();
                            let app = app.clone();
                            tokio::spawn(async move {
                                for _ in 0..100 {
                                    let id = app.pending_permissions.lock().unwrap().keys().next().cloned();
                                    if let Some(id) = id {
                                        crate::plugins::mcp::answer(&app, &id, crate::plugins::mcp::Decision::Denied);
                                        return;
                                    }
                                    tokio::time::sleep(Duration::from_millis(10)).await;
                                }
                                panic!("approval was not presented");
                            });
                            continue;
                        }
                        "interrupt" => continue,
                        _ => {}
                    }
                    complete(&mut io, &mode).await;
                    return;
                }
                "turn/steer" => {
                    assert_eq!(value["params"]["expectedTurnId"], "turn-1");
                    assert!(value["params"]["input"].to_string().contains("new direction"));
                    io.send(json!({ "id": value["id"], "result": { "turnId": "turn-1" } })).await.unwrap();
                    complete(&mut io, &mode).await;
                    return;
                }
                "turn/interrupt" => {
                    io.send(json!({ "id": value["id"], "result": {} })).await.unwrap();
                    io.send(json!({ "method": "turn/completed", "params": { "threadId": "thread-1", "turn": { "id": "turn-1", "status": "interrupted" } } })).await.unwrap();
                    return;
                }
                "" => {
                    assert_eq!(value["id"], "approval-1");
                    assert_eq!(value["result"]["decision"], "decline");
                    complete(&mut io, &mode).await;
                    return;
                }
                other => panic!("unexpected request {other}"),
            };
            io.send(json!({ "id": value["id"], "result": result })).await.unwrap();
            if method == "account/read" && mode == "unsigned" {
                return;
            }
        }
    });
    (io_connection(client), requests, task)
}

async fn complete(io: &mut Connection, mode: &str) {
    io.send(json!({ "method": "item/completed", "params": { "threadId": "thread-1", "turnId": "turn-1", "item": { "type": "agentMessage", "id": "reply", "text": "Done with Codex." } } })).await.unwrap();
    io.send(json!({ "method": "turn/completed", "params": { "threadId": "thread-1", "turn": { "id": "turn-1", "status": if mode == "failed" { "failed" } else { "completed" }, "error": { "message": "model unavailable" } } } })).await.unwrap();
}

#[tokio::test]
async fn codex_starts_then_resumes_without_replaying_its_replies_or_needing_provider_tokens() {
    let fixture = Fixture::new();
    for i in 0..2 {
        if i == 1 {
            fixture.app.upsert_message(Message::new("chat", Author::You, Body::text("second request")), false);
        }
        let (mut connection, requests, task) = server("ok", fixture.app.clone());
        let output = fixture.run(&mut connection).await.unwrap();
        task.await.unwrap();
        assert_eq!(output.last_text.as_deref(), Some("Done with Codex."));
        let requests = requests.lock().unwrap();
        assert!(requests.iter().any(|r| r["method"] == if i == 0 { "thread/start" } else { "thread/resume" }));
        let input = &requests.iter().find(|r| r["method"] == "turn/start").unwrap()["params"]["input"].to_string();
        assert!(!input.contains("Done with Codex"));
        assert!(input.contains(if i == 0 { "first request" } else { "second request" }));
        if i == 1 {
            assert!(!input.contains("first request"));
        }
    }
    assert!(fixture.app.store.all("chat").unwrap().iter().all(|m| !matches!(&m.body, Body::Text { text, .. } if text == "hidden")));
    let store = crate::local_store::LocalStore::open(&fixture.home.join("lorca.sqlite3")).unwrap();
    assert_eq!(store.codex_session("chat", "bot", &fixture.home.to_string_lossy()).unwrap().unwrap().thread_id, "thread-1");
    assert!(store.codex_session("chat", "bot", "/another-workspace").unwrap().is_none());
    store.clear().unwrap();
    assert!(store.codex_session("chat", "bot", &fixture.home.to_string_lossy()).unwrap().is_none());
}

#[tokio::test]
async fn codex_settings_reach_native_turns_and_default_resets_persisted_overrides() {
    let mut fixture = Fixture::new();
    for mode in 0..3 {
        fixture.bot.model = (mode == 1).then(|| "native-other".into());
        fixture.bot.thinking = (mode == 1).then(|| "high".into());
        fixture.bot.codex_options = match mode {
            0 => CodexOptions { speed: CodexSpeed::Fast, approvals: CodexApprovals::AutoReview },
            1 => CodexOptions { speed: CodexSpeed::Standard, approvals: CodexApprovals::User },
            _ => CodexOptions::default(),
        };
        let (mut connection, requests, task) = server("ok", fixture.app.clone());
        fixture.run(&mut connection).await.unwrap();
        task.await.unwrap();
        let requests = requests.lock().unwrap();
        let turn = &requests.iter().find(|r| r["method"] == "turn/start").unwrap()["params"];
        assert_eq!(turn["model"], if mode == 1 { "native-other" } else { "native-default" });
        assert_eq!(turn["effort"], if mode == 1 { "high" } else { "medium" });
        assert_eq!(turn["serviceTierForTurn"], if mode == 1 { "default" } else { "priority" });
        assert_eq!(turn["approvalsReviewer"], if mode == 1 { "user" } else { "auto_review" });
        assert_eq!(turn["approvalPolicy"], "on-request");
    }
}

#[tokio::test]
async fn codex_catalog_keeps_account_config_private_and_checks_supported_settings() {
    let mut fixture = Fixture::new();
    let (mut connection, _, task) = server("ok", fixture.app.clone());
    let catalog = read_models(&mut connection, &fixture.home).await.unwrap();
    assert!(!catalog.to_string().contains("must-not-leak"));
    assert_eq!(catalog["models"][0]["fast_tier"], "priority");
    fixture.bot.model = Some("native-other".into());
    assert_eq!(runtime_params(&fixture.bot, &catalog).unwrap()["effort"], "high");
    fixture.bot.thinking = Some("ultra".into());
    assert!(runtime_params(&fixture.bot, &catalog).unwrap_err().to_string().contains("does not support"));
    fixture.bot.thinking = None;
    fixture.bot.codex_options.speed = CodexSpeed::Fast;
    assert!(runtime_params(&fixture.bot, &catalog).unwrap_err().to_string().contains("does not offer Fast"));
    task.abort();
}

#[tokio::test]
async fn codex_steers_and_claims_only_acknowledged_messages() {
    let fixture = Fixture::new();
    let (mut connection, _, task) = server("steer", fixture.app.clone());
    fixture.run(&mut connection).await.unwrap();
    task.await.unwrap();
    let message = fixture
        .app
        .store
        .all("chat")
        .unwrap()
        .into_iter()
        .find(|m| matches!(&m.body, Body::Text { text, .. } if text == "new direction"))
        .unwrap();
    assert!(fixture.app.take_steering_message("chat", &message.id));
}

#[tokio::test]
async fn codex_approval_denial_reaches_server_and_settles_the_card() {
    let fixture = Fixture::new();
    let (mut connection, _, task) = server("approval", fixture.app.clone());
    fixture.run(&mut connection).await.unwrap();
    task.await.unwrap();
    assert!(fixture.app.pending_permissions.lock().unwrap().is_empty());
    assert!(fixture
        .app
        .store
        .all("chat")
        .unwrap()
        .iter()
        .any(|m| matches!(&m.body, Body::Permission { decision, .. } if decision == "denied")));
}

#[tokio::test]
async fn codex_missing_login_disconnect_and_failed_turn_are_errors() {
    for (mode, expected) in [("unsigned", "codex login"), ("disconnect", "closed"), ("failed", "model unavailable")] {
        let fixture = Fixture::new();
        let (mut connection, _, task) = server(mode, fixture.app.clone());
        let error = fixture.run(&mut connection).await.err().expect("must fail").to_string();
        assert!(error.contains(expected), "{mode}: {error}");
        task.await.unwrap();
    }
}

#[tokio::test]
async fn codex_stop_interrupts_the_active_native_turn() {
    let fixture = Fixture::new();
    let (mut connection, requests, task) = server("interrupt", fixture.app.clone());
    let _ = tokio::time::timeout(Duration::from_millis(200), fixture.run(&mut connection)).await;
    assert!(connection.active_turn.is_some());
    connection.interrupt().await;
    task.await.unwrap();
    assert!(requests.lock().unwrap().iter().any(|r| r["method"] == "turn/interrupt"));
}

#[tokio::test]
async fn codex_lorca_tools_validate_arguments_and_keep_provider_review_out_of_the_path() {
    let fixture = Fixture::new();
    let (plugins, _) = crate::plugins::mcp::turn_tools(&fixture.app, "chat", &Trigger::default(), &fixture.bot, false);
    let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(ListTeammates { app: fixture.app.clone(), chat_id: "chat".into() })];
    let call = json!({ "tool": "lorca_call", "callId": "call", "arguments": { "tool": "list_teammates", "arguments": {} } });
    let result = call_tool(&call, &tools, &plugins, &CancellationToken::new()).await;
    assert_eq!(result["success"], true);
    assert!(result.to_string().contains("Coder"));
    let result =
        call_tool(&json!({ "arguments": { "tool": "unknown", "arguments": {} } }), &tools, &plugins, &CancellationToken::new()).await;
    assert_eq!(result["success"], false);
    let verdict = crate::plugins::review::decide(
        &fixture.app,
        &fixture.bot,
        "chat",
        &Trigger::default(),
        "plugin",
        "Plugin",
        "write",
        "Write a file",
        &json!({}),
        &CancellationToken::new(),
    )
    .await;
    assert!(matches!(verdict, crate::plugins::review::Outcome::Ask { reason: Some(reason), .. } if reason.contains("Codex runs this bot")));
}

#[tokio::test]
async fn codex_reads_teammate_handoffs_and_routine_markers() {
    let fixture = Fixture::new();
    let handoff = Message::new(
        "chat",
        Author::Bot { bot_id: "scout".into() },
        Body::Handoff { from: "scout".into(), to: fixture.bot.id.clone(), reason: "Inspect the release changes".into() },
    );
    let routine = Message::new(
        "chat",
        Author::System,
        Body::Notice { text: "Routine · Daily report".into(), routine_id: Some("deleted-routine".into()) },
    );
    let input = input_for(&fixture.app, &fixture.bot, &fixture.home, &[handoff, routine], false).await;
    assert_eq!(input[0]["text"], "[Message from scout]: Inspect the release changes");
    assert!(input[1]["text"].as_str().unwrap().contains("Daily report"));
}

#[tokio::test]
async fn codex_harness_defaults_and_api_validation_are_compatible() {
    let fixture = Fixture::new();
    let mut value = serde_json::to_value(&fixture.bot).unwrap();
    value.as_object_mut().unwrap().remove("harness");
    value.as_object_mut().unwrap().remove("codex_options");
    assert_eq!(serde_json::from_value::<Bot>(value.clone()).unwrap().codex_options.approvals, CodexApprovals::AutoReview);
    assert_eq!(serde_json::from_value::<Bot>(value).unwrap().harness, Harness::Lorca);
    assert!(crate::api::dispatch(&fixture.app, "bots.update", json!({ "id": "bot", "harness": "unknown" })).await.is_err());
    let result = crate::api::dispatch(&fixture.app, "bots.update", json!({ "id": "bot", "harness": "lorca" })).await.unwrap();
    assert_eq!(result["bot"]["harness"], "lorca");
    assert!(crate::api::dispatch(&fixture.app, "bots.update", json!({ "id": "bot", "codex_options": { "approvals": "never" } }))
        .await
        .is_err());
    let result = crate::api::dispatch(
        &fixture.app,
        "bots.update",
        json!({ "id": "bot", "codex_options": { "speed": "fast", "approvals": "auto_review" } }),
    )
    .await
    .unwrap();
    assert_eq!(result["bot"]["codex_options"]["speed"], "fast");
}

/// Opt in on a signed-in Runner. Reads a scratch file and the isolated fixture's team list.
#[tokio::test]
#[ignore = "uses the Runner's Codex installation and login"]
async fn codex_live_read_and_resume() {
    let mut fixture = Fixture::new();
    let catalog = {
        let mut connection = Connection::spawn(&fixture.home).await.unwrap();
        connection.initialize().await.unwrap();
        read_models(&mut connection, &fixture.home).await.unwrap()
    };
    std::fs::write(fixture.home.join("probe.txt"), "LORCA_CODEX_NATIVE_7391").unwrap();
    let mut original = fixture.app.message("chat", &fixture.job.trigger_message_id).unwrap();
    original.body = Body::text(
        "Read probe.txt using your native file or shell tool and reply with its contents only. Do not edit files or call other tools.",
    );
    fixture.app.upsert_message(original, false);
    for i in 0..3 {
        if i < 2 {
            let models = catalog["models"].as_array().unwrap();
            let model = &models[i.min(models.len() - 1)];
            fixture.bot.model = model["id"].as_str().map(str::to_string);
            fixture.bot.thinking = model["levels"][0].as_str().map(str::to_string);
            fixture.bot.codex_options.speed =
                if i == 0 && model["fast_tier"].is_string() { CodexSpeed::Fast } else { CodexSpeed::Standard };
        } else {
            fixture.bot.model = None;
            fixture.bot.thinking = None;
            fixture.bot.codex_options = CodexOptions::default();
        }
        eprintln!("native turn {}: {}", i + 1, runtime_params(&fixture.bot, &catalog).unwrap());
        if i == 1 {
            fixture.app.upsert_message(
                Message::new(
                    "chat",
                    Author::You,
                    Body::text("Repeat the exact contents from the previous turn without reading the file again."),
                ),
                false,
            );
        }
        if i == 2 {
            fixture.app.upsert_message(
                Message::new(
                    "chat",
                    Author::You,
                    Body::text("Call lorca_call with tool=list_teammates and arguments={}, then reply with the teammate's name only."),
                ),
                false,
            );
        }
        let mut connection = Connection::spawn(&fixture.home).await.unwrap();
        let output = tokio::time::timeout(Duration::from_secs(120), fixture.run(&mut connection)).await.unwrap().unwrap();
        let expected = if i == 2 { "Coder" } else { "LORCA_CODEX_NATIVE_7391" };
        assert!(output.last_text.as_deref().unwrap_or("").contains(expected), "{:?}", output.last_text);
        if i == 0 || i == 2 {
            assert!(!output.tools_used.is_empty(), "The test must actually execute a tool");
        }
        if i == 2 {
            let session = fixture.app.store.codex_session("chat", "bot", &fixture.home.to_string_lossy()).unwrap().unwrap();
            connection.request("thread/archive", json!({ "threadId": session.thread_id })).await.unwrap();
        }
    }
}
