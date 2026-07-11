use crate::{
    domain::{AuditEvent, RetryPolicy, Role, Route, Task},
    provider::PiAdapter,
    routing::{Router, RoutingRequest},
    runtime::Runtime,
    verification::{DurableEvidence, VerificationOwnerRole, VerificationRun, VerificationStatus},
    workflow::{CheckerVerdict, DagRole, MakerCheckerFixerDag, VerificationPolicy},
};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelegationPolicy {
    pub max_depth: u32,
    pub max_fanout: usize,
}
impl Default for DelegationPolicy {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_fanout: 8,
        }
    }
}
impl DelegationPolicy {
    pub fn validate(&self, depth: u32, current_children: usize, requested: usize) -> Result<()> {
        if depth >= self.max_depth {
            bail!("delegation depth limit exceeded")
        }
        if requested > self.max_fanout.saturating_sub(current_children) {
            bail!("delegation fan-out limit exceeded")
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct WorkflowRun {
    pub dag: MakerCheckerFixerDag,
    pub task_ids: Vec<Uuid>,
    pub results: Vec<Task>,
    pub checker_verdicts: Vec<(Uuid, CheckerVerdict)>,
}

/// The stable, inspectable acceptance payload returned by the control plane.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalResult {
    pub objective: String,
    pub delegated: bool,
    pub delegation_reason: String,
    pub artifacts: Vec<ArtifactEvidence>,
    pub verification_evidence: Vec<String>,
    pub routing_trace: Vec<Route>,
    pub audit: Vec<AuditEvent>,
    pub context: ContextAccounting,
    pub usage: UsageAccounting,
    pub durable_task_ids: Vec<Uuid>,
    pub lifecycle_events: Vec<String>,
    pub isolated_implementer: bool,
    pub recovered_after_restart: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactEvidence {
    pub task_id: Uuid,
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextAccounting {
    pub prompt_bytes: usize,
    pub execution_budget_tokens: u32,
    pub executions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageAccounting {
    pub tokens: u64,
    pub attempts: u32,
}

/// Handles a mechanical request in-process and records why no agent was spawned.
pub fn direct_result(objective: &str) -> FinalResult {
    let benefit = Router::default().delegation_benefit(&RoutingRequest {
        prompt: objective.into(),
        required_quality: 1,
        estimated_tokens: 1,
        overrides: Default::default(),
    });
    FinalResult {
        objective: objective.into(),
        delegated: benefit.delegated,
        delegation_reason: benefit.reason,
        artifacts: vec![ArtifactEvidence {
            task_id: Uuid::nil(),
            kind: "direct-result".into(),
            value: objective.into(),
        }],
        verification_evidence: vec!["PASS: deterministic direct operation".into()],
        routing_trace: vec![],
        audit: vec![],
        context: ContextAccounting {
            prompt_bytes: objective.len(),
            execution_budget_tokens: 0,
            executions: 0,
        },
        usage: UsageAccounting {
            tokens: 0,
            attempts: 0,
        },
        durable_task_ids: vec![],
        lifecycle_events: vec!["handled.directly".into()],
        isolated_implementer: false,
        recovered_after_restart: false,
    }
}
impl<A: PiAdapter> Runtime<A> {
    fn persist_workflow_verification(
        &self,
        task: &Task,
        role: DagRole,
        policy: &VerificationPolicy,
    ) -> Result<()> {
        let owner_role = match role {
            DagRole::Maker => VerificationOwnerRole::Maker,
            DagRole::DeterministicChecker => VerificationOwnerRole::DeterministicChecker,
            DagRole::IndependentChecker => VerificationOwnerRole::IndependentChecker,
            DagRole::Fixer => VerificationOwnerRole::Fixer,
            DagRole::Finalizer => VerificationOwnerRole::Finalizer,
        };
        let outcome = if task.state == crate::domain::TaskState::Succeeded {
            VerificationStatus::Passed
        } else {
            VerificationStatus::Failed
        };
        let now = chrono::Utc::now();
        let run = VerificationRun {
            id: Uuid::new_v4(),
            task_id: task.id,
            attempt: task.attempts,
            checker_id: task.id,
            owner_role,
            policy: serde_json::to_string(policy)?,
            command_identity: format!("pi:{}", task.route.model),
            environment_profile: "provider-adapter/v1".into(),
            isolation_profile: task.route.dimensions.isolation.clone(),
            started_at: task.created_at,
            completed_at: Some(now),
            outcome,
            exit_status: None,
        };
        self.store.save_verification_run(&run)?;
        if let Some(output) = &task.output {
            let digest = format!("sha256:{}", crate::verification::digest(output.as_bytes()));
            self.store.save_verification_evidence(&DurableEvidence {
                id: Uuid::new_v4(),
                run_id: run.id,
                kind: "provider-output".into(),
                payload_ref: Some(format!("task:{}:output", task.id)),
                digest,
                media_type: "text/plain".into(),
                size: output.len() as u64,
                created_at: now,
            })?;
        }
        Ok(())
    }

    pub async fn run_maker_checker_fixer(
        &self,
        objective: &str,
        policy: VerificationPolicy,
        delegation: DelegationPolicy,
    ) -> Result<WorkflowRun> {
        let mut dag = MakerCheckerFixerDag::template(policy)?;
        delegation.validate(0, 0, dag.nodes.len().saturating_sub(1))?;
        let mut ids = std::collections::HashMap::new();
        let mut task_ids = Vec::new();
        for node in dag
            .nodes
            .iter()
            .filter(|node| node.role != DagRole::Finalizer)
        {
            let deps = node
                .dependencies
                .iter()
                .map(|id| {
                    ids.get(id)
                        .copied()
                        .ok_or_else(|| anyhow::anyhow!("DAG is not topologically ordered"))
                })
                .collect::<Result<Vec<_>>>()?;
            let role = match node.role {
                DagRole::Maker => "maker",
                DagRole::DeterministicChecker => "deterministic checker",
                DagRole::IndependentChecker => "independent checker",
                DagRole::Fixer => "fixer",
                DagRole::Finalizer => "finalizer",
            };
            let prompt = format!("{role} for objective: {objective}");
            let mut task = self.submit_with(prompt, deps, RetryPolicy::default(), None, None)?;
            if node.role == DagRole::Maker {
                let mut isolated = task.route.clone();
                isolated.role = Role::Implementer;
                isolated.dimensions.isolation = vec![
                    "isolated-worktree".into(),
                    "network-denied".into(),
                    "credentials-denied".into(),
                ];
                task.route = isolated;
                self.store.save_task_with_event(
                    &task,
                    "route.profile_applied",
                    "isolated implementer profile",
                )?;
            }
            ids.insert(node.id, task.id);
            task_ids.push(task.id);
        }
        let mut results = self.run_ready().await?;
        let by_id: std::collections::HashMap<_, _> =
            results.iter().map(|task| (task.id, task)).collect();
        let mut checker_verdicts = Vec::new();
        for (node, task_id) in dag.nodes.iter().zip(task_ids.iter()) {
            if matches!(
                node.role,
                DagRole::DeterministicChecker | DagRole::IndependentChecker
            ) {
                let output = by_id
                    .get(task_id)
                    .and_then(|task| task.output.as_deref())
                    .ok_or_else(|| anyhow::anyhow!("checker task {task_id} produced no output"))?;
                checker_verdicts.push((*task_id, CheckerVerdict::decode(output)?));
            }
        }
        let failed_nodes: Vec<_> = checker_verdicts
            .iter()
            .filter(|(_, verdict)| verdict.requires_fix())
            .filter_map(|(task_id, _)| {
                ids.iter()
                    .find_map(|(node_id, mapped)| (*mapped == *task_id).then_some(*node_id))
            })
            .collect();
        if !failed_nodes.is_empty() {
            let fixer_node = dag.append_fixer_round(&failed_nodes)?;
            let dependencies = failed_nodes.iter().map(|id| ids[id]).collect();
            let fixer = self.submit_with(
                format!("fixer for objective: {objective}"),
                dependencies,
                RetryPolicy::default(),
                None,
                None,
            )?;
            ids.insert(fixer_node, fixer.id);
            task_ids.push(fixer.id);
            results.extend(self.run_ready().await?);
        }
        let finalizer = dag
            .nodes
            .iter()
            .find(|node| node.role == DagRole::Finalizer)
            .ok_or_else(|| anyhow::anyhow!("missing finalizer"))?;
        let dependencies = finalizer.dependencies.iter().map(|id| ids[id]).collect();
        let task = self.submit_with(
            format!("finalizer for objective: {objective}"),
            dependencies,
            RetryPolicy::default(),
            None,
            None,
        )?;
        ids.insert(finalizer.id, task.id);
        task_ids.push(task.id);
        results.extend(self.run_ready().await?);
        for (node_id, task_id) in &ids {
            if let (Some(node), Some(result)) = (
                dag.nodes.iter().find(|n| n.id == *node_id),
                results.iter().find(|t| t.id == *task_id),
            ) {
                self.persist_workflow_verification(result, node.role.clone(), &dag.policy)?;
            }
        }
        Ok(WorkflowRun {
            dag,
            task_ids,
            results,
            checker_verdicts,
        })
    }
}

impl<A: PiAdapter> Runtime<A> {
    /// Builds the end-to-end acceptance payload from durable task and audit state.
    pub fn finalize_workflow(
        &self,
        objective: &str,
        run: &WorkflowRun,
        lifecycle_events: Vec<String>,
        recovered_after_restart: bool,
    ) -> Result<FinalResult> {
        let benefit = Router::default().delegation_benefit(&RoutingRequest {
            prompt: objective.into(),
            required_quality: 7,
            estimated_tokens: 2_000,
            overrides: Default::default(),
        });
        let mut audit = Vec::new();
        for id in &run.task_ids {
            audit.extend(self.store.audit_for(*id)?);
        }
        Ok(FinalResult {
            objective: objective.into(),
            delegated: benefit.delegated,
            delegation_reason: benefit.reason,
            artifacts: run
                .results
                .iter()
                .filter_map(|task| {
                    task.output.as_ref().map(|value| ArtifactEvidence {
                        task_id: task.id,
                        kind: "provider-output".into(),
                        value: value.clone(),
                    })
                })
                .collect(),
            verification_evidence: run
                .results
                .iter()
                .filter_map(|task| task.verification.clone())
                .collect(),
            routing_trace: run.results.iter().map(|task| task.route.clone()).collect(),
            audit,
            context: ContextAccounting {
                prompt_bytes: objective.len(),
                execution_budget_tokens: run
                    .results
                    .iter()
                    .map(|task| task.route.dimensions.context_tokens)
                    .sum(),
                executions: run.results.len(),
            },
            usage: UsageAccounting {
                tokens: run.results.iter().map(|task| task.tokens_used).sum(),
                attempts: run.results.iter().map(|task| task.attempts).sum(),
            },
            durable_task_ids: run.task_ids.clone(),
            lifecycle_events,
            isolated_implementer: run.results.iter().any(|task| {
                task.route.role == "implementer"
                    && task
                        .route
                        .dimensions
                        .isolation
                        .iter()
                        .any(|item| item.contains("worktree"))
            }),
            recovered_after_restart,
        })
    }
}
