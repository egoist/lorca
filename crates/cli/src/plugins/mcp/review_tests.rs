use super::*;
use crate::review_execution;
use crate::review_queue::{self as queue, ReviewPayload, ReviewState};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

async fn connected(app: &Arc<App>, name: &str, seen: Arc<Mutex<Vec<Value>>>) -> Arc<Server> {
    let send = json!({ "name": "send", "description": "Send the draft", "inputSchema": {
        "type": "object", "properties": { "body": { "type": "string" } }, "required": ["body"] } });
    serve_fake(app, "review-mail", name, vec![send], json!({ "content": [{ "type": "text", "text": "sent reviewed draft" }] }), seen).await
}

/// A server of `plugin_id` in the pool that offers `tools`, answers every call with `reply`, and
/// keeps the calls it got in `seen`.
async fn serve_fake(app: &Arc<App>, plugin_id: &str, name: &str, tools: Vec<Value>, reply: Value, seen: Arc<Mutex<Vec<Value>>>) -> Arc<Server> {
    use rmcp::ServiceExt;
    let (transport, server) = tokio::io::duplex(64 * 1024);
    let reply = reply.clone();
    tokio::spawn(async move {
        let (read, mut write) = tokio::io::split(server);
        let mut lines = tokio::io::BufReader::new(read).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let message: Value = serde_json::from_str(&line).unwrap();
            let result = match message["method"].as_str() {
                Some("initialize") => {
                    json!({ "protocolVersion": message["params"]["protocolVersion"], "capabilities": { "tools": {} }, "serverInfo": { "name": "review-server", "version": "1" } })
                }
                Some("tools/call") => {
                    seen.lock().unwrap().push(message["params"].clone());
                    reply.clone()
                }
                _ => continue,
            };
            if write
                .write_all(
                    format!(
                        "{}\n",
                        json!({ "jsonrpc": "2.0", "id": message["id"], "result": result })
                    )
                    .as_bytes(),
                )
                .await
                .is_err()
            {
                return;
            }
        }
    });
    let client = Client {
        info: ClientConfig::default(),
        app: Arc::downgrade(app),
        plugin_id: plugin_id.into(),
        server: name.into(),
        generation: 0,
    };
    let service = client.serve(transport).await.unwrap();
    let tools: Vec<rmcp::model::Tool> = tools.into_iter().map(|tool| serde_json::from_value(tool).unwrap()).collect();
    save_catalog(
        app,
        plugin_id,
        name,
        SavedServer {
            instructions: None,
            tools: tools.clone(),
            resources: false,
        },
    );
    let server = Arc::new(Server {
        plugin_id: plugin_id.into(),
        name: name.into(),
        service,
        tools: std::sync::RwLock::new(tools),
        instructions: None,
        resources: false,
        auth: None,
        bearer_expires_at: None,
        generation: app.mcp.generation(plugin_id),
    });
    app.mcp
        .servers
        .lock()
        .unwrap()
        .insert(format!("{plugin_id}/{name}"), server.clone());
    server
}

