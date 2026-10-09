use super::*;

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
