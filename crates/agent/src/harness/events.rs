//! What a harness tells the outside about a run, after pi's harness events: the loop's own
//! events plus runs, retries, compaction, queues, configuration, and usage.

use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::provider::AssistantEvent;
use crate::tool::ToolResult;
use crate::types::{AgentMessage, AssistantMessage, ToolResultMessage, Usage};

use super::hooks::CompactionReason;

/// How a run or a compaction ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    Declined,
    Aborted,
    Failed { error: String },
}

/// A message waiting in a queue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueuedItem {
    pub id: String,
    pub kind: QueueKind,
    pub message: AgentMessage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueKind {
    Steer,
    FollowUp,
    NextRun,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HarnessEvent {
    RunStart { run_id: String },
    RunEnd { run_id: String, outcome: Outcome },
    TurnStart { run_id: String },
    TurnEnd { run_id: String, message: AssistantMessage, tool_results: Vec<ToolResultMessage> },
    RetryScheduled { run_id: String, attempt: u32, max_attempts: u32, delay_ms: u64, error: String },
    MessageStart { run_id: String, message: AgentMessage },
    MessageUpdate { run_id: String, message: AgentMessage, event: AssistantEvent },
    MessageEnd { run_id: String, message: AgentMessage },
    ToolStart { run_id: String, tool_call_id: String, tool_name: String, args: Value },
    ToolUpdate { run_id: String, tool_call_id: String, tool_name: String, partial_result: ToolResult },
    ToolEnd { run_id: String, tool_call_id: String, tool_name: String, result: ToolResult, is_error: bool },
    QueueUpdate { queues: Vec<QueuedItem> },
    /// A setting changed; `property` names it and `value` is its new JSON.
    ConfigUpdate { property: String, value: Value },
    CompactionStart { run_id: String, reason: CompactionReason },
    CompactionEnd { run_id: String, reason: CompactionReason, outcome: Outcome, tokens_before: Option<u64> },
    /// A model call's usage, and the totals since the harness was made.
    Usage { usage: Usage, totals: Usage },
    /// A hook or listener failed; the run goes on.
    HandlerError { hook: String, error: String },
}

/// A listener the bus awaits before it goes on, for state that must be in step with the run.
#[async_trait]
pub trait HarnessListener: Send + Sync {
    async fn on_event(&self, event: &HarnessEvent);
}

/// Delivers events to awaited listeners first, then to every subscribed channel. A closed
/// channel is dropped; a full one makes the run wait.
#[derive(Default)]
pub struct EventBus {
    listeners: Mutex<Vec<std::sync::Arc<dyn HarnessListener>>>,
    subscribers: Mutex<Vec<mpsc::Sender<HarnessEvent>>>,
}

impl EventBus {
    pub fn subscribe(&self) -> mpsc::Receiver<HarnessEvent> {
        let (tx, rx) = mpsc::channel(256);
        self.subscribers.lock().unwrap().push(tx);
        rx
    }

    pub fn listen(&self, listener: std::sync::Arc<dyn HarnessListener>) {
        self.listeners.lock().unwrap().push(listener);
    }

    pub async fn emit(&self, event: HarnessEvent) {
        let listeners: Vec<_> = self.listeners.lock().unwrap().clone();
        for listener in listeners {
            listener.on_event(&event).await;
        }
        let subscribers: Vec<_> = self.subscribers.lock().unwrap().clone();
        let mut closed = Vec::new();
        for (index, subscriber) in subscribers.iter().enumerate() {
            if subscriber.send(event.clone()).await.is_err() {
                closed.push(index);
            }
        }
        if !closed.is_empty() {
            let mut live = self.subscribers.lock().unwrap();
            let mut index = 0;
            live.retain(|_| {
                let keep = !closed.contains(&index);
                index += 1;
                keep
            });
        }
    }
}