#[tokio::test]
async fn an_unattended_plugin_proposal_executes_the_edited_version_on_its_exact_server() {
    let home = std::env::temp_dir().join(format!("lorca-mcp-review-{}", uuid::Uuid::new_v4()));
    let app = App::load(crate::config::Config {
        home: home.clone(),
        port: 0,
    })
    .unwrap();
    crate::identity::create(&app, Some("Review Runner".into())).unwrap();
    let bot = app.state.lock().unwrap().bots[0].clone();
    let dm = app.dm_with(&bot.id, None).unwrap();
    let manifest = super::super::Manifest::parse(&json!({ "id": "review-mail", "name": "Mail", "servers": {
        "main": { "type": "http", "url": "http://unused.invalid/main" }, "secondary": { "type": "http", "url": "http://unused.invalid/secondary" }
    } })).unwrap();
    app.plugins.lock().unwrap().installed.push(Installed {
        manifest,
        source: "inline".into(),
        installed_at: 0.0,
        variables: Default::default(),
        service_id: None,
        account_name: None,
    });
    let first_seen = Arc::new(Mutex::new(Vec::new()));
    let second_seen = Arc::new(Mutex::new(Vec::new()));
    let _first = connected(&app, "main", first_seen.clone()).await;
    let second = connected(&app, "secondary", second_seen.clone()).await;
    let catalog = turn_catalog(&app, Vec::new());
    let call_name = catalog
        .state
        .lock()
        .unwrap()
        .tools
        .values()
        .find(|tool| tool.server_name == "secondary")
        .unwrap()
        .name
        .clone();
    app.set_auto_review(crate::model::AutoReview { is_enabled: false, ..Default::default() });
    let routine = crate::routines::create(
        &app,
        &bot.id,
        "Mail",
        "every 1h",
        "Draft a report",
        None,
        true,
    )
    .unwrap();
    let trigger = super::super::review::Trigger {
        message_id: "routine-marker".into(),
        routine: Some(routine),
        event: None,
    };
    let arguments = json!({ "body": "original draft" });
    let call = lorca_agent::ToolCall {
        id: "mail-proposal".into(),
        name: call_name,
        arguments: arguments.clone(),
    };
    let assistant = lorca_agent::AssistantMessage::empty("test", "test");
    let context = lorca_agent::AgentContext {
        system_prompt: String::new(),
        messages: Vec::new(),
        tools: Vec::new(),
        cache_points: Vec::new(),
    };
    let cancel = CancellationToken::new();
    let ctx = BeforeToolCallContext {
        assistant_message: &assistant,
        tool_call: &call,
        args: &arguments,
        context: &context,
        cancel: &cancel,
        parent: None,
    };
    let staged = review_call(&app, &catalog, &dm.meta.id, &trigger, &bot, true, &ctx)
        .await
        .unwrap();
    assert!(staged.block && staged.reason.unwrap().contains("Staged review"));
    assert!(first_seen.lock().unwrap().is_empty() && second_seen.lock().unwrap().is_empty());
    assert!(app.pending_permissions.lock().unwrap().is_empty());
    let item = queue::list(&app).unwrap().remove(0);
    let payload = ReviewPayload::Plugin {
        plugin_id: "review-mail".into(),
        server_name: "secondary".into(),
        tool: "send".into(),
        arguments: json!({ "body": "edited draft" }),
    };
    let edited = review_execution::mutate(
        &app,
        "reviews.edit",
        &json!({ "id": item.id, "expected_version": item.version, "payload": payload }),
        &bot.runner_id,
    )
    .await
    .unwrap();
    review_execution::mutate(
        &app,
        "reviews.approve",
        &json!({ "id": item.id, "expected_version": edited.version }),
        &bot.runner_id,
    )
    .await
    .unwrap();
    // Live schema/description changes after approval invalidate the old version.
    second.tools.write().unwrap()[0].description =
        Some("Send a draft using the updated service".into());
    review_execution::execute_approved(&app, &item.id)
        .await
        .unwrap();
    let refreshed = queue::get(&app, &item.id).unwrap();
    assert_eq!(refreshed.state, ReviewState::Pending);
    assert!(second_seen.lock().unwrap().is_empty());
    review_execution::mutate(
        &app,
        "reviews.approve",
        &json!({ "id": item.id, "expected_version": refreshed.version }),
        &bot.runner_id,
    )
    .await
    .unwrap();
    let (a, b) = tokio::join!(
        review_execution::execute_approved(&app, &item.id),
        review_execution::execute_approved(&app, &item.id)
    );
    a.unwrap();
    b.unwrap();
    assert!(
        first_seen.lock().unwrap().is_empty(),
        "a same-name tool on another server never runs"
    );
    let calls = second_seen.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["name"], "send");
    assert_eq!(calls[0]["arguments"], json!({ "body": "edited draft" }));
    drop(calls);
    assert_eq!(
        queue::get(&app, &item.id).unwrap().state,
        ReviewState::Succeeded
    );
    app.mcp.servers.lock().unwrap().clear();
    let _ = std::fs::remove_dir_all(home);
}

