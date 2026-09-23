//! The harness: a transcript, a model, tools, and everything that happens around a run.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::agent::QueueMode;
use crate::agent_loop::{
    run_agent_loop, run_agent_loop_continue, AfterToolCallContext, AfterToolCallResult, AgentContext, AgentLoopConfig,
    BeforeToolCallContext, BeforeToolCallResult, LoopError, LoopHooks, PrepareNextTurnContext, ToolExecutionMode, TurnUpdate,
};
use crate::compaction::{self, CompactResult, CompactionSettings};
use crate::estimate::{context_tokens, estimate_context_tokens, estimate_text_tokens};
use crate::request::RequestOptions;
use crate::retry::{is_context_overflow, RetryPolicy};
use crate::tool::Tool;
use crate::types::{AgentEvent, AgentMessage, ContentPart, LlmMessage, StopReason, ThinkingLevel, ToolResultMessage, Usage, UserMessage};
use crate::Provider;

use super::events::{EventBus, HarnessEvent, Outcome, QueueKind, QueuedItem};
use super::factory::{ModelIdentity, ProviderFactory};
use super::hooks::{CompactionDecision, CompactionReason, HookRegistry, RegistryRequestHooks, RequestStep, Resources};
use super::skills::format_skill_invocation;
use super::templates::format_prompt_template_invocation;

/// What a harness starts with. `system_prompt` is sent as is; build one with
/// [`super::build_system_prompt`].
pub struct HarnessOptions {
    pub provider: Arc<dyn Provider>,
    /// How `set_model` and `set_thinking_level` get a new provider. Without one they fail.
    pub factory: Option<Arc<dyn ProviderFactory>>,
    pub thinking_level: Option<ThinkingLevel>,
    pub system_prompt: String,
    pub tools: Vec<Arc<dyn Tool>>,
    /// The tools the model sees, by name. `None` is all of them.
    pub active_tools: Option<Vec<String>>,
    pub resources: Resources,
    pub request: RequestOptions,
    pub retry: RetryPolicy,
    pub compaction: CompactionSettings,
    pub steering_mode: QueueMode,
    pub follow_up_mode: QueueMode,
    pub tool_execution: ToolExecutionMode,
    /// The transcript to start from.
    pub messages: Vec<AgentMessage>,
}

