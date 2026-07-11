use anyhow::{Result, anyhow};
use aster::{
    domain::{Operation, OperationState, RetryPolicy, TaskState},
    provider::{ExecutionResult, PiAdapter},
    runtime::Runtime,
    store::Store,
};
use async_trait::async_trait;
use chrono::Utc;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use uuid::Uuid;

struct Flaky(Arc<AtomicUsize>);
#[async_trait]
impl PiAdapter for Flaky {
    async fn execute(&self, p: &str, _: &aster::domain::Route) -> Result<ExecutionResult> {
        let n = self.0.fetch_add(1, Ordering::SeqCst);
        if p == "slow" {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        if n == 0 {
            Err(anyhow!("injected failure"))
        } else {
            Ok(ExecutionResult {
                output: "ok".into(),
                usage_tokens: 10,
            })
        }
    }
}
fn runtime() -> (tempfile::TempDir, Runtime<Flaky>) {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(d.path().join("db")).unwrap();
    (d, Runtime::new(s, Flaky(Arc::new(AtomicUsize::new(0)))))
}

#[tokio::test]
async fn dependency_blocks_until_parent_succeeds() {
    let (_d, r) = runtime();
    let a = r.submit("a".into()).unwrap();
    let b = r
        .submit_with("b".into(), vec![a.id], RetryPolicy::default(), None, None)
        .unwrap();
    assert_eq!(r.run(b.clone()).await.unwrap().state, TaskState::Queued);
    let mut a = a;
    a.retry.max_attempts = 2;
    r.store.save_task(&a).unwrap();
    assert_eq!(r.run(a).await.unwrap().state, TaskState::Succeeded);
    assert_eq!(r.run(b).await.unwrap().state, TaskState::Succeeded);
}

#[tokio::test]
async fn retries_have_distinct_durable_operations() {
    let (_d, r) = runtime();
    let t = r
        .submit_with(
            "retry".into(),
            vec![],
            RetryPolicy {
                max_attempts: 2,
                initial_backoff_ms: 1,
                max_backoff_ms: 1,
            },
            None,
            None,
        )
        .unwrap();
    let done = r.run(t).await.unwrap();
    assert_eq!(done.attempts, 2);
    let ops = r.store.operations_for(done.id).unwrap();
    assert_eq!(ops.len(), 2);
    assert_ne!(ops[0].id, ops[1].id);
}

#[tokio::test]
async fn timeout_and_budget_are_terminal() {
    let (_d, r) = runtime();
    let slow = r
        .submit_with("slow".into(), vec![], RetryPolicy::default(), Some(1), None)
        .unwrap();
    assert_eq!(r.run(slow).await.unwrap().state, TaskState::TimedOut);
    let budget = r
        .submit_with(
            "budget".into(),
            vec![],
            RetryPolicy {
                max_attempts: 2,
                ..Default::default()
            },
            None,
            Some(5),
        )
        .unwrap();
    assert_eq!(r.run(budget).await.unwrap().state, TaskState::Failed);
}

#[tokio::test]
async fn pause_resume_cancel_enforce_safe_boundaries() {
    let (_d, mut r) = runtime();
    let t = r.submit("x".into()).unwrap();
    assert_eq!(r.pause(t.id).unwrap().state, TaskState::Paused);
    assert_eq!(r.resume(t.id).unwrap().state, TaskState::Queued);
    assert_eq!(r.cancel(t.id).unwrap().state, TaskState::Cancelled);
    assert!(r.resume(t.id).is_err());
}

#[test]
fn crash_recovery_requires_explicit_reconciliation() {
    let (_d, mut r) = runtime();
    let mut t = r.submit("x".into()).unwrap();
    t.state = TaskState::Running;
    r.store.save_task(&t).unwrap();
    r.store
        .create_operation(&Operation {
            id: Uuid::new_v4(),
            task_id: t.id,
            attempt: 1,
            state: OperationState::Running,
            retry_safe: false,
            started_at: Utc::now(),
            completed_at: None,
        })
        .unwrap();
    assert_eq!(r.recover().unwrap(), 1);
    assert_eq!(
        r.store.task(t.id).unwrap().unwrap().state,
        TaskState::OutcomeUnknown
    );
    assert_eq!(r.reconcile(t.id, false).unwrap().state, TaskState::Failed);
    assert!(
        r.store
            .audit_for(t.id)
            .unwrap()
            .iter()
            .any(|e| e.kind == "recovery.outcome_unknown")
    );
}

#[test]
fn audit_is_append_only_and_duplicate_ids_fail() {
    let (_d, r) = runtime();
    let t = r.submit("x".into()).unwrap();
    let mut events = r.store.audit_for(t.id).unwrap();
    let e = events.pop().unwrap();
    assert!(r.store.append(&e).is_err());
    assert_eq!(r.store.audit_for(t.id).unwrap().len(), 2);
}

#[tokio::test]
async fn scheduler_fails_cycles_and_impossible_dependencies() {
    let (_d, r) = runtime();
    let mut a = r.submit("a".into()).unwrap();
    let mut b = r.submit("b".into()).unwrap();
    a.dependencies = vec![b.id];
    b.dependencies = vec![a.id];
    r.store.save_task(&a).unwrap();
    r.store.save_task(&b).unwrap();
    r.run_ready().await.unwrap();
    assert_eq!(
        r.store.task(a.id).unwrap().unwrap().state,
        TaskState::Failed
    );
    assert_eq!(
        r.store.task(b.id).unwrap().unwrap().state,
        TaskState::Failed
    );

    let failed = r.submit("failed".into()).unwrap();
    let mut failed = failed;
    failed.state = TaskState::Failed;
    r.store.save_task(&failed).unwrap();
    let child = r
        .submit_with(
            "child".into(),
            vec![failed.id],
            RetryPolicy::default(),
            None,
            None,
        )
        .unwrap();
    r.run_ready().await.unwrap();
    assert_eq!(
        r.store.task(child.id).unwrap().unwrap().state,
        TaskState::Failed
    );
}

#[test]
fn retry_override_and_operation_reconciliation_are_audited() {
    let (_d, mut r) = runtime();
    let mut task = r.submit("x".into()).unwrap();
    task.state = TaskState::Failed;
    r.store.save_task(&task).unwrap();
    assert_eq!(r.retry(task.id).unwrap().state, TaskState::Queued);
    r.override_retry(
        task.id,
        RetryPolicy {
            max_attempts: 3,
            ..Default::default()
        },
    )
    .unwrap();
    let kinds: Vec<_> = r
        .store
        .audit_for(task.id)
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert!(kinds.contains(&"task.retry_requested".into()));
    assert!(kinds.contains(&"retry.overridden".into()));
}