/// An app over a scratch home with an identity, its first bot, and the bot's DM.
fn drafting_app(name: &str) -> (Arc<App>, std::path::PathBuf, Bot, String) {
    let home = std::env::temp_dir().join(format!("lorca-{name}-{}", uuid::Uuid::new_v4()));
    let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
    crate::identity::create(&app, Some("Runner".into())).unwrap();
    let bot = app.state.lock().unwrap().bots[0].clone();
    let chat_id = app.dm_with(&bot.id, None).unwrap().meta.id;
    (app, home, bot, chat_id)
}

fn install(app: &Arc<App>, manifest: Value) {
    app.plugins.lock().unwrap().installed.push(Installed {
        manifest: super::super::Manifest::parse(&manifest).unwrap(),
        source: "inline".into(),
        installed_at: 0.0,
        variables: Default::default(),
        service_id: None,
        account_name: None,
    });
}

/// What the turn's `before_tool_call` decides for a script's call to `tool` of `plugin_id`.
async fn before_call(app: &Arc<App>, chat_id: &str, trigger: &super::super::review::Trigger, bot: &Bot, plugin_id: &str, tool: &str, arguments: &Value) -> Option<BeforeToolCallResult> {
    before_call_until(app, chat_id, trigger, bot, plugin_id, tool, arguments, &CancellationToken::new(), false).await
}

/// `before_call` in a turn that `cancel` stops, attended or `unattended` (a routine's or an
/// event's).
#[allow(clippy::too_many_arguments)]
async fn before_call_until(app: &Arc<App>, chat_id: &str, trigger: &super::super::review::Trigger, bot: &Bot, plugin_id: &str, tool: &str, arguments: &Value, cancel: &CancellationToken, unattended: bool) -> Option<BeforeToolCallResult> {
    let catalog = turn_catalog(app, Vec::new());
    let name = catalog.state.lock().unwrap().tools.values().find(|candidate| candidate.plugin_id == plugin_id && candidate.original_name == tool).unwrap().name.clone();
    let call = lorca_agent::ToolCall { id: format!("call-{}", uuid::Uuid::new_v4()), name, arguments: arguments.clone() };
    let assistant = lorca_agent::AssistantMessage::empty("test", "test");
    let context = lorca_agent::AgentContext { system_prompt: String::new(), messages: Vec::new(), tools: Vec::new(), cache_points: Vec::new() };
    let ctx = BeforeToolCallContext { assistant_message: &assistant, tool_call: &call, args: arguments, context: &context, cancel, parent: None };
    review_call(app, &catalog, chat_id, trigger, bot, unattended, &ctx).await
}

/// The user's message that asks for the work, as the trigger of the turn that answers it.
fn asked(app: &Arc<App>, chat_id: &str, text: &str) -> super::super::review::Trigger {
    let message = Message::new(chat_id, Author::You, Body::text(text));
    app.upsert_message(message.clone(), false);
    super::super::review::Trigger { message_id: message.id, routine: None, event: None }
}