impl HarnessOptions {
    pub fn new(provider: Arc<dyn Provider>) -> Self {
        HarnessOptions {
            provider,
            factory: None,
            thinking_level: None,
            system_prompt: String::new(),
            tools: Vec::new(),
            active_tools: None,
            resources: Resources::default(),
            request: RequestOptions::default(),
            retry: RetryPolicy::default(),
            compaction: CompactionSettings::default(),
            steering_mode: QueueMode::All,
            follow_up_mode: QueueMode::All,
            tool_execution: ToolExecutionMode::Parallel,
            messages: Vec::new(),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HarnessError {
    #[error("A run is active. Queue with steer(), follow_up(), or next_run(), or wait for it.")]
    Busy,
    #[error("No messages to continue from")]
    NoMessages,
    #[error("Cannot continue from message role: assistant")]
    LastIsAssistant,
    #[error("Unknown skill {0}")]
    UnknownSkill(String),
    #[error("Unknown prompt template {0}")]
    UnknownTemplate(String),
    #[error("No provider factory: give the harness one to change models")]
    NoFactory,
    #[error("{0}")]
    Provider(String),
    #[error("{0}")]
    Compaction(String),
    #[error("Nothing to compact")]
    NothingToCompact,
}

/// What a run produced.
#[derive(Debug, Clone, PartialEq)]
pub struct RunResult {
    pub run_id: String,
    pub outcome: Outcome,
    /// The messages the run added to the transcript, in order.
    pub messages: Vec<AgentMessage>,
}

/// What a compaction produced.
#[derive(Debug, Clone, PartialEq)]
pub struct CompactionOutcome {
    pub outcome: Outcome,
    pub tokens_before: Option<u64>,
}

/// The harness as the outside sees it between events.
#[derive(Debug, Clone, PartialEq)]
pub struct HarnessStats {
    pub model: ModelIdentity,
    pub thinking_level: Option<ThinkingLevel>,
    /// The context as it would be sent now, estimated.
    pub context_tokens: u64,
    /// The model's window, 0 when unknown.
    pub context_window: u64,
    /// Every model call since the harness was made.
    pub totals: Usage,
    pub messages: usize,
    pub running: bool,
}

struct Queued {
    id: String,
    kind: QueueKind,
    message: AgentMessage,
}

struct Queues {
    items: Mutex<Vec<Queued>>,
    steering_mode: Mutex<QueueMode>,
    follow_up_mode: Mutex<QueueMode>,
    next_id: AtomicU64,
}

impl Queues {
    fn push(&self, kind: QueueKind, message: AgentMessage) -> String {
        let id = format!("q-{}", self.next_id.fetch_add(1, Ordering::SeqCst) + 1);
        self.items.lock().unwrap().push(Queued { id: id.clone(), kind, message });
        id
    }

    fn drain(&self, kind: QueueKind, mode: QueueMode) -> Vec<AgentMessage> {
        let mut items = self.items.lock().unwrap();
        let mut taken = Vec::new();
        let mut kept = Vec::new();
        for item in items.drain(..) {
            let take = item.kind == kind && (mode == QueueMode::All || taken.is_empty());
            if take {
                taken.push(item.message);
            } else {
                kept.push(item);
            }
        }
        *items = kept;
        taken
    }

    fn snapshot(&self) -> Vec<QueuedItem> {
        self.items.lock().unwrap().iter().map(|q| QueuedItem { id: q.id.clone(), kind: q.kind, message: q.message.clone() }).collect()
    }
}

/// Steering, follow-ups, and abort from other tasks while a run is in flight.
#[derive(Clone)]
pub struct HarnessHandle {
    queues: Arc<Queues>,
    events: Arc<EventBus>,
    active: Arc<Mutex<Option<CancellationToken>>>,
}

impl HarnessHandle {
    fn queue(&self, kind: QueueKind, message: impl Into<AgentMessage>) -> String {
        let id = self.queues.push(kind, message.into());
        let events = self.events.clone();
        let queues = self.queues.snapshot();
        tokio::spawn(async move { events.emit(HarnessEvent::QueueUpdate { queues }).await });
        id
    }

    /// A message for after the current turn's tool calls, before the next model call.
    pub fn steer(&self, message: impl Into<AgentMessage>) -> String {
        self.queue(QueueKind::Steer, message)
    }

    /// A message for when the run would otherwise stop; it keeps the run going.
    pub fn follow_up(&self, message: impl Into<AgentMessage>) -> String {
        self.queue(QueueKind::FollowUp, message)
    }

    /// A prompt for a new run once the current one has ended.
    pub fn next_run(&self, message: impl Into<AgentMessage>) -> String {
        self.queue(QueueKind::NextRun, message)
    }

    /// Removes a queued message. `true` when it was still waiting.
    pub fn cancel_queued(&self, id: &str) -> bool {
        let removed = {
            let mut items = self.queues.items.lock().unwrap();
            let before = items.len();
            items.retain(|q| q.id != id);
            items.len() != before
        };
        if removed {
            let events = self.events.clone();
            let queues = self.queues.snapshot();
            tokio::spawn(async move { events.emit(HarnessEvent::QueueUpdate { queues }).await });
        }
        removed
    }

    pub fn queued(&self) -> Vec<QueuedItem> {
        self.queues.snapshot()
    }

    pub fn abort(&self) {
        if let Some(token) = self.active.lock().unwrap().as_ref() {
            token.cancel();
        }
    }

    pub fn is_running(&self) -> bool {
        self.active.lock().unwrap().is_some()
    }
}

impl From<&str> for AgentMessage {
    fn from(text: &str) -> Self {
        AgentMessage::user(text)
    }
}

impl From<String> for AgentMessage {
    fn from(text: String) -> Self {
        AgentMessage::user(text)
    }
}

/// A context replaced by a mid-run compaction, handed from the loop's hooks to the harness at
/// the next turn start so the transcript follows.
type Replacement = Arc<Mutex<Option<(Vec<AgentMessage>, u64)>>>;

/// What the loop's hooks need from the harness during a run.
struct RunHooks {
    registry: Arc<HookRegistry>,
    queues: Arc<Queues>,
    provider: Arc<dyn Provider>,
    compaction: CompactionSettings,
    request: RequestOptions,
    replacement: Replacement,
    events: Arc<EventBus>,
    run_id: String,
    skip_initial_steering_poll: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl LoopHooks for RunHooks {
    async fn transform_context(&self, messages: Vec<AgentMessage>, _cancel: &CancellationToken) -> Vec<AgentMessage> {
        let (messages, _) = self.registry.transform_context(messages, String::new()).await;
        messages
    }

    fn convert_to_llm(&self, messages: &[AgentMessage]) -> Vec<LlmMessage> {
        convert_with_summaries(messages)
    }

    async fn steering_messages(&self) -> Vec<AgentMessage> {
        if self.skip_initial_steering_poll.swap(false, Ordering::SeqCst) {
            return Vec::new();
        }
        let mode = *self.queues.steering_mode.lock().unwrap();
        self.queues.drain(QueueKind::Steer, mode)
    }

    async fn follow_up_messages(&self) -> Vec<AgentMessage> {
        let mode = *self.queues.follow_up_mode.lock().unwrap();
        self.queues.drain(QueueKind::FollowUp, mode)
    }

    async fn before_tool_call(&self, ctx: BeforeToolCallContext<'_>) -> Option<BeforeToolCallResult> {
        if self.registry.is_empty() {
            return None;
        }
        let decision = self.registry.before_tool(ctx.tool_call, ctx.args).await;
        match decision.block {
            Some(block) => Some(BeforeToolCallResult { block: true, reason: Some(block.reason), args: None, terminate: block.terminate }),
            None => decision.args.map(|args| BeforeToolCallResult { block: false, reason: None, args: Some(args), terminate: false }),
        }
    }

    async fn after_tool_call(&self, ctx: AfterToolCallContext<'_>) -> Option<AfterToolCallResult> {
        if self.registry.is_empty() {
            return None;
        }
        let decision = self.registry.after_tool(ctx.tool_call, ctx.args, ctx.result, ctx.is_error).await?;
        Some(AfterToolCallResult { content: decision.content, details: decision.details, is_error: decision.is_error, terminate: decision.terminate })
    }

    async fn prepare_next_turn(&self, ctx: PrepareNextTurnContext<'_>) -> Option<TurnUpdate> {
        let window = self.provider.model_info().map(|i| i.context_window).unwrap_or(0);
        if window == 0 || !self.compaction.enabled {
            return None;
        }
        let size = estimate_context_tokens(&ctx.context.messages).tokens + estimate_text_tokens(&ctx.context.system_prompt);
        if !compaction::should_compact(size, window, &self.compaction) {
            return None;
        }
        self.events.emit(HarnessEvent::CompactionStart { run_id: self.run_id.clone(), reason: CompactionReason::Threshold }).await;
        let cancel = CancellationToken::new();
        let result = compact_transcript(&self.registry, self.provider.as_ref(), &ctx.context.messages, &self.compaction, CompactionReason::Threshold, None, &self.request, &cancel).await;
        match result {
            Ok(Some((messages, tokens_before))) => {
                *self.replacement.lock().unwrap() = Some((messages.clone(), tokens_before));
                self.events
                    .emit(HarnessEvent::CompactionEnd { run_id: self.run_id.clone(), reason: CompactionReason::Threshold, outcome: Outcome::Completed, tokens_before: Some(tokens_before) })
                    .await;
                let mut context = ctx.context.clone();
                context.messages = messages;
                Some(TurnUpdate { context: Some(context), provider: None })
            }
            Ok(None) => {
                self.events.emit(HarnessEvent::CompactionEnd { run_id: self.run_id.clone(), reason: CompactionReason::Threshold, outcome: Outcome::Declined, tokens_before: None }).await;
                None
            }
            Err(error) => {
                self.events
                    .emit(HarnessEvent::CompactionEnd { run_id: self.run_id.clone(), reason: CompactionReason::Threshold, outcome: Outcome::Failed { error }, tokens_before: None })
                    .await;
                None
            }
        }
    }
}

/// The model reads compaction summaries as user messages; other custom entries it never sees.
pub fn convert_with_summaries(messages: &[AgentMessage]) -> Vec<LlmMessage> {
    messages
        .iter()
        .filter_map(|message| match message {
            AgentMessage::Custom { kind, data, timestamp } if kind == "compaction" => Some(compaction::summary_as_llm(data, *timestamp)),
            other => other.as_llm(),
        })
        .collect()
}

/// Summarizes the older part of `messages` (which may start with an earlier summary), asking
/// the hooks first. `Ok(Some((messages, tokens_before)))` is the transcript to go on with.
#[allow(clippy::too_many_arguments)]
async fn compact_transcript(
    registry: &HookRegistry,
    provider: &dyn Provider,
    messages: &[AgentMessage],
    settings: &CompactionSettings,
    reason: CompactionReason,
    custom_instructions: Option<&str>,
    request: &RequestOptions,
    cancel: &CancellationToken,
) -> Result<Option<(Vec<AgentMessage>, u64)>, String> {
    let (previous, skip) = match messages.first() {
        Some(AgentMessage::Custom { kind, data, .. }) if kind == "compaction" => (data["summary"].as_str().map(str::to_string), 1),
        _ => (None, 0),
    };
    let body = &messages[skip..];
    let result: CompactResult = match registry.before_compaction(reason, body, custom_instructions).await {
        Some(CompactionDecision::Decline) => return Ok(None),
        Some(CompactionDecision::Replace(result)) => result,
        None => {
            let options = registry.before_request(RequestStep::Compaction, 1, request).await;
            match compaction::compact(provider, body, previous.as_deref(), settings, custom_instructions, &options, cancel).await? {
                Some(result) => result,
                None => return Ok(None),
            }
        }
    };
    let mut kept = vec![compaction::summary_message(&result.summary, result.tokens_before)];
    kept.extend(messages[skip + result.first_kept..].iter().cloned());
    // The kept messages' thinking is bound to the history the summary replaced.
    crate::providers::anthropic::drop_bound_thinking(provider.model_id(), &mut kept);
    Ok(Some((kept, result.tokens_before)))
}

/// A general agent: a transcript in memory, a model that can change, tools, skills and
/// prompt templates, compaction, retry, queues, hooks, and events. The host keeps the
/// transcript (`messages`) wherever it likes and hands it back through `HarnessOptions`.
pub struct AgentHarness {
    provider: Arc<dyn Provider>,
    factory: Option<Arc<dyn ProviderFactory>>,
    thinking_level: Option<ThinkingLevel>,
    pub system_prompt: String,
    tools: Vec<Arc<dyn Tool>>,
    active_tools: Option<Vec<String>>,
    resources: Resources,
    request: RequestOptions,
    retry: RetryPolicy,
    compaction: CompactionSettings,
    tool_execution: ToolExecutionMode,
    messages: Vec<AgentMessage>,
    totals: Usage,
    hooks: Arc<HookRegistry>,
    events: Arc<EventBus>,
    queues: Arc<Queues>,
    active: Arc<Mutex<Option<CancellationToken>>>,
    runs: u64,
}

impl AgentHarness {
    pub fn new(options: HarnessOptions) -> Self {
        AgentHarness {
            provider: options.provider,
            factory: options.factory,
            thinking_level: options.thinking_level,
            system_prompt: options.system_prompt,
            tools: options.tools,
            active_tools: options.active_tools,
            resources: options.resources,
            request: options.request,
            retry: options.retry,
            compaction: options.compaction,
            tool_execution: options.tool_execution,
            messages: options.messages,
            totals: Usage::default(),
            hooks: Arc::new(HookRegistry::default()),
            events: Arc::new(EventBus::default()),
            queues: Arc::new(Queues {
                items: Mutex::new(Vec::new()),
                steering_mode: Mutex::new(options.steering_mode),
                follow_up_mode: Mutex::new(options.follow_up_mode),
                next_id: AtomicU64::new(0),
            }),
            active: Arc::new(Mutex::new(None)),
            runs: 0,
        }
    }

    // MARK: - Access

    pub fn handle(&self) -> HarnessHandle {
        HarnessHandle { queues: self.queues.clone(), events: self.events.clone(), active: self.active.clone() }
    }

    pub fn hooks(&self) -> &Arc<HookRegistry> {
        &self.hooks
    }

    pub fn events(&self) -> &Arc<EventBus> {
        &self.events
    }

    /// A channel that gets every event from now on.
    pub fn subscribe(&self) -> mpsc::Receiver<HarnessEvent> {
        self.events.subscribe()
    }

    pub fn messages(&self) -> &[AgentMessage] {
        &self.messages
    }

    /// Replaces the transcript. Not while a run is active.
    pub fn set_messages(&mut self, messages: Vec<AgentMessage>) -> Result<(), HarnessError> {
        self.ensure_idle()?;
        self.messages = messages;
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.active.lock().unwrap().is_some()
    }

    pub fn model(&self) -> ModelIdentity {
        ModelIdentity::new(self.provider.provider_id(), self.provider.model_id())
    }

    pub fn provider(&self) -> &Arc<dyn Provider> {
        &self.provider
    }

    pub fn thinking_level(&self) -> Option<ThinkingLevel> {
        self.thinking_level
    }

    pub fn tools(&self) -> &[Arc<dyn Tool>] {
        &self.tools
    }

    pub fn active_tools(&self) -> Vec<String> {
        match &self.active_tools {
            Some(names) => names.clone(),
            None => self.tools.iter().map(|t| t.name().to_string()).collect(),
        }
    }

    pub fn resources(&self) -> &Resources {
        &self.resources
    }

    pub fn request_options(&self) -> &RequestOptions {
        &self.request
    }

    pub fn retry_policy(&self) -> &RetryPolicy {
        &self.retry
    }

    pub fn compaction_settings(&self) -> &CompactionSettings {
        &self.compaction
    }

    pub fn totals(&self) -> &Usage {
        &self.totals
    }

    pub fn stats(&self) -> HarnessStats {
        HarnessStats {
            model: self.model(),
            thinking_level: self.thinking_level,
            context_tokens: estimate_context_tokens(&self.messages).tokens + estimate_text_tokens(&self.system_prompt),
            context_window: self.provider.model_info().map(|i| i.context_window).unwrap_or(0),
            totals: self.totals.clone(),
            messages: self.messages.len(),
            running: self.is_running(),
        }
    }

    /// Adds usage the host incurred elsewhere (a side request) to the totals.
    pub async fn record_usage(&mut self, usage: &Usage) {
        self.totals.add(usage);
        self.events.emit(HarnessEvent::Usage { usage: usage.clone(), totals: self.totals.clone() }).await;
    }

    // MARK: - Configuration

    fn ensure_idle(&self) -> Result<(), HarnessError> {
        if self.is_running() {
            return Err(HarnessError::Busy);
        }
        Ok(())
    }

    async fn config_changed(&self, property: &str, value: Value) {
        self.events.emit(HarnessEvent::ConfigUpdate { property: property.into(), value }).await;
    }

    /// Runs the given provider from the next turn on.
    pub async fn set_provider(&mut self, provider: Arc<dyn Provider>) {
        self.provider = provider;
        self.config_changed("model", json!(self.model())).await;
    }

    /// Switches to a model through the factory.
    pub async fn set_model(&mut self, model: &ModelIdentity) -> Result<(), HarnessError> {
        let factory = self.factory.clone().ok_or(HarnessError::NoFactory)?;
        self.provider = factory.provider(model, self.thinking_level).map_err(HarnessError::Provider)?;
        self.config_changed("model", json!(self.model())).await;
        Ok(())
    }

    /// Changes how much the model thinks, rebuilding the provider through the factory.
    pub async fn set_thinking_level(&mut self, level: Option<ThinkingLevel>) -> Result<(), HarnessError> {
        let factory = self.factory.clone().ok_or(HarnessError::NoFactory)?;
        self.provider = factory.provider(&self.model(), level).map_err(HarnessError::Provider)?;
        self.thinking_level = level;
        self.config_changed("thinking_level", json!(level)).await;
        Ok(())
    }

    pub async fn set_tools(&mut self, tools: Vec<Arc<dyn Tool>>) {
        self.tools = tools;
        self.config_changed("tools", json!(self.tools.iter().map(|t| t.name()).collect::<Vec<_>>())).await;
    }

    pub async fn set_active_tools(&mut self, names: Option<Vec<String>>) {
        self.active_tools = names;
        self.config_changed("active_tools", json!(self.active_tools())).await;
    }

    pub async fn set_resources(&mut self, resources: Resources) {
        self.resources = resources;
        self.config_changed("resources", json!({ "skills": self.resources.skills.len(), "prompt_templates": self.resources.prompt_templates.len() })).await;
    }

    pub async fn set_request_options(&mut self, request: RequestOptions) {
        self.request = request;
        self.config_changed("request", json!(format!("{:?}", self.request))).await;
    }

    pub async fn set_retry_policy(&mut self, policy: RetryPolicy) {
        self.retry = policy;
        self.config_changed("retry", json!(self.retry)).await;
    }

    pub async fn set_compaction_settings(&mut self, settings: CompactionSettings) {
        self.compaction = settings;
        self.config_changed("compaction", json!(self.compaction)).await;
    }

    pub async fn set_steering_mode(&mut self, mode: QueueMode) {
        *self.queues.steering_mode.lock().unwrap() = mode;
        self.config_changed("steering_mode", json!(format!("{mode:?}"))).await;
    }

    pub async fn set_follow_up_mode(&mut self, mode: QueueMode) {
        *self.queues.follow_up_mode.lock().unwrap() = mode;
        self.config_changed("follow_up_mode", json!(format!("{mode:?}"))).await;
    }

    pub async fn set_tool_execution(&mut self, mode: ToolExecutionMode) {
        self.tool_execution = mode;
        self.config_changed("tool_execution", json!(format!("{mode:?}"))).await;
    }

    // MARK: - Operations

    /// Runs a prompt: text, text with images, one message, or several.
    pub async fn prompt(&mut self, input: impl Into<crate::agent::PromptInput>) -> Result<RunResult, HarnessError> {
        self.ensure_idle()?;
        let messages = input.into().into_messages();
        self.run(Some(messages)).await
    }

    /// Runs a skill by name, with anything the user added.
    pub async fn skill(&mut self, name: &str, additional_instructions: Option<&str>) -> Result<RunResult, HarnessError> {
        let skill = self.resources.skills.iter().find(|s| s.name == name).ok_or_else(|| HarnessError::UnknownSkill(name.into()))?;
        let text = format_skill_invocation(skill, additional_instructions);
        self.prompt(text).await
    }

    /// Runs a prompt template by name with its arguments.
    pub async fn prompt_from_template(&mut self, name: &str, args: &[String]) -> Result<RunResult, HarnessError> {
        let template = self.resources.prompt_templates.iter().find(|t| t.name == name).ok_or_else(|| HarnessError::UnknownTemplate(name.into()))?;
        let text = format_prompt_template_invocation(template, args);
        self.prompt(text).await
    }

    /// Continues from the transcript as it is: after a user message or tool results, or after
    /// an assistant message whose tool calls never got results (an interrupted run), which get
    /// error results first. After a finished assistant message, queued messages start the run.
    pub async fn continue_run(&mut self) -> Result<RunResult, HarnessError> {
        self.ensure_idle()?;
        let Some(last) = self.messages.last() else { return Err(HarnessError::NoMessages) };
        if let AgentMessage::Assistant(assistant) = last {
            let calls: Vec<_> = assistant.tool_calls().into_iter().cloned().collect();
            if !calls.is_empty() {
                for call in calls {
                    self.messages.push(AgentMessage::ToolResult(ToolResultMessage {
                        tool_call_id: call.id,
                        tool_name: call.name,
                        content: vec![ContentPart::text("The run was interrupted before this tool ran. Call it again if it is still needed.")],
                        details: Value::Null,
                        is_error: true,
                        timestamp: crate::now_ms(),
                    }));
                }
                return self.run(None).await;
            }
            let steering = self.queues.drain(QueueKind::Steer, QueueMode::All);
            if !steering.is_empty() {
                return self.run(Some(steering)).await;
            }
            let follow_ups = self.queues.drain(QueueKind::FollowUp, QueueMode::All);
            if !follow_ups.is_empty() {
                return self.run(Some(follow_ups)).await;
            }
            let next = self.queues.drain(QueueKind::NextRun, QueueMode::All);
            if !next.is_empty() {
                return self.run(Some(next)).await;
            }
            return Err(HarnessError::LastIsAssistant);
        }
        self.run(None).await
    }

    /// Summarizes the older part of the transcript now.
    pub async fn compact(&mut self, custom_instructions: Option<&str>) -> Result<CompactionOutcome, HarnessError> {
        self.ensure_idle()?;
        self.compact_now(CompactionReason::Manual, custom_instructions, &CancellationToken::new()).await
    }

    async fn compact_now(&mut self, reason: CompactionReason, custom_instructions: Option<&str>, cancel: &CancellationToken) -> Result<CompactionOutcome, HarnessError> {
        let run_id = format!("compaction-{}", self.runs + 1);
        self.events.emit(HarnessEvent::CompactionStart { run_id: run_id.clone(), reason }).await;
        let result = compact_transcript(&self.hooks, self.provider.as_ref(), &self.messages, &self.compaction, reason, custom_instructions, &self.request, cancel).await;
        let outcome = match result {
            Ok(Some((messages, tokens_before))) => {
                self.messages = messages;
                CompactionOutcome { outcome: Outcome::Completed, tokens_before: Some(tokens_before) }
            }
            Ok(None) => CompactionOutcome { outcome: Outcome::Declined, tokens_before: None },
            Err(error) => CompactionOutcome { outcome: Outcome::Failed { error }, tokens_before: None },
        };
        self.events.emit(HarnessEvent::CompactionEnd { run_id, reason, outcome: outcome.outcome.clone(), tokens_before: outcome.tokens_before }).await;
        match &outcome.outcome {
            Outcome::Failed { error } => Err(HarnessError::Compaction(error.clone())),
            _ => Ok(outcome),
        }
    }

    fn window(&self) -> u64 {
        self.provider.model_info().map(|i| i.context_window).unwrap_or(0)
    }

    /// One run: prompts (or a continuation), then any follow-up the hooks ask for and any
    /// prompt queued for the next run.
    async fn run(&mut self, prompts: Option<Vec<AgentMessage>>) -> Result<RunResult, HarnessError> {
        let mut prompts = prompts;
        let mut all_added = Vec::new();
        let mut result = loop {
            let result = self.run_once(prompts.take()).await?;
            all_added.extend(result.messages.iter().cloned());
            if !matches!(result.outcome, Outcome::Completed) {
                break result;
            }
            if let Some(text) = self.hooks.before_run_end(&result.run_id, &self.messages).await {
                prompts = Some(vec![AgentMessage::user(text)]);
                continue;
            }
            let next = self.queues.drain(QueueKind::NextRun, QueueMode::All);
            if !next.is_empty() {
                self.events.emit(HarnessEvent::QueueUpdate { queues: self.queues.snapshot() }).await;
                prompts = Some(next);
                continue;
            }
            break result;
        };
        result.messages = all_added;
        Ok(result)
    }

    async fn run_once(&mut self, prompts: Option<Vec<AgentMessage>>) -> Result<RunResult, HarnessError> {
        self.runs += 1;
        let run_id = format!("run-{}", self.runs);
        let cancel = CancellationToken::new();
        *self.active.lock().unwrap() = Some(cancel.clone());
        let result = self.run_loop(&run_id, prompts, cancel.clone()).await;
        *self.active.lock().unwrap() = None;
        result
    }

    async fn run_loop(&mut self, run_id: &str, prompts: Option<Vec<AgentMessage>>, cancel: CancellationToken) -> Result<RunResult, HarnessError> {
        self.events.emit(HarnessEvent::RunStart { run_id: run_id.into() }).await;

        // The hooks may add to the prompt, and change the system prompt for this run.
        let mut prompts = prompts;
        if let Some(messages) = &mut prompts {
            let injected = self.hooks.before_run(messages, &self.resources).await;
            messages.extend(injected);
        }
        let (_, system_prompt) = self.hooks.transform_context(Vec::new(), self.system_prompt.clone()).await;

        // A transcript that no longer fits is compacted before the model sees it.
        let window = self.window();
        if window > 0 && self.compaction.enabled {
            let mut candidate = self.messages.clone();
            candidate.extend(prompts.iter().flatten().cloned());
            let size = estimate_context_tokens(&candidate).tokens + estimate_text_tokens(&system_prompt);
            if compaction::should_compact(size, window, &self.compaction) {
                if let Err(error) = self.compact_now(CompactionReason::Threshold, None, &cancel).await {
                    tracing::warn!(%error, "compacting before the run");
                }
            }
        }

        let request = self.hooks.before_request(RequestStep::Assistant, 1, &self.request).await.with_hooks(Arc::new(RegistryRequestHooks(self.hooks.clone())));
        let replacement: Replacement = Arc::new(Mutex::new(None));
        let hooks = Arc::new(RunHooks {
            registry: self.hooks.clone(),
            queues: self.queues.clone(),
            provider: self.provider.clone(),
            compaction: self.compaction.clone(),
            request: request.clone(),
            replacement: replacement.clone(),
            events: self.events.clone(),
            run_id: run_id.into(),
            skip_initial_steering_poll: std::sync::atomic::AtomicBool::new(false),
        });
        let active = self.active_tools();
        let tools: Vec<Arc<dyn Tool>> = self.tools.iter().filter(|t| active.iter().any(|n| n == t.name())).cloned().collect();
        let mut recovered = false;

        loop {
            let context = AgentContext { system_prompt: system_prompt.clone(), messages: self.messages.clone(), tools: tools.clone() };
            let config = AgentLoopConfig {
                provider: self.provider.clone(),
                hooks: hooks.clone(),
                tool_execution: self.tool_execution,
                sink: None,
                retry: Some(self.retry.clone()),
                request: request.clone(),
            };
            let (tx, mut rx) = mpsc::channel::<AgentEvent>(256);
            let loop_cancel = cancel.clone();
            let loop_prompts = prompts.take();
            let task = tokio::spawn(async move {
                match loop_prompts {
                    Some(messages) => Ok(run_agent_loop(messages, context, &config, &tx, loop_cancel).await),
                    None => run_agent_loop_continue(context, &config, &tx, loop_cancel).await,
                }
            });

            let mut added: Vec<AgentMessage> = Vec::new();
            while let Some(event) = rx.recv().await {
                self.process_event(run_id, event, &replacement, &mut added).await;
            }
            let loop_result = task.await.map_err(|e| HarnessError::Provider(e.to_string()))?;
            if let Err(error) = loop_result {
                let outcome = match error {
                    LoopError::Empty => Err(HarnessError::NoMessages),
                    LoopError::LastIsAssistant => Err(HarnessError::LastIsAssistant),
                };
                self.events.emit(HarnessEvent::RunEnd { run_id: run_id.into(), outcome: Outcome::Failed { error: error.to_string() } }).await;
                return outcome;
            }

            let last_assistant = added.iter().rev().find_map(|m| match m {
                AgentMessage::Assistant(a) => Some(a),
                _ => None,
            });
            let outcome = match last_assistant.map(|a| (a.stop_reason, a.error_message.clone())) {
                Some((StopReason::Aborted, _)) => Outcome::Aborted,
                Some((StopReason::Error, error)) => Outcome::Failed { error: error.unwrap_or_default() },
                _ => Outcome::Completed,
            };

            // The model said the context no longer fits: compact and try the turn once more.
            let overflow = last_assistant.is_some_and(|a| is_context_overflow(a, None));
            if overflow && !recovered && self.compaction.enabled && !cancel.is_cancelled() {
                recovered = true;
                if let AgentMessage::Assistant(_) = self.messages.last().cloned().unwrap_or_else(|| AgentMessage::user("")) {
                    self.messages.pop();
                }
                match self.compact_now(CompactionReason::Overflow, None, &cancel).await {
                    Ok(CompactionOutcome { outcome: Outcome::Completed, .. }) => continue,
                    Ok(_) => {}
                    Err(error) => tracing::warn!(%error, "compacting after an overflow"),
                }
            }

            self.events.emit(HarnessEvent::RunEnd { run_id: run_id.into(), outcome: outcome.clone() }).await;
            return Ok(RunResult { run_id: run_id.into(), outcome, messages: added });
        }
    }

    /// Applies one loop event to the transcript and the totals, and tells the outside.
    async fn process_event(&mut self, run_id: &str, event: AgentEvent, replacement: &Replacement, added: &mut Vec<AgentMessage>) {
        let run_id = run_id.to_string();
        match event {
            AgentEvent::AgentStart | AgentEvent::AgentEnd { .. } => {}
            AgentEvent::TurnStart => {
                // A mid-run compaction replaced the loop's context: the transcript follows.
                if let Some((messages, _)) = replacement.lock().unwrap().take() {
                    self.messages = messages;
                }
                self.events.emit(HarnessEvent::TurnStart { run_id }).await;
            }
            AgentEvent::TurnEnd { message: AgentMessage::Assistant(message), tool_results } => {
                self.events.emit(HarnessEvent::TurnEnd { run_id, message, tool_results }).await;
            }
            AgentEvent::TurnEnd { .. } => {}
            AgentEvent::Retry { attempt, max_attempts, delay_ms, error } => {
                self.events.emit(HarnessEvent::RetryScheduled { run_id, attempt, max_attempts, delay_ms, error }).await;
            }
            AgentEvent::MessageStart { message } => {
                self.events.emit(HarnessEvent::MessageStart { run_id, message }).await;
            }
            AgentEvent::MessageUpdate { message, assistant_message_event } => {
                self.events.emit(HarnessEvent::MessageUpdate { run_id, message, event: assistant_message_event }).await;
            }
            AgentEvent::MessageEnd { message } => {
                let message = match message {
                    AgentMessage::Assistant(assistant) => {
                        let assistant = if self.hooks.is_empty() { assistant } else { self.hooks.after_message(assistant).await };
                        if !matches!(assistant.stop_reason, StopReason::Error | StopReason::Aborted) && context_tokens(&assistant.usage) > 0 {
                            self.totals.add(&assistant.usage);
                            self.events.emit(HarnessEvent::Usage { usage: assistant.usage.clone(), totals: self.totals.clone() }).await;
                        }
                        AgentMessage::Assistant(assistant)
                    }
                    other => other,
                };
                self.messages.push(message.clone());
                added.push(message.clone());
                self.events.emit(HarnessEvent::MessageEnd { run_id, message }).await;
            }
            AgentEvent::ToolExecutionStart { tool_call_id, tool_name, args } => {
                self.events.emit(HarnessEvent::ToolStart { run_id, tool_call_id, tool_name, args }).await;
            }
            AgentEvent::ToolExecutionUpdate { tool_call_id, tool_name, partial_result, .. } => {
                self.events.emit(HarnessEvent::ToolUpdate { run_id, tool_call_id, tool_name, partial_result }).await;
            }
            AgentEvent::ToolExecutionEnd { tool_call_id, tool_name, result, is_error } => {
                self.events.emit(HarnessEvent::ToolEnd { run_id, tool_call_id, tool_name, result, is_error }).await;
            }
        }
    }
}

/// A user message for a harness prompt.
pub fn user_message(text: &str) -> AgentMessage {
    AgentMessage::User(UserMessage::text(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{channel_stream, AssistantEvent, AssistantEventStream, ModelRequest};
    use crate::tool::{ToolError, ToolResult, ToolUpdateFn};
    use crate::types::{AssistantMessage, AssistantPart, ToolCall};
    use serde_json::json;

    /// Answers with the next script entry; records every request; reports a fake catalog
    /// window when asked to.
    struct Scripted {
        script: Mutex<Vec<Turn>>,
        requests: Mutex<Vec<ModelRequest>>,
        info: Option<&'static crate::models::ModelInfo>,
    }

    #[derive(Clone)]
    enum Turn {
        Text(&'static str),
        Call { name: &'static str, args: &'static str },
        Fail(&'static str),
    }

    impl Scripted {
        fn new(script: Vec<Turn>) -> Arc<Self> {
            Arc::new(Scripted { script: Mutex::new(script), requests: Mutex::new(vec![]), info: None })
        }
        fn with_window(script: Vec<Turn>) -> Arc<Self> {
            Arc::new(Scripted { script: Mutex::new(script), requests: Mutex::new(vec![]), info: crate::models::find("anthropic", "claude-haiku-4-5") })
        }
    }

    #[async_trait]
    impl Provider for Scripted {
        fn provider_id(&self) -> &str {
            "scripted"
        }
        fn model_id(&self) -> &str {
            "s1"
        }
        fn model_info(&self) -> Option<&'static crate::models::ModelInfo> {
            self.info
        }
        async fn stream(&self, request: ModelRequest, _cancel: CancellationToken) -> AssistantEventStream {
            self.requests.lock().unwrap().push(request);
            let turn = { let mut script = self.script.lock().unwrap(); if script.is_empty() { Turn::Text("done") } else { script.remove(0) } };
            let (tx, rx) = mpsc::channel(16);
            tokio::spawn(async move {
                match turn {
                    Turn::Fail(message) => {
                        let _ = tx.send(AssistantEvent::Error { message: message.into(), aborted: false }).await;
                    }
                    Turn::Text(text) => {
                        let _ = tx.send(AssistantEvent::Start).await;
                        let _ = tx.send(AssistantEvent::TextStart { index: 0 }).await;
                        let _ = tx.send(AssistantEvent::TextDelta { index: 0, delta: text.into() }).await;
                        let _ = tx.send(AssistantEvent::TextEnd { index: 0 }).await;
                        let _ = tx.send(AssistantEvent::Done { stop_reason: StopReason::Stop, usage: Usage { input: 100, output: 10, total_tokens: 110, ..Usage::default() } }).await;
                    }
                    Turn::Call { name, args } => {
                        let _ = tx.send(AssistantEvent::Start).await;
                        let _ = tx.send(AssistantEvent::ToolCallStart { index: 0, id: "c1".into(), name: name.into() }).await;
                        let _ = tx.send(AssistantEvent::ToolCallDelta { index: 0, delta: args.into() }).await;
                        let _ = tx.send(AssistantEvent::ToolCallEnd { index: 0 }).await;
                        let _ = tx.send(AssistantEvent::Done { stop_reason: StopReason::ToolUse, usage: Usage::default() }).await;
                    }
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
            json!({ "type": "object", "properties": { "text": { "type": "string" } } })
        }
        async fn execute(&self, _id: &str, args: Value, _cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::text(format!("echo: {}", args["text"].as_str().unwrap_or(""))))
        }
    }

    fn collect(mut rx: mpsc::Receiver<HarnessEvent>) -> tokio::task::JoinHandle<Vec<HarnessEvent>> {
        tokio::spawn(async move {
            let mut events = Vec::new();
            while let Some(event) = rx.recv().await {
                events.push(event);
            }
            events
        })
    }

    fn kinds(events: &[HarnessEvent]) -> Vec<String> {
        events.iter().map(|e| serde_json::to_value(e).unwrap()["type"].as_str().unwrap().to_string()).collect()
    }

    #[tokio::test]
    async fn a_prompt_runs_tools_records_usage_and_reports_events() {
        let provider = Scripted::new(vec![Turn::Call { name: "echo", args: r#"{"text":"hi"}"# }, Turn::Text("done")]);
        let mut options = HarnessOptions::new(provider.clone());
        options.tools = vec![Arc::new(Echo)];
        options.system_prompt = "be brief".into();
        let mut harness = AgentHarness::new(options);
        let events = collect(harness.subscribe());
        let result = harness.prompt("go").await.unwrap();
        drop(harness);
        assert_eq!(result.outcome, Outcome::Completed);
        assert_eq!(result.messages.len(), 4, "prompt, call, result, answer");
        let events = events.await.unwrap();
        let kinds = kinds(&events);
        assert_eq!(kinds.first().map(String::as_str), Some("run_start"));
        assert_eq!(kinds.last().map(String::as_str), Some("run_end"));
        assert!(kinds.contains(&"tool_end".to_string()) && kinds.contains(&"usage".to_string()));
        assert!(events.iter().any(|e| matches!(e, HarnessEvent::Usage { totals, .. } if totals.input == 100)));
        assert_eq!(provider.requests.lock().unwrap()[0].system_prompt, "be brief");
    }

    #[tokio::test]
    async fn skills_templates_and_hooks_shape_the_prompt() {
        struct Inject;
        #[async_trait]
        impl super::super::hooks::HarnessHooks for Inject {
            async fn before_run(&self, _prompt: &[AgentMessage], _resources: &Resources) -> Option<Vec<AgentMessage>> {
                Some(vec![AgentMessage::user("(injected)")])
            }
            async fn transform_context(&self, messages: Vec<AgentMessage>, system_prompt: String) -> (Vec<AgentMessage>, String) {
                (messages, format!("{system_prompt} + hooked"))
            }
        }
        let provider = Scripted::new(vec![Turn::Text("a"), Turn::Text("b")]);
        let mut options = HarnessOptions::new(provider.clone());
        options.system_prompt = "base".into();
        options.resources.skills.push(super::super::skills::Skill { name: "deploy".into(), description: "d".into(), content: "Deploy steps".into(), path: "/skills/deploy/SKILL.md".into(), disable_model_invocation: false });
        options.resources.prompt_templates.push(super::super::templates::PromptTemplate { name: "review".into(), description: String::new(), argument_hint: None, content: "Review $1".into() });
        let mut harness = AgentHarness::new(options);
        harness.hooks().register("inject", Arc::new(Inject));
        harness.skill("deploy", Some("carefully")).await.unwrap();
        harness.prompt_from_template("review", &["main.rs".into()]).await.unwrap();
        assert_eq!(harness.skill("nope", None).await.unwrap_err(), HarnessError::UnknownSkill("nope".into()));
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests[0].system_prompt, "base + hooked");
        let LlmMessage::User(first) = &requests[0].messages[0] else { panic!() };
        assert!(first.content[0].as_text().unwrap().starts_with("<skill name=\"deploy\"") && first.content[0].as_text().unwrap().ends_with("carefully"));
        let LlmMessage::User(injected) = &requests[0].messages[1] else { panic!() };
        assert_eq!(injected.content[0].as_text(), Some("(injected)"));
        let LlmMessage::User(review) = &requests[1].messages[3] else { panic!() };
        assert_eq!(review.content[0].as_text(), Some("Review main.rs"));
    }

    #[tokio::test]
    async fn queues_and_follow_up_hooks_extend_a_run() {
        struct Again(Mutex<bool>);
        #[async_trait]
        impl super::super::hooks::HarnessHooks for Again {
            async fn before_run_end(&self, _run_id: &str, _messages: &[AgentMessage]) -> Option<String> {
                let mut asked = self.0.lock().unwrap();
                if *asked { None } else { *asked = true; Some("one more".into()) }
            }
        }
        let provider = Scripted::new(vec![Turn::Text("a"), Turn::Text("b"), Turn::Text("c")]);
        let mut harness = AgentHarness::new(HarnessOptions::new(provider.clone()));
        harness.hooks().register("again", Arc::new(Again(Mutex::new(false))));
        let handle = harness.handle();
        let id = handle.next_run("queued");
        assert_eq!(handle.queued().len(), 1);
        let result = harness.prompt("first").await.unwrap();
        assert_eq!(result.messages.len(), 6, "three prompts and three answers across one result");
        assert!(handle.queued().is_empty());
        assert!(!handle.cancel_queued(&id));
        let texts: Vec<String> = harness.messages().iter().filter_map(|m| match m { AgentMessage::User(u) => u.content[0].as_text().map(str::to_string), _ => None }).collect();
        assert_eq!(texts, vec!["first", "one more", "queued"]);
    }

    #[tokio::test]
    async fn manual_compaction_and_interrupted_runs() {
        let provider = Scripted::with_window(vec![Turn::Text("## Goal\nsummary"), Turn::Text("after")]);
        let mut options = HarnessOptions::new(provider.clone());
        options.compaction.keep_recent_tokens = 1;
        let mut long = Vec::new();
        for i in 0..4 {
            long.push(AgentMessage::user(format!("question {i} {}", "x".repeat(200))));
            let mut a = AssistantMessage::empty("scripted", "s1");
            a.content = vec![AssistantPart::Text { text: format!("answer {i}") }];
            long.push(AgentMessage::Assistant(a));
        }
        options.messages = long;
        let mut harness = AgentHarness::new(options);
        let outcome = harness.compact(Some("keep the numbers")).await.unwrap();
        assert_eq!(outcome.outcome, Outcome::Completed);
        assert!(matches!(harness.messages().first(), Some(AgentMessage::Custom { kind, .. }) if kind == "compaction"));
        assert!(harness.messages().len() < 9);
        assert!(provider.requests.lock().unwrap()[0].messages[0].as_llm_text().contains("Additional focus: keep the numbers"));

        // An interrupted run: the last message is a call without a result.
        let mut call = AssistantMessage::empty("scripted", "s1");
        call.content = vec![AssistantPart::ToolCall(ToolCall { id: "t9".into(), name: "echo".into(), arguments: json!({}) })];
        call.stop_reason = StopReason::ToolUse;
        let mut messages = harness.messages().to_vec();
        messages.push(AgentMessage::Assistant(call));
        harness.set_messages(messages).unwrap();
        let result = harness.continue_run().await.unwrap();
        assert_eq!(result.outcome, Outcome::Completed);
        let last_messages = provider.requests.lock().unwrap().last().unwrap().messages.clone();
        assert!(matches!(last_messages.last(), Some(LlmMessage::ToolResult(r)) if r.tool_call_id == "t9" && r.is_error));
        assert_eq!(harness.continue_run().await.unwrap_err(), HarnessError::LastIsAssistant);
    }

    #[tokio::test]
    async fn an_overflow_error_is_compacted_and_retried_once() {
        let provider = Scripted::with_window(vec![Turn::Fail("400: prompt is too long: 213462 tokens > 200000 maximum"), Turn::Text("## Goal\nS"), Turn::Text("recovered")]);
        let mut options = HarnessOptions::new(provider.clone());
        options.retry.enabled = false;
        options.compaction.keep_recent_tokens = 1;
        options.messages = vec![AgentMessage::user("old ".repeat(50)), { let mut a = AssistantMessage::empty("scripted", "s1"); a.content = vec![AssistantPart::Text { text: "old answer".into() }]; AgentMessage::Assistant(a) }];
        let mut harness = AgentHarness::new(options);
        let events = collect(harness.subscribe());
        let result = harness.prompt("now").await.unwrap();
        drop(harness);
        assert_eq!(result.outcome, Outcome::Completed);
        let events = events.await.unwrap();
        assert!(events.iter().any(|e| matches!(e, HarnessEvent::CompactionEnd { reason: CompactionReason::Overflow, outcome: Outcome::Completed, .. })));
        assert_eq!(provider.requests.lock().unwrap().len(), 3);
    }

    impl LlmMessage {
        fn as_llm_text(&self) -> String {
            match self {
                LlmMessage::User(u) => u.content.iter().filter_map(|p| p.as_text()).collect::<Vec<_>>().join("\n"),
                _ => String::new(),
            }
        }
    }
}
