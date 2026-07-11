use anyhow::Result;
use aster::{
    domain::{Artifact, Operation, OperationState, TaskState},
    provider::{ExecutionResult, PiAdapter},
    runtime::Runtime,
    store::Store,
};
use async_trait::async_trait;
use chrono::Utc;
use sha2::{Digest, Sha256};
use uuid::Uuid;

struct Stable;
#[async_trait(?Send)]
impl PiAdapter for Stable {
    async fn execute(&self, prompt: &str, _: &aster::domain::Route) -> Result<ExecutionResult> {
        Ok(ExecutionResult {
            output: format!("result:{prompt}"),
            usage_tokens: 3,
        })
    }
}

#[tokio::test]
async fn dag_artifacts_and_checkpoints_are_normalized_and_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runtime.db");
    let parent_id;
    let parent_digest;
    {
        let runtime = Runtime::new(Store::open(&path).unwrap(), Stable);
        let parent = runtime.submit("parent".into()).unwrap();
        parent_id = parent.id;
        let parent = runtime.run(parent).await.unwrap();
        assert_eq!(parent.state, TaskState::Succeeded);
        let artifacts = runtime.store.artifacts_for(parent.id).unwrap();
        assert_eq!(artifacts.len(), 1);
        parent_digest = artifacts[0].digest.clone();
        assert_eq!(
            parent_digest,
            format!("sha256:{:x}", Sha256::digest(&artifacts[0].content))
        );
        let parent_ref = artifacts[0].id.to_string();
        let checkpoints = runtime.store.checkpoints_for(parent.id).unwrap();
        assert_eq!(
            checkpoints
                .iter()
                .map(|c| c.phase.as_str())
                .collect::<Vec<_>>(),
            ["operation-intent", "operation-terminal"]
        );

        let child = runtime
            .submit_with(
                "child".into(),
                vec![parent.id],
                Default::default(),
                None,
                None,
            )
            .unwrap();
        let child = runtime.run(child).await.unwrap();
        assert_eq!(child.state, TaskState::Succeeded);
        assert!(
            runtime
                .store
                .audit_for(child.id)
                .unwrap()
                .iter()
                .any(|event| event.kind == "artifact.inputs_resolved"
                    && event.detail.contains(&parent_ref)
                    && !event.detail.contains(&parent_digest))
        );
    }
    let reopened = Store::open(&path).unwrap();
    assert_eq!(
        reopened.artifacts_for(parent_id).unwrap()[0].digest,
        parent_digest
    );
    assert_eq!(reopened.checkpoints_for(parent_id).unwrap().len(), 2);
}

#[test]
fn failure_injection_rejects_wrong_owner_and_preserves_records_during_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recovery.db");
    let mut runtime = Runtime::new(Store::open(&path).unwrap(), Stable);
    let mut task = runtime.submit("crash".into()).unwrap();
    task.state = TaskState::Running;
    task.attempts = 1;
    runtime.store.save_task(&task).unwrap();
    let operation = Operation {
        id: Uuid::new_v4(),
        task_id: task.id,
        attempt: 1,
        state: OperationState::Running,
        retry_safe: false,
        started_at: Utc::now(),
        completed_at: None,
    };
    runtime.store.create_operation(&operation).unwrap();
    let content = b"durable-before-crash".to_vec();
    let artifact = Artifact {
        id: Uuid::new_v4(),
        task_id: task.id,
        attempt: 1,
        operation_id: operation.id,
        name: "partial.txt".into(),
        media_type: "text/plain".into(),
        digest: format!("sha256:{:x}", Sha256::digest(&content)),
        content,
        provenance: "failure-injection:before-crash".into(),
        created_at: Utc::now(),
    };
    runtime.store.save_artifact(&artifact).unwrap();
    let mut wrong = artifact.clone();
    wrong.id = Uuid::new_v4();
    wrong.attempt = 2;
    assert!(runtime.store.save_artifact(&wrong).is_err());
    assert_eq!(runtime.recover().unwrap(), 1);
    drop(runtime);

    let reopened = Store::open(&path).unwrap();
    assert_eq!(
        reopened.task(task.id).unwrap().unwrap().state,
        TaskState::OutcomeUnknown
    );
    assert_eq!(reopened.artifacts_for(task.id).unwrap(), vec![artifact]);
}