#[tokio::test]
async fn a_message_in_a_chat_waits_as_a_draft_the_user_edits_and_sends() {
    let (app, home, bot, chat_id) = drafting_app("slack-draft");
    let actor = app.this_device_id().unwrap();
    install(&app, json!({ "id": "slack-test", "name": "Slack", "servers": { "slack": { "type": "http", "url": "http://unused.invalid/mcp" } },
        "tools": { "messages": [{ "tool": "slack_send_message", "kind": "slack", "to": "channel_id", "body": "message", "reply": "thread_ts" }] } }));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let send = json!({ "name": "slack_send_message", "description": "Send a message", "inputSchema": { "type": "object",
        "properties": { "channel_id": { "type": "string" }, "message": { "type": "string" }, "thread_ts": { "type": "string" } }, "required": ["channel_id", "message"] } });
    let _server = serve_fake(&app, "slack-test", "slack", vec![send], json!({ "content": [{ "type": "text", "text": "ok" }] }), seen.clone()).await;
    // With Auto-review off every change asks on a card; a draft is the user's to send instead.
    app.set_auto_review(crate::model::AutoReview { is_enabled: false, ..Default::default() });
    let trigger = asked(&app, &chat_id, "Tell #launch we ship Friday");

    let arguments = json!({ "channel_id": "C024BE91L", "message": "We ship Friday." });
    let staged = before_call(&app, &chat_id, &trigger, &bot, "slack-test", "slack_send_message", &arguments).await.unwrap();
    assert!(staged.block && !staged.terminate);
    assert!(staged.reason.as_deref().unwrap().contains("Slack message to C024BE91L is ready as a draft"), "{:?}", staged.reason);
    assert!(seen.lock().unwrap().is_empty() && app.pending_permissions.lock().unwrap().is_empty(), "nothing ran or asked");
    let item = queue::list(&app).unwrap().remove(0);
    assert!(item.is_message);
    let card = || app.message(&chat_id, &item.message_id()).unwrap();
    assert_eq!(card().author, Author::Bot { bot_id: bot.id.clone() });
    let Body::Draft { state, version, draft, direct, account, .. } = card().body else { panic!("{:?}", card().body) };
    assert_eq!((state.as_str(), version, direct, account.as_str()), ("pending", 1, true, "Slack"));
    assert_eq!((draft.to, draft.body.as_str()), (vec!["C024BE91L".to_string()], "We ship Friday."));

    // The card's edit, then Send: the edited version is the one that goes out.
    let edit = json!({ "id": item.id, "expected_version": 1, "message": { "kind": "slack", "to": ["C024BE91L"], "body": "We ship Friday at noon." } });
    let edited = queue::serve(&app, "reviews.edit", &edit, &actor).await.unwrap();
    queue::serve(&app, "reviews.approve", &json!({ "id": item.id, "expected_version": edited["version"] }), &actor).await.unwrap();
    review_execution::execute_approved(&app, &item.id).await.unwrap();
    let calls = seen.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["arguments"], json!({ "channel_id": "C024BE91L", "message": "We ship Friday at noon." }));
    let Body::Draft { state, draft, .. } = card().body else { panic!() };
    assert_eq!((state.as_str(), draft.body.as_str()), ("succeeded", "We ship Friday at noon."));
    assert_eq!(queue::get(&app, &item.id).unwrap().outcome.unwrap().summary, "Sent");

    // What the user changed is the bot's feedback, as the message reads.
    let notes = crate::feedback::recorded(&app, &bot.id);
    let edited = notes.iter().find(|note| note.kind == crate::feedback::Kind::Edited).unwrap();
    assert_eq!(edited.before.as_deref(), Some("To: C024BE91L\n\nWe ship Friday."));
    assert_eq!(edited.after.as_deref(), Some("To: C024BE91L\n\nWe ship Friday at noon."));
    assert_eq!(edited.origin.review_id.as_deref(), Some(item.id.as_str()));

    // Turning drafts off on the card: the call is the tool's own again, and Auto-review decides.
    crate::api::dispatch(&app, "bots.update", json!({ "id": bot.id, "drafts": false })).await.unwrap();
    let bot = app.bot(&bot.id).unwrap();
    assert!(!crate::permissions::drafts_messages(&bot));
    app.set_auto_review(crate::model::AutoReview { is_enabled: true, ..Default::default() });
    app.add_auto_review_rule(crate::model::AutoReviewRule { id: "r".into(), text: "use Slack slack_send_message".into(), behavior: "allow".into(), tool: Some("slack-test/slack_send_message".into()) });
    assert!(before_call(&app, &chat_id, &trigger, &bot, "slack-test", "slack_send_message", &arguments).await.is_none(), "it runs on its own");
    assert_eq!(queue::list(&app).unwrap().len(), 1);
    app.mcp.servers.lock().unwrap().clear();
    let _ = std::fs::remove_dir_all(home);
}

