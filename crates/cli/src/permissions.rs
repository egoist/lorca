//! A bot's Access: the plugins it may use and how, and whether it reads or changes files and
//! runs shell commands. Checked before Auto-review and again when a call runs.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::model::Bot;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Read,
    Draft,
    Write,
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Read => "read",
            Self::Draft => "draft",
            Self::Write => "write",
        })
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemAccess {
    None,
    Read,
    #[default]
    Write,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionPermissions {
    /// Independent grants. An empty set denies the connection entirely.
    #[serde(default)]
    pub capabilities: BTreeSet<Capability>,
    /// Original MCP tool names, scoped to this connection instance. None permits all names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<BTreeSet<String>>,
}

/// None on a Bot means it has the Runner's every plugin, files, and shell. An explicit
/// `connections` map denies every plugin it does not list, a plugin installed later included.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BotPermissions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connections: Option<BTreeMap<String, ConnectionPermissions>>,
    #[serde(default)]
    pub filesystem: FilesystemAccess,
    #[serde(default = "enabled")]
    pub shell: bool,
    /// Whether an email or Slack message the bot writes in a chat waits as a draft card for the
    /// user to send ([`crate::drafts`]). Off, the bot calls the service's tool itself.
    #[serde(default = "enabled")]
    pub drafts: bool,
}

fn enabled() -> bool {
    true
}

impl Default for BotPermissions {
    fn default() -> Self {
        Self { connections: None, filesystem: FilesystemAccess::Write, shell: true, drafts: true }
    }
}

/// Whether `bot` drafts messages for the user to send: on unless its Access turns it off.
pub fn drafts_messages(bot: &Bot) -> bool {
    bot.permissions.as_ref().is_none_or(|permissions| permissions.drafts)
}

impl BotPermissions {
    pub fn validate(&self) -> Result<(), String> {
        let valid = |name: &str| !name.trim().is_empty() && name.len() <= 256 && name == name.trim();
        if let Some(connections) = &self.connections {
            if connections.len() > 1000 || connections.keys().any(|id| !valid(id)) {
                return Err("Connection allowlists need at most 1000 non-empty instance IDs.".into());
            }
            if connections.values().any(|grant| grant.tools.as_ref().is_some_and(|tools| tools.len() > 1000 || tools.iter().any(|name| !valid(name)))) {
                return Err("Connection tool allowlists need at most 1000 non-empty exact names.".into());
            }
        }
        Ok(())
    }

    /// Whether the bot may use the plugin at all: its catalog and system prompt leave out one
    /// it may not.
    pub fn allows_connection(&self, connection: &str) -> bool {
        self.connections.as_ref().is_none_or(|connections| connections.get(connection).is_some_and(|grant| !grant.capabilities.is_empty()))
    }

    fn local_denial(&self, tool: &str) -> Option<String> {
        if matches!(tool, "bash" | "bash_input" | "bash_output") && !self.shell {
            return Some("shell commands are off for this bot".into());
        }
        if matches!(tool, "read" | "grep" | "find" | "ls") && self.filesystem == FilesystemAccess::None {
            return Some("reading files is off for this bot".into());
        }
        if matches!(tool, "write" | "edit") && self.filesystem != FilesystemAccess::Write {
            return Some("changing files is off for this bot".into());
        }
        None
    }

    fn connection_denial(&self, connection: &str, name: &str, tool: &str, capability: Option<Capability>) -> Option<String> {
        let connections = self.connections.as_ref()?;
        let Some(grant) = connections.get(connection).filter(|grant| !grant.capabilities.is_empty()) else {
            return Some(format!("{name} is off for this bot"));
        };
        if grant.tools.as_ref().is_some_and(|tools| !tools.contains(tool)) {
            return Some(format!("{tool} is not among the {name} tools this bot may use"));
        }
        if let Some(capability) = capability.filter(|capability| !grant.capabilities.contains(capability)) {
            return Some(format!("{capability} access to {name} is off for this bot"));
        }
        None
    }
}

