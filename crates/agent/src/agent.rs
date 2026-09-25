//! Stateful wrapper around the loop, after pi-agent-core's `Agent` class.
//!
//! Owns the transcript, executes runs, and exposes steering and follow-up queues through an
//! [`AgentHandle`] that other tasks can hold while a run is in flight.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::agent_loop::{
    run_agent_loop, run_agent_loop_continue, AfterToolCallContext, AfterToolCallResult, AgentContext,
    AgentLoopConfig, BeforeToolCallContext, BeforeToolCallResult, LoopHooks, NoHooks, PrepareNextTurnContext,
    ShouldStopAfterTurnContext, ToolExecutionMode, TurnUpdate,
};
use crate::provider::Provider;
use crate::tool::Tool;
use crate::types::{AgentEvent, AgentMessage, AssistantMessage, ContentPart, LlmMessage, StopReason, UserMessage};

/// How a queue drains: one message per poll, or everything queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueMode {
    OneAtATime,
    All,
}

struct PendingQueue {
    mode: QueueMode,
    messages: VecDeque<AgentMessage>,
}

impl PendingQueue {
    fn drain(&mut self) -> Vec<AgentMessage> {
        match self.mode {
            QueueMode::All => self.messages.drain(..).collect(),
            QueueMode::OneAtATime => self.messages.pop_front().into_iter().collect(),
        }
    }
}

/// A thread-safe queue the low-level agent loop's host can use for steering or follow-ups.
/// [`Agent`] uses the same type internally; hosts that call `run_agent_loop*` directly can
/// retain a clone and drain it from their [`LoopHooks`] implementation.
#[derive(Clone)]
pub struct AgentMessageQueue {
    inner: Arc<Mutex<PendingQueue>>,
}

impl AgentMessageQueue {
    pub fn new(mode: QueueMode) -> Self {
        AgentMessageQueue {
            inner: Arc::new(Mutex::new(PendingQueue { mode, messages: VecDeque::new() })),
        }
    }

    pub fn push(&self, message: AgentMessage) {
        self.inner.lock().unwrap().messages.push_back(message);
    }

    pub fn drain(&self) -> Vec<AgentMessage> {
        self.inner.lock().unwrap().drain()
    }

    pub fn clear(&self) {
        self.inner.lock().unwrap().messages.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().messages.is_empty()
    }

    pub fn set_mode(&self, mode: QueueMode) {
        self.inner.lock().unwrap().mode = mode;
    }
}

struct Queues {
    steering: AgentMessageQueue,
    follow_up: AgentMessageQueue,
}

/// Cheap handle for steering, follow-ups, and aborting from other tasks.
#[derive(Clone)]
pub struct AgentHandle {
    queues: Arc<Queues>,
    active: Arc<Mutex<Option<CancellationToken>>>,
}

impl AgentHandle {
    /// Queue a message to inject after the current assistant turn finishes its tool calls.
    pub fn steer(&self, message: AgentMessage) {
        self.queues.steering.push(message);
    }

    /// Queue a message to run only after the agent would otherwise stop.
    pub fn follow_up(&self, message: AgentMessage) {
        self.queues.follow_up.push(message);
    }

    pub fn clear_steering_queue(&self) {
        self.queues.steering.clear();
    }

    pub fn clear_follow_up_queue(&self) {
        self.queues.follow_up.clear();
    }

    pub fn clear_all_queues(&self) {
        self.clear_steering_queue();
        self.clear_follow_up_queue();
    }

    pub fn has_queued_messages(&self) -> bool {
        !self.queues.steering.is_empty() || !self.queues.follow_up.is_empty()
    }

    /// Abort the current run, if one is active.
    pub fn abort(&self) {
        if let Some(token) = self.active.lock().unwrap().as_ref() {
            token.cancel();
        }
    }

    pub fn is_running(&self) -> bool {
        self.active.lock().unwrap().is_some()
    }
}

/// Bridges the agent's queues into the loop's hook interface and forwards everything else to
/// the caller's hooks.
struct AgentHooks {
    queues: Arc<Queues>,
    inner: Arc<dyn LoopHooks>,
    skip_initial_steering_poll: AtomicBool,
}

