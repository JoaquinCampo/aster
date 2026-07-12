use crate::{
    domain::{
        Artifact, AuditEvent, Checkpoint, ExecutionMode, Operation, OperationState, RetryPolicy,
        Route, Task, TaskState, TerminalReason,
    },
    hooks::{HookTrigger, LifecycleHooks},
    provider::PiAdapter,
    routing::{Router, RoutingDecision, RoutingRequest, UserOverrides},
    routing_policy::OutcomeAggregate,
    store::Store,
};
use anyhow::{Result, bail};
use chrono::Utc;
use futures_util::future::join_all;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use uuid::Uuid;

pub struct Runtime<A: PiAdapter> {
    pub store: Store,
    pub adapter: A,
    router: Router,
    concurrency: usize,
    hooks: Option<Arc<dyn LifecycleHooks>>,
    default_retry: RetryPolicy,
    default_timeout_ms: Option<u64>,
    default_token_budget: Option<u64>,
}
impl<A: PiAdapter> Runtime<A> {
    pub fn new(store: Store, adapter: A) -> Self {
        Self {
            store,
            adapter,
            router: Router::default(),
            concurrency: 4,
            hooks: None,
            default_retry: RetryPolicy::default(),
            default_timeout_ms: None,
            default_token_budget: None,
        }
    }
    pub fn from_config(store: Store, adapter: A, config: &crate::config::Config) -> Result<Self> {
        config.validate()?;
        let router = if config.routing.enabled {
            config
                .routing
                .policy_path
                .as_ref()
                .map(Router::from_policy_path)
                .transpose()?
                .unwrap_or_default()
        } else {
            Router::default()
        };
        Ok(Self {
            store,
            adapter,
            router,
            concurrency: config.lifecycle.concurrency,
            hooks: None,
            default_retry: RetryPolicy {
                max_attempts: config.lifecycle.retry_limit.saturating_add(1),
                ..RetryPolicy::default()
            },
            default_timeout_ms: config
                .budgets
                .timeout_ms
                .or(config.lifecycle.task_timeout_ms),
            default_token_budget: config.budgets.token_budget,
        })
    }
    pub fn concurrency(&self) -> usize {
        self.concurrency
    }
    pub fn router(&self) -> &Router {
        &self.router
    }
    pub fn with_hooks(mut self, hooks: Arc<dyn LifecycleHooks>) -> Self {
        self.hooks = Some(hooks);
        self
    }
    fn hook(&self, task: &Task, trigger: HookTrigger, phase: &str) -> Result<()> {
        let Some(hooks) = &self.hooks else {
            return Ok(());
        };
        let outcomes = hooks.invoke(
            trigger,
            serde_json::json!({
                "task_id": task.id,
                "phase": phase,
                "state": task.state,
                "attempt": task.attempts,
            }),
        )?;
        self.event(
            task,
            "hook.invoked",
            format!(
                "trigger={trigger:?} phase={phase} outcomes={}",
                outcomes.len()
            ),
        )
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
        self.submit_with_overrides(
            prompt,
            UserOverrides::default(),
            vec![],
            self.default_retry.clone(),
            self.default_timeout_ms,
            self.default_token_budget,
        )
    }
    /// Queue durable background work without waiting for execution.
    pub fn submit_background(&self, prompt: String) -> Result<Task> {
        self.submit(prompt)
    }
    /// Execute foreground work through the same durable scheduler and lifecycle.
    pub async fn run_foreground(&self, prompt: String) -> Result<Task> {
        let mut task = self.submit(prompt)?;
        task.execution_mode = ExecutionMode::Foreground;
        self.store.save_task_with_event(
            &task,
            "task.foreground",
            "foreground caller waiting on durable execution",
        )?;
        self.run(task).await
    }
    /// Selects and durably records a route with explicit, independently applied user overrides.
    pub fn submit_with_overrides(
        &self,
        prompt: String,
        overrides: UserOverrides,
        dependencies: Vec<Uuid>,
        retry: RetryPolicy,
        timeout_ms: Option<u64>,
        token_budget: Option<u64>,
    ) -> Result<Task> {
        let substantive = prompt.len() > 120
            || ["implement", "refactor", "review", "debug"]
                .iter()
                .any(|word| prompt.to_ascii_lowercase().contains(word));
        let base_quality = if substantive { 75 } else { 50 };
        let mut required_quality = base_quality;
        let initial = self
            .router
            .decide(RoutingRequest {
                estimated_tokens: prompt.len() as u32 * 2,
                prompt: prompt.clone(),
                required_quality,
                overrides: overrides.clone(),
            })
            .map_err(anyhow::Error::new)?;
        let outcomes = self.store.routing_outcomes(self.router.policy_revision)?;
        let history: Vec<bool> = outcomes
            .iter()
            .filter(|o| o.role == initial.route.role)
            .flat_map(|o| {
                let failures = o.failures.min(usize::MAX as u64) as usize;
                let successes = o.verified_successes.min(usize::MAX as u64) as usize;
                std::iter::repeat_n(false, failures).chain(std::iter::repeat_n(true, successes))
            })
            .collect();
        required_quality = Router::adapt_quality(required_quality, &history);
        let mut decision = self
            .router
            .decide(RoutingRequest {
                estimated_tokens: prompt.len() as u32 * 2,
                prompt: prompt.clone(),
                required_quality,
                overrides: overrides.clone(),
            })
            .map_err(anyhow::Error::new)?;
        if required_quality != base_quality {
            let direction = if required_quality > base_quality {
                "escalated"
            } else {
                "de-escalated"
            };
            decision.route.rationale.push_str(&format!(
                "; complete persisted revision {} history {direction} quality from {base_quality} to {required_quality}",
                self.router.policy_revision
            ));
        }
        self.submit_decision(
            prompt,
            decision,
            overrides,
            dependencies,
            retry,
            timeout_ms,
            token_budget,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn submit_decision(
        &self,
        prompt: String,
        decision: RoutingDecision,
        overrides: UserOverrides,
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
        let mut task = Task::new(prompt, decision.route);
        task.dependencies = dependencies;
        task.retry = retry;
        task.timeout_ms = timeout_ms;
        task.token_budget = token_budget;
        let trace = serde_json::json!({
            "decision_id": task.route.decision_id,
            "policy_revision": self.router.policy_revision,
            "route": task.route,
            "signals": decision.evidence,
            "budgets": {"token_budget": token_budget, "timeout_ms": timeout_ms},
            "overrides": overrides,
        })
        .to_string();
        self.store.create_task(
            &task,
            &[
                ("route.selected", trace.as_str()),
                ("task.queued", "Task durably queued"),
            ],
        )?;
        if task.route.rationale.contains("history de-escalated") {
            self.event(&task, "route.deescalated", &task.route.rationale)?;
        } else if task.route.rationale.contains("history escalated") {
            self.event(&task, "route.escalated", &task.route.rationale)?;
        }
        Ok(task)
    }
    pub fn submit_with(
        &self,
        prompt: String,
        dependencies: Vec<Uuid>,
        retry: RetryPolicy,
        timeout_ms: Option<u64>,
        token_budget: Option<u64>,
    ) -> Result<Task> {
        self.submit_with_overrides(
            prompt,
            UserOverrides::default(),
            dependencies,
            retry,
            timeout_ms,
            token_budget,
        )
    }
    fn record_routing_outcome(&self, task: &Task, latency_ms: u64) -> Result<()> {
        let mut outcome = self
            .store
            .routing_outcomes(self.router.policy_revision)?
            .into_iter()
            .find(|o| o.role == task.route.role && o.model == task.route.model)
            .unwrap_or(OutcomeAggregate {
                policy_revision: self.router.policy_revision,
                role: task.route.role.clone(),
                model: task.route.model.clone(),
                attempts: 0,
                verified_successes: 0,
                failures: 0,
                total_cost_micros: 0,
                total_latency_ms: 0,
            });
        outcome.attempts += 1;
        outcome.total_latency_ms = outcome.total_latency_ms.saturating_add(latency_ms);
        if task.state == TaskState::Succeeded {
            outcome.verified_successes += 1;
        } else {
            outcome.failures += 1;
        }
        self.store.save_routing_outcome(&outcome)?;
        self.event(
            task,
            "route.outcome",
            serde_json::to_string(&serde_json::json!({
                "policy_revision": self.router.policy_revision,
                "decision_id": task.route.decision_id,
                "role": task.route.role,
                "model": task.route.model,
                "verified": task.state == TaskState::Succeeded,
                "state": task.state,
                "attempt": task.attempts,
                "tokens": task.tokens_used,
                "latency_ms": latency_ms,
            }))?,
        )
    }
    fn checkpoint(&self, task: &Task, op: &Operation, phase: &str, payload: String) -> Result<()> {
        let checkpoint = Checkpoint {
            id: Uuid::new_v4(),
            task_id: task.id,
            attempt: op.attempt,
            operation_id: op.id,
            phase: phase.into(),
            digest: format!("sha256:{:x}", Sha256::digest(payload.as_bytes())),
            payload,
            created_at: Utc::now(),
        };
        self.store.save_checkpoint(&checkpoint)?;
        self.event(
            task,
            "checkpoint.saved",
            format!(
                "id={} operation={} attempt={} phase={} payload_ref=checkpoint:{}",
                checkpoint.id, op.id, op.attempt, phase, checkpoint.id
            ),
        )
    }
    fn persist_output_artifact(&self, task: &Task, op: &Operation, output: &str) -> Result<()> {
        let content = output.as_bytes().to_vec();
        let artifact = Artifact {
            id: Uuid::new_v4(),
            task_id: task.id,
            attempt: op.attempt,
            operation_id: op.id,
            name: "provider-output.txt".into(),
            media_type: "text/plain; charset=utf-8".into(),
            digest: format!("sha256:{:x}", Sha256::digest(&content)),
            content,
            provenance: format!(
                "provider:{};decision:{}",
                task.route.model, task.route.decision_id
            ),
            created_at: Utc::now(),
        };
        self.store.save_artifact(&artifact)?;
        self.event(
            task,
            "artifact.persisted",
            format!(
                "id={} operation={} attempt={} name={} payload_ref=artifact:{} provenance={}",
                artifact.id, op.id, op.attempt, artifact.name, artifact.id, artifact.provenance
            ),
        )
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
            let recorded_at = Utc::now();
            let isolation = self
                .adapter
                .launch_isolation(&task.route)
                .into_iter()
                .map(|mut record| {
                    record.task_id = task.id;
                    record.attempt = op.attempt;
                    record.operation_id = op.id;
                    record.recorded_at = recorded_at;
                    record
                })
                .collect::<Vec<_>>();
            self.store.save_execution_isolation(&isolation)?;
            self.event(
                &task,
                "isolation.recorded",
                format!(
                    "operation={} attempt={} dimensions=6 source=adapter-launch",
                    op.id, op.attempt
                ),
            )?;
            let inputs = self.store.dependency_artifacts(&task)?;
            self.checkpoint(&task, &op, "operation-intent", serde_json::json!({"state": task.state, "dependency_artifacts": inputs.iter().map(|a| &a.digest).collect::<Vec<_>>()}).to_string())?;
            if !inputs.is_empty() {
                self.event(
                    &task,
                    "artifact.inputs_resolved",
                    format!(
                        "count={} payload_refs={}",
                        inputs.len(),
                        inputs
                            .iter()
                            .map(|a| a.id.to_string())
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                )?;
            }
            self.hook(&task, HookTrigger::BeforeTask, "attempt_started")?;
            op.state = OperationState::Running;
            self.store.save_operation(&op)?;
            self.event(&task, "operation.started", op.id.to_string())?;
            self.hook(&task, HookTrigger::BeforeTool, "provider_execute")?;
            let remaining_ms = task
                .timeout_ms
                .map(|budget| budget.saturating_sub(task.elapsed_ms));
            if remaining_ms == Some(0) {
                task.state = TaskState::TimedOut;
                task.failure_reason = Some("cumulative execution timeout exceeded".into());
                task.terminal_reason = Some(TerminalReason::TimeoutExceeded);
                op.state = OperationState::TimedOut;
                op.completed_at = Some(Utc::now());
                self.store.finish_operation(&task, &op)?;
                return Ok(task);
            }
            let result = match remaining_ms {
                Some(ms) => match tokio::time::timeout(
                    Duration::from_millis(ms),
                    self.adapter.execute_controlled(
                        &task.prompt,
                        &task.route,
                        &self.store,
                        task.id,
                    ),
                )
                .await
                {
                    Ok(v) => v.map(Some),
                    Err(_) => Ok(None),
                },
                None => self
                    .adapter
                    .execute_controlled(&task.prompt, &task.route, &self.store, task.id)
                    .await
                    .map(Some),
            };
            self.hook(&task, HookTrigger::AfterTool, "provider_execute")?;
            // Lifecycle requests made while the adapter was running take effect at this
            // durable operation boundary; no subsequent attempt is started.
            match self.store.task(task.id)?.map(|t| t.state) {
                Some(TaskState::Pausing) => {
                    task.state = TaskState::Paused;
                    task.failure_reason = None;
                    op.state = OperationState::Cancelled;
                }
                Some(TaskState::Cancelling) => {
                    task.state = TaskState::Cancelled;
                    task.failure_reason = Some("cancelled by user at safe boundary".into());
                    task.terminal_reason = Some(TerminalReason::CancelledByUser);
                    op.state = OperationState::Cancelled;
                }
                _ => {}
            }
            if matches!(task.state, TaskState::Paused | TaskState::Cancelled) {
                op.completed_at = Some(Utc::now());
                task.updated_at = Utc::now();
                task.elapsed_ms = task.elapsed_ms.saturating_add(
                    (task.updated_at - op.started_at).num_milliseconds().max(0) as u64,
                );
                self.store.finish_operation(&task, &op)?;
                self.event(
                    &task,
                    "task.safe_boundary",
                    format!("state={:?}", task.state),
                )?;
                return Ok(task);
            }
            match result {
                Ok(None) => {
                    task.state = TaskState::TimedOut;
                    task.failure_reason = Some("cumulative execution timeout exceeded".into());
                    task.terminal_reason = Some(TerminalReason::TimeoutExceeded);
                    op.state = OperationState::TimedOut;
                }
                Err(e) => {
                    task.state = TaskState::Failed;
                    task.failure_reason = Some(e.to_string());
                    task.verification = Some(format!("FAIL: {e}"));
                    task.terminal_reason = Some(TerminalReason::ProviderFailed);
                    op.state = OperationState::Failed;
                }
                Ok(Some(r)) => {
                    task.tokens_used = task.tokens_used.saturating_add(r.usage_tokens);
                    task.output = Some(r.output.clone());
                    self.persist_output_artifact(&task, &op, &r.output)?;
                    if task.token_budget.is_some_and(|b| task.tokens_used > b) {
                        task.state = TaskState::Failed;
                        task.failure_reason = Some("cumulative token budget exceeded".into());
                        task.terminal_reason = Some(TerminalReason::TokenBudgetExceeded);
                        op.state = OperationState::Failed;
                    } else if r.output.trim().is_empty() {
                        task.state = TaskState::Failed;
                        task.failure_reason = Some("empty output failed verification".into());
                        task.verification = Some("FAIL: output is empty".into());
                        task.terminal_reason = Some(TerminalReason::VerificationFailed);
                        op.state = OperationState::Failed;
                    } else {
                        task.state = TaskState::Succeeded;
                        task.verification = Some("PASS: output is non-empty".into());
                        task.terminal_reason = Some(TerminalReason::Completed);
                        op.state = OperationState::Succeeded;
                    }
                }
            }
            op.completed_at = Some(Utc::now());
            task.updated_at = Utc::now();
            task.elapsed_ms = task
                .elapsed_ms
                .saturating_add((task.updated_at - op.started_at).num_milliseconds().max(0) as u64);
            if matches!(task.state, TaskState::Failed | TaskState::TimedOut) {
                self.hook(&task, HookTrigger::OnFailure, "attempt_terminal")?;
            }
            self.store.finish_operation(&task, &op)?;
            self.checkpoint(&task, &op, "operation-terminal", serde_json::json!({"state": task.state, "tokens_used": task.tokens_used, "failure_reason": task.failure_reason}).to_string())?;
            let latency_ms = op
                .completed_at
                .map(|end| (end - op.started_at).num_milliseconds().max(0) as u64)
                .unwrap_or(0);
            self.record_routing_outcome(&task, latency_ms)?;
            self.hook(&task, HookTrigger::OnCheckpoint, "operation_persisted")?;
            self.hook(&task, HookTrigger::AfterTask, "attempt_terminal")?;
            if task.state == TaskState::Failed && task.attempts < task.retry.max_attempts {
                let prior = task.route.clone();
                let decision = self
                    .router
                    .decide(RoutingRequest {
                        prompt: task.prompt.clone(),
                        required_quality: 85u8.saturating_add((task.attempts - 1) as u8 * 10),
                        estimated_tokens: task.prompt.len() as u32 * 2,
                        overrides: UserOverrides {
                            role: Some(prior.role.clone()),
                            effort: Some(match prior.dimensions.effort {
                                crate::domain::Effort::Low => crate::domain::Effort::Medium,
                                _ => crate::domain::Effort::High,
                            }),
                            context_tokens: Some(prior.dimensions.context_tokens),
                            output_tokens: Some(prior.dimensions.output_tokens),
                            capabilities: Some(prior.dimensions.capabilities.clone()),
                            tools: Some(prior.dimensions.tools.clone()),
                            isolation: Some(prior.dimensions.isolation.clone()),
                            lifecycle: Some(prior.dimensions.lifecycle.clone()),
                            verification: Some(prior.dimensions.verification.clone()),
                            ..UserOverrides::default()
                        },
                    })
                    .map_err(anyhow::Error::new)?;
                task.route = decision.route;
                task.route
                    .rationale
                    .push_str("; failed verification escalated quality and effort");
                self.event(
                    &task,
                    "route.escalated",
                    serde_json::json!({
                        "from": prior,
                        "to": task.route,
                        "reason": task.verification,
                        "signals": decision.evidence,
                        "policy_revision": self.router.policy_revision,
                    })
                    .to_string(),
                )?;
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
                        t.terminal_reason = Some(TerminalReason::DependencyCycle);
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
                t.terminal_reason = Some(TerminalReason::DependencyFailed);
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
        let state = self
            .store
            .task(id)?
            .ok_or_else(|| anyhow::anyhow!("task not found"))?
            .state;
        if state == TaskState::Running {
            self.store.transition(
                id,
                &[TaskState::Running],
                TaskState::Pausing,
                "task.pause_requested",
                "running operation will stop at safe boundary",
            )
        } else {
            self.store.transition(
                id,
                &[TaskState::Queued],
                TaskState::Paused,
                "task.paused",
                "no new operation will start",
            )
        }
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
        let state = self
            .store
            .task(id)?
            .ok_or_else(|| anyhow::anyhow!("task not found"))?
            .state;
        if matches!(state, TaskState::Running | TaskState::Pausing) {
            self.store.transition(
                id,
                &[TaskState::Running, TaskState::Pausing],
                TaskState::Cancelling,
                "task.cancel_requested",
                "running operation will stop at safe boundary",
            )
        } else {
            let mut task = self.store.transition(
                id,
                &[
                    TaskState::Queued,
                    TaskState::Paused,
                    TaskState::OutcomeUnknown,
                ],
                TaskState::Cancelled,
                "task.cancelled",
                "cancelled at safe boundary",
            )?;
            task.terminal_reason = Some(TerminalReason::CancelledByUser);
            self.store.save_task(&task)?;
            Ok(task)
        }
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
        self.router
            .validate_route(&route)
            .map_err(anyhow::Error::new)?;
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
    /// Ask a capable live adapter to reconcile every unknown operation. Unknown remains
    /// unknown when the provider cannot prove an outcome; it is never guessed or replayed.
    pub async fn reconcile_unknown(&mut self) -> Result<usize> {
        if !self.adapter.supports_reconciliation() {
            bail!("adapter does not support outcome reconciliation")
        }
        let mut reconciled = 0;
        let tasks = self.store.tasks()?;
        for task in tasks
            .into_iter()
            .filter(|t| t.state == TaskState::OutcomeUnknown)
        {
            for op in self
                .store
                .operations_for(task.id)?
                .into_iter()
                .filter(|o| o.state == OperationState::OutcomeUnknown)
            {
                if let Some(outcome) = self.adapter.reconcile(op.id).await? {
                    self.reconcile_operation(task.id, op.id, outcome)?;
                    reconciled += 1;
                }
            }
        }
        Ok(reconciled)
    }
    pub fn recover(&mut self) -> Result<usize> {
        self.store.recover()
    }
}
