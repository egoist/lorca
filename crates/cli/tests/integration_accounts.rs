#![cfg(all(feature = "runner", feature = "server"))]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Form, Json, Router,
};
use lorca::{
    api,
    app::App,
    config::Config,
    model::{AutoReview, Bot},
    plugins::{self, mcp, review::Trigger},
};
use lorca_agent::{agent_loop::AgentContext, codemode::Catalog, AssistantMessage, BeforeToolCallContext, ToolCall};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

struct Home(std::path::PathBuf);
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn token(Form(form): Form<HashMap<String, String>>) -> Json<Value> {
    Json(json!({ "access_token": format!("token-{}", form["code"]), "refresh_token": "refresh", "token_type": "Bearer", "expires_in": 3600 }))
}

async fn server(State(calls): State<Arc<Mutex<Vec<String>>>>, headers: HeaderMap, Json(request): Json<Value>) -> Response {
    let result = match request["method"].as_str() {
        Some("initialize") => {
            json!({ "protocolVersion": request["params"]["protocolVersion"], "serverInfo": { "name": "Gmail", "version": "1" }, "capabilities": { "tools": {} } })
        }
        Some("tools/list") => json!({ "tools": [
            { "name": "get_thread", "description": "Read a message and its source.", "inputSchema": { "type": "object" }, "annotations": { "readOnlyHint": true } },
            { "name": "create_draft", "description": "Create an unsent email draft.", "inputSchema": { "type": "object" } }
        ] }),
        Some("tools/call") => {
            calls.lock().unwrap().push(headers["authorization"].to_str().unwrap().to_string());
            json!({ "content": [ { "type": "text", "text": "Message" }, { "type": "resource_link", "name": "Message source", "uri": "https://mail.google.com/mail/u/0/#all/thread" } ],
                "structuredContent": { "thread_id": "thread", "source_url": "https://mail.google.com/mail/u/0/#all/thread", "event": { "htmlLink": "https://calendar.google.com/event?eid=1" }, "document": { "webViewLink": "https://docs.google.com/document/d/1/edit" } }, "isError": false })
        }
        _ => return StatusCode::ACCEPTED.into_response(),
    };
    Json(json!({ "jsonrpc": "2.0", "id": request["id"], "result": result })).into_response()
}

