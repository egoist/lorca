//! The low-level loop, after pi-agent-core's `agent-loop.ts`.
//!
//! Works with [`AgentMessage`] throughout and converts to [`LlmMessage`] only at the provider
//! boundary. Emits [`AgentEvent`]s over a channel in the same order pi does.

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::provider::{AssistantAccumulator, AssistantEvent, ModelRequest, Provider};
use crate::tool::{Tool, ToolResult};
use crate::types::{
    AgentEvent, AgentMessage, AssistantMessage, ContentPart, LlmMessage, StopReason, ToolCall,
    ToolResultMessage,
};

/// How tool calls from one assistant message run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolExecutionMode {
    /// Each call is prepared, executed, and finalized before the next starts.
    Sequential,
    /// Calls are prepared in order, then run concurrently. `tool_execution_end` fires in
    /// completion order; tool-result messages are emitted in source order afterwards.
    Parallel,
}

/// Snapshot passed into the loop.
#[derive(Clone)]
pub struct AgentContext {
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<Arc<dyn Tool>>,
}

#[derive(Debug, Clone, Default)]
pub struct BeforeToolCallResult {
    pub block: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AfterToolCallResult {
    pub content: Option<Vec<ContentPart>>,
    pub details: Option<Value>,
    pub is_error: Option<bool>,
    pub terminate: Option<bool>,
}

pub struct BeforeToolCallContext<'a> {
    pub assistant_message: &'a AssistantMessage,
    pub tool_call: &'a ToolCall,
    pub args: &'a Value,
    pub context: &'a AgentContext,
}

pub struct AfterToolCallContext<'a> {
    pub assistant_message: &'a AssistantMessage,
    pub tool_call: &'a ToolCall,
    pub args: &'a Value,
    pub result: &'a ToolResult,
    pub is_error: bool,
    pub context: &'a AgentContext,
}

pub struct ShouldStopAfterTurnContext<'a> {
    pub message: &'a AssistantMessage,
    pub tool_results: &'a [ToolResultMessage],
    pub context: &'a AgentContext,
    pub new_messages: &'a [AgentMessage],
}

/// Hooks the loop calls. Every method has a safe default. None may fail: return a fallback.
#[async_trait]
pub trait LoopHooks: Send + Sync {
    /// Prune or enrich the transcript before it is converted for the model.
    async fn transform_context(
        &self,
        messages: Vec<AgentMessage>,
        _cancel: &CancellationToken,
    ) -> Vec<AgentMessage> {
        messages
    }

    /// Filter UI-only messages and rewrite custom ones into something the model understands.
    fn convert_to_llm(&self, messages: &[AgentMessage]) -> Vec<LlmMessage> {
        default_convert_to_llm(messages)
    }

    /// Messages to inject after the current turn's tool calls finish.
    async fn steering_messages(&self) -> Vec<AgentMessage> {
        Vec::new()
    }

    /// Messages to process once the agent would otherwise stop.
    async fn follow_up_messages(&self) -> Vec<AgentMessage> {
        Vec::new()
    }

    async fn before_tool_call(&self, _ctx: BeforeToolCallContext<'_>) -> Option<BeforeToolCallResult> {
        None
    }

    async fn after_tool_call(&self, _ctx: AfterToolCallContext<'_>) -> Option<AfterToolCallResult> {
        None
    }

    async fn should_stop_after_turn(&self, _ctx: ShouldStopAfterTurnContext<'_>) -> bool {
        false
    }
}

pub struct NoHooks;

#[async_trait]
impl LoopHooks for NoHooks {}

pub fn default_convert_to_llm(messages: &[AgentMessage]) -> Vec<LlmMessage> {
    messages.iter().filter_map(AgentMessage::as_llm).collect()
}

/// A listener the loop awaits before it moves on, so state derived from events is updated in
/// order with the loop's own side effects (pi awaits its subscribers the same way).
#[async_trait]
pub trait EventSink: Send + Sync {
    async fn on_event(&self, event: &AgentEvent);
}

#[derive(Clone)]
pub struct AgentLoopConfig {
    pub provider: Arc<dyn Provider>,
    pub hooks: Arc<dyn LoopHooks>,
    pub tool_execution: ToolExecutionMode,
    /// Awaited for every event, before the event is also sent on the channel.
    pub sink: Option<Arc<dyn EventSink>>,
}

impl AgentLoopConfig {
    pub fn new(provider: Arc<dyn Provider>) -> Self {
        AgentLoopConfig { provider, hooks: Arc::new(NoHooks), tool_execution: ToolExecutionMode::Parallel, sink: None }
    }

