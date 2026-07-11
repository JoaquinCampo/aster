use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// A stable, serializable execution role. Custom roles keep routing extensible.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    Orchestrator,
    Implementer,
    Reviewer,
    Researcher,
    Tester,
    Custom(String),
}
impl Role {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Orchestrator => "orchestrator",
            Self::Implementer => "implementer",
            Self::Reviewer => "reviewer",
            Self::Researcher => "researcher",
            Self::Tester => "tester",
            Self::Custom(v) => v,
        }
    }
}
impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
impl PartialEq<&str> for Role {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
}
impl fmt::Display for Effort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        })
    }
}
impl PartialEq<&str> for Effort {
    fn eq(&self, other: &&str) -> bool {
        match self {
            Self::Low => *other == "low",
            Self::Medium => *other == "medium",
            Self::High => *other == "high",
        }
    }
}

/// Independent controls: changing model does not silently alter permissions or budgets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionDimensions {
    pub effort: Effort,
    pub context_tokens: u32,
    pub output_tokens: u32,
    pub max_latency_ms: u64,
    pub capabilities: Vec<String>,
    pub isolation: Vec<String>,
    pub verification: String,
}

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
    pub role: Role,
    pub model: String,
    pub dimensions: ExecutionDimensions,
    pub rationale: String,
    pub decision_id: String,
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
    TimedOut,
    Cancelled,
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
