//! Coding agents end to end, against stand-ins for Claude Code and Codex that speak their
//! protocols: a bot starts one, it asks to run a command, the user allows it on its card, it
//! finishes, Lorca publishes its work, and the bot hears it.

#![cfg(unix)]

use super::*;
use crate::model::{Bot, Chat, ChatMeta, MessageState};
use crate::plugins::mcp::Decision;
use lorca_agent::Tool;
use tokio_util::sync::CancellationToken;

/// Claude Code's stream-json, scripted: a job asks to run `git push` through the Bash hook, then
/// edits a file, saves proof, names a pull request, and is done; a "(follow-up)" is answered at
/// once. Every run appends its arguments to `args.log` beside it.
const FAKE_CLAUDE: &str = r#"#!/usr/bin/env python3
import json, os, sys
here = os.path.dirname(os.path.abspath(__file__))
with open(os.path.join(here, "args.log"), "a") as log:
    log.write(" ".join(sys.argv[1:]) + "\n")
def out(value):
    sys.stdout.write(json.dumps(value) + "\n"); sys.stdout.flush()
def read():
    line = sys.stdin.readline()
    return json.loads(line) if line else None
turn = 0
while True:
    message = read()
    if message is None:
        break
    if message.get("type") == "control_request":
        out({"type": "control_response", "response": {"subtype": "success", "request_id": message["request_id"], "response": {}}})
        continue
    if message.get("type") != "user":
        continue
    turn += 1
    out({"type": "system", "subtype": "init", "session_id": "sess-1"})
    out({"type": "user", "isReplay": True, "message": {"role": "user", "content": message["message"]["content"]}, "parent_tool_use_id": None})
    if "(follow-up)" not in str(message["message"]["content"]):
        out({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "git push origin feature"}}]}, "parent_tool_use_id": None})
        out({"type": "control_request", "request_id": "h1", "request": {"subtype": "hook_callback", "callback_id": "lorca-bash", "input": {"tool_name": "Bash", "tool_input": {"command": "git push origin feature"}}}})
        while True:
            answer = read()
            if answer is None:
                sys.exit(0)
            if answer.get("type") == "control_response" and answer["response"]["request_id"] == "h1":
                break
        decision = answer["response"]["response"]["hookSpecificOutput"]["permissionDecision"]
        out({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t1", "content": "pushed" if decision == "allow" else "denied", "is_error": decision != "allow"}]}, "parent_tool_use_id": None})
        with open("README.md", "a") as readme:
            readme.write("login fixed\n")
        with open(os.path.join(".lorca-proof", "tests.txt"), "w") as proof:
            proof.write("test result: ok. 3 passed\n")
        out({"type": "assistant", "message": {"content": [{"type": "text", "text": "Fixed it and opened https://github.com/acme/shop/pull/7"}]}, "parent_tool_use_id": None})
        out({"type": "result", "subtype": "success", "is_error": False, "result": "Fixed it and opened https://github.com/acme/shop/pull/7"})
    else:
        out({"type": "assistant", "message": {"content": [{"type": "text", "text": "Followed up."}]}, "parent_tool_use_id": None})
        out({"type": "result", "subtype": "success", "is_error": False, "result": "Followed up."})
"#;

/// Codex's app-server, scripted: a thread, and a turn per message that asks to run `git status`
/// and answers. A turn started while one runs is refused, as Codex does; steering adds to it.
const FAKE_CODEX: &str = r#"#!/usr/bin/env python3
import json, sys
def out(value):
    sys.stdout.write(json.dumps(value) + "\n"); sys.stdout.flush()
def read():
    line = sys.stdin.readline()
    return json.loads(line) if line else None
turns = 0
while True:
    message = read()
    if message is None:
        break
    method, ident = message.get("method"), message.get("id")
    if method == "initialize":
        out({"id": ident, "result": {"userAgent": "fake"}})
    elif method in ("thread/start", "thread/resume"):
        out({"id": ident, "result": {"thread": {"id": "thread-1"}}})
    elif method == "turn/steer":
        out({"id": ident, "result": {}})
    elif method == "turn/start":
        turns += 1
        turn = "turn-%d" % turns
        out({"id": ident, "result": {"turn": {"id": turn, "items": [], "status": "inProgress"}}})
        out({"method": "turn/started", "params": {"threadId": "thread-1", "turn": {"id": turn, "items": [], "status": "inProgress"}}})
        text = message["params"]["input"][0]["text"]
        out({"method": "item/completed", "params": {"item": {"type": "userMessage", "id": "u%d" % turns, "content": [{"type": "text", "text": text}]}}})
        out({"id": 900 + turns, "method": "item/commandExecution/requestApproval", "params": {"threadId": "thread-1", "turnId": turn, "itemId": "c1", "command": "/bin/zsh -lc 'git status'", "commandActions": [{"command": "git status"}]}})
        while True:
            answer = read()
            if answer is None:
                sys.exit(0)
            if answer.get("id") == 900 + turns:
                break
        decision = answer["result"]["decision"]
        out({"method": "item/completed", "params": {"item": {"type": "commandExecution", "id": "c1", "commandActions": [{"command": "git status"}], "aggregatedOutput": "clean\n", "exitCode": 0, "status": "completed" if decision == "accept" else "declined"}}})
        out({"method": "item/completed", "params": {"item": {"type": "agentMessage", "id": "m%d" % turns, "text": "Codex is done (%s)." % decision}}})
        out({"method": "turn/completed", "params": {"threadId": "thread-1", "turn": {"id": turn, "items": [], "status": "completed"}}})
"#;

struct Scratch {
    app: Arc<App>,
    home: PathBuf,
    bot: Bot,
    repo: PathBuf,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        for agent in self.app.coding_agents.all() {
            if let Some(driver) = agent.driver.lock().unwrap().take() {
                tokio::spawn(async move { driver.stop().await });
            }
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// The stand-ins, in a folder of this test run's own that outlives each test: the tests run at
/// once and share what `process::find` answers.
fn programs() -> &'static Path {
    static FOLDER: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    FOLDER.get_or_init(|| {
        use std::os::unix::fs::PermissionsExt;
        let folder = std::env::temp_dir().join(format!("lorca-coding-programs-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        for (name, script) in [("claude", FAKE_CLAUDE), ("codex", FAKE_CODEX)] {
            let path = folder.join(name);
            std::fs::write(&path, script).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            process::tests_support::install(name, path);
        }
        folder
    })
}

/// Chef in a DM on this Runner, a repository with one commit, and the stand-ins installed.
async fn setup() -> Scratch {
    let home = std::env::temp_dir().join(format!("lorca-coding-{}", uuid::Uuid::new_v4()));
    let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
    crate::identity::create(&app, Some("Workbench".into())).unwrap();
    let bot = Bot {
        id: "b1".into(),
        name: "Chef".into(),
        description: String::new(),
        symbol_name: String::new(),
        accent: String::new(),
        avatar: None,
        runner_id: app.this_device_id().unwrap(),
        provider: "deepseek".into(),
        model: None,
        thinking: None,
        legacy_instructions: String::new(),
        workdir: Some(home.join("work").to_string_lossy().to_string()),
        permissions: None,
        created_at: 0.0,
    };
    {
        let mut state = app.state.lock().unwrap();
        state.bots = vec![bot.clone()];
        state.chats.push(Chat {
            meta: ChatMeta { id: "chat".into(), kind: "dm".into(), title: None, bot_ids: vec!["b1".into()], owner_bot_id: None, description: None, is_pinned: false, created_at: 0.0 },
            unread_count: 0,
            usage: None,
            compactions: Vec::new(),
        });
    }
    let repo = home.join("work").join("shop");
    std::fs::create_dir_all(&repo).unwrap();
    workspace::git(&repo, &["init", "-q"]).await.unwrap();
    std::fs::write(repo.join("README.md"), "shop\n").unwrap();
    workspace::git(&repo, &["add", "."]).await.unwrap();
    workspace::git(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]).await.unwrap();
    programs();
    Scratch { app, home, bot, repo }
}

fn tool(scratch: &Scratch) -> CodingAgentTool {
    CodingAgentTool { app: scratch.app.clone(), bot: scratch.bot.clone(), chat_id: "chat".into(), workdir: scratch.home.join("work"), trigger_message_id: "msg-trigger".into() }
}

/// The row a `coding_agent` start call puts up as it begins, as the turn does.
fn start_row(app: &Arc<App>, call_id: &str) -> String {
    let mut message = Message::new(
        "chat",
        Author::Bot { bot_id: "b1".into() },
        Body::Tool {
            name: "coding_agent".into(),
            summary: "Running coding_agent…".into(),
            detail: String::new(),
            is_running: true,
            call_id: call_id.into(),
            arguments: Value::Null,
            result: None,
            is_error: false,
            description: None,
            target_bot_id: None,
            script_command: None,
            run: None,
            agent: Some(AgentRun { state: "starting".into(), ..AgentRun::default() }),
        },
    );
    message.state = MessageState::Streaming;
    app.upsert_message(message.clone(), false);
    app.coding_agents.begin("chat", call_id, &message.id);
    message.id
}

fn card(app: &App, message_id: &str) -> AgentRun {
    match app.message("chat", message_id).unwrap().body {
        Body::Tool { agent: Some(card), .. } => card,
        _ => panic!("no card"),
    }
}

async fn card_when(app: &App, message_id: &str, done: impl Fn(&AgentRun) -> bool) -> AgentRun {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let card = card(app, message_id);
            if done(&card) {
                return card;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the card never got there: {:?}", card(app, message_id)))
}

async fn call(tool: &CodingAgentTool, call_id: &str, args: Value) -> Result<String, String> {
    tool.execute(call_id, args, CancellationToken::new(), Arc::new(|_| {})).await.map(|result| result.text_content()).map_err(|error| error.0)
}

#[tokio::test]
async fn claude_code_works_in_a_worktree_asks_on_its_card_and_its_bot_hears_how_it_went() {
    let scratch = setup().await;
    let app = &scratch.app;
    let mut auto_review = app.auto_review();
    auto_review.is_enabled = false;
    app.set_auto_review(auto_review);
    let tool = tool(&scratch);
    let row = start_row(app, "call-1");
    // Held, so the turn that tells the bot waits, and the test reads what it would.
    let chat_lock = app.chat_lock("chat");
    let held = chat_lock.lock().await;

    let started = call(&tool, "call-1", json!({ "action": "start", "agent": "claude", "folder": "shop", "worktree": "fix-login", "prompt": "Fix the login bug\nwith a test", "proof": "the test output" })).await.unwrap();
    assert!(started.starts_with("Started Claude Code (agent-") && started.contains("branch fix-login"), "{started}");
    let agent = app.coding_agents.of("chat", "b1").pop().unwrap();
    let record = agent.record();
    assert_eq!((record.kind.as_str(), record.branch.as_deref(), record.task.as_str()), ("claude", Some("fix-login"), "Fix the login bug"));
    assert!(record.folder.starts_with(scratch.home.join("worktrees")));

    // Its push asks the user on its card, Auto-review being off; Allow lets it go on.
    let asking = card_when(app, &row, |card| card.state == "asking").await;
    let question = asking.question.unwrap();
    assert_eq!((question.kind.as_str(), question.command.as_deref()), ("command", Some("git push origin feature")));
    assert!(app.message("chat", &row).unwrap().confirmation().unwrap().starts_with("Claude Code: $ git push"));
    assert!(crate::plugins::mcp::answer(app, &row, Decision::Allowed));
    assert!(std::fs::read_to_string(programs().join("args.log")).unwrap().contains("--permission-prompt-tool stdio"));

    let done = card_when(app, &row, |card| card.state == "idle").await;
    assert_eq!(done.pull_request.as_deref(), Some("https://github.com/acme/shop/pull/7"));
    assert!(done.output.as_deref().unwrap().contains("● Bash(git push origin feature)"), "{:?}", done.output);
    assert_eq!(done.branch.as_deref(), Some("fix-login"));

    // Its diff, its pull request, and its proof are outputs in the chat, published as it is done.
    let outputs = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let outputs = crate::outputs::list(app, "chat", None).unwrap();
            if outputs.len() >= 3 {
                return outputs;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("its outputs were published");
    let names: Vec<String> = outputs.iter().filter_map(|message| message.output.as_ref().map(|output| output.name.clone())).collect();
    assert_eq!(names, vec!["fix-login.diff", "Pull request #7", "tests.txt"]);
    let tests = outputs.iter().find_map(|message| message.output.clone().filter(|output| output.name == "tests.txt")).unwrap();
    assert_eq!(tests.evidence.unwrap().kind, crate::outputs::EvidenceKind::TestResult);

    // The bot hears it with their references, once.
    let job = crate::runtime::agent_job(app, "chat", "b1", &row);
    let cue = wake_cue(app, &job).unwrap();
    assert!(cue.contains("is done") && cue.contains("Fixed it and opened") && cue.contains("fix-login.diff (message msg-"), "{cue}");
    assert!(wake_cue(app, &job).is_none(), "heard once");
    drop(held);

    // A follow-up while it waits.
    let id = agent.id();
    let sent = call(&tool, "call-2", json!({ "action": "send", "id": id, "message": "Also update the docs (follow-up)" })).await.unwrap();
    assert!(sent.starts_with("Sent."), "{sent}");
    card_when(app, &row, |card| card.output.as_deref().is_some_and(|output| output.contains("Followed up."))).await;
    let read = call(&tool, "call-3", json!({ "action": "read", "id": id, "lines": 3 })).await.unwrap();
    assert!(read.contains("Followed up."), "{read}");

    // Stop in the chat stops it; its card says so and the bot hears nothing more.
    app.coding_agents.stop_chat(app, "chat");
    let stopped = card_when(app, &row, |card| card.state == "stopped").await;
    assert_eq!(stopped.outcome.as_deref(), Some("Stopped"));
    assert!(agent.record().news.is_none());
}

#[tokio::test]
async fn a_restart_keeps_the_handle_and_a_follow_up_resumes_the_session() {
    let scratch = setup().await;
    let app = &scratch.app;
    let mut auto_review = app.auto_review();
    auto_review.is_enabled = false;
    app.set_auto_review(auto_review);
    let tool = tool(&scratch);
    let row = start_row(app, "call-1");
    call(&tool, "call-1", json!({ "action": "start", "folder": "shop", "prompt": "Add dark mode" })).await.unwrap();
    card_when(app, &row, |card| card.state == "asking").await;
    crate::plugins::mcp::answer(app, &row, Decision::Denied);
    card_when(app, &row, |card| card.state == "idle").await;
    let id = app.coding_agents.of("chat", "b1")[0].id();
    let agent = app.coding_agents.of("chat", "b1").pop().unwrap();
    assert_eq!(agent.record().session.as_deref(), Some("sess-1"));
    let denied = agent.read(Some(50)).0;
    assert!(denied.contains("Error: denied"), "{denied}");

    // Lorca quits and starts again: the agent is done, so it waits idle with no process.
    app.coding_agents.shutdown(app).await;
    app.coding_agents.agents.lock().unwrap().clear();
    load(app);
    let agent = app.coding_agents.find("chat", "b1", &id).unwrap();
    assert!(!agent.is_running() && agent.record().state == "idle");
    let sent = call(&tool, "call-2", json!({ "action": "send", "id": id, "message": "Now the settings page (follow-up)" })).await.unwrap();
    assert!(sent.contains("started again on its session"), "{sent}");
    card_when(app, &row, |card| card.output.as_deref().is_some_and(|output| output.contains("Followed up."))).await;
    let args = std::fs::read_to_string(programs().join("args.log")).unwrap();
    assert!(args.lines().any(|line| line.contains("--resume sess-1")), "{args}");

    // A Lorca that quits while it works leaves it stopped, as its card says.
    let working = app.coding_agents.find("chat", "b1", &id).unwrap();
    working.record.lock().unwrap().state = "working".into();
    save(app);
    app.coding_agents.agents.lock().unwrap().clear();
    load(app);
    assert_eq!(card(app, &row).outcome.as_deref(), Some("Stopped when Lorca quit"));
}

#[tokio::test]
async fn codex_runs_read_only_commands_at_once_and_takes_a_message_while_it_works() {
    let scratch = setup().await;
    let app = &scratch.app;
    let tool = tool(&scratch);
    let row = start_row(app, "call-1");
    let started = call(&tool, "call-1", json!({ "action": "start", "agent": "codex", "folder": scratch.repo.to_str().unwrap(), "prompt": "Tidy the readme" })).await.unwrap();
    assert!(started.starts_with("Started Codex"), "{started}");
    // `git status` only reads: with Auto-review on, it runs without a review or a question.
    let done = card_when(app, &row, |card| card.state == "idle").await;
    assert!(done.output.as_deref().unwrap().contains("Codex is done (accept)."), "{:?}", done.output);
    let agent = app.coding_agents.of("chat", "b1").pop().unwrap();
    assert_eq!(agent.record().session.as_deref(), Some("thread-1"));
    let list = call(&tool, "call-2", json!({ "action": "list" })).await.unwrap();
    assert!(list.contains(&agent.id()) && list.contains("Codex: done, waiting for a follow-up"), "{list}");
    call(&tool, "call-3", json!({ "action": "stop", "id": agent.id() })).await.unwrap();
    assert_eq!(card(app, &row).state, "stopped");
    assert!(call(&tool, "call-4", json!({ "action": "read", "id": "agent-nope" })).await.unwrap_err().contains("no coding agent"));
}

#[tokio::test]
async fn a_bot_without_shell_access_lets_its_agent_run_nothing() {
    let scratch = setup().await;
    let app = &scratch.app;
    let policy = serde_json::from_value(json!({ "shell": false })).unwrap();
    app.update_bot("b1", |bot| bot.permissions = Some(policy)).unwrap();
    let agent = Agent::new(Record {
        id: "agent-1".into(),
        chat_id: "chat".into(),
        bot_id: "b1".into(),
        message_id: "msg-none".into(),
        trigger_message_id: String::new(),
        kind: "claude".into(),
        host: None,
        target: None,
        session: None,
        folder: scratch.repo.clone(),
        branch: None,
        base: None,
        task: "x".into(),
        state: "working".into(),
        stalled: false,
        outcome: None,
        pull_request: None,
        started_at: 0.0,
        ended_at: None,
        news: None,
        published: proof::Published::default(),
    });
    let refused = approve(app, &agent, Approval::Command { command: "ls".into() }).await.unwrap_err();
    assert!(refused.contains("Shell commands are off"), "{refused}");
}

/// A pane that records what it is given, for an agent in a terminal host.
struct FakePane(Mutex<Vec<Answer>>);

#[async_trait::async_trait]
impl Driver for FakePane {
    async fn send(&self, _text: &str, _interrupt: bool) -> Result<(), String> {
        Ok(())
    }
    async fn answer(&self, answer: &Answer) -> Result<(), String> {
        self.0.lock().unwrap().push(answer.clone());
        Ok(())
    }
    async fn stop(&self) {}
}

#[tokio::test]
async fn what_a_pane_asks_goes_on_the_card_and_only_the_users_answer_is_pressed() {
    let scratch = setup().await;
    let app = &scratch.app;
    let mut auto_review = app.auto_review();
    auto_review.is_enabled = false;
    app.set_auto_review(auto_review);
    let row = start_row(app, "call-1");
    let agent = Agent::new(Record {
        id: "agent-pane".into(),
        chat_id: "chat".into(),
        bot_id: "b1".into(),
        message_id: row.clone(),
        trigger_message_id: String::new(),
        kind: "claude".into(),
        host: Some("herdr".into()),
        target: None,
        session: None,
        folder: scratch.repo.clone(),
        branch: None,
        base: None,
        task: "Fix it".into(),
        state: "working".into(),
        stalled: false,
        outcome: None,
        pull_request: None,
        started_at: 0.0,
        ended_at: None,
        news: None,
        published: proof::Published::default(),
    });
    let pane = Arc::new(FakePane(Mutex::new(Vec::new())));
    *agent.driver.lock().unwrap() = Some(pane.clone());
    app.coding_agents.agents.lock().unwrap().push(agent.clone());
    let screen = " Bash command\n   git push origin fix\n Do you want to proceed?\n ❯ 1. Yes\n   2. Yes, and don't ask again for git push commands\n   3. No, and tell Claude what to do differently (esc)\n";
    handle(app, &agent, Event::Blocked(screen.into()), 0).await;
    let asking = card_when(app, &row, |card| card.state == "asking").await;
    let question = asking.question.unwrap();
    assert_eq!(question.kind, "choices");
    assert_eq!(question.choices[0], "Yes");
    assert!(question.text.contains("git push origin fix"));
    serve(app, "coding.answer", &json!({ "chat_id": "chat", "message_id": row, "choice": 2 })).await.unwrap();
    card_when(app, &row, |card| card.state == "working").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(*pane.0.lock().unwrap(), vec![Answer::Keys(vec!["down".into(), "down".into(), "enter".into()])]);

    // One the user answers in the pane itself goes from the card.
    handle(app, &agent, Event::Blocked("Which port?\n> ".into()), 0).await;
    let asking = card_when(app, &row, |card| card.state == "asking").await;
    assert_eq!(asking.question.unwrap().kind, "text");
    handle(app, &agent, Event::Working, 0).await;
    card_when(app, &row, |card| card.state == "working").await;
    assert!(serve(app, "coding.answer", &json!({ "chat_id": "chat", "message_id": row, "text": "3000" })).await.is_err());
}
