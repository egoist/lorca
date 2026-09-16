//! What a host can do at each point of a harness run, after pi's hook map. Hooks are
//! registered in order; each one sees what the ones before it changed.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use crate::compaction::CompactResult;
use crate::request::{RequestOptions, RequestOptionsPatch};
use crate::tool::ToolResult;
use crate::types::{AgentMessage, AssistantMessage, ContentPart, ToolCall};

use super::skills::Skill;
use super::templates::PromptTemplate;

/// Skills and prompt templates a harness can invoke.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resources {
    pub skills: Vec<Skill>,
    pub prompt_templates: Vec<PromptTemplate>,
}

/// Which request a `before_request` hook is shaping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestStep {
    Assistant,
    Compaction,
}

/// Why a compaction runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionReason {
    Manual,
    Threshold,
    Overflow,
}

/// What `before_tool` decided.
#[derive(Debug, Clone, Default)]
pub struct BeforeToolDecision {
    /// Arguments to run with instead.
    pub args: Option<Value>,
    /// Skip the call with this reason; `terminate` counts toward ending the run after the batch.
    pub block: Option<BlockedTool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedTool {
    pub reason: String,
    pub terminate: bool,
}

/// What `after_tool` overrides; unset fields keep the tool's result.
#[derive(Debug, Clone, Default)]
pub struct AfterToolDecision {
    pub content: Option<Vec<ContentPart>>,
    pub details: Option<Value>,
    pub is_error: Option<bool>,
    pub terminate: Option<bool>,
}

/// What `before_compaction` decided: leave it, skip it, or hand over a summary of its own.
#[derive(Debug, Clone)]
pub enum CompactionDecision {
    Decline,
    Replace(CompactResult),
}

/// Every method has a default that changes nothing.
#[async_trait]
pub trait HarnessHooks: Send + Sync {
    /// Before a run: messages to add after the prompt.
    async fn before_run(&self, _prompt: &[AgentMessage], _resources: &Resources) -> Option<Vec<AgentMessage>> {
        None
    }

    /// Before every model call: the messages and system prompt the model gets.
    async fn transform_context(&self, messages: Vec<AgentMessage>, system_prompt: String) -> (Vec<AgentMessage>, String) {
        (messages, system_prompt)
    }

    /// Before every request: a patch to its options.
    async fn before_request(&self, _step: RequestStep, _attempt: u32, _options: &RequestOptions) -> Option<RequestOptionsPatch> {
        None
    }

    /// The request body, right before it is sent. Change it in place.
    fn before_payload(&self, _payload: &mut Value) {}

    /// The response status and headers, once they are in.
    fn after_response(&self, _response: &crate::request::ResponseInfo) {}

    /// The assistant message a call ended with, before it is recorded. Return a replacement.
    async fn after_message(&self, _message: &AssistantMessage) -> Option<AssistantMessage> {
        None
    }

    /// Before a tool runs, with its checked arguments.
    async fn before_tool(&self, _call: &ToolCall, _args: &Value) -> Option<BeforeToolDecision> {
        None
    }

    /// After a tool ran, before its result is recorded.
    async fn after_tool(&self, _call: &ToolCall, _args: &Value, _result: &ToolResult, _is_error: bool) -> Option<AfterToolDecision> {
        None
    }

    /// Before a compaction, with the messages that would be summarized.
    async fn before_compaction(&self, _reason: CompactionReason, _messages: &[AgentMessage], _custom_instructions: Option<&str>) -> Option<CompactionDecision> {
        None
    }

    /// When a run would end: a follow-up prompt that starts another.
    async fn before_run_end(&self, _run_id: &str, _messages: &[AgentMessage]) -> Option<String> {
        None
    }
}

struct Registration {
    id: String,
    hooks: Arc<dyn HarnessHooks>,
}

/// The hooks a harness runs, in registration order.
#[derive(Default)]
pub struct HookRegistry {
    registrations: Mutex<Vec<Registration>>,
}

impl HookRegistry {
    /// Registers hooks under an id; registering the same id again replaces them.
    pub fn register(&self, id: &str, hooks: Arc<dyn HarnessHooks>) {
        let mut registrations = self.registrations.lock().unwrap();
        registrations.retain(|r| r.id != id);
        registrations.push(Registration { id: id.to_string(), hooks });
    }

    pub fn unregister(&self, id: &str) {
        self.registrations.lock().unwrap().retain(|r| r.id != id);
    }

    pub fn is_empty(&self) -> bool {
        self.registrations.lock().unwrap().is_empty()
    }

    fn all(&self) -> Vec<Arc<dyn HarnessHooks>> {
        self.registrations.lock().unwrap().iter().map(|r| r.hooks.clone()).collect()
    }

    pub async fn before_run(&self, prompt: &[AgentMessage], resources: &Resources) -> Vec<AgentMessage> {
        let mut injected = Vec::new();
        let mut seen: Vec<AgentMessage> = prompt.to_vec();
        for hooks in self.all() {
            if let Some(messages) = hooks.before_run(&seen, resources).await {
                seen.extend(messages.iter().cloned());
                injected.extend(messages);
            }
        }
        injected
    }

    pub async fn transform_context(&self, mut messages: Vec<AgentMessage>, mut system_prompt: String) -> (Vec<AgentMessage>, String) {
        for hooks in self.all() {
            (messages, system_prompt) = hooks.transform_context(messages, system_prompt).await;
        }
        (messages, system_prompt)
    }

