use serde::{Deserialize, Serialize};

/// A task outlives the Jobs that work on it. Its initial Runner arbitrates all revisions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub id: String,
    pub revision: u64,
    pub authority_runner_id: String,
    pub owner_bot_id: String,
    pub runner_id: String,
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub next_action: String,
    pub chat_ids: Vec<String>,
    #[serde(default)]
    pub links: Vec<TaskLink>,
    pub state: TaskState,
    pub reason: Option<String>,
    pub result: Option<String>,
    #[serde(default)]
    pub evidence: Vec<TaskEvidence>,
    pub active_run: Option<TaskRun>,
    #[serde(default)]
    pub history: Vec<TaskChange>,
    pub created_at: f64,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    Working,
    Blocked,
    AwaitingReview,
    Completed,
    Cancelled,
}

impl TaskState {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskLink {
    pub label: String,
    pub url: String,
}

/// References immutable records. Evidence is a claim, not an approval or proof of truth.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskEvidence {
    pub kind: EvidenceKind,
    pub label: String,
    pub chat_id: Option<String>,
    pub message_id: Option<String>,
    pub attachment_id: Option<String>,
    pub url: Option<String>,
    pub output_id: Option<String>,
    pub version: Option<u64>,
    pub review_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Message,
    Output,
    File,
    Url,
    Review,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskRun {
    pub id: String,
    pub bot_id: String,
    pub runner_id: String,
    pub chat_id: String,
    pub started_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskChange {
    pub revision: u64,
    pub owner_bot_id: String,
    pub runner_id: String,
    pub state: TaskState,
    pub reason: Option<String>,
    pub at: f64,
}

impl Task {
    pub(super) fn record_change(&mut self) {
        self.history.push(TaskChange {
            revision: self.revision,
            owner_bot_id: self.owner_bot_id.clone(),
            runner_id: self.runner_id.clone(),
            state: self.state,
            reason: self.reason.clone(),
            at: self.updated_at,
        });
    }
}