#[derive(Debug, Clone)]
pub struct AccessDenied {
    pub tool: String,
    pub connection_id: Option<String>,
    pub capability: Option<Capability>,
    pub reason: String,
    /// The bot's Access refused it, so the user can grant it. A bot deleted or moved to another
    /// Runner mid-turn is refused without asking anyone.
    pub grantable: bool,
}

impl fmt::Display for AccessDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.grantable {
            return write!(f, "{} cannot run: {}.", self.tool, self.reason);
        }
        write!(f, "Access refused for {}: {}. The user can change this bot's Access settings in its profile. Auto-review and Always allow cannot override this restriction.", self.tool, self.reason)
    }
}

/// The bot as it is stored now, or why it may do nothing: every call reads the current
/// profile, so a change takes effect during an active turn.
fn current(app: &Arc<App>, bot: &Bot, tool: &str, connection_id: Option<&str>) -> Result<Bot, AccessDenied> {
    let gone = |reason: &str| AccessDenied { tool: tool.into(), connection_id: connection_id.map(str::to_string), capability: None, reason: reason.into(), grantable: false };
    match app.bot(&bot.id) {
        Some(current) if current.runner_id != bot.runner_id => Err(gone("the bot was moved to another Runner")),
        Some(current) => Ok(current),
        None => Err(gone("the bot no longer exists")),
    }
}

pub fn check_tool(app: &Arc<App>, bot: &Bot, tool: &str) -> Result<(), AccessDenied> {
    let current = current(app, bot, tool, None)?;
    match current.permissions.as_ref().and_then(|policy| policy.local_denial(tool)) {
        Some(reason) => Err(AccessDenied { tool: tool.into(), connection_id: None, capability: None, reason, grantable: true }),
        None => Ok(()),
    }
}

/// A call that reads a Runner file through another tool, as `publish_output` with a `path` does,
/// needs what `read` needs.
pub fn check_file_read(app: &Arc<App>, bot: &Bot, tool: &str) -> Result<(), AccessDenied> {
    let current = current(app, bot, tool, None)?;
    match current.permissions.as_ref().and_then(|policy| policy.local_denial("read")) {
        Some(reason) => Err(AccessDenied { tool: tool.into(), connection_id: None, capability: None, reason, grantable: true }),
        None => Ok(()),
    }
}

/// The instance and tool selection with no capability, before a server starts to say what a
/// tool does; with one, the whole grant.
pub fn check_connection(app: &Arc<App>, bot: &Bot, connection: &str, tool: &str, capability: Option<Capability>) -> Result<(), AccessDenied> {
    let current = current(app, bot, tool, Some(connection))?;
    let name = connection_name(app, connection);
    match current.permissions.as_ref().and_then(|policy| policy.connection_denial(connection, &name, tool, capability)) {
        Some(reason) => Err(AccessDenied { tool: tool.into(), connection_id: Some(connection.into()), capability, reason, grantable: true }),
        None => Ok(()),
    }
}

/// The plugin as a whole, for a tool of Lorca's own that works with it (`browser_session` for
/// Browser): any grant to the plugin covers it, whichever of the plugin's tools the grant lists.
pub fn check_plugin(app: &Arc<App>, bot: &Bot, connection: &str, tool: &str) -> Result<(), AccessDenied> {
    let current = current(app, bot, tool, Some(connection))?;
    match current.permissions.as_ref().filter(|policy| !policy.allows_connection(connection)) {
        Some(_) => Err(AccessDenied { tool: tool.into(), connection_id: Some(connection.into()), capability: None, reason: format!("{} is off for this bot", connection_name(app, connection)), grantable: true }),
        None => Ok(()),
    }
}

/// What the user calls an installed plugin: its status name, which tells two accounts of one
/// service apart.
fn connection_name(app: &App, connection: &str) -> String {
    app.plugins.lock().unwrap().status(connection).map(|status| status.name).unwrap_or_else(|| connection.to_string())
}

