use super::*;
use crate::review_execution;
use crate::review_queue::{self as queue, ReviewPayload, ReviewState};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

async fn connected(app: &Arc<App>, name: &str, seen: Arc<Mutex<Vec<Value>>>) -> Arc<Server> {
    use rmcp::ServiceExt;
    let (transport, server) = tokio::io::duplex(64 * 1024);
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
                    json!({ "content": [{ "type": "text", "text": "sent reviewed draft" }] })
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
        plugin_id: "review-mail".into(),
        server: name.into(),
        generation: 0,
    };
    let service = client.serve(transport).await.unwrap();
    let tools = vec![
        serde_json::from_value(
            json!({ "name": "send", "description": "Send the draft", "inputSchema": {
        "type": "object", "properties": { "body": { "type": "string" } }, "required": ["body"] } }),
        )
        .unwrap(),
    ];
    save_catalog(
        app,
        "review-mail",
        name,
        SavedServer {
            instructions: None,
            tools: tools.clone(),
            resources: false,
        },
    );
    let server = Arc::new(Server {
        plugin_id: "review-mail".into(),
        name: name.into(),
        service,
        tools: std::sync::RwLock::new(tools),
        instructions: None,
        resources: false,
        auth: None,
        bearer_expires_at: None,
        generation: app.mcp.generation("review-mail"),
    });
    app.mcp
        .servers
        .lock()
        .unwrap()
        .insert(format!("review-mail/{name}"), server.clone());
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
