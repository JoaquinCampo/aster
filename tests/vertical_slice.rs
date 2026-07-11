use aster::{
    domain::{ExecutionIsolation, IsolationDimension, Operation, OperationState, TaskState},
    provider::FakePiAdapter,
    runtime::Runtime,
    store::Store,
};
use chrono::Utc;
use uuid::Uuid;

#[tokio::test]
async fn task_route_execution_and_history_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let runtime = Runtime::new(Store::open(&path).unwrap(), FakePiAdapter);
    let queued = runtime
        .submit("implement a durable feature and test it".into())
        .unwrap();
    assert_eq!(queued.state, TaskState::Queued);
    assert_eq!(queued.route.role, "implementer");
    let done = runtime.run(queued).await.unwrap();
    assert_eq!(done.state, TaskState::Succeeded);
    drop(runtime);
    let reopened = Store::open(&path).unwrap();
    let tasks = reopened.tasks().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].state, TaskState::Succeeded);
    assert!(reopened.audit_for(tasks[0].id).unwrap().len() >= 4);
    let records = reopened.execution_isolation_for_task(tasks[0].id).unwrap();
    assert_eq!(records.len(), 6);
    assert_eq!(
        records
            .iter()
            .map(|r| r.dimension)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        6
    );
    assert!(records.iter().all(|r| !r.active && !r.enforced));
    assert!(
        records
            .iter()
            .all(|r| !r.mechanism.is_empty() && !r.limitation.is_empty())
    );
}

#[test]
fn execution_isolation_schema_rejects_wrong_owner_and_incomplete_dimensions() {
    let store = Store::open(":memory:").unwrap();
    let runtime = Runtime::new(store, FakePiAdapter);
    let task = runtime.submit("ownership fixture".into()).unwrap();
    let operation = Operation {
        id: Uuid::new_v4(),
        task_id: task.id,
        attempt: 1,
        state: OperationState::IntentRecorded,
        retry_safe: false,
        started_at: Utc::now(),
        completed_at: None,
    };
    runtime.store.create_operation(&operation).unwrap();
    let record = ExecutionIsolation {
        task_id: task.id,
        attempt: 1,
        operation_id: operation.id,
        dimension: IsolationDimension::Process,
        active: true,
        enforced: true,
        mechanism: "fixture".into(),
        limitation: "fixture".into(),
        recorded_at: Utc::now(),
    };
    assert!(
        runtime
            .store
            .save_execution_isolation(std::slice::from_ref(&record))
            .unwrap_err()
            .to_string()
            .contains("exactly six")
    );
    let wrong = IsolationDimension::ALL
        .into_iter()
        .map(|dimension| ExecutionIsolation {
            dimension,
            operation_id: Uuid::new_v4(),
            ..record.clone()
        })
        .collect::<Vec<_>>();
    assert!(
        runtime
            .store
            .save_execution_isolation(&wrong)
            .unwrap_err()
            .to_string()
            .contains("ownership mismatch")
    );
}

#[test]
fn trivial_work_stays_direct_and_cheap() {
    let route = aster::routing::Router::default().route("remember this");
    assert_eq!(route.role, "orchestrator");
    assert_eq!(route.dimensions.effort, "low");
}