/// A roster from a Device that predates policies cannot silently remove restrictions. Users
/// restore full access by writing an explicit default policy, never by dropping the field.
pub fn keep_policies(current: &[Bot], incoming: &mut [Bot]) -> bool {
    let mut kept = false;
    for bot in incoming {
        if bot.permissions.is_none() {
            if let Some(policy) = current.iter().find(|old| old.id == bot.id).and_then(|old| old.permissions.clone()) {
                bot.permissions = Some(policy);
                kept = true;
            }
        }
    }
    kept
}

/// Editing access answers the outstanding requests; it never resumes refused calls.
pub fn dismiss_requests(app: &Arc<App>, bot_id: &str) {
    use crate::model::{Author, Body};
    let chats: Vec<String> =
        app.state.lock().unwrap().chats.iter().filter(|chat| chat.meta.bot_ids.iter().any(|id| id == bot_id)).map(|chat| chat.meta.id.clone()).collect();
    for chat_id in chats {
        for mut message in app.store.page(&chat_id, None, 200).map(|(messages, _)| messages).unwrap_or_default() {
            if message.author != (Author::Bot { bot_id: bot_id.into() }) {
                continue;
            }
            if let Body::Permission { tool, decision, .. } = &mut message.body {
                if tool == "access" && decision == "pending" {
                    *decision = "dismissed".into();
                    app.upsert_message(message, true);
                }
            }
        }
    }
}

/// Refuses the call and, when the user could grant it, asks them in the chat: a card that
/// opens the bot's Access settings or is dismissed, and never allows the call itself.
#[cfg(feature = "runner")]
pub fn refuse(app: &Arc<App>, chat_id: &str, bot: &Bot, denied: AccessDenied) -> lorca_agent::BeforeToolCallResult {
    use crate::model::{Author, Body, Message};
    let reason = denied.to_string();
    if !denied.grantable {
        return crate::local_review::blocked(reason);
    }
    // One waiting request per missing grant; repeated calls never flood the user's chat.
    let arguments = serde_json::json!({ "connection_id": denied.connection_id, "requested_tool": denied.tool, "capability": denied.capability });
    let recent = app.store.page(chat_id, None, 200).map(|(messages, _)| messages).unwrap_or_default();
    let duplicate = recent.iter().any(|message| {
        message.author == (Author::Bot { bot_id: bot.id.clone() })
            && matches!(&message.body, Body::Permission { tool, decision, arguments: existing, .. } if tool == "access" && decision == "pending" && *existing == arguments)
    });
    if !duplicate {
        // The apps word the card: a plugin and its tool, or what the bot wanted to do here.
        let (plugin_id, plugin_name, summary) = match &denied.connection_id {
            Some(connection) => {
                let name = connection_name(app, connection);
                (connection.clone(), name.clone(), format!("{name} · {}", denied.tool))
            }
            None => {
                let what = match denied.tool.as_str() {
                    "bash" | "bash_input" | "bash_output" => "Shell commands",
                    "write" | "edit" => "Changing files",
                    _ => "Reading files",
                };
                ("computer".into(), String::new(), what.into())
            }
        };
        let message = Message::new(
            chat_id,
            Author::Bot { bot_id: bot.id.clone() },
            Body::Permission {
                plugin_id,
                plugin_name,
                tool: "access".into(),
                summary,
                arguments,
                decision: "pending".into(),
                reason: None,
                command: None,
                rule: None,
                code: None,
                link: None,
                secret: None,
            },
        );
        app.upsert_message(message, true);
    }
    crate::local_review::blocked(reason)
}

#[cfg(feature = "runner")]
pub(crate) mod guarded {
    use super::*;
    use async_trait::async_trait;
    use lorca_agent::{Tool, ToolError, ToolExecutionMode, ToolResult, ToolRunner, ToolUpdateFn};
    use serde_json::Value;
    use tokio_util::sync::CancellationToken;

    /// A local tool that checks the bot's Access again when it starts, after any review.
    struct GuardedTool {
        app: Arc<App>,
        bot: Bot,
        chat_id: String,
        tool: Arc<dyn Tool>,
    }

    pub fn tools(app: &Arc<App>, bot: &Bot, chat_id: &str, tools: Vec<Arc<dyn Tool>>) -> Vec<Arc<dyn Tool>> {
        tools.into_iter().map(|tool| Arc::new(GuardedTool { app: app.clone(), bot: bot.clone(), chat_id: chat_id.into(), tool }) as Arc<dyn Tool>).collect()
    }