#[tokio::test]
async fn a_gmail_draft_is_made_on_send_then_sent_with_gmails_api() {
    let (app, home, bot, chat_id) = drafting_app("gmail-draft");
    let actor = app.this_device_id().unwrap();
    let message = json!({ "tool": "create_draft", "kind": "email", "to": "to", "cc": "cc", "subject": "subject", "body": "body",
        "attachments": "attachments", "reply": "replyToMessageId", "send": "gmail" });
    install(&app, json!({ "id": "gmail-test", "name": "Gmail", "servers": { "gmail": { "type": "http", "url": "http://unused.invalid/mcp",
        "auth": { "type": "oauth", "scopes": ["mail"], "authorization_endpoint": "http://127.0.0.1:9/auth", "token_endpoint": "http://127.0.0.1:9/token" } } },
        "tools": { "draft": ["create_draft"], "messages": [message] } }));
    super::super::set_oauth(&app, "gmail-test", "gmail", Some(json!({ "native_flow": true, "client_id": "client", "token_endpoint": "http://127.0.0.1:9/token",
        "tokens": { "access_token": "token-1", "expires_in": 3600 }, "signed_in_at": crate::config::now_secs() }))).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let create = json!({ "name": "create_draft", "description": "Create a draft", "inputSchema": { "type": "object", "properties": {
        "to": { "type": "array", "items": { "type": "string" } }, "subject": { "type": "string" }, "body": { "type": "string" },
        "attachments": { "type": "array", "items": { "type": "object" } } } } });
    let made = json!({ "content": [{ "type": "text", "text": "{\"id\":\"r-42\"}" }], "structuredContent": { "id": "r-42" } });
    let _server = serve_fake(&app, "gmail-test", "gmail", vec![create], made, seen.clone()).await;
    // Gmail's API: the draft's send, with the account's token.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    std::env::set_var("LORCA_GMAIL_API_URL", format!("http://{}", listener.local_addr().unwrap()));
    let sent = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        while !String::from_utf8_lossy(&request).contains("\"id\"") {
            let read = socket.read(&mut buffer).await.unwrap();
            request.extend_from_slice(&buffer[..read]);
        }
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").await.unwrap();
        String::from_utf8_lossy(&request).to_string()
    });
    let trigger = asked(&app, &chat_id, "Email Ana the menu");

    let arguments = json!({ "to": ["ana@example.com"], "subject": "Lunch", "body": "Menu attached.", "attachments": [{ "filename": "menu.txt", "mimeType": "text/plain", "content": "aGVsbG8=" }] });
    let staged = before_call(&app, &chat_id, &trigger, &bot, "gmail-test", "create_draft", &arguments).await.unwrap();
    assert!(staged.reason.as_deref().unwrap().contains("Email to ana@example.com: Lunch is ready as a draft"), "{:?}", staged.reason);
    let item = queue::list(&app).unwrap().remove(0);
    // The attachment waits on the Runner; the reviewed call names it by size and hash.
    let ReviewPayload::Plugin { arguments: reviewed, .. } = &item.payload else { panic!() };
    assert!(reviewed["attachments"][0]["content"].as_str().unwrap().starts_with("lorca-draft-file:5:"));
    assert_eq!(std::fs::read_dir(home.join("drafts")).unwrap().count(), 1);
    let Body::Draft { draft, direct, .. } = app.message(&chat_id, &item.message_id()).unwrap().body else { panic!() };
    assert!(!direct, "Gmail's server only drafts");
    assert_eq!(draft.attachments, vec![crate::model::DraftAttachment { name: "menu.txt".into(), size: 5 }]);

    queue::serve(&app, "reviews.approve", &json!({ "id": item.id, "expected_version": item.version }), &actor).await.unwrap();
    review_execution::execute_approved(&app, &item.id).await.unwrap();
    assert_eq!(seen.lock().unwrap()[0]["arguments"]["attachments"][0]["content"], "aGVsbG8=", "the content goes out as the bot wrote it");
    let request = sent.await.unwrap();
    assert!(request.starts_with("POST /gmail/v1/users/me/drafts/send"), "{request}");
    assert!(request.to_ascii_lowercase().contains("authorization: bearer token-1") && request.contains("{\"id\":\"r-42\"}"), "{request}");
    let Body::Draft { state, .. } = app.message(&chat_id, &item.message_id()).unwrap().body else { panic!() };
    assert_eq!(state, "succeeded");
    assert_eq!(std::fs::read_dir(home.join("drafts")).unwrap().count(), 0, "a sent message's files go");
    app.mcp.servers.lock().unwrap().clear();
    let _ = std::fs::remove_dir_all(home);
}