#[tokio::test]
async fn paired_sign_in_selects_only_its_account_and_preserves_sources_and_review() {
    let home = Home(std::env::temp_dir().join(format!("lorca-integrations-{}", uuid::Uuid::new_v4())));
    let app = App::load(Config { home: home.0.clone(), port: 0 }).unwrap();
    api::dispatch(&app, "identity.create", json!({})).await.unwrap();
    let runner_id = app.this_device_id().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let router =
        Router::new().route("/token", post(token)).route("/mcp", post(server).get(|| async { StatusCode::METHOD_NOT_ALLOWED })).with_state(calls.clone());
    let running = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let manifest = json!({ "id": "gmail", "name": "Gmail", "named_accounts": true,
        "servers": { "api": { "type": "http", "url": format!("{origin}/mcp"), "auth": { "type": "oauth", "client_id": "registered",
            "authorization_endpoint": format!("{origin}/authorize"), "token_endpoint": format!("{origin}/token"), "scopes": ["read", "compose"] } } },
        "tools": { "readonly": ["get_thread"], "draft": ["create_draft"] } });
    let mut ids = Vec::new();
    for label in ["Work", "Personal"] {
        let installed = api::dispatch(&app, "plugins.install", json!({ "runner_id": runner_id, "manifest": manifest, "account_name": label })).await.unwrap();
        let id = installed["status"]["id"].as_str().unwrap().to_string();
        assert_eq!(installed["status"]["service_id"], "gmail");
        // The Runner receives these verbs through the existing sealed request dispatcher. A
        // paired Device supplies its own loopback, never a token or PKCE verifier.
        let redirect = "http://127.0.0.1:54321/callback";
        let started =
            plugins::serve_request(&app, "plugins.connect", &json!({ "plugin_id": id, "redirect_uri": redirect }), Some("paired-device")).await.unwrap();
        let page = reqwest::Url::parse(started["url"].as_str().unwrap()).unwrap();
        let state = page.query_pairs().find(|(key, _)| key == "state").unwrap().1.into_owned();
        assert!(!started.to_string().contains("code_verifier") && !started.to_string().contains("token-"));
        plugins::serve_request(
            &app,
            "plugins.sign_in.finish",
            &json!({ "plugin_id": id, "sign_in": started["sign_in"], "url": format!("{redirect}?state={state}&code={label}") }),
            Some("paired-device"),
        )
        .await
        .unwrap();
        ids.push(id);
    }
    assert_ne!(ids[0], ids[1]);
    let renamed = api::dispatch(&app, "plugins.rename", json!({ "runner_id": runner_id, "plugin_id": ids[0], "account_name": "Office" })).await.unwrap();
    assert_eq!(renamed["status"]["id"], ids[0]);
    let catalog = mcp::turn_catalog(&app, Vec::new());
    let cancel = CancellationToken::new();
    assert!(catalog.namespaces().iter().any(|ns| ns.description.contains("Office")));
    for (id, expected) in ids.iter().zip(["Bearer token-Work", "Bearer token-Personal"]) {
        let name = mcp::tool_name(id, "get_thread");
        let entry = catalog.find(&name, &cancel).await.unwrap();
        let result = entry.tool.execute("read", json!({}), cancel.clone(), Arc::new(|_| {})).await.unwrap();
        let structured = result.structured.unwrap();
        assert_eq!(structured["content"][1]["uri"], "https://mail.google.com/mail/u/0/#all/thread");
        assert_eq!(structured["structuredContent"]["event"]["htmlLink"], "https://calendar.google.com/event?eid=1");
        assert_eq!(structured["structuredContent"]["document"]["webViewLink"], "https://docs.google.com/document/d/1/edit");
        assert_eq!(calls.lock().unwrap().last().unwrap(), expected);
    }
    app.set_auto_review(AutoReview { is_enabled: false, ..AutoReview::default() });
    let bot: Bot = app.state.lock().unwrap().bots.first().unwrap().clone();
    let assistant = AssistantMessage::empty("test", "test");
    let context = AgentContext { system_prompt: String::new(), messages: Vec::new(), tools: Vec::new(), cache_points: Vec::new() };
    for (tool, blocked) in [("get_thread", false), ("create_draft", true)] {
        let name = mcp::tool_name(&ids[0], tool);
        catalog.find(&name, &cancel).await.unwrap();
        let call = ToolCall { id: "review".into(), name, arguments: json!({}) };
        let ctx =
            BeforeToolCallContext { assistant_message: &assistant, tool_call: &call, args: &call.arguments, context: &context, cancel: &cancel, parent: None };
        assert_eq!(mcp::review_call(&app, &catalog, "chat", &Trigger::default(), &bot, true, &ctx).await.is_some_and(|review| review.block), blocked);
    }
    assert_eq!(calls.lock().unwrap().len(), 2, "the held draft did not execute");
    let visible = plugins::detail(&app, &ids[0]).unwrap().to_string() + &app.snapshot().to_string();
    assert!(!visible.contains("token-Work") && !visible.contains("token-Personal"));

    // A sign-out after consent starts fences the old callback, leaving the other account
    // authorized. The old code is never exchanged and cannot resurrect the signed-out account.
    let started = plugins::serve_request(
        &app,
        "plugins.connect",
        &json!({ "plugin_id": ids[0], "redirect_uri": "http://127.0.0.1:54321/callback" }),
        Some("paired-device"),
    )
    .await
    .unwrap();
    plugins::sign_out(&app, &ids[0], None).unwrap();
    assert!(plugins::serve_request(
        &app,
        "plugins.sign_in.finish",
        &json!({ "plugin_id": ids[0], "sign_in": started["sign_in"], "url": "http://127.0.0.1:54321/callback?code=late" }),
        Some("paired-device")
    )
    .await
    .is_err());
    assert!(app.plugins.lock().unwrap().sign_in_secret(&ids[0], "oauth", "api").is_none());
    assert!(app.plugins.lock().unwrap().sign_in_secret(&ids[1], "oauth", "api").is_some());
    running.abort();
}