#[async_trait]
impl LoopHooks for AgentHooks {
    async fn transform_context(&self, messages: Vec<AgentMessage>, cancel: &CancellationToken) -> Vec<AgentMessage> {
        self.inner.transform_context(messages, cancel).await
    }

    fn convert_to_llm(&self, messages: &[AgentMessage]) -> Vec<LlmMessage> {
        self.inner.convert_to_llm(messages)
    }

    async fn steering_messages(&self) -> Vec<AgentMessage> {
        if self.skip_initial_steering_poll.swap(false, Ordering::SeqCst) {
            return Vec::new();
        }
        self.queues.steering.drain()
    }

    async fn follow_up_messages(&self) -> Vec<AgentMessage> {
        self.queues.follow_up.drain()
    }

    async fn before_tool_call(&self, ctx: BeforeToolCallContext<'_>) -> Option<BeforeToolCallResult> {
        self.inner.before_tool_call(ctx).await
    }

    async fn after_tool_call(&self, ctx: AfterToolCallContext<'_>) -> Option<AfterToolCallResult> {
        self.inner.after_tool_call(ctx).await
    }

    async fn should_stop_after_turn(&self, ctx: ShouldStopAfterTurnContext<'_>) -> bool {
        self.inner.should_stop_after_turn(ctx).await
    }

    async fn prepare_next_turn(&self, ctx: PrepareNextTurnContext<'_>) -> Option<TurnUpdate> {
        self.inner.prepare_next_turn(ctx).await
    }
}

pub struct AgentOptions {
    pub system_prompt: String,
    pub provider: Arc<dyn Provider>,
    pub tools: Vec<Arc<dyn Tool>>,
    pub messages: Vec<AgentMessage>,
    pub hooks: Arc<dyn LoopHooks>,
    pub steering_mode: QueueMode,
    pub follow_up_mode: QueueMode,
    pub tool_execution: ToolExecutionMode,
    /// Retries of a model call that fails before it streams anything.
    pub retry: Option<crate::retry::RetryPolicy>,
    /// Headers, timeout, session affinity, metadata, and hooks for every model call.
    pub request: crate::request::RequestOptions,
}

impl AgentOptions {
    pub fn new(provider: Arc<dyn Provider>) -> Self {
        AgentOptions {
            system_prompt: String::new(),
            provider,
            tools: Vec::new(),
            messages: Vec::new(),
            hooks: Arc::new(NoHooks),
            steering_mode: QueueMode::OneAtATime,
            follow_up_mode: QueueMode::OneAtATime,
            tool_execution: ToolExecutionMode::Parallel,
            retry: None,
            request: Default::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("Agent is already processing a prompt. Use steer() or follow_up() to queue messages, or wait for completion.")]
    Busy,
    #[error("No messages to continue from")]
    NoMessages,
    #[error("Cannot continue from message role: assistant")]
    LastIsAssistant,
}

/// Stateful agent. Holds the transcript between runs; a run streams [`AgentEvent`]s to the
/// sender you pass in and updates the transcript as messages complete.
pub struct Agent {
    pub system_prompt: String,
    pub provider: Arc<dyn Provider>,
    pub tools: Vec<Arc<dyn Tool>>,
    pub messages: Vec<AgentMessage>,
    pub tool_execution: ToolExecutionMode,
    pub retry: Option<crate::retry::RetryPolicy>,
    pub request: crate::request::RequestOptions,
    hooks: Arc<dyn LoopHooks>,
    queues: Arc<Queues>,
    active: Arc<Mutex<Option<CancellationToken>>>,
    streaming_message: Option<AgentMessage>,
    pending_tool_calls: HashSet<String>,
    error_message: Option<String>,
}

impl Agent {
    pub fn new(options: AgentOptions) -> Self {
        Agent {
            system_prompt: options.system_prompt,
            provider: options.provider,
            tools: options.tools,
            messages: options.messages,
            tool_execution: options.tool_execution,
            retry: options.retry,
            request: options.request,
            hooks: options.hooks,
            queues: Arc::new(Queues {
                steering: AgentMessageQueue::new(options.steering_mode),
                follow_up: AgentMessageQueue::new(options.follow_up_mode),
            }),
            active: Arc::new(Mutex::new(None)),
            streaming_message: None,
            pending_tool_calls: HashSet::new(),
            error_message: None,
        }
    }