    pub fn with_hooks(mut self, hooks: Arc<dyn LoopHooks>) -> Self {
        self.hooks = hooks;
        self
    }

    pub fn with_sink(mut self, sink: Arc<dyn EventSink>) -> Self {
        self.sink = Some(sink);
        self
    }
}

/// Delivers events to the sink, then the channel. A closed channel only means nobody is
/// listening on that side; the run still finishes.
pub struct Emitter<'a> {
    tx: &'a mpsc::Sender<AgentEvent>,
    sink: Option<&'a Arc<dyn EventSink>>,
}

impl<'a> Emitter<'a> {
    pub fn new(tx: &'a mpsc::Sender<AgentEvent>, config: &'a AgentLoopConfig) -> Self {
        Emitter { tx, sink: config.sink.as_ref() }
    }

    pub async fn send(&self, event: AgentEvent) {
        if let Some(sink) = self.sink {
            sink.on_event(&event).await;
        }
        let _ = self.tx.send(event).await;
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    #[error("Cannot continue: no messages in context")]
    Empty,
    #[error("Cannot continue from message role: assistant")]
    LastIsAssistant,
}

/// Start a loop with new prompt messages. Returns the event receiver and a handle that resolves
/// to the messages the run produced.
pub fn agent_loop(
    prompts: Vec<AgentMessage>,
    context: AgentContext,
    config: AgentLoopConfig,
    cancel: CancellationToken,
) -> (mpsc::Receiver<AgentEvent>, tokio::task::JoinHandle<Vec<AgentMessage>>) {
    let (tx, rx) = mpsc::channel(256);
    let handle = tokio::spawn(async move { run_agent_loop(prompts, context, &config, &tx, cancel).await });
    (rx, handle)
}

/// Run the loop with new prompt messages, emitting events on `emit`.
pub async fn run_agent_loop(
    prompts: Vec<AgentMessage>,
    mut context: AgentContext,
    config: &AgentLoopConfig,
    emit: &mpsc::Sender<AgentEvent>,
    cancel: CancellationToken,
) -> Vec<AgentMessage> {
    let emit = Emitter::new(emit, config);
    let mut new_messages = prompts.clone();
    context.messages.extend(prompts.iter().cloned());

    emit.send(AgentEvent::AgentStart).await;
    emit.send(AgentEvent::TurnStart).await;
    for prompt in &prompts {
        emit.send(AgentEvent::MessageStart { message: prompt.clone() }).await;
        emit.send(AgentEvent::MessageEnd { message: prompt.clone() }).await;
    }

    run_loop(&mut context, &mut new_messages, config, &emit, cancel).await;
    new_messages
}

/// Continue from the current context without adding a message. The last message must convert
/// to a user or tool-result message.
pub async fn run_agent_loop_continue(
    mut context: AgentContext,
    config: &AgentLoopConfig,
    emit: &mpsc::Sender<AgentEvent>,
    cancel: CancellationToken,
) -> Result<Vec<AgentMessage>, LoopError> {
    match context.messages.last() {
        None => return Err(LoopError::Empty),
        Some(last) if last.is_assistant() => return Err(LoopError::LastIsAssistant),
        _ => {}
    }
    let emit = Emitter::new(emit, config);
    let mut new_messages = Vec::new();
    emit.send(AgentEvent::AgentStart).await;
    emit.send(AgentEvent::TurnStart).await;
    run_loop(&mut context, &mut new_messages, config, &emit, cancel).await;
    Ok(new_messages)
}

async fn run_loop(
    context: &mut AgentContext,
    new_messages: &mut Vec<AgentMessage>,
    config: &AgentLoopConfig,
    emit: &Emitter<'_>,
    cancel: CancellationToken,
) {
    let hooks = &config.hooks;
    let mut first_turn = true;
    // The user may have typed while waiting.
    let mut pending = hooks.steering_messages().await;

    // Outer loop: continues when follow-up messages arrive after the agent would stop.
    loop {
        let mut has_more_tool_calls = true;

        // Inner loop: tool calls and steering messages.
        while has_more_tool_calls || !pending.is_empty() {
            if !first_turn {
                emit.send(AgentEvent::TurnStart).await;
            } else {
                first_turn = false;
            }

            for message in pending.drain(..) {
                emit.send(AgentEvent::MessageStart { message: message.clone() }).await;
                emit.send(AgentEvent::MessageEnd { message: message.clone() }).await;
                context.messages.push(message.clone());
                new_messages.push(message);
            }

            let message = stream_assistant_response(context, config, emit, &cancel).await;
            new_messages.push(AgentMessage::Assistant(message.clone()));

            if matches!(message.stop_reason, StopReason::Error | StopReason::Aborted) {
                emit.send(AgentEvent::TurnEnd { message: AgentMessage::Assistant(message), tool_results: vec![] })
                    .await;
                emit.send(AgentEvent::AgentEnd { messages: new_messages.clone() }).await;
                return;
            }

            let mut tool_results = Vec::new();
            has_more_tool_calls = false;
            if !message.tool_calls().is_empty() {
                let batch = execute_tool_calls(context, &message, config, emit, &cancel).await;
                has_more_tool_calls = !batch.terminate;
                for result in batch.messages {
                    context.messages.push(AgentMessage::ToolResult(result.clone()));
                    new_messages.push(AgentMessage::ToolResult(result.clone()));
                    tool_results.push(result);
                }
            }

            emit.send(AgentEvent::TurnEnd { message: AgentMessage::Assistant(message.clone()), tool_results: tool_results.clone() },
            )
            .await;

            let stop = hooks
                .should_stop_after_turn(ShouldStopAfterTurnContext {
                    message: &message,
                    tool_results: &tool_results,
                    context,
                    new_messages,
                })
                .await;
            if stop {
                emit.send(AgentEvent::AgentEnd { messages: new_messages.clone() }).await;
                return;
            }

            pending = hooks.steering_messages().await;
        }

        // The agent would stop here. Check for follow-ups.
        let follow_ups = hooks.follow_up_messages().await;
        if !follow_ups.is_empty() {
            pending = follow_ups;
            continue;
        }
        break;
    }

    emit.send(AgentEvent::AgentEnd { messages: new_messages.clone() }).await;
}

/// Stream one assistant message. This is where `AgentMessage`s become `LlmMessage`s.
async fn stream_assistant_response(
    context: &mut AgentContext,
    config: &AgentLoopConfig,
    emit: &Emitter<'_>,
    cancel: &CancellationToken,
) -> AssistantMessage {
    let hooks = &config.hooks;
    let provider = &config.provider;

    let transformed = hooks.transform_context(context.messages.clone(), cancel).await;
    let llm_messages = hooks.convert_to_llm(&transformed);
    let request = ModelRequest {
        system_prompt: context.system_prompt.clone(),
        messages: llm_messages,
        tools: context.tools.iter().map(|tool| tool.spec()).collect(),
    };

    let mut stream = provider.stream(request, cancel.clone()).await;
    let mut acc = AssistantAccumulator::new(provider.provider_id(), provider.model_id());
    let mut added_partial = false;

    while let Some(event) = stream.next().await {
        match event {
            AssistantEvent::Start => {
                context.messages.push(AgentMessage::Assistant(acc.message()));
                added_partial = true;
                emit.send(AgentEvent::MessageStart { message: AgentMessage::Assistant(acc.message()) }).await;
            }
            AssistantEvent::Done { .. } | AssistantEvent::Error { .. } => {
                acc.apply(&event);
                break;
            }
            other => {
                acc.apply(&other);
                if added_partial {
                    let partial = AgentMessage::Assistant(acc.message());
                    if let Some(last) = context.messages.last_mut() {
                        *last = partial.clone();
                    }
                    emit.send(AgentEvent::MessageUpdate { message: partial, assistant_message_event: other }).await;
                }
            }
        }
    }

    let final_message = acc.finish(cancel.is_cancelled());
    let wrapped = AgentMessage::Assistant(final_message.clone());
    if added_partial {
        if let Some(last) = context.messages.last_mut() {
            *last = wrapped.clone();
        }
    } else {
        context.messages.push(wrapped.clone());
        emit.send(AgentEvent::MessageStart { message: wrapped.clone() }).await;
    }
    emit.send(AgentEvent::MessageEnd { message: wrapped }).await;
    final_message
}

// MARK: - Tool execution

struct ToolBatch {
    messages: Vec<ToolResultMessage>,
    terminate: bool,
}

struct FinalizedCall {
    tool_call: ToolCall,
    result: ToolResult,
    is_error: bool,
}

enum Preparation {
    Immediate { result: ToolResult, is_error: bool },
    Prepared { tool: Arc<dyn Tool>, args: Value },
}

async fn execute_tool_calls(
    context: &AgentContext,
    assistant: &AssistantMessage,
    config: &AgentLoopConfig,
    emit: &Emitter<'_>,
    cancel: &CancellationToken,
) -> ToolBatch {
    let tool_calls: Vec<ToolCall> = assistant.tool_calls().into_iter().cloned().collect();
    let has_sequential = tool_calls.iter().any(|call| {
        context
            .tools
            .iter()
            .find(|tool| tool.name() == call.name)
            .and_then(|tool| tool.execution_mode())
            == Some(ToolExecutionMode::Sequential)
    });

    if config.tool_execution == ToolExecutionMode::Sequential || has_sequential {
        execute_sequential(context, assistant, tool_calls, config, emit, cancel).await
    } else {
        execute_parallel(context, assistant, tool_calls, config, emit, cancel).await
    }
}

async fn execute_sequential(
    context: &AgentContext,
    assistant: &AssistantMessage,
    tool_calls: Vec<ToolCall>,
    config: &AgentLoopConfig,
    emit: &Emitter<'_>,
    cancel: &CancellationToken,
) -> ToolBatch {
    let mut finalized_calls = Vec::new();
    let mut messages = Vec::new();

    for tool_call in tool_calls {
        emit.send(AgentEvent::ToolExecutionStart {
                tool_call_id: tool_call.id.clone(),
                tool_name: tool_call.name.clone(),
                args: tool_call.arguments.clone(),
            },
        )
        .await;

        let finalized = match prepare_tool_call(context, assistant, &tool_call, config).await {
            Preparation::Immediate { result, is_error } => FinalizedCall { tool_call, result, is_error },
            Preparation::Prepared { tool, args } => {
                let (result, is_error) = execute_prepared(&tool, &tool_call, &args, emit, cancel).await;
                finalize_executed(context, assistant, tool_call, args, result, is_error, config).await
            }
        };

        emit_tool_execution_end(&finalized, emit).await;
        let message = tool_result_message(&finalized);
        emit_tool_result_message(&message, emit).await;
        finalized_calls.push(finalized);
        messages.push(message);
    }

    ToolBatch { terminate: should_terminate(&finalized_calls), messages }
}

async fn execute_parallel(
    context: &AgentContext,
    assistant: &AssistantMessage,
    tool_calls: Vec<ToolCall>,
    config: &AgentLoopConfig,
    emit: &Emitter<'_>,
    cancel: &CancellationToken,
) -> ToolBatch {
    enum Slot {
        Done(FinalizedCall),
        Pending { tool: Arc<dyn Tool>, tool_call: ToolCall, args: Value },
    }

    let mut slots = Vec::new();
    for tool_call in tool_calls {
        emit.send(AgentEvent::ToolExecutionStart {
                tool_call_id: tool_call.id.clone(),
                tool_name: tool_call.name.clone(),
                args: tool_call.arguments.clone(),
            },
        )
        .await;
        match prepare_tool_call(context, assistant, &tool_call, config).await {
            Preparation::Immediate { result, is_error } => {
                let finalized = FinalizedCall { tool_call, result, is_error };
                emit_tool_execution_end(&finalized, emit).await;
                slots.push(Slot::Done(finalized));
            }
            Preparation::Prepared { tool, args } => slots.push(Slot::Pending { tool, tool_call, args }),
        }
    }

    let futures = slots.into_iter().map(|slot| async move {
        match slot {
            Slot::Done(finalized) => finalized,
            Slot::Pending { tool, tool_call, args } => {
                let (result, is_error) = execute_prepared(&tool, &tool_call, &args, emit, cancel).await;
                let finalized =
                    finalize_executed(context, assistant, tool_call, args, result, is_error, config).await;
                emit_tool_execution_end(&finalized, emit).await;
                finalized
            }
        }
    });
    let ordered = futures::future::join_all(futures).await;

    let mut messages = Vec::new();
    for finalized in &ordered {
        let message = tool_result_message(finalized);
        emit_tool_result_message(&message, emit).await;
        messages.push(message);
    }

    ToolBatch { terminate: should_terminate(&ordered), messages }
}

fn should_terminate(calls: &[FinalizedCall]) -> bool {
    !calls.is_empty() && calls.iter().all(|call| call.result.terminate)
}

async fn prepare_tool_call(
    context: &AgentContext,
    assistant: &AssistantMessage,
    tool_call: &ToolCall,
    config: &AgentLoopConfig,
) -> Preparation {
    let Some(tool) = context.tools.iter().find(|tool| tool.name() == tool_call.name).cloned() else {
        return Preparation::Immediate {
            result: error_result(format!("Tool {} not found", tool_call.name)),
            is_error: true,
        };
    };

    let args = match validate_arguments(&tool_call.arguments) {
        Ok(args) => args,
        Err(message) => return Preparation::Immediate { result: error_result(message), is_error: true },
    };

    if let Some(before) = config
        .hooks
        .before_tool_call(BeforeToolCallContext { assistant_message: assistant, tool_call, args: &args, context })
        .await
    {
        if before.block {
            return Preparation::Immediate {
                result: error_result(before.reason.unwrap_or_else(|| "Tool execution was blocked".into())),
                is_error: true,
            };
        }
    }

    Preparation::Prepared { tool, args }
}

/// Arguments must be a JSON object. Schema validation is the tool's job when it deserializes.
fn validate_arguments(arguments: &Value) -> Result<Value, String> {
    match arguments {
        Value::Object(_) => Ok(arguments.clone()),
        Value::Null => Ok(Value::Object(Default::default())),
        Value::String(raw) => serde_json::from_str::<Value>(raw)
            .ok()
            .filter(Value::is_object)
            .ok_or_else(|| format!("Tool arguments are not a JSON object: {raw}")),
        other => Err(format!("Tool arguments are not a JSON object: {other}")),
    }
}

async fn execute_prepared(
    tool: &Arc<dyn Tool>,
    tool_call: &ToolCall,
    args: &Value,
    emit: &Emitter<'_>,
    cancel: &CancellationToken,
) -> (ToolResult, bool) {
    let (update_tx, mut update_rx) = mpsc::unbounded_channel::<ToolResult>();
    let on_update: crate::tool::ToolUpdateFn = Arc::new(move |partial| {
        let _ = update_tx.send(partial);
    });

    let call_id = tool_call.id.clone();
    let name = tool_call.name.clone();
    let call_args = tool_call.arguments.clone();

    let execution = tool.execute(&call_id, args.clone(), cancel.clone(), on_update);
    tokio::pin!(execution);

    let outcome = loop {
        tokio::select! {
            result = &mut execution => break result,
            Some(partial) = update_rx.recv() => {
                emit.send(AgentEvent::ToolExecutionUpdate {
                    tool_call_id: call_id.clone(),
                    tool_name: name.clone(),
                    args: call_args.clone(),
                    partial_result: partial,
                }).await;
            }
        }
    };

    // Flush updates the tool sent right before it returned.
    while let Ok(partial) = update_rx.try_recv() {
        emit.send(AgentEvent::ToolExecutionUpdate {
                tool_call_id: call_id.clone(),
                tool_name: name.clone(),
                args: call_args.clone(),
                partial_result: partial,
            },
        )
        .await;
    }

    match outcome {
        Ok(result) => (result, false),
        Err(error) => (error_result(error.0), true),
    }
}

#[allow(clippy::too_many_arguments)]
async fn finalize_executed(
    context: &AgentContext,
    assistant: &AssistantMessage,
    tool_call: ToolCall,
    args: Value,
    mut result: ToolResult,
    mut is_error: bool,
    config: &AgentLoopConfig,
) -> FinalizedCall {
    if let Some(after) = config
        .hooks
        .after_tool_call(AfterToolCallContext {
            assistant_message: assistant,
            tool_call: &tool_call,
            args: &args,
            result: &result,
            is_error,
            context,
        })
        .await
    {
        if let Some(content) = after.content {
            result.content = content;
        }
        if let Some(details) = after.details {
            result.details = details;
        }
        if let Some(terminate) = after.terminate {
            result.terminate = terminate;
        }
        if let Some(flag) = after.is_error {
            is_error = flag;
        }
    }
    FinalizedCall { tool_call, result, is_error }
}

fn error_result(message: String) -> ToolResult {
    ToolResult { content: vec![ContentPart::text(message)], details: Value::Object(Default::default()), terminate: false }
}

async fn emit_tool_execution_end(finalized: &FinalizedCall, emit: &Emitter<'_>) {
    emit.send(AgentEvent::ToolExecutionEnd {
            tool_call_id: finalized.tool_call.id.clone(),
            tool_name: finalized.tool_call.name.clone(),
            result: finalized.result.clone(),
            is_error: finalized.is_error,
        },
    )
    .await;
}

fn tool_result_message(finalized: &FinalizedCall) -> ToolResultMessage {
    ToolResultMessage {
        tool_call_id: finalized.tool_call.id.clone(),
        tool_name: finalized.tool_call.name.clone(),
        content: finalized.result.content.clone(),
        details: finalized.result.details.clone(),
        is_error: finalized.is_error,
        timestamp: crate::now_ms(),
    }
}

async fn emit_tool_result_message(message: &ToolResultMessage, emit: &Emitter<'_>) {
    emit.send(AgentEvent::MessageStart { message: AgentMessage::ToolResult(message.clone()) }).await;
    emit.send(AgentEvent::MessageEnd { message: AgentMessage::ToolResult(message.clone()) }).await;
}
