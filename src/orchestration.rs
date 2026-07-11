use crate::{
    domain::{RetryPolicy, Task},
    provider::PiAdapter,
    runtime::Runtime,
    workflow::{DagRole, MakerCheckerFixerDag, VerificationPolicy},
};
use anyhow::{Result, bail};
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
}
impl<A: PiAdapter> Runtime<A> {
    pub async fn run_maker_checker_fixer(
        &self,
        objective: &str,
        policy: VerificationPolicy,
        delegation: DelegationPolicy,
    ) -> Result<WorkflowRun> {
        let dag = MakerCheckerFixerDag::template(policy)?;
        delegation.validate(0, 0, dag.nodes.len().saturating_sub(1))?;
        let mut ids = std::collections::HashMap::new();
        let mut task_ids = Vec::new();
        for node in &dag.nodes {
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
            let task = self.submit_with(prompt, deps, RetryPolicy::default(), None, None)?;
            ids.insert(node.id, task.id);
            task_ids.push(task.id);
        }
        let results = self.run_ready().await?;
        Ok(WorkflowRun {
            dag,
            task_ids,
            results,
        })
    }
}