    pub fn handle(&self) -> AgentHandle {
        AgentHandle { queues: self.queues.clone(), active: self.active.clone() }
    }

    pub fn is_streaming(&self) -> bool {
        self.active.lock().unwrap().is_some()
    }

    pub fn streaming_message(&self) -> Option<&AgentMessage> {
        self.streaming_message.as_ref()
    }

    pub fn pending_tool_calls(&self) -> &HashSet<String> {
        &self.pending_tool_calls
    }

    pub fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    pub fn set_steering_mode(&self, mode: QueueMode) {
        self.queues.steering.set_mode(mode);
    }

    pub fn set_follow_up_mode(&self, mode: QueueMode) {
        self.queues.follow_up.set_mode(mode);
    }

    /// Clear transcript, runtime state, and queued messages.
    pub fn reset(&mut self) {
        self.messages.clear();
        self.streaming_message = None;
        self.pending_tool_calls.clear();
        self.error_message = None;
        self.handle().clear_all_queues();
    }

    /// Run a prompt. Resolves when the run has settled; events arrive on `events` as they happen.
    pub async fn prompt(
        &mut self,
        input: impl Into<PromptInput>,
        events: mpsc::Sender<AgentEvent>,
    ) -> Result<Vec<AgentMessage>, AgentError> {
        if self.is_streaming() {
            return Err(AgentError::Busy);
        }
        let prompts = input.into().into_messages();
        self.run(Some(prompts), false, events).await
    }

    /// Continue from the current transcript. The last message must be a user or tool-result
    /// message, or a queued steering/follow-up message is used.
    pub async fn continue_run(&mut self, events: mpsc::Sender<AgentEvent>) -> Result<Vec<AgentMessage>, AgentError> {
        if self.is_streaming() {
            return Err(AgentError::Busy);
        }
        let Some(last) = self.messages.last() else { return Err(AgentError::NoMessages) };
        if last.is_assistant() {
            let steering = self.queues.steering.drain();
            if !steering.is_empty() {
                return self.run(Some(steering), true, events).await;
            }
            let follow_ups = self.queues.follow_up.drain();
            if !follow_ups.is_empty() {
                return self.run(Some(follow_ups), false, events).await;
            }
            return Err(AgentError::LastIsAssistant);
        }
        self.run(None, false, events).await
    }

    async fn run(
        &mut self,
        prompts: Option<Vec<AgentMessage>>,
        skip_initial_steering_poll: bool,
        events: mpsc::Sender<AgentEvent>,
    ) -> Result<Vec<AgentMessage>, AgentError> {
        let cancel = CancellationToken::new();
        *self.active.lock().unwrap() = Some(cancel.clone());
        self.streaming_message = None;
        self.error_message = None;

        let context = AgentContext {
            system_prompt: self.system_prompt.clone(),
            messages: self.messages.clone(),
            tools: self.tools.clone(),
            cache_points: Vec::new(),
        };
        let config = AgentLoopConfig {
            provider: self.provider.clone(),
            hooks: Arc::new(AgentHooks {
                queues: self.queues.clone(),
                inner: self.hooks.clone(),
                skip_initial_steering_poll: AtomicBool::new(skip_initial_steering_poll),
            }),
            tool_execution: self.tool_execution,
            sink: None,
            retry: self.retry.clone(),
            request: self.request.clone(),
        };

        let (tx, mut rx) = mpsc::channel::<AgentEvent>(256);
        let loop_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            match prompts {
                Some(prompts) => Ok(run_agent_loop(prompts, context, &config, &tx, loop_cancel).await),
                None => run_agent_loop_continue(context, &config, &tx, loop_cancel).await,
            }
        });

        // Reduce state as events arrive, then forward them.
        while let Some(event) = rx.recv().await {
            self.process_event(&event);
            let _ = events.send(event).await;
        }

