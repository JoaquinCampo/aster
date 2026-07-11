use crate::{
    domain::{RetryPolicy, Task},
    provider::PiAdapter,
    runtime::Runtime,
    workflow::{CheckerVerdict, DagRole, MakerCheckerFixerDag, VerificationPolicy},
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
    pub checker_verdicts: Vec<(Uuid, CheckerVerdict)>,
}
impl<A: PiAdapter> Runtime<A> {
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
            let task = self.submit_with(prompt, deps, RetryPolicy::default(), None, None)?;
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
        Ok(WorkflowRun {
            dag,
            task_ids,
            results,
            checker_verdicts,
        })
    }
}