    impl GuardedTool {
        fn check(&self) -> Result<(), ToolError> {
            check_tool(&self.app, &self.bot, self.name()).map_err(|denied| {
                let result = refuse(&self.app, &self.chat_id, &self.bot, denied);
                ToolError(result.reason.unwrap_or_default())
            })
        }
    }

    #[async_trait]
    impl Tool for GuardedTool {
        fn name(&self) -> &str {
            self.tool.name()
        }
        fn label(&self) -> &str {
            self.tool.label()
        }
        fn description(&self) -> &str {
            self.tool.description()
        }
        fn parameters(&self) -> Value {
            self.tool.parameters()
        }
        fn output_schema(&self) -> Option<Value> {
            self.tool.output_schema()
        }
        fn execution_mode(&self) -> Option<ToolExecutionMode> {
            self.tool.execution_mode()
        }
        fn prepare_arguments(&self, args: Value) -> Value {
            self.tool.prepare_arguments(args)
        }
        async fn execute(&self, id: &str, args: Value, cancel: CancellationToken, update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
            self.check()?;
            self.tool.execute(id, args, cancel, update).await
        }
        async fn execute_with(
            &self,
            id: &str,
            args: Value,
            cancel: CancellationToken,
            update: ToolUpdateFn,
            runner: &dyn ToolRunner,
        ) -> Result<ToolResult, ToolError> {
            self.check()?;
            self.tool.execute_with(id, args, cancel, update, runner).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Scratch(Arc<App>, std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }
    fn scratch() -> (Scratch, Bot, String) {
        let home = std::env::temp_dir().join(format!("lorca-permissions-{}", uuid::Uuid::new_v4()));
        let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        crate::identity::create(&app, Some("Policy Runner".into())).unwrap();
        let bot = app.state.lock().unwrap().bots[0].clone();
        let chat_id = app.dm_with(&bot.id, None).unwrap().meta.id;
        (Scratch(app, home), bot, chat_id)
    }

    fn inbox_policy() -> BotPermissions {
        serde_json::from_value(json!({
            "connections": {"gmail-work": {"capabilities": ["read", "draft"], "tools": ["list_messages", "create_draft"]}},
            "filesystem": "read", "shell": false
        }))
        .unwrap()
    }

    #[test]
    fn connection_instances_tools_and_capabilities_are_independent_grants() {
        let policy = inbox_policy();
        let denial = |connection: &str, tool: &str, capability| policy.connection_denial(connection, connection, tool, Some(capability));
        assert_eq!(denial("gmail-work", "list_messages", Capability::Read), None);
        assert_eq!(denial("gmail-work", "create_draft", Capability::Draft), None);
        assert!(denial("gmail-work", "create_draft", Capability::Write).is_some());
        assert!(denial("gmail-personal", "list_messages", Capability::Read).is_some());
        assert!(denial("gmail-work", "send_message", Capability::Draft).is_some());
        assert!(policy.allows_connection("gmail-work") && !policy.allows_connection("gmail-personal"));
        let mut only_write = inbox_policy();
        only_write.connections.as_mut().unwrap().get_mut("gmail-work").unwrap().capabilities = BTreeSet::from([Capability::Write]);
        assert!(only_write.connection_denial("gmail-work", "Work", "list_messages", Some(Capability::Read)).is_some(), "write does not imply read");
        only_write.connections.as_mut().unwrap().get_mut("gmail-work").unwrap().capabilities.clear();
        assert!(!only_write.allows_connection("gmail-work"), "no capability is no access");
        assert!(BotPermissions::default().allows_connection("anything"));
    }

    #[test]
    fn shell_and_filesystem_gates_apply_even_to_readonly_commands() {
        let policy = inbox_policy();
        for tool in ["bash", "bash_input", "bash_output", "write", "edit"] {
            assert!(policy.local_denial(tool).is_some(), "{tool}");
        }
        for tool in ["read", "grep", "find", "ls", "codemode", "memory_update", "message_bot", "attention", "project_context", "workflow_feedback"] {
            assert_eq!(policy.local_denial(tool), None, "{tool}");
        }
        let none = BotPermissions { filesystem: FilesystemAccess::None, ..Default::default() };
        assert!(none.local_denial("read").is_some());
        assert!(serde_json::from_value::<BotPermissions>(json!({"filesystem": "sandbox"})).is_err());
        assert!(serde_json::from_value::<BotPermissions>(json!({"connections": {"mail": {"capabilities": ["admin"]}}})).is_err());
    }

    #[test]
    fn a_running_turn_reads_revocation_and_a_moved_bot_is_refused_without_a_request() {
        let (scratch, snapshot, chat_id) = scratch();
        let app = &scratch.0;
        assert!(check_tool(app, &snapshot, "bash").is_ok());
        app.update_bot(&snapshot.id, |bot| bot.permissions = Some(inbox_policy())).unwrap();
        assert!(check_tool(app, &snapshot, "bash").unwrap_err().grantable);
        assert!(check_connection(app, &snapshot, "gmail-personal", "list_messages", Some(Capability::Read)).is_err());
        let reopened = App::load(crate::config::Config { home: scratch.1.clone(), port: 0 }).unwrap();
        assert_eq!(reopened.bot(&snapshot.id).unwrap().permissions, Some(inbox_policy()), "the persisted profile retains policy");
        app.state.lock().unwrap().bots.iter_mut().find(|bot| bot.id == snapshot.id).unwrap().runner_id = "another-runner".into();
        let moved = check_tool(app, &snapshot, "read").unwrap_err();
        assert!(!moved.grantable && moved.reason.contains("moved"));
        #[cfg(feature = "runner")]
        {
            assert!(refuse(app, &chat_id, &snapshot, moved).block);
            assert!(app.store.page(&chat_id, None, 200).unwrap().0.is_empty(), "nobody is asked about a moved bot");
        }
        let _ = chat_id;
    }

    #[test]
    fn an_old_roster_cannot_drop_an_explicit_policy() {
        let (_scratch, mut bot, _) = scratch();
        bot.permissions = Some(inbox_policy());
        let mut old = bot.clone();
        old.permissions = None;
        assert!(keep_policies(&[bot.clone()], std::slice::from_mut(&mut old)));
        assert_eq!(old.permissions, bot.permissions);
        old.permissions = Some(BotPermissions::default());
        assert!(!keep_policies(&[bot], std::slice::from_mut(&mut old)), "a user's explicit full policy stands");
    }

    #[tokio::test]
    async fn api_policy_edits_reject_null_and_invalid_fields_without_changing_access() {
        let (scratch, bot, _) = scratch();
        let app = &scratch.0;
        crate::api::dispatch(app, "bots.update", json!({"id": bot.id, "permissions": inbox_policy()})).await.unwrap();
        for invalid in [json!(null), json!({"connections": []}), json!({"shell": "false"}), json!({"connections": {"": {"capabilities": []}}})] {
            assert!(crate::api::dispatch(app, "bots.update", json!({"id": bot.id, "permissions": invalid})).await.is_err());
            assert_eq!(app.bot(&bot.id).unwrap().permissions, Some(inbox_policy()));
        }
        crate::api::dispatch(app, "bots.update", json!({"id": bot.id, "name": "Inbox"})).await.unwrap();
        assert_eq!(app.bot(&bot.id).unwrap().permissions, Some(inbox_policy()));
    }

    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn nested_file_writes_and_shell_calls_cannot_execute_after_revocation() {
        use lorca_agent::codemode::{CodemodeOptions, CodemodeTool};
        use lorca_agent::{DirectRunner, ToolUpdateFn};
        use tokio_util::sync::CancellationToken;
        let (scratch, bot, chat_id) = scratch();
        let app = &scratch.0;
        // Build the execution catalog before the user changes the policy, like an active turn.
        let tools = guarded::tools(app, &bot, &chat_id, lorca_agent::tools::coding_tools(scratch.1.clone()));
        let catalog = crate::plugins::mcp::bot_catalog(app, &bot, &chat_id, tools);
        app.update_bot(&bot.id, |bot| bot.permissions = Some(inbox_policy())).unwrap();
        let script =
            "await tools.write({path:'forbidden.txt',content:'write happened'}); await tools.bash({command:'touch forbidden-shell',description:'touch'});";
        let codemode = CodemodeTool::new(catalog, CodemodeOptions::default());
        let run = codemode.run_script("nested", script, CancellationToken::new(), &DirectRunner).await.unwrap();
        assert!(run.result.is_error, "{}", run.result.text_content());
        assert!(run.result.text_content().contains("changing files is off"));
        assert!(!scratch.1.join("forbidden.txt").exists());
        assert!(!scratch.1.join("forbidden-shell").exists());
        let tools = guarded::tools(app, &bot, &chat_id, lorca_agent::tools::coding_tools(scratch.1.clone()));
        let bash = tools.iter().find(|tool| tool.name() == "bash").unwrap();
        let update: ToolUpdateFn = Arc::new(|_| {});
        let refused =
            bash.execute("direct", json!({"command": "touch forbidden-shell", "description": "Touch"}), CancellationToken::new(), update).await.unwrap_err();
        assert!(refused.0.contains("shell commands are off"));
        assert!(!scratch.1.join("forbidden-shell").exists());
    }

    #[cfg(feature = "runner")]
    #[tokio::test]
    async fn access_requests_route_to_the_user_and_never_add_grants_or_always_allow() {
        use crate::model::{AutoReviewRule, Body};
        let (scratch, bot, chat_id) = scratch();
        let app = &scratch.0;
        app.update_bot(&bot.id, |bot| bot.permissions = Some(inbox_policy())).unwrap();
        app.add_auto_review_rule(AutoReviewRule {
            id: "always".into(),
            text: "allow everything".into(),
            behavior: "allow".into(),
            tool: Some("gmail-personal/send_message".into()),
        });
        let denied = check_connection(app, &bot, "gmail-personal", "send_message", Some(Capability::Write)).unwrap_err();
        assert!(refuse(app, &chat_id, &bot, denied.clone()).block);
        refuse(app, &chat_id, &bot, denied);
        let requests = app.store.page(&chat_id, None, 200).unwrap().0;
        assert_eq!(requests.len(), 1, "repeated missing grant makes one request");
        let card = &requests[0];
        assert!(matches!(&card.body, Body::Permission { tool, decision, summary, rule: None, reason: None, .. }
            if tool == "access" && decision == "pending" && summary == "gmail-personal · send_message"));
        for decision in ["allow", "always"] {
            let response = crate::api::dispatch(app, "chats.permission", json!({"chat_id": chat_id, "message_id": card.id, "decision": decision})).await;
            assert!(response.unwrap_err().contains("cannot grant permissions"));
        }
        assert_eq!(app.auto_review().rules.len(), 1);
        assert!(check_connection(app, &bot, "gmail-personal", "send_message", Some(Capability::Write)).is_err());
        crate::api::dispatch(app, "chats.permission", json!({"chat_id": chat_id, "message_id": card.id, "decision": "deny"})).await.unwrap();
        assert!(matches!(app.message(&chat_id, &card.id).unwrap().body, Body::Permission { decision, .. } if decision == "dismissed"));
        // A shell refusal is worded for the apps, and saving Access answers what is still open.
        refuse(app, &chat_id, &bot, check_tool(app, &bot, "bash").unwrap_err());
        let shell = app.store.page(&chat_id, None, 200).unwrap().0.into_iter().find(|message| message.id != card.id).unwrap();
        assert!(matches!(&shell.body, Body::Permission { plugin_id, summary, .. } if plugin_id == "computer" && summary == "Shell commands"));
        crate::api::dispatch(app, "bots.update", json!({"id": bot.id, "permissions": {"shell": true}})).await.unwrap();
        assert!(matches!(app.message(&chat_id, &shell.id).unwrap().body, Body::Permission { decision, .. } if decision == "dismissed"));
    }
}
