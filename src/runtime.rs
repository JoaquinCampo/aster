use crate::{
    domain::{AuditEvent, Operation, OperationState, RetryPolicy, Task, TaskState},
    provider::PiAdapter,
    routing::Router,
    store::Store,
};
use anyhow::{Result, bail};
use chrono::Utc;
use std::time::Duration;
use uuid::Uuid;

pub struct Runtime<A: PiAdapter> {
    pub store: Store,
    pub adapter: A,
    router: Router,
    concurrency: usize,
}
impl<A: PiAdapter> Runtime<A> {
    pub fn new(store: Store, adapter: A) -> Self {
        Self {
            store,
            adapter,
            router: Router::default(),
            concurrency: 4,
        }
    }
    pub fn with_concurrency(mut self, n: usize) -> Self {
        self.concurrency = n.max(1);
        self
    }
    fn event(&self, t: &Task, k: &str, d: impl Into<String>) -> Result<()> {
        self.store.append(&AuditEvent {
            id: Uuid::new_v4(),
            task_id: t.id,
            kind: k.into(),
            detail: d.into(),
            at: Utc::now(),
        })
    }
    pub fn submit(&self, prompt: String) -> Result<Task> {
        self.submit_with(prompt, vec![], RetryPolicy::default(), None, None)
    }
    pub fn submit_with(
        &self,
        prompt: String,
        dependencies: Vec<Uuid>,
        retry: RetryPolicy,
        timeout_ms: Option<u64>,
        token_budget: Option<u64>,
    ) -> Result<Task> {
        for id in &dependencies {
            if self.store.task(*id)?.is_none() {
                bail!("dependency {id} does not exist")
            }
        }
        let mut t = Task::new(prompt, self.router.route("placeholder"));
        t.route = self.router.route(&t.prompt);
        t.dependencies = dependencies;
        t.retry = retry;
        t.timeout_ms = timeout_ms;
        t.token_budget = token_budget;
        self.store.save_task(&t)?;
        self.event(&t, "route.selected", &t.route.rationale)?;
        self.event(&t, "task.queued", "Task durably queued")?;
        Ok(t)
    }
    pub async fn run(&self, mut task: Task) -> Result<Task> {
        if task.state != TaskState::Queued {
            bail!("task is not queued")
        }
        for id in &task.dependencies {
            if self
                .store
                .task(*id)?
                .is_none_or(|d| d.state != TaskState::Succeeded)
            {
                return Ok(task);
            }
        }
        loop {
            task.attempts += 1;
            task.state = TaskState::Running;
            task.updated_at = Utc::now();
            self.store.save_task(&task)?;
            let mut op = Operation {
                id: Uuid::new_v4(),
                task_id: task.id,
                attempt: task.attempts,
                state: OperationState::IntentRecorded,
                retry_safe: false,
                started_at: Utc::now(),
                completed_at: None,
            };
            self.store.create_operation(&op)?;
            self.event(&task, "operation.intent", op.id.to_string())?;
            op.state = OperationState::Running;
            self.store.save_operation(&op)?;
            self.event(&task, "operation.started", op.id.to_string())?;
            let execution = async {
                match task.timeout_ms {
                    Some(ms) => match tokio::time::timeout(
                        Duration::from_millis(ms),
                        self.adapter.execute(&task.prompt, &task.route),
                    )
                    .await
                    {
                        Ok(v) => v,
                        Err(_) => {
                            task.state = TaskState::TimedOut;
                            return Ok(None);
                        }
                    },
                    None => self.adapter.execute(&task.prompt, &task.route).await,
                }
                .map(Some)
            }
            .await;
            match execution {
                Ok(Some(result))
                    if task.token_budget.is_some_and(|b| {
                        task.tokens_used.saturating_add(result.usage_tokens) > b
                    }) =>
                {
                    task.tokens_used = task.tokens_used.saturating_add(result.usage_tokens);
                    task.state = TaskState::Failed;
                    task.failure_reason = Some("token budget exceeded".into());
                    op.state = OperationState::Failed;
                }
                Ok(Some(result)) => {
                    task.tokens_used += result.usage_tokens;
                    task.output = Some(result.output.clone());
                    if result.output.trim().is_empty() {
                        task.verification = Some("FAIL: output is empty".into());
                        task.failure_reason = Some("empty output failed verification".into());
                        task.state = TaskState::Failed;
                        op.state = OperationState::Failed;
                    } else {
                        task.verification = Some("PASS: output is non-empty".into());
                        task.state = TaskState::Succeeded;
                        op.state = OperationState::Succeeded;
                    }
                }
                Ok(None) => {
                    task.failure_reason = Some("execution timeout exceeded".into());
                    op.state = OperationState::Failed;
                }
                Err(e) => {
                    task.state = TaskState::Failed;
                    task.failure_reason = Some(e.to_string());
                    task.verification = Some(format!("FAIL: {e}"));
                    op.state = OperationState::Failed;
                }
            }
            op.completed_at = Some(Utc::now());
            self.store.save_operation(&op)?;
            task.updated_at = Utc::now();
            self.store.save_task(&task)?;
            self.event(
                &task,
                "operation.completed",
                format!("{}: {:?}", op.id, task.state),
            )?;
            if task.state == TaskState::Failed && task.attempts < task.retry.max_attempts {
                let shift = (task.attempts - 1).min(63);
                let delay = task
                    .retry
                    .initial_backoff_ms
                    .saturating_mul(1u64 << shift)
                    .min(task.retry.max_backoff_ms);
                task.state = TaskState::Queued;
                self.store.save_task(&task)?;
                self.event(&task, "task.retry_scheduled", format!("backoff_ms={delay}"))?;
                tokio::time::sleep(Duration::from_millis(delay)).await;
                continue;
            }
            return Ok(task);
        }
    }
    pub async fn run_ready(&self) -> Result<Vec<Task>> {
        let mut out = Vec::new();
        loop {
            let all = self.store.tasks()?;
            let ready: Vec<_> = all
                .iter()
                .filter(|t| {
                    t.state == TaskState::Queued
                        && t.dependencies.iter().all(|id| {
                            all.iter()
                                .any(|d| d.id == *id && d.state == TaskState::Succeeded)
                        })
                })
                .take(self.concurrency)
                .cloned()
                .collect();
            if ready.is_empty() {
                break;
            }
            for t in ready {
                out.push(self.run(t).await?)
            }
        }
        Ok(out)
    }
    pub fn pause(&mut self, id: Uuid) -> Result<Task> {
        self.store.transition(
            id,
            &[TaskState::Queued],
            TaskState::Paused,
            "task.paused",
            "no new operation will start",
        )
    }
    pub fn resume(&mut self, id: Uuid) -> Result<Task> {
        self.store.transition(
            id,
            &[TaskState::Paused],
            TaskState::Queued,
            "task.resumed",
            "task returned to queue",
        )
    }
    pub fn cancel(&mut self, id: Uuid) -> Result<Task> {
        self.store.transition(
            id,
            &[
                TaskState::Queued,
                TaskState::Paused,
                TaskState::OutcomeUnknown,
            ],
            TaskState::Cancelled,
            "task.cancelled",
            "cancelled at safe boundary",
        )
    }
    pub fn reconcile(&mut self, id: Uuid, succeeded: bool) -> Result<Task> {
        let to = if succeeded {
            TaskState::Succeeded
        } else {
            TaskState::Failed
        };
        self.store.transition(
            id,
            &[TaskState::OutcomeUnknown],
            to,
            "operation.reconciled",
            if succeeded {
                "confirmed succeeded"
            } else {
                "confirmed failed"
            },
        )
    }
    pub fn recover(&mut self) -> Result<usize> {
        self.store.recover()
    }
}
