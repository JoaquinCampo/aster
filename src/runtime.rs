use crate::{
    domain::{AuditEvent, Operation, OperationState, RetryPolicy, Route, Task, TaskState},
    provider::PiAdapter,
    routing::Router,
    store::Store,
};
use anyhow::{Result, bail};
use chrono::Utc;
use futures_util::future::join_all;
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
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
    fn event(&self, t: &Task, kind: &str, detail: impl Into<String>) -> Result<()> {
        self.store.append(&AuditEvent {
            id: Uuid::new_v4(),
            task_id: t.id,
            kind: kind.into(),
            detail: detail.into(),
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
        if dependencies.iter().collect::<HashSet<_>>().len() != dependencies.len() {
            bail!("duplicate dependency")
        }
        for id in &dependencies {
            if self.store.task(*id)?.is_none() {
                bail!("dependency {id} does not exist")
            }
        }
        let mut task = Task::new(prompt, self.router.route("placeholder"));
        task.route = self.router.route(&task.prompt);
        task.dependencies = dependencies;
        task.retry = retry;
        task.timeout_ms = timeout_ms;
        task.token_budget = token_budget;
        self.store.create_task(
            &task,
            &[
                ("route.selected", &task.route.rationale),
                ("task.queued", "Task durably queued"),
            ],
        )?;
        Ok(task)
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
            let mut op = Operation {
                id: Uuid::new_v4(),
                task_id: task.id,
                attempt: task.attempts,
                state: OperationState::IntentRecorded,
                retry_safe: false,
                started_at: Utc::now(),
                completed_at: None,
            };
            self.store.start_operation(&task, &op)?;
            op.state = OperationState::Running;
            self.store.save_operation(&op)?;
            self.event(&task, "operation.started", op.id.to_string())?;
            let result = match task.timeout_ms {
                Some(ms) => match tokio::time::timeout(
                    Duration::from_millis(ms),
                    self.adapter.execute(&task.prompt, &task.route),
                )
                .await
                {
                    Ok(v) => v.map(Some),
                    Err(_) => Ok(None),
                },
                None => self
                    .adapter
                    .execute(&task.prompt, &task.route)
                    .await
                    .map(Some),
            };
            match result {
                Ok(None) => {
                    task.state = TaskState::TimedOut;
                    task.failure_reason = Some("execution timeout exceeded".into());
                    op.state = OperationState::TimedOut;
                }
                Err(e) => {
                    task.state = TaskState::Failed;
                    task.failure_reason = Some(e.to_string());
                    task.verification = Some(format!("FAIL: {e}"));
                    op.state = OperationState::Failed;
                }
                Ok(Some(r)) => {
                    task.tokens_used = task.tokens_used.saturating_add(r.usage_tokens);
                    task.output = Some(r.output.clone());
                    if task.token_budget.is_some_and(|b| task.tokens_used > b) {
                        task.state = TaskState::Failed;
                        task.failure_reason = Some("token budget exceeded".into());
                        op.state = OperationState::Failed;
                    } else if r.output.trim().is_empty() {
                        task.state = TaskState::Failed;
                        task.failure_reason = Some("empty output failed verification".into());
                        task.verification = Some("FAIL: output is empty".into());
                        op.state = OperationState::Failed;
                    } else {
                        task.state = TaskState::Succeeded;
                        task.verification = Some("PASS: output is non-empty".into());
                        op.state = OperationState::Succeeded;
                    }
                }
            }
            op.completed_at = Some(Utc::now());
            task.updated_at = Utc::now();
            self.store.finish_operation(&task, &op)?;
            if task.state == TaskState::Failed && task.attempts < task.retry.max_attempts {
                let delay = task
                    .retry
                    .initial_backoff_ms
                    .saturating_mul(1u64 << (task.attempts - 1).min(63))
                    .min(task.retry.max_backoff_ms);
                task.state = TaskState::Queued;
                self.store.save_task(&task)?;
                self.event(&task, "task.retry_scheduled", format!("backoff_ms={delay}"))?;
                tokio::time::sleep(Duration::from_millis(delay)).await;
            } else {
                return Ok(task);
            }
        }
    }
    pub async fn run_ready(&self) -> Result<Vec<Task>> {
        let mut out = Vec::new();
        loop {
            self.fail_impossible_dependencies(&mut out)?;
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
                let queued: Vec<_> = all
                    .into_iter()
                    .filter(|t| t.state == TaskState::Queued)
                    .collect();
                if !queued.is_empty() {
                    for mut t in queued {
                        t.state = TaskState::Failed;
                        t.failure_reason = Some("dependency cycle".into());
                        self.store.save_task_with_event(
                            &t,
                            "task.dependency_cycle",
                            "no schedulable root",
                        )?;
                        out.push(t);
                    }
                }
                break;
            }
            for r in join_all(ready.into_iter().map(|t| self.run(t))).await {
                out.push(r?);
            }
        }
        Ok(out)
    }
    fn fail_impossible_dependencies(&self, out: &mut Vec<Task>) -> Result<()> {
        let all = self.store.tasks()?;
        let states: HashMap<_, _> = all.iter().map(|t| (t.id, t.state)).collect();
        for mut t in all.into_iter().filter(|t| t.state == TaskState::Queued) {
            if t.dependencies.iter().any(|id| {
                states
                    .get(id)
                    .is_none_or(|s| s.is_terminal() && *s != TaskState::Succeeded)
            }) {
                t.state = TaskState::Failed;
                t.failure_reason = Some("dependency cannot succeed".into());
                self.store.save_task_with_event(
                    &t,
                    "task.dependency_failed",
                    "dependency cannot succeed",
                )?;
                out.push(t);
            }
        }
        Ok(())
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
    pub fn retry(&mut self, id: Uuid) -> Result<Task> {
        self.store.transition(
            id,
            &[TaskState::Failed, TaskState::TimedOut, TaskState::Cancelled],
            TaskState::Queued,
            "task.retry_requested",
            "operator requested retry",
        )
    }
    pub fn override_route(&mut self, id: Uuid, route: Route) -> Result<Task> {
        let mut t = self
            .store
            .task(id)?
            .ok_or_else(|| anyhow::anyhow!("task not found"))?;
        t.route = route;
        self.store.save_task_with_event(
            &t,
            "route.overridden",
            "operator changed execution route",
        )?;
        Ok(t)
    }
    pub fn override_retry(&mut self, id: Uuid, retry: RetryPolicy) -> Result<Task> {
        let mut t = self
            .store
            .task(id)?
            .ok_or_else(|| anyhow::anyhow!("task not found"))?;
        t.retry = retry;
        self.store
            .save_task_with_event(&t, "retry.overridden", "operator changed retry policy")?;
        Ok(t)
    }
    pub fn reconcile_operation(
        &mut self,
        task_id: Uuid,
        operation_id: Uuid,
        outcome: OperationState,
    ) -> Result<Task> {
        self.store
            .reconcile_operation(task_id, operation_id, outcome)
    }
    pub fn reconcile(&mut self, id: Uuid, succeeded: bool) -> Result<Task> {
        let op = self
            .store
            .operations_for(id)?
            .into_iter()
            .find(|o| o.state == OperationState::OutcomeUnknown)
            .ok_or_else(|| anyhow::anyhow!("unknown operation not found"))?;
        self.reconcile_operation(
            id,
            op.id,
            if succeeded {
                OperationState::Succeeded
            } else {
                OperationState::Failed
            },
        )
    }
    pub fn recover(&mut self) -> Result<usize> {
        self.store.recover()
    }
}