        let outcome = match task.await {
            Ok(Ok(messages)) => Ok(messages),
            Ok(Err(error)) => {
                let message = error.to_string();
                self.handle_run_failure(message, cancel.is_cancelled(), &events).await;
                Err(match error {
                    crate::agent_loop::LoopError::Empty => AgentError::NoMessages,
                    crate::agent_loop::LoopError::LastIsAssistant => AgentError::LastIsAssistant,
                })
            }
            Err(join_error) => {
                self.handle_run_failure(join_error.to_string(), cancel.is_cancelled(), &events).await;
                Ok(Vec::new())
            }
        };

        self.streaming_message = None;
        self.pending_tool_calls.clear();
        *self.active.lock().unwrap() = None;
        outcome
    }

    /// A run that failed outside the loop's own event sequence still ends like one: the
    /// failure is an assistant message with its own start, end, turn end, and agent end.
    async fn handle_run_failure(&mut self, message: String, aborted: bool, events: &mpsc::Sender<AgentEvent>) {
        let mut failure = AssistantMessage::empty(self.provider.provider_id(), self.provider.model_id());
        failure.stop_reason = if aborted { StopReason::Aborted } else { StopReason::Error };
        failure.error_message = Some(message);
        let wrapped = AgentMessage::Assistant(failure);
        for event in [
            AgentEvent::MessageStart { message: wrapped.clone() },
            AgentEvent::MessageEnd { message: wrapped.clone() },
            AgentEvent::TurnEnd { message: wrapped.clone(), tool_results: vec![] },
            AgentEvent::AgentEnd { messages: vec![wrapped] },
        ] {
            self.process_event(&event);
            let _ = events.send(event).await;
        }
    }

    fn process_event(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::MessageStart { message } | AgentEvent::MessageUpdate { message, .. } => {
                self.streaming_message = Some(message.clone());
            }
            AgentEvent::MessageEnd { message } => {
                self.streaming_message = None;
                self.messages.push(message.clone());
            }
            AgentEvent::ToolExecutionStart { tool_call_id, .. } => {
                self.pending_tool_calls.insert(tool_call_id.clone());
            }
            AgentEvent::ToolExecutionEnd { tool_call_id, .. } => {
                self.pending_tool_calls.remove(tool_call_id);
            }
            AgentEvent::TurnEnd { message, .. } => {
                if let AgentMessage::Assistant(assistant) = message {
                    if let Some(error) = &assistant.error_message {
                        self.error_message = Some(error.clone());
                    }
                }
            }
            AgentEvent::AgentEnd { .. } => {
                self.streaming_message = None;
            }
            _ => {}
        }
    }
}

/// What `prompt` accepts: a string, text with images, one message, or several.
pub enum PromptInput {
    Text(String),
    /// A user message of text and image parts.
    Content(Vec<ContentPart>),
    Message(AgentMessage),
    Messages(Vec<AgentMessage>),
}

impl PromptInput {
    /// Text followed by images, as one user message.
    pub fn with_images(text: impl Into<String>, images: Vec<ContentPart>) -> Self {
        let mut content = vec![ContentPart::text(text)];
        content.extend(images);
        PromptInput::Content(content)
    }

    pub fn into_messages(self) -> Vec<AgentMessage> {
        match self {
            PromptInput::Text(text) => vec![AgentMessage::User(UserMessage::text(text))],
            PromptInput::Content(content) => vec![AgentMessage::User(UserMessage { content, timestamp: crate::now_ms() })],
            PromptInput::Message(message) => vec![message],
            PromptInput::Messages(messages) => messages,
        }
    }
}

impl From<Vec<ContentPart>> for PromptInput {
    fn from(value: Vec<ContentPart>) -> Self {
        PromptInput::Content(value)
    }
}

impl From<&str> for PromptInput {
    fn from(value: &str) -> Self {
        PromptInput::Text(value.to_string())
    }
}

impl From<String> for PromptInput {
    fn from(value: String) -> Self {
        PromptInput::Text(value)
    }
}

impl From<AgentMessage> for PromptInput {
    fn from(value: AgentMessage) -> Self {
        PromptInput::Message(value)
    }
}

