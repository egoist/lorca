use super::*;
use lorca_agent::ContentPart;

struct Scratch(Arc<App>, std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        self.0.browser_sessions.reset();
        let _ = std::fs::remove_dir_all(&self.1);
    }
}

fn setup() -> Scratch {
    let home = std::env::temp_dir().join(format!("lorca-browser-{}", uuid::Uuid::new_v4()));
    let app = App::load(crate::config::Config {
        home: home.clone(),
        port: 0,
    })
    .unwrap();
    crate::identity::create(&app, Some("Browser test Runner".into())).unwrap();
    let manifest = crate::plugins::Manifest::parse(&json!({ "id": super::super::PLUGIN_ID, "name": "Browser", "servers": { "browser": { "type": "stdio", "command": "false", "args": ["--headless"] } } })).unwrap();
    crate::plugins::install(&app, manifest, "inline").unwrap();
    Scratch(app, home)
}

fn bot(app: &App) -> Bot {
    app.state.lock().unwrap().bots[0].clone()
}

async fn opened(app: &Arc<App>, owner: &Bot) -> (Session, Arc<Mutex<Vec<String>>>) {
    let session = app.browser_sessions.create(app, &owner.id, "Work").unwrap();
    let runtime = app.browser_sessions.owned(app, &owner.id, &session.id).unwrap();
    let (server, calls) = crate::plugins::mcp::tests::fake_browser(app).await;
    *runtime.server.lock().unwrap() = Some(server);
    {
        let mut meta = runtime.meta.lock().unwrap();
        meta.state = Control::Bot;
        meta.revision += 1;
    }
    app.browser_sessions.select(&runtime);
    app.browser_sessions.save(app).unwrap();
    let session = runtime.meta.lock().unwrap().clone();
    (session, calls)
}

fn state(app: &Arc<App>, bot: &Bot) -> Control {
    app.browser_sessions.list(app, &bot.id).unwrap()[0].state
}

#[tokio::test]
async fn records_are_encrypted_owned_and_closed_after_a_restart() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let (session, _) = opened(app, &owner).await;
    let bytes = std::fs::read(scratch.1.join("browser/sessions.enc")).unwrap();
    assert!(!bytes.windows(owner.id.len()).any(|part| part == owner.id.as_bytes()));
    let mut other = owner.clone();
    other.id = "another-bot".into();
    app.state.lock().unwrap().bots.push(other.clone());
    assert!(app.browser_sessions.owned(app, &other.id, &session.id).err().unwrap().contains("another bot"));
    app.browser_sessions.reset();
    let restored = app.browser_sessions.list(app, &owner.id).unwrap();
    assert_eq!((restored[0].id.as_str(), restored[0].name.as_str()), (session.id.as_str(), "Work"));
    assert_eq!(restored[0].state, Control::Stopped);
    // Closed, the profile hands the bot nothing: its calls go to the shared headless browser.
    assert!(app.browser_sessions.input(app, &owner.id, &CancellationToken::new()).await.unwrap().is_none());
    app.state.lock().unwrap().bots[0].runner_id = "another-runner".into();
    assert!(app.browser_sessions.list(app, &owner.id).unwrap_err().contains("Runner"));
}

#[tokio::test]
async fn a_bot_without_an_open_profile_uses_the_shared_browser() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let cancel = CancellationToken::new();
    assert!(app.browser_sessions.input(app, &owner.id, &cancel).await.unwrap().is_none());
    let first = app.browser_sessions.create(app, &owner.id, "Work").unwrap();
    let second = app.browser_sessions.create(app, &owner.id, "Personal").unwrap();
    assert!(first.selected && !second.selected, "the first profile is the one the bot opens");
    assert!(app.browser_sessions.input(app, &owner.id, &cancel).await.unwrap().is_none());
    assert!(app.browser_sessions.create(app, &owner.id, "  ").is_err());
}

