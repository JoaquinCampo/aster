use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskState {
    Queued,
    Running,
    Paused,
    Succeeded,
    Failed,
    Cancelled,
    OutcomeUnknown,
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
pub struct Task {
    pub id: Uuid,
    pub prompt: String,
    pub state: TaskState,
    pub route: Route,
    pub output: Option<String>,
    pub verification: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
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
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: Uuid,
    pub task_id: Uuid,
    pub kind: String,
    pub detail: String,
    pub at: DateTime<Utc>,
}
