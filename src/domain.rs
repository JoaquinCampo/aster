use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskState {
    Queued,
    Running,
    Pausing,
    Paused,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    OutcomeUnknown,
}

impl TaskState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::TimedOut
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    pub role: String,
    pub model: String,
    pub effort: String,
    pub context_budget: u32,
    pub capabilities: Vec<String>,
    pub isolation: Vec<String>,
    pub verification: String,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
}
impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            initial_backoff_ms: 100,
            max_backoff_ms: 30_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub prompt: String,
    pub state: TaskState,
    pub route: Route,
    pub output: Option<String>,
    pub verification: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub dependencies: Vec<Uuid>,
    #[serde(default)]
    pub retry: RetryPolicy,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub token_budget: Option<u64>,
    #[serde(default)]
    pub tokens_used: u64,
    #[serde(default)]
    pub failure_reason: Option<String>,
}

impl Task {
    pub fn new(prompt: String, route: Route) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            prompt,
            state: TaskState::Queued,
            route,
            output: None,
            verification: None,
            created_at: now,
            updated_at: now,
            dependencies: vec![],
            retry: RetryPolicy::default(),
            attempts: 0,
            timeout_ms: None,
            token_budget: None,
            tokens_used: 0,
            failure_reason: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub id: Uuid,
    pub task_id: Uuid,
    pub attempt: u32,
    pub state: OperationState,
    pub retry_safe: bool,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum OperationState {
    IntentRecorded,
    Running,
    Succeeded,
    Failed,
    OutcomeUnknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: Uuid,
    pub task_id: Uuid,
    pub kind: String,
    pub detail: String,
    pub at: DateTime<Utc>,
}