#[tokio::test]
async fn takeover_drains_active_input_and_parks_next_call_until_explicit_resume() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let (session, calls) = opened(app, &owner).await;
    let active = app.browser_sessions.input(app, &owner.id, &CancellationToken::new()).await.unwrap().unwrap();
    let takeover = {
        let (app, bot_id, id) = (app.clone(), owner.id.clone(), session.id.clone());
        tokio::spawn(async move { app.browser_sessions.takeover(&app, &bot_id, &id, false).await })
    };
    // The takeover shows before the call in flight lets go of the gate.
    for _ in 0..100 {
        if state(app, &owner) == Control::TakingOver {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(state(app, &owner), Control::TakingOver);
    assert!(!takeover.is_finished());
    let next = {
        let (app, bot_id) = (app.clone(), owner.id.clone());
        tokio::spawn(async move {
            let input = app.browser_sessions.input(&app, &bot_id, &CancellationToken::new()).await?.ok_or("no profile")?;
            input.server.browser_call("browser_snapshot", json!({})).await.map(|_| ())
        })
    };
    active.server.browser_call("browser_snapshot", json!({})).await.unwrap();
    drop(active);
    let human = takeover.await.unwrap().unwrap();
    assert_eq!(human.state, Control::Human);
    assert_eq!(calls.lock().unwrap().len(), 1, "only the active call ran");
    assert!(!next.is_finished(), "the next call waits with its task state");
    assert!(app.browser_sessions.resume(app, &owner.id, &session.id, human.revision - 1).await.unwrap_err().contains("changed"));
    app.browser_sessions.resume(app, &owner.id, &session.id, human.revision).await.unwrap();
    next.await.unwrap().unwrap();
    assert_eq!(calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn stop_during_takeover_wakes_waiters_and_closes_only_that_browser() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let (session, _) = opened(app, &owner).await;
    let mut other = owner.clone();
    other.id = "other-browser-bot".into();
    app.state.lock().unwrap().bots.push(other.clone());
    let (other_session, _) = opened(app, &other).await;
    let other_runtime = app.browser_sessions.owned(app, &other.id, &other_session.id).unwrap();
    app.browser_sessions.takeover(app, &owner.id, &session.id, false).await.unwrap();
    let pending = {
        let (app, bot_id) = (app.clone(), owner.id.clone());
        tokio::spawn(async move { app.browser_sessions.input(&app, &bot_id, &CancellationToken::new()).await.map(|input| input.is_some()) })
    };
    tokio::task::yield_now().await;
    assert!(!pending.is_finished());
    let stopped = app.browser_sessions.stop(app, &owner.id, &session.id).await.unwrap();
    assert_eq!(stopped.state, Control::Stopped);
    assert_eq!(pending.await.unwrap(), Ok(false), "the call goes on without the closed profile");
    assert!(app.browser_sessions.resume(app, &owner.id, &session.id, stopped.revision).await.is_err());
    assert!(!other_runtime.server.lock().unwrap().as_ref().unwrap().is_closed());
    assert!(app.browser_sessions.input(app, &other.id, &CancellationToken::new()).await.unwrap().is_some());
}

#[tokio::test]
async fn a_stopped_call_keeps_the_browser_and_holds_a_takeover_until_it_answers() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let (session, calls) = opened(app, &owner).await;
    let chat_id = app.state.lock().unwrap().chats[0].meta.id.clone();
    let tool = crate::plugins::mcp::tests::browser_tool(app, &owner, &chat_id, "browser_wait_for");
    let cancel = CancellationToken::new();
    let call = {
        let cancel = cancel.clone();
        tokio::spawn(async move { tool.execute("call-1", json!({}), cancel, Arc::new(|_| {})).await.map(|_| ()) })
    };
    while !calls.lock().unwrap().contains(&"browser_wait_for".to_string()) {
        tokio::task::yield_now().await;
    }
    cancel.cancel();
    assert!(call.await.unwrap().is_err(), "the chat's Stop ends the call at once");
    let takeover = {
        let (app, bot_id, id) = (app.clone(), owner.id.clone(), session.id.clone());
        tokio::spawn(async move { app.browser_sessions.takeover(&app, &bot_id, &id, false).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!takeover.is_finished(), "the server hasn't said the stopped call is over");
    assert_eq!(takeover.await.unwrap().unwrap().state, Control::Human);
    let runtime = app.browser_sessions.owned(app, &owner.id, &session.id).unwrap();
    assert!(runtime.open_server().is_some(), "the browser stays open");
}

#[tokio::test]
async fn screenshot_evidence_uses_encrypted_file_and_chat_blobs() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let (session, _) = opened(app, &owner).await;
    let chat_id = app.state.lock().unwrap().chats[0].meta.id.clone();
    let message = app.browser_sessions.screenshot(app, &owner.id, &session.id, &chat_id).await.unwrap();
    let crate::model::Body::Text { attachments, .. } = &message.body else { panic!("a text message with the image") };
    assert_eq!(attachments.len(), 1);
    let output = message.output.as_ref().unwrap();
    assert_eq!((output.name.as_str(), output.version, output.bot_id.as_str()), ("Browser · Work.png", 1, owner.id.as_str()));
    let evidence = output.evidence.as_ref().unwrap();
    assert_eq!((evidence.kind, evidence.status), (crate::outputs::EvidenceKind::AfterScreenshot, crate::outputs::EvidenceStatus::Unverified));
    let pending = app.store.outbox().unwrap();
    let file = pending.iter().find(|item| item.id == attachments[0].id).unwrap();
    assert_eq!(file.kind, "file");
    assert_eq!(file.group.as_deref(), Some(chat_id.as_str()));
    let decrypted = crate::crypto::decrypt(&app.dek().unwrap(), "file", &file.ciphertext).unwrap();
    assert_eq!(&decrypted[..8], b"\x89PNG\r\n\x1a\n");
    // The profile's next screenshot is the next version of the same output.
    let next = app.browser_sessions.screenshot(app, &owner.id, &session.id, &chat_id).await.unwrap();
    let next = next.output.unwrap();
    assert_eq!((next.id.as_str(), next.version, next.previous_message_id.as_deref()), (output.id.as_str(), 2, Some(message.id.as_str())));
    assert!(app.browser_sessions.screenshot(app, &owner.id, &session.id, "wrong-chat").await.is_err());
    // Another Device lists and controls the profile, but no window opens for it here.
    let remote = crate::browser::serve(app, "browser.sessions", &json!({ "bot_id": owner.id }), true).await.unwrap();
    assert_eq!(remote["sessions"][0]["id"], session.id.as_str());
    assert!(crate::browser::serve(app, "browser.open", &json!({ "bot_id": owner.id, "session_id": session.id }), true).await.is_err());
}

#[tokio::test]
async fn cancellation_releases_a_parked_call_and_the_bot_cannot_override_the_user() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let (session, _) = opened(app, &owner).await;
    app.browser_sessions.takeover(app, &owner.id, &session.id, false).await.unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(app.browser_sessions.wait_if_taken_over(&owner.id, &cancel).await.unwrap_err(), "Stopped");
    assert!(app.browser_sessions.open(app, &owner.id, &session.id, false).await.unwrap_err().contains("hand it back"));
    assert_eq!(state(app, &owner), Control::Human);
}

#[tokio::test]
async fn deleting_a_profile_closes_it_and_forgets_its_folder() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let (session, _) = opened(app, &owner).await;
    let later = app.browser_sessions.create(app, &owner.id, "Personal").unwrap();
    let folder = scratch.1.join("browser/profiles").join(&session.id);
    std::fs::create_dir_all(folder.join("Default")).unwrap();
    let server = app.browser_sessions.owned(app, &owner.id, &session.id).unwrap().server.lock().unwrap().clone().unwrap();
    app.browser_sessions.delete(app, &owner.id, &session.id).await.unwrap();
    assert!(server.is_closed());
    assert!(!folder.exists());
    let left = app.browser_sessions.list(app, &owner.id).unwrap();
    assert_eq!(left.len(), 1);
    assert!(left[0].id == later.id && left[0].selected, "the profile left is the one the bot opens");
}