#[tokio::test]
async fn a_stopped_turn_leaves_no_draft() {
    let (app, home, bot, chat_id) = drafting_app("stopped-draft");
    install(&app, json!({ "id": "gmail-test", "name": "Gmail", "servers": { "gmail": { "type": "http", "url": "http://unused.invalid/mcp" } },
        "tools": { "draft": ["create_draft"], "messages": [{ "tool": "create_draft", "kind": "email", "to": "to", "subject": "subject", "body": "body", "attachments": "attachments", "send": "gmail" }] } }));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let create = json!({ "name": "create_draft", "description": "Create a draft", "inputSchema": { "type": "object", "properties": {
        "to": { "type": "array", "items": { "type": "string" } }, "body": { "type": "string" }, "attachments": { "type": "array", "items": { "type": "object" } } } } });
    let _server = serve_fake(&app, "gmail-test", "gmail", vec![create], json!({ "content": [] }), seen.clone()).await;
    let trigger = asked(&app, &chat_id, "Email Ana the menu");
    let arguments = json!({ "to": ["ana@example.com"], "body": "Menu attached.", "attachments": [{ "filename": "menu.txt", "content": "aGVsbG8=" }] });
    let cancel = CancellationToken::new();
    cancel.cancel();
    let stopped = before_call_until(&app, &chat_id, &trigger, &bot, "gmail-test", "create_draft", &arguments, &cancel, false).await.unwrap();
    assert!(stopped.block && stopped.reason.as_deref() == Some("Stopped"), "{:?}", stopped.reason);
    assert!(queue::list(&app).unwrap().is_empty(), "no draft");
    assert!(std::fs::read_dir(home.join("drafts")).map(|files| files.count()).unwrap_or(0) == 0, "no stashed file");
    // Staging itself takes the turn's cancellation and saves nothing once it is stopped.
    let payload = ReviewPayload::Plugin { plugin_id: "gmail-test".into(), server_name: "gmail".into(), tool: "create_draft".into(), arguments: json!({ "to": ["ana@example.com"], "body": "Hi" }) };
    let target = crate::review_queue::ReviewTarget { account: "Gmail".into(), resource: "Email to ana@example.com".into() };
    let staged = review_execution::stage_call(&app, &bot, &chat_id, &trigger, "call-stopped", payload, target, None, &cancel).await;
    assert_eq!(staged.unwrap_err(), "Stopped");
    assert!(queue::list(&app).unwrap().is_empty());
    // A draft that could not be staged takes its files with it.
    let tool = app.plugins.lock().unwrap().get("gmail-test").unwrap().manifest.tools.message("create_draft").cloned().unwrap();
    let wrong = json!({ "to": ["ana@example.com"], "body": 5, "attachments": [{ "filename": "menu.txt", "content": "aGVsbG8=" }] });
    let refused = crate::drafts::stage(&app, &bot, &chat_id, &trigger, "call-wrong", ("gmail-test", "gmail", "Gmail"), "create_draft", &tool, &wrong, &CancellationToken::new()).await;
    assert!(refused.reason.as_deref().unwrap().starts_with("Could not put the draft in the chat"), "{:?}", refused.reason);
    assert_eq!(std::fs::read_dir(home.join("drafts")).map(|files| files.count()).unwrap_or(0), 0, "no stashed file left");
    assert!(seen.lock().unwrap().is_empty());
    app.mcp.servers.lock().unwrap().clear();
    let _ = std::fs::remove_dir_all(home);
}