impl From<Vec<AgentMessage>> for PromptInput {
    fn from(value: Vec<AgentMessage>) -> Self {
        PromptInput::Messages(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{channel_stream, AssistantEvent, AssistantEventStream, ModelRequest};
    use crate::tool::{ToolError, ToolResult, ToolUpdateFn};
    use crate::types::Usage;
    use serde_json::{json, Value};

    /// Calls `echo` on the first turn, then answers with text.
    struct ScriptedProvider;

    #[async_trait]
    impl Provider for ScriptedProvider {
        fn provider_id(&self) -> &str {
            "test"
        }
        fn model_id(&self) -> &str {
            "scripted"
        }
        async fn stream(&self, request: ModelRequest, _cancel: CancellationToken) -> AssistantEventStream {
            let (tx, rx) = mpsc::channel(16);
            let saw_tool_result = request.messages.iter().any(|m| matches!(m, LlmMessage::ToolResult(_)));
            tokio::spawn(async move {
                tx.send(AssistantEvent::Start).await.unwrap();
                if saw_tool_result {
                    tx.send(AssistantEvent::TextStart { index: 0 }).await.unwrap();
                    tx.send(AssistantEvent::TextDelta { index: 0, delta: "done".into() }).await.unwrap();
                    tx.send(AssistantEvent::TextEnd { index: 0 }).await.unwrap();
                    tx.send(AssistantEvent::Done { stop_reason: StopReason::Stop, usage: Usage::default() })
                        .await
                        .unwrap();
                } else {
                    tx.send(AssistantEvent::ToolCallStart { index: 0, id: "c1".into(), name: "echo".into() })
                        .await
                        .unwrap();
                    tx.send(AssistantEvent::ToolCallDelta { index: 0, delta: "{\"text\":\"hi\"}".into() })
                        .await
                        .unwrap();
                    tx.send(AssistantEvent::ToolCallEnd { index: 0 }).await.unwrap();
                    tx.send(AssistantEvent::Done { stop_reason: StopReason::ToolUse, usage: Usage::default() })
                        .await
                        .unwrap();
                }
            });
            channel_stream(rx)
        }
    }

    struct Echo;

    #[async_trait]
    impl Tool for Echo {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "echo"
        }
        fn parameters(&self) -> Value {
            json!({"type":"object","properties":{"text":{"type":"string"}}})
        }
        async fn execute(
            &self,
            _id: &str,
            args: Value,
            _cancel: CancellationToken,
            _on_update: ToolUpdateFn,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::text(args["text"].as_str().unwrap_or("").to_string()))
        }
    }

    #[test]
    fn public_message_queue_uses_the_selected_drain_mode() {
        let queue = AgentMessageQueue::new(QueueMode::OneAtATime);
        queue.push(AgentMessage::User(UserMessage::text("one")));
        queue.push(AgentMessage::User(UserMessage::text("two")));
        assert_eq!(queue.drain().len(), 1);
        queue.set_mode(QueueMode::All);
        queue.push(AgentMessage::User(UserMessage::text("three")));
        assert_eq!(queue.drain().len(), 2);
        assert!(queue.is_empty());
    }

    #[tokio::test]
    async fn runs_tool_then_answers() {
        let mut options = AgentOptions::new(Arc::new(ScriptedProvider));
        options.tools = vec![Arc::new(Echo)];
        let mut agent = Agent::new(options);
        let (tx, mut rx) = mpsc::channel(64);
        let collector = tokio::spawn(async move {
            let mut kinds = Vec::new();
            while let Some(event) = rx.recv().await {
                kinds.push(serde_json::to_value(&event).unwrap()["type"].as_str().unwrap().to_string());
            }
            kinds
        });
        let produced = agent.prompt("go", tx).await.unwrap();
        let kinds = collector.await.unwrap();

        assert_eq!(kinds.first().map(String::as_str), Some("agent_start"));
        assert_eq!(kinds.last().map(String::as_str), Some("agent_end"));
        assert!(kinds.contains(&"tool_execution_end".to_string()));
        assert_eq!(kinds.iter().filter(|k| k.as_str() == "turn_end").count(), 2);
        // user, assistant(tool call), tool result, assistant(text)
        assert_eq!(produced.len(), 4);
        assert_eq!(agent.messages.len(), 4);
        match agent.messages.last() {
            Some(AgentMessage::Assistant(m)) => assert_eq!(m.text(), "done"),
            other => panic!("unexpected {other:?}"),
        }
    }
}