#[tokio::test]
async fn removing_the_plugin_and_forgetting_the_identity_close_every_browser() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let (session, _) = opened(app, &owner).await;
    let runtime = app.browser_sessions.owned(app, &owner.id, &session.id).unwrap();
    let server = runtime.server.lock().unwrap().clone().unwrap();
    crate::plugins::uninstall(app, super::super::PLUGIN_ID).unwrap();
    assert!(server.is_closed());
    assert_eq!(state(app, &owner), Control::Stopped);
    app.forget_identity().unwrap();
    assert!(!scratch.1.join("browser").exists());
    assert!(app.browser_sessions.list(app, &owner.id).is_err());
}

#[tokio::test]
async fn stop_invalidates_an_open_that_queued_behind_active_input() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let (session, _) = opened(app, &owner).await;
    let active = app.browser_sessions.input(app, &owner.id, &CancellationToken::new()).await.unwrap().unwrap();
    let pending_open = {
        let (app, bot_id, id) = (app.clone(), owner.id.clone(), session.id.clone());
        tokio::spawn(async move { app.browser_sessions.open(&app, &bot_id, &id, true).await })
    };
    tokio::task::yield_now().await;
    let stop = {
        let (app, bot_id, id) = (app.clone(), owner.id.clone(), session.id.clone());
        tokio::spawn(async move { app.browser_sessions.stop(&app, &bot_id, &id).await })
    };
    tokio::task::yield_now().await;
    drop(active);
    assert!(pending_open.await.unwrap().unwrap_err().contains("changed"));
    stop.await.unwrap().unwrap();
    let runtime = app.browser_sessions.owned(app, &owner.id, &session.id).unwrap();
    assert!(runtime.server.lock().unwrap().is_none());
    assert_eq!(runtime.meta.lock().unwrap().state, Control::Stopped);
}

/// Opens a real headed browser on this computer with a fresh profile and a loopback-only sign-in
/// page, and checks that the sign-in outlives Return to Bot and a close and reopen.
#[tokio::test]
#[ignore = "requires Node/npx, Chrome, and a desktop; opens a visible browser"]
async fn live_visible_browser_keeps_its_sign_in() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let manifest = crate::plugins::Manifest::parse(&json!({ "id": super::super::PLUGIN_ID, "name": "Browser", "servers": { "browser": { "type": "stdio", "command": "npx", "args": ["-y", "@playwright/mcp@latest", "--headless"] } } })).unwrap();
    crate::plugins::install(app, manifest, "inline").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let fixture = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request).await;
            let page = r#"<!doctype html><title>Sign in</title><form><input name=email value=demo@example.test><button type=submit>Sign in</button></form><p id=result></p><script>const update=()=>{const signed=document.cookie.includes('demo=signed-in');document.querySelector('#result').textContent=signed?'Signed in':'Signed out';document.querySelector('form').style.display=signed?'none':'block'};document.querySelector('form').onsubmit=e=>{e.preventDefault();document.cookie='demo=signed-in; Max-Age=86400; Path=/';update()};update();</script>"#;
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", page.len(), page);
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });
    let session = app.browser_sessions.create(app, &owner.id, "Demo").unwrap();
    let human = app.browser_sessions.open(app, &owner.id, &session.id, true).await.unwrap();
    let runtime = app.browser_sessions.owned(app, &owner.id, &session.id).unwrap();
    let server = runtime.server.lock().unwrap().clone().unwrap();
    server.browser_call("browser_navigate", json!({ "url": url })).await.unwrap();
    // The person at the Runner signs in; this call stands in for their click.
    server.browser_call("browser_click", json!({ "element": "Sign in button", "target": "button[type=submit]" })).await.unwrap();
    app.browser_sessions.resume(app, &owner.id, &session.id, human.revision).await.unwrap();
    let input = app.browser_sessions.input(app, &owner.id, &CancellationToken::new()).await.unwrap().unwrap();
    let snapshot = input.server.browser_call("browser_snapshot", json!({})).await.unwrap();
    assert!(serde_json::to_string(&snapshot).unwrap().contains("Signed in"));
    drop(input);
    app.browser_sessions.stop(app, &owner.id, &session.id).await.unwrap();
    app.browser_sessions.open(app, &owner.id, &session.id, true).await.unwrap();
    let server = runtime.server.lock().unwrap().clone().unwrap();
    server.browser_call("browser_navigate", json!({ "url": url })).await.unwrap();
    let snapshot = server.browser_call("browser_snapshot", json!({})).await.unwrap();
    assert!(serde_json::to_string(&snapshot).unwrap().contains("Signed in"), "the sign-in outlives a close and reopen");
    let chat_id = app.state.lock().unwrap().chats[0].meta.id.clone();
    app.browser_sessions.screenshot(app, &owner.id, &session.id, &chat_id).await.unwrap();
    app.browser_sessions.stop(app, &owner.id, &session.id).await.unwrap();
    fixture.abort();
}

#[tokio::test]
async fn the_bots_access_to_browser_covers_its_profiles() {
    let scratch = setup();
    let app = &scratch.0;
    let mut owner = bot(app);
    // A grant that lists only some of Browser's own tools still covers browser_session.
    owner.permissions = Some(serde_json::from_value(json!({ "connections": { "playwright": { "capabilities": ["read"], "tools": ["browser_snapshot"] } } })).unwrap());
    app.state.lock().unwrap().bots[0] = owner.clone();
    assert!(crate::permissions::check_plugin(app, &owner, super::super::PLUGIN_ID, "browser_session").is_ok());
    owner.permissions = Some(serde_json::from_value(json!({ "connections": {} })).unwrap());
    app.state.lock().unwrap().bots[0] = owner.clone();
    let denied = crate::permissions::check_plugin(app, &owner, super::super::PLUGIN_ID, "browser_session").unwrap_err();
    assert!(denied.grantable && denied.reason.contains("Browser is off"), "{denied}");
}