    pub async fn before_request(&self, step: RequestStep, attempt: u32, options: &RequestOptions) -> RequestOptions {
        let mut options = options.clone();
        for hooks in self.all() {
            if let Some(patch) = hooks.before_request(step, attempt, &options).await {
                options.apply(patch);
            }
        }
        options
    }

    pub fn before_payload(&self, payload: &mut Value) {
        for hooks in self.all() {
            hooks.before_payload(payload);
        }
    }

    pub fn after_response(&self, response: &crate::request::ResponseInfo) {
        for hooks in self.all() {
            hooks.after_response(response);
        }
    }

    pub async fn after_message(&self, message: AssistantMessage) -> AssistantMessage {
        let mut message = message;
        for hooks in self.all() {
            if let Some(replacement) = hooks.after_message(&message).await {
                message = replacement;
            }
        }
        message
    }

    /// Arguments chain through the hooks; the first block ends it.
    pub async fn before_tool(&self, call: &ToolCall, args: &Value) -> BeforeToolDecision {
        let mut current = args.clone();
        let mut changed = false;
        for hooks in self.all() {
            if let Some(decision) = hooks.before_tool(call, &current).await {
                if let Some(args) = decision.args {
                    current = args;
                    changed = true;
                }
                if decision.block.is_some() {
                    return BeforeToolDecision { args: changed.then_some(current), block: decision.block };
                }
            }
        }
        BeforeToolDecision { args: changed.then_some(current), block: None }
    }

    pub async fn after_tool(&self, call: &ToolCall, args: &Value, result: &ToolResult, is_error: bool) -> Option<AfterToolDecision> {
        let mut merged: Option<AfterToolDecision> = None;
        let mut result = result.clone();
        let mut is_error = is_error;
        for hooks in self.all() {
            if let Some(decision) = hooks.after_tool(call, args, &result, is_error).await {
                if let Some(content) = &decision.content {
                    result.content = content.clone();
                }
                if let Some(details) = &decision.details {
                    result.details = details.clone();
                }
                if let Some(flag) = decision.is_error {
                    is_error = flag;
                }
                if let Some(terminate) = decision.terminate {
                    result.terminate = terminate;
                }
                let target = merged.get_or_insert_with(AfterToolDecision::default);
                if decision.content.is_some() {
                    target.content = decision.content;
                }
                if decision.details.is_some() {
                    target.details = decision.details;
                }
                if decision.is_error.is_some() {
                    target.is_error = decision.is_error;
                }
                if decision.terminate.is_some() {
                    target.terminate = decision.terminate;
                }
            }
        }
        merged
    }

    /// The first decision wins.
    pub async fn before_compaction(&self, reason: CompactionReason, messages: &[AgentMessage], custom_instructions: Option<&str>) -> Option<CompactionDecision> {
        for hooks in self.all() {
            if let Some(decision) = hooks.before_compaction(reason, messages, custom_instructions).await {
                return Some(decision);
            }
        }
        None
    }

    /// The last follow-up wins.
    pub async fn before_run_end(&self, run_id: &str, messages: &[AgentMessage]) -> Option<String> {
        let mut follow_up = None;
        for hooks in self.all() {
            if let Some(text) = hooks.before_run_end(run_id, messages).await {
                follow_up = Some(text);
            }
        }
        follow_up
    }
}

/// The registry as the adapters' request hooks, so `before_payload` and `after_response`
/// reach every model call.
pub struct RegistryRequestHooks(pub Arc<HookRegistry>);

#[async_trait]
impl crate::request::RequestHooks for RegistryRequestHooks {
    fn before_payload(&self, payload: &mut Value) {
        self.0.before_payload(payload);
    }

    fn after_response(&self, response: &crate::request::ResponseInfo) {
        self.0.after_response(response);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Rename;

    #[async_trait]
    impl HarnessHooks for Rename {
        async fn before_tool(&self, _call: &ToolCall, args: &Value) -> Option<BeforeToolDecision> {
            let mut args = args.clone();
            args["path"] = Value::String("renamed".into());
            Some(BeforeToolDecision { args: Some(args), block: None })
        }
    }

    struct Block;

    #[async_trait]
    impl HarnessHooks for Block {
        async fn before_tool(&self, _call: &ToolCall, args: &Value) -> Option<BeforeToolDecision> {
            assert_eq!(args["path"], "renamed", "sees the earlier hook's arguments");
            Some(BeforeToolDecision { args: None, block: Some(BlockedTool { reason: "no".into(), terminate: false }) })
        }
        async fn before_run_end(&self, _run_id: &str, _messages: &[AgentMessage]) -> Option<String> {
            Some("and then?".into())
        }
    }

    #[tokio::test]
    async fn hooks_chain_in_order_and_the_first_block_wins() {
        let registry = HookRegistry::default();
        registry.register("rename", Arc::new(Rename));
        registry.register("block", Arc::new(Block));
        let call = ToolCall { id: "c".into(), name: "read".into(), arguments: json!({ "path": "a" }) };
        let decision = registry.before_tool(&call, &call.arguments).await;
        assert_eq!(decision.args.unwrap()["path"], "renamed");
        assert_eq!(decision.block.unwrap().reason, "no");
        assert_eq!(registry.before_run_end("r", &[]).await.as_deref(), Some("and then?"));
        registry.unregister("block");
        assert!(registry.before_tool(&call, &call.arguments).await.block.is_none());
    }
}
