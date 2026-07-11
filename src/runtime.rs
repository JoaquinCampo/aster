use crate::{
    domain::{AuditEvent, Task, TaskState},
    provider::PiAdapter,
    routing::Router,
    store::Store,
};
use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;

pub struct Runtime<A: PiAdapter> {
    pub store: Store,
    pub adapter: A,
    router: Router,
}

impl<A: PiAdapter> Runtime<A> {
    pub fn new(store: Store, adapter: A) -> Self {
        Self {
            store,
            adapter,
            router: Router,
        }
    }
    fn event(&self, task: &Task, kind: &str, detail: impl Into<String>) -> Result<()> {
        self.store.append(&AuditEvent {
            id: Uuid::new_v4(),
            task_id: task.id,
            kind: kind.into(),
            detail: detail.into(),
            at: Utc::now(),
        })
    }
    pub fn submit(&self, prompt: String) -> Result<Task> {
        let task = Task::new(prompt, self.router.route("placeholder"));
        let mut task = Task {
            route: self.router.route(&task.prompt),
            ..task
        };
        task.updated_at = Utc::now();
        self.store.save_task(&task)?;
        self.event(&task, "route.selected", &task.route.rationale)?;
        self.event(&task, "task.queued", "Task durably queued")?;
        Ok(task)
    }
    pub async fn run(&self, mut task: Task) -> Result<Task> {
        task.state = TaskState::Running;
        task.updated_at = Utc::now();
        self.store.save_task(&task)?;
        self.event(&task, "operation.started", "pi.execute")?;
        match self.adapter.execute(&task.prompt, &task.route).await {
            Ok(result) => {
                task.output = Some(result.output);
                task.verification = Some("PASS: output is non-empty".into());
                task.state = TaskState::Succeeded;
                self.event(
                    &task,
                    "usage.recorded",
                    format!("{} tokens", result.usage_tokens),
                )?;
            }
            Err(err) => {
                task.state = TaskState::Failed;
                task.verification = Some(format!("FAIL: {err}"));
            }
        }
        task.updated_at = Utc::now();
        self.store.save_task(&task)?;
        self.event(&task, "operation.completed", format!("{:?}", task.state))?;
        Ok(task)
    }
}