/// A profile open with the bot in control, on a scripted Browser server.
async fn scripted(app: &Arc<App>, owner: &Bot, name: &str, answer: crate::plugins::mcp::tests::BrowserAnswer) -> (Session, Arc<Mutex<Vec<(String, Value)>>>) {
    let session = app.browser_sessions.create(app, &owner.id, name).unwrap();
    let runtime = app.browser_sessions.owned(app, &owner.id, &session.id).unwrap();
    let tools = ["browser_run_code_unsafe", "browser_click", "browser_type", "browser_tabs", "browser_take_screenshot", "browser_navigate", "browser_wait_for"];
    let (server, calls) = crate::plugins::mcp::tests::scripted_browser(app, &tools, answer).await;
    *runtime.server.lock().unwrap() = Some(server);
    {
        let mut meta = runtime.meta.lock().unwrap();
        meta.state = Control::Bot;
        meta.revision += 1;
    }
    app.browser_sessions.select(&runtime);
    app.browser_sessions.save(app).unwrap();
    let session = runtime.meta.lock().unwrap().clone();
    (session, calls)
}

fn text(value: &str) -> Value {
    json!([{ "type": "text", "text": value }])
}

#[tokio::test]
async fn a_recording_reaches_the_bot_with_secrets_left_out_and_stays_encrypted() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let chat_id = app.state.lock().unwrap().chats[0].meta.id.clone();
    app.credentials.lock().unwrap().deepseek = Some(crate::credentials::ApiKeyCredential { api_key: "sk-account-key-0123456789".into(), base_url: None, connected_at: 0 });
    let home = scratch.1.clone();
    let profile = Arc::new(Mutex::new(String::new()));
    let seen = profile.clone();
    let answer: crate::plugins::mcp::tests::BrowserAnswer = Arc::new(move |name, args| {
        if name != "browser_run_code_unsafe" {
            return Ok(text("ok"));
        }
        let code = args["code"].as_str().unwrap();
        assert!(code.contains("context.exposeBinding"), "the recorder goes with the call");
        if code.ends_with(r#"recorder(page, "start", {"binding":"#) || code.contains(r#"recorder(page, "start""#) {
            return Ok(text("### Result\n{\"lorca\":1,\"started\":true}"));
        }
        let dir = home.join("browser/output").join(seen.lock().unwrap().as_str()).join("recording");
        let mut jpeg = Vec::new();
        image::DynamicImage::ImageRgb8(image::ImageBuffer::from_pixel(4, 4, image::Rgb([10, 20, 30]))).write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
        std::fs::write(dir.join("step-2.jpg"), jpeg).unwrap();
        let steps = json!({ "lorca": 1, "truncated": false, "steps": [
            { "action": "goto", "url": "https://ads.example.com/", "page": { "url": "https://ads.example.com/", "title": "Ads" }, "shot": "step-1.jpg" },
            { "action": "fill", "element": "“Campaign name” textbox", "targets": ["getByLabel('Campaign name', { exact: true })", "locator('#name')"], "value": "Spring sale", "page": { "url": "https://ads.example.com/", "title": "Ads" }, "shot": "step-2.jpg" },
            { "action": "fill", "element": "“Password” input", "targets": ["getByLabel('Password', { exact: true })"], "secret": true },
            { "action": "fill", "element": "“Key” textbox", "targets": ["locator('#key')"], "value": "sk-account-key-0123456789" },
            { "action": "click", "element": "“Create” button", "targets": ["getByRole('button', { name: 'Create', exact: true })"], "after": { "url": "https://ads.example.com/new", "title": "New" }, "shot": "../../sessions.enc" },
            { "action": "drag", "targets": ["#a"] },
            { "action": "click", "targets": ["#a\n#b"] },
        ] });
        Ok(text(&format!("### Result\n{steps}\n\n### Page\n- Page URL: https://ads.example.com/new")))
    });
    let (session, _) = scripted(app, &owner, "Work", answer).await;
    *profile.lock().unwrap() = session.id.clone();

    // From another Device, a closed browser doesn't record: the window opens on the Runner.
    let closed = app.browser_sessions.create(app, &owner.id, "Personal").unwrap();
    let error = crate::browser::serve(app, "browser.record", &json!({ "bot_id": owner.id, "session_id": closed.id }), true).await.unwrap_err();
    assert!(error.contains("Record it on Browser test Runner"), "{error}");

    let recording = crate::browser::serve(app, "browser.record", &json!({ "bot_id": owner.id, "session_id": session.id }), true).await.unwrap();
    assert_eq!((recording["session"]["state"].as_str(), recording["session"]["recording"].as_bool()), (Some("human"), Some(true)), "the user takes the browser to record");
    let revision = recording["session"]["revision"].as_u64().unwrap();
    assert!(app.browser_sessions.resume(app, &owner.id, &session.id, revision).await.unwrap_err().contains("Stop recording"));

    let stopped = crate::browser::serve(app, "browser.stop_recording", &json!({ "bot_id": owner.id, "session_id": session.id, "chat_id": chat_id, "text": "" }), true).await.unwrap();
    assert_eq!((stopped["session"]["state"].as_str(), stopped["session"]["recording"].as_bool()), (Some("human"), Some(false)), "the user keeps the browser");
    let message = app.message(&chat_id, stopped["message_id"].as_str().unwrap()).unwrap();
    let reference = message.recording.clone().unwrap();
    assert_eq!((reference.profile.as_str(), reference.steps, reference.bot_id.as_str()), ("Work", 5, owner.id.as_str()), "a step that isn't one is left out");
    let crate::model::Body::Text { text: words, .. } = &message.body else { panic!("the user's message") };
    assert!(words.starts_with("I recorded this in the Work browser"));
    assert!(!scratch.1.join("browser/output").join(&session.id).join("recording").exists(), "the screenshots move into the encrypted recording");

    let stored = std::fs::read_dir(scratch.1.join("browser/recordings")).unwrap().flatten().next().unwrap();
    assert!(stored.file_name().to_string_lossy().starts_with(&format!("{}--rec-", session.id)));
    let bytes = std::fs::read(stored.path()).unwrap();
    assert!(!bytes.windows(11).any(|part| part == b"Spring sale"), "a recording is kept encrypted");

    let shown = super::recording_content(app, &message.id, &reference, true);
    let words: String = shown.iter().filter_map(ContentPart::as_text).collect::<Vec<_>>().join("\n");
    assert!(words.contains(r#""value":"Spring sale""#) && words.contains(r#""secret":true"#), "{words}");
    assert!(!words.contains("sk-account-key"), "what looks like the account's credential is kept as a secret");
    assert!(words.contains(&format!("message_ids [\"{}\"]", message.id)) && words.contains("kind \"recording\""));
    assert!(words.contains("on “Ads” https://ads.example.com/") && words.contains("→ “New” https://ads.example.com/new"));
    assert_eq!(shown.iter().filter(|part| matches!(part, ContentPart::Image { .. })).count(), 1, "the screenshot the recorder took, and no file outside its folder");
    assert!(!super::recording_content(app, &message.id, &reference, false).iter().any(|part| matches!(part, ContentPart::Image { .. })));
    // The bot's turn reads it with the user's words.
    let chat = app.chat(&chat_id).unwrap();
    let transcript = crate::turns::transcript_for(app, &chat, &owner, &scratch.1.join("work"));
    let lorca_agent::AgentMessage::User(said) = transcript.iter().rev().find(|message| matches!(message, lorca_agent::AgentMessage::User(_))).unwrap() else { unreachable!() };
    let said: Vec<&str> = said.content.iter().filter_map(ContentPart::as_text).collect();
    assert!(said[0].starts_with("I recorded this in the Work browser") && said[1].starts_with(&format!("[Recording {}, message {}", reference.id, message.id)), "{said:?}");

    // The bot drafts a skill from it, citing the user's message.
    let steps_file = json!({ "profile": "Work", "inputs": { "campaign": "The campaign's name" }, "steps": [
        { "action": "goto", "url": "https://ads.example.com/" },
        { "action": "fill", "targets": ["getByLabel('Campaign name', { exact: true })"], "value": "{{campaign}}" },
    ] });
    let content = crate::playbooks::PlaybookContent {
        name: "new-campaign".into(), description: "Start an ad campaign".into(), instructions: "Run it with browser_session run.".into(), examples: String::new(),
        references: Vec::new(), scripts: vec![crate::playbooks::Resource { path: crate::browser::steps::PATH.into(), text: steps_file.to_string() }],
    };
    let scope = crate::playbooks::Scope::bot(&owner.id);
    crate::playbooks::selected_evidence(app, &scope, &owner.id, &chat_id, "recording", std::slice::from_ref(&message.id)).unwrap();
    let mut broken = content.clone();
    broken.scripts[0].text = json!({ "profile": "Work", "steps": [{ "action": "click" }] }).to_string();
    assert!(crate::playbooks::draft(app, &scope, broken, Default::default()).unwrap_err().contains("step 1 needs 1 to 8 targets"));
    crate::playbooks::draft(app, &scope, content, Default::default()).unwrap();
    assert!(super::replay::skill_steps(app, &owner.id, &chat_id, "new-campaign").unwrap_err().contains("once the user saves it"), "a draft isn't run");

    // Closing the browser drops a recording in progress.
    crate::browser::serve(app, "browser.record", &json!({ "bot_id": owner.id, "session_id": session.id }), true).await.unwrap();
    app.browser_sessions.stop(app, &owner.id, &session.id).await.unwrap();
    assert!(!app.browser_sessions.list(app, &owner.id).unwrap()[0].recording);
    // Deleting the profile forgets its recordings.
    app.browser_sessions.delete(app, &owner.id, &session.id).await.unwrap();
    assert!(super::recording_content(app, &message.id, &reference, false)[0].as_text().unwrap().contains("kept on the Runner that recorded it"));
}

fn save_skill(app: &App, owner: &Bot, name: &str, steps: Value) {
    let content = crate::playbooks::PlaybookContent {
        name: name.into(), description: "Repeat the recorded workflow".into(), instructions: "Run it with browser_session run.".into(), examples: String::new(),
        references: Vec::new(), scripts: vec![crate::playbooks::Resource { path: crate::browser::steps::PATH.into(), text: steps.to_string() }],
    };
    crate::playbooks::save(app, &crate::playbooks::Scope::bot(&owner.id), None, content, 0, "", Default::default()).unwrap();
}

/// A page with a Create button that leads to /new, and nothing else to click.
fn page_answer() -> crate::plugins::mcp::tests::BrowserAnswer {
    let url = Arc::new(Mutex::new("https://ads.example.com/".to_string()));
    Arc::new(move |name, args| {
        let target = args["target"].as_str().unwrap_or_default();
        match name {
            "browser_navigate" => {
                *url.lock().unwrap() = args["url"].as_str().unwrap().to_string();
                Ok(text("ok"))
            }
            "browser_click" | "browser_type" if target == "locator('#create')" || target == "locator('#name')" => {
                if target == "locator('#create')" {
                    *url.lock().unwrap() = "https://ads.example.com/new".into();
                }
                Ok(text("ok"))
            }
            "browser_click" | "browser_type" => Err(format!("\"{target}\" does not match any elements.")),
            "browser_tabs" => Ok(text(&format!("### Result\n- 0: (current) [Ads]({})", url.lock().unwrap()))),
            "browser_take_screenshot" => Ok(crate::plugins::mcp::tests::png_content()),
            _ => Ok(text("ok")),
        }
    })
}

#[tokio::test]
async fn a_run_takes_the_first_target_the_page_has_and_checks_what_it_expects() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let chat_id = app.state.lock().unwrap().chats[0].meta.id.clone();
    let (_, calls) = scripted(app, &owner, "Work", page_answer()).await;
    save_skill(app, &owner, "new-campaign", json!({ "profile": "Work", "inputs": { "campaign": "The campaign's name" }, "steps": [
        { "action": "goto", "url": "https://ads.example.com/" },
        { "action": "fill", "targets": ["locator('#title')", "locator('#name')"], "value": "{{campaign}}" },
        { "action": "click", "element": "“Create” button", "targets": ["getByRole('button', { name: 'New', exact: true })", "locator('#create')"], "expect": { "url": "/new" } },
    ] }));
    let cancel = CancellationToken::new();
    let missing = super::replay::run(app, &owner, &chat_id, "new-campaign", &Default::default(), &cancel).await.err().unwrap();
    assert!(missing.0.contains("campaign (The campaign's name)"), "{}", missing.0);
    let inputs = std::collections::BTreeMap::from([("campaign".to_string(), "Spring sale".to_string())]);
    let result = super::replay::run(app, &owner, &chat_id, "new-campaign", &inputs, &cancel).await.unwrap();
    assert!(!result.is_error && !result.terminate);
    assert_eq!(result.content[0].as_text().unwrap(), "Ran the 3 steps of new-campaign in the Work browser.");
    let calls = calls.lock().unwrap();
    let typed = calls.iter().find(|(name, args)| name == "browser_type" && args["target"] == "locator('#name')").unwrap();
    assert_eq!(typed.1["text"], "Spring sale");
    let names: Vec<&str> = calls.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["browser_navigate", "browser_type", "browser_type", "browser_click", "browser_click", "browser_tabs"]);
}