#[tokio::test]
async fn a_saved_secret_never_goes_into_a_draft_and_unattended_runs_ask_auto_review() {
    let (app, home, bot, chat_id) = drafting_app("secret-draft");
    install(&app, json!({ "id": "gmail-test", "name": "Gmail", "servers": { "gmail": { "type": "http", "url": "http://unused.invalid/mcp" } },
        "tools": { "draft": ["create_draft"], "messages": [{ "tool": "create_draft", "kind": "email", "to": "to", "body": "body", "attachments": "attachments", "send": "gmail" }] } }));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let create = json!({ "name": "create_draft", "description": "Create a draft", "inputSchema": { "type": "object", "properties": {
        "to": { "type": "array", "items": { "type": "string" } }, "body": { "type": "string" }, "attachments": { "type": "array", "items": { "type": "object" } } } } });
    let _server = serve_fake(&app, "gmail-test", "gmail", vec![create], json!({ "content": [] }), seen.clone()).await;
    let ask = crate::model::SecretAsk { target: "command".into(), site: None, fields: vec![crate::model::SecretField { name: "NPM_TOKEN".into(), label: "npm token".into() }] };
    crate::secrets::keep(&app, &bot.id, &ask, &std::collections::BTreeMap::from([("NPM_TOKEN".to_string(), "npm_abcdef123456".to_string())])).unwrap();
    let trigger = asked(&app, &chat_id, "Email Ana the token");
    use base64::Engine;
    let file = base64::engine::general_purpose::STANDARD.encode("NPM_TOKEN=npm_abcdef123456\n");
    for arguments in [
        json!({ "to": ["ana@example.com"], "body": "The token is npm_abcdef123456." }),
        json!({ "to": ["ana@example.com"], "body": "The token is {{secret:NPM_TOKEN}}." }),
        json!({ "to": ["ana@example.com"], "body": "Attached.", "attachments": [{ "filename": ".env", "content": file }] }),
    ] {
        let refused = before_call(&app, &chat_id, &trigger, &bot, "gmail-test", "create_draft", &arguments).await.unwrap();
        assert!(refused.block && refused.reason.as_deref().unwrap().contains("saved secret"), "{:?}", refused.reason);
    }
    assert!(queue::list(&app).unwrap().is_empty(), "nothing reached a review item or the chat");
    assert_eq!(std::fs::read_dir(home.join("drafts")).map(|files| files.count()).unwrap_or(0), 0);

    // A routine's, a watch's, or an event's run (a webhook's included) has nobody to send a draft:
    // its message goes to Auto-review, which here lets it run.
    app.set_auto_review(crate::model::AutoReview { is_enabled: true, ..Default::default() });
    app.add_auto_review_rule(crate::model::AutoReviewRule { id: "r".into(), text: "use Gmail create_draft".into(), behavior: "allow".into(), tool: Some("gmail-test/create_draft".into()) });
    let plain = json!({ "to": ["ana@example.com"], "body": "Weekly numbers attached." });
    assert!(before_call_until(&app, &chat_id, &trigger, &bot, "gmail-test", "create_draft", &plain, &CancellationToken::new(), true).await.is_none());
    assert!(queue::list(&app).unwrap().is_empty());
    app.mcp.servers.lock().unwrap().clear();
    let _ = std::fs::remove_dir_all(home);
}
