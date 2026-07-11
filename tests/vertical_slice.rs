use aster::{domain::TaskState, provider::FakePiAdapter, runtime::Runtime, store::Store};

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
}

#[test]
fn trivial_work_stays_direct_and_cheap() {
    let route = aster::routing::Router.route("remember this");
    assert_eq!(route.role, "orchestrator");
    assert_eq!(route.effort, "low");
}