#[tokio::test]
async fn a_step_the_page_no_longer_matches_stops_the_run_with_a_screenshot() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let chat_id = app.state.lock().unwrap().chats[0].meta.id.clone();
    let (_, calls) = scripted(app, &owner, "Work", page_answer()).await;
    save_skill(app, &owner, "upload-creative", json!({ "profile": "Work", "steps": [
        { "action": "goto", "url": "https://ads.example.com/" },
        { "action": "click", "element": "“Upload” button", "targets": ["getByRole('button', { name: 'Upload', exact: true })", "locator('#upload')"], "timeout": 1 },
        { "action": "click", "targets": ["locator('#create')"] },
    ] }));
    let result = super::replay::run(app, &owner, &chat_id, "upload-creative", &Default::default(), &CancellationToken::new()).await.unwrap();
    assert!(result.is_error && result.terminate, "the turn ends there");
    let words = result.content[0].as_text().unwrap();
    assert!(words.starts_with("upload-creative stopped at step 2 of 3 (click “Upload” button): none of its 2 ways to find the element matches the page"), "{words}");
    assert!(!calls.lock().unwrap().iter().any(|(_, args)| args["target"] == "locator('#create')"), "nothing after the step runs");
    let messages = app.store.page(&chat_id, None, 10).unwrap().0;
    assert!(messages.iter().any(|message| matches!(&message.body, crate::model::Body::Notice { text, .. } if text.starts_with("upload-creative stopped at step 2 of 3"))));
    assert!(messages.iter().any(|message| message.output.as_ref().is_some_and(|output| output.name == "Browser · Work.png")), "a screenshot of the page is in the chat");
    // A skill that types a password, or runs in a profile the bot lacks, doesn't start.
    save_skill(app, &owner, "sign-in", json!({ "profile": "Work", "steps": [{ "action": "fill", "targets": ["#pw"], "secret": true }] }));
    assert!(super::replay::run(app, &owner, &chat_id, "sign-in", &Default::default(), &CancellationToken::new()).await.err().unwrap().0.contains("types a password"));
    save_skill(app, &owner, "elsewhere", json!({ "profile": "Personal", "steps": [{ "action": "goto", "url": "https://example.com/" }] }));
    assert!(super::replay::run(app, &owner, &chat_id, "elsewhere", &Default::default(), &CancellationToken::new()).await.err().unwrap().0.contains("“Personal” browser profile, which you don't have"));
}

/// Records a workflow done in a real headless browser on this computer's Playwright MCP, makes a
/// skill of it, runs it, and runs it again after the page changed: the recorder, its locators,
/// and the run against the real Browser tools.
#[tokio::test]
#[ignore = "requires Node/npx and Playwright's Chromium; runs Playwright MCP headless"]
async fn live_record_then_run_against_real_playwright() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let chat_id = app.state.lock().unwrap().chats[0].meta.id.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let renamed = Arc::new(AtomicBool::new(false));
    let fixture = {
        let renamed = renamed.clone();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0u8; 4096];
                let read = socket.read(&mut request).await.unwrap_or(0);
                let path = String::from_utf8_lossy(&request[..read]).split_whitespace().nth(1).unwrap_or("/").to_string();
                let button = if renamed.load(Ordering::SeqCst) { r#"<button type="submit" id="launch">Launch</button>"# } else { r#"<button type="submit" id="create">Create</button>"# };
                let page = if path.starts_with("/done") {
                    "<!doctype html><title>Created</title><h1>Campaign created</h1>".to_string()
                } else {
                    format!(r#"<!doctype html><title>Campaigns</title><h1>Campaigns</h1><form onsubmit="event.preventDefault(); location.href='/done'"><label for="campaign">Campaign name</label><input id="campaign" name="campaign"><label>Password <input type="password" name="pw"></label><label><input type="checkbox" id="agree"> I agree</label>{button}</form>"#)
                };
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", page.len(), page);
                let _ = socket.write_all(response.as_bytes()).await;
            }
        })
    };
    let session = app.browser_sessions.create(app, &owner.id, "Work").unwrap();
    let runtime = app.browser_sessions.owned(app, &owner.id, &session.id).unwrap();
    let server = crate::plugins::mcp::tests::headless_browser(app, &scratch.1).await;
    *runtime.server.lock().unwrap() = Some(server.clone());
    {
        let mut meta = runtime.meta.lock().unwrap();
        meta.state = Control::Bot;
        meta.revision += 1;
    }
    app.browser_sessions.select(&runtime);
    server.browser_call("browser_navigate", json!({ "url": format!("{base}/") })).await.unwrap();

    app.browser_sessions.record(app, &owner.id, &session.id, false).await.unwrap();
    // The user's own input, which Playwright gives the page as trusted events.
    server.browser_call("browser_type", json!({ "target": "#campaign", "text": "Spring sale" })).await.unwrap();
    server.browser_call("browser_type", json!({ "target": "input[name=pw]", "text": "hunter2-secret" })).await.unwrap();
    server.browser_call("browser_click", json!({ "target": "#agree" })).await.unwrap();
    server.browser_call("browser_click", json!({ "target": "#create" })).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let (stopped, message) = app.browser_sessions.stop_recording(app, &owner.id, &session.id, &chat_id, "").await.unwrap();
    let message = message.expect("a recording with steps");
    let recording = super::recording::load(app, &message.recording.as_ref().unwrap().id).unwrap();
    let actions: Vec<_> = recording.steps.iter().map(|recorded| recorded.step.action).collect();
    use super::super::steps::Action;
    assert_eq!(actions, [Action::Goto, Action::Fill, Action::Fill, Action::Check, Action::Click], "{recording:#?}");
    assert_eq!(recording.steps[1].step.value.as_deref(), Some("Spring sale"));
    assert!(recording.steps[2].step.secret && recording.steps[2].step.value.is_none());
    assert!(!serde_json::to_string(&recording).unwrap().contains("hunter2"), "the password is never kept");
    assert!(recording.steps[4].after.as_ref().unwrap().url.ends_with("/done"));
    assert!(recording.steps.iter().filter(|recorded| recorded.shot.is_some()).count() >= 4, "a screenshot per step");
    app.browser_sessions.resume(app, &owner.id, &session.id, stopped.revision).await.unwrap();

    // The skill the bot would write: the steps less the password, the name as an input, and
    // what the page shows after Create.
    let mut steps: Vec<_> = recording.steps.iter().map(|recorded| recorded.step.clone()).filter(|step| !step.secret).collect();
    steps[1].value = Some("{{campaign}}".into());
    steps[3].expect = Some(super::super::steps::Expect { url: Some("/done".into()), text: Some("Campaign created".into()), title: None });
    save_skill(app, &owner, "new-campaign", json!({ "profile": "Work", "inputs": { "campaign": "The campaign's name" }, "steps": steps }));
    let inputs = std::collections::BTreeMap::from([("campaign".to_string(), "Autumn".to_string())]);
    let ran = super::replay::run(app, &owner, &chat_id, "new-campaign", &inputs, &CancellationToken::new()).await.unwrap();
    assert!(!ran.is_error, "{:?}", ran.content);

    renamed.store(true, Ordering::SeqCst);
    let stopped = super::replay::run(app, &owner, &chat_id, "new-campaign", &inputs, &CancellationToken::new()).await.unwrap();
    assert!(stopped.is_error && stopped.terminate);
    assert!(stopped.content[0].as_text().unwrap().contains("stopped at step 4 of 4"), "{:?}", stopped.content);
    let messages = app.store.page(&chat_id, None, 20).unwrap().0;
    assert!(messages.iter().any(|message| message.output.as_ref().is_some_and(|output| output.name == "Browser · Work.png")));
    server.stop();
    fixture.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_ends_a_run_at_once_and_its_step_keeps_the_browser_until_it_answers() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let chat_id = app.state.lock().unwrap().chats[0].meta.id.clone();
    let answer: crate::plugins::mcp::tests::BrowserAnswer = Arc::new(|name, args| {
        if name == "browser_click" && args["target"] == "locator('#slow')" {
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
        Ok(text("ok"))
    });
    let (session, _) = scripted(app, &owner, "Work", answer).await;
    save_skill(app, &owner, "slow-step", json!({ "profile": "Work", "steps": [{ "action": "click", "targets": ["locator('#slow')"] }] }));
    let cancel = CancellationToken::new();
    let run = {
        let (app, owner, chat_id, cancel) = (app.clone(), owner.clone(), chat_id.clone(), cancel.clone());
        tokio::spawn(async move { super::replay::run(&app, &owner, &chat_id, "slow-step", &Default::default(), &cancel).await.map(|_| ()).map_err(|error| error.0) })
    };
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let stopped_at = std::time::Instant::now();
    cancel.cancel();
    assert_eq!(run.await.unwrap(), Err("Stopped".to_string()));
    assert!(stopped_at.elapsed() < std::time::Duration::from_millis(500), "Stop ends the run at once");
    let takeover = {
        let (app, bot_id, id) = (app.clone(), owner.id.clone(), session.id.clone());
        tokio::spawn(async move { app.browser_sessions.takeover(&app, &bot_id, &id, false).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(!takeover.is_finished(), "the step's call keeps the browser until its server answers");
    assert_eq!(takeover.await.unwrap().unwrap().state, Control::Human);
    let runtime = app.browser_sessions.owned(app, &owner.id, &session.id).unwrap();
    assert!(runtime.open_server().is_some(), "a call that answered in time leaves the browser open");
}

fn save_secret(app: &App, owner: &Bot, name: &str, site: &str, value: &str) {
    let ask = crate::model::SecretAsk { target: crate::secrets::BROWSER.into(), site: Some(site.into()), fields: vec![crate::model::SecretField { name: name.into(), label: "Password".into() }] };
    crate::secrets::keep(app, &owner.id, &ask, &std::collections::BTreeMap::from([(name.to_string(), value.to_string())])).unwrap();
}

#[tokio::test]
async fn a_recording_keeps_a_saved_secret_only_as_its_placeholder() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let chat_id = app.state.lock().unwrap().chats[0].meta.id.clone();
    save_secret(app, &owner, "SHOP_PIN", "shop.example.com", "4242-secret-pin");
    let home = scratch.1.clone();
    let profile = Arc::new(Mutex::new(String::new()));
    let seen = profile.clone();
    let answer: crate::plugins::mcp::tests::BrowserAnswer = Arc::new(move |name, args| {
        if name != "browser_run_code_unsafe" {
            return Ok(text("ok"));
        }
        if args["code"].as_str().unwrap().contains(r#"recorder(page, "start""#) {
            return Ok(text("{\"lorca\":1,\"started\":true}"));
        }
        let dir = home.join("browser/output").join(seen.lock().unwrap().as_str()).join("recording");
        let mut jpeg = Vec::new();
        image::DynamicImage::ImageRgb8(image::ImageBuffer::from_pixel(4, 4, image::Rgb([10, 20, 30]))).write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
        for n in 1..=4 {
            std::fs::write(dir.join(format!("step-{n}.jpg")), &jpeg).unwrap();
        }
        let shop = json!({ "url": "https://shop.example.com/checkout", "title": "Checkout" });
        let steps = json!({ "lorca": 1, "steps": [
            { "action": "fill", "element": "“PIN” textbox", "targets": ["getByLabel('PIN', { exact: true })", "getByText('4242-secret-pin', { exact: true })"], "value": "4242-secret-pin", "page": shop, "shot": "step-1.jpg" },
            { "action": "click", "element": "“Pay” button", "targets": ["getByRole('button', { name: 'Pay', exact: true })"], "page": shop, "shot": "step-2.jpg" },
            { "action": "click", "element": "“Done” button", "targets": ["getByRole('button', { name: 'Done', exact: true })"], "page": { "url": "https://shop.example.com/thanks", "title": "Thanks 4242-secret-pin" }, "shot": "step-3.jpg" },
        ] });
        Ok(text(&format!("### Result\n{steps}")))
    });
    let (session, _) = scripted(app, &owner, "Shop", answer).await;
    *profile.lock().unwrap() = session.id.clone();
    app.browser_sessions.record(app, &owner.id, &session.id, false).await.unwrap();
    let (_, message) = app.browser_sessions.stop_recording(app, &owner.id, &session.id, &chat_id, "").await.unwrap();
    let recording = super::recording::load(app, &message.unwrap().recording.unwrap().id).unwrap();
    assert!(!serde_json::to_string(&recording).unwrap().contains("4242-secret-pin"), "a saved value is never kept");
    assert_eq!(recording.steps[0].step.value.as_deref(), Some("{{secret:SHOP_PIN}}"));
    assert_eq!(recording.steps[0].step.targets, ["getByLabel('PIN', { exact: true })"], "a target that finds the element by the value goes");
    let shots: Vec<bool> = recording.steps.iter().map(|recorded| recorded.shot.is_some()).collect();
    assert_eq!(shots, [false, false, true], "no screenshot of the page the secret was typed into in plain sight");
    assert_eq!(recording.steps[2].page.as_ref().unwrap().title, "Thanks {{secret:SHOP_PIN}}");
}

#[tokio::test]
async fn a_run_types_a_saved_secret_on_its_site_only() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let chat_id = app.state.lock().unwrap().chats[0].meta.id.clone();
    let url = Arc::new(Mutex::new("https://shop.example.com/login".to_string()));
    let page = url.clone();
    let answer: crate::plugins::mcp::tests::BrowserAnswer = Arc::new(move |name, args| match name {
        "browser_navigate" => {
            *page.lock().unwrap() = args["url"].as_str().unwrap().to_string();
            Ok(text("ok"))
        }
        "browser_tabs" => Ok(text(&format!("### Result\n- 0: (current) [Shop]({})", page.lock().unwrap()))),
        "browser_type" => Err(format!("Timeout typing {} into the field", args["text"].as_str().unwrap())),
        "browser_take_screenshot" => Ok(crate::plugins::mcp::tests::png_content()),
        _ => Ok(text("ok")),
    });
    let (_, calls) = scripted(app, &owner, "Shop", answer).await;
    save_skill(app, &owner, "pay", json!({ "profile": "Shop", "steps": [
        { "action": "goto", "url": "https://shop.example.com/login" },
        { "action": "fill", "targets": ["#pin"], "value": "{{secret:SHOP_PIN}}", "timeout": 1 },
    ] }));
    let cancel = CancellationToken::new();
    let missing = match super::replay::run(app, &owner, &chat_id, "pay", &Default::default(), &cancel).await {
        Ok(result) => panic!("ran: {:?}", result.content),
        Err(error) => error,
    };
    assert!(missing.0.contains("SHOP_PIN, which isn't saved for your Browser"), "{}", missing.0);
    save_secret(app, &owner, "SHOP_PIN", "shop.example.com", "4242-secret-pin");
    let stopped = super::replay::run(app, &owner, &chat_id, "pay", &Default::default(), &cancel).await.unwrap();
    let typed: Vec<String> = calls.lock().unwrap().iter().filter(|(name, _)| name == "browser_type").map(|(_, args)| args["text"].as_str().unwrap().to_string()).collect();
    assert!(!typed.is_empty() && typed.iter().all(|text| text == "4242-secret-pin"), "the run types the saved value on its site: {typed:?}");
    let words = stopped.content[0].as_text().unwrap();
    assert!(stopped.is_error && !words.contains("4242-secret-pin") && words.contains("{{secret:SHOP_PIN}}"), "what the browser said is redacted: {words}");
    let messages = app.store.page(&chat_id, None, 20).unwrap().0;
    assert!(!messages.iter().any(|message| serde_json::to_string(&message.body).unwrap().contains("4242-secret-pin")), "nor does the notice show it");

    // Off its site, the value isn't typed at all.
    calls.lock().unwrap().clear();
    save_skill(app, &owner, "elsewhere", json!({ "profile": "Shop", "steps": [
        { "action": "goto", "url": "https://evil.example.net/login" },
        { "action": "fill", "targets": ["#pin"], "value": "{{secret:SHOP_PIN}}" },
    ] }));
    let refused = super::replay::run(app, &owner, &chat_id, "elsewhere", &Default::default(), &cancel).await.unwrap();
    assert!(refused.is_error && refused.content[0].as_text().unwrap().contains("is saved for shop.example.com"));
    assert!(!calls.lock().unwrap().iter().any(|(name, _)| name == "browser_type"));
}
