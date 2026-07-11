use aster::{
    effects::{
        Approval, Capability, EffectBroker, EffectRequest, FilesystemIsolation, IsolationProfile,
        NetworkIsolation, ProcessIsolation, ScopedGrant, SecretIsolation, SystemAdapter,
    },
    pi_gateway::{FixtureTool, PI_PROTOCOL_VERSION, PiGateway, PiRunInput},
    provider::{ProviderEvent, ReasoningEffort},
    store::Store,
};
use futures_util::StreamExt;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
use uuid::Uuid;

fn installed_modules() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("node_modules")
}

fn gateway() -> PiGateway {
    PiGateway::new(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/pi-sidecar.mjs"))
        .with_node_modules(installed_modules())
}
fn authorization(gateway: &PiGateway, task_id: Uuid) -> (ScopedGrant, Approval) {
    let request = gateway.launch_request().unwrap();
    let (program, cwd) = match &request {
        EffectRequest::Exec { program, cwd, .. } => (program.clone(), cwd.clone()),
        _ => unreachable!(),
    };
    let grant = ScopedGrant {
        id: Uuid::new_v4(),
        task_id,
        capabilities: [Capability::ProcessExec].into_iter().collect(),
        workspace: cwd,
        worktrees: vec![],
        executable_allowlist: [program].into_iter().collect(),
        network_allowlist: BTreeSet::new(),
        external_allowlist: BTreeSet::new(),
        secret_destinations: BTreeMap::new(),
        isolation: IsolationProfile {
            filesystem: FilesystemIsolation::WorkspaceReadOnly,
            process: ProcessIsolation::ScrubbedEnvironment,
            network: NetworkIsolation::Denied,
            secrets: SecretIsolation::Denied,
        },
        expires_at: None,
    };
    let approval = Approval::for_request(
        task_id,
        grant.id,
        &request,
        chrono::Utc::now() + chrono::Duration::minutes(5),
    )
    .unwrap();
    (grant, approval)
}

fn input(tool: Option<FixtureTool>) -> PiRunInput {
    PiRunInput {
        prompt: "hello".into(),
        model: "fixture/model".into(),
        effort: ReasoningEffort::High,
        context: Some("bounded context".into()),
        fixture_tool: tool,
    }
}

#[tokio::test]
async fn deterministic_mode_imports_pi_and_normalizes_messages_tools_and_usage() {
    let tool = FixtureTool {
        name: "read".into(),
        capability: "fs.read".into(),
        arguments: json!({"path":"safe"}),
    };
    let gateway = gateway();
    let task_id = Uuid::new_v4();
    let (grant, approval) = authorization(&gateway, task_id);
    let store = Store::open(":memory:").unwrap();
    let broker = EffectBroker {
        store: &store,
        adapter: SystemAdapter,
    };
    let mut stream = gateway
        .run_deterministic(
            input(Some(tool)),
            &["fs.read".to_string()].into_iter().collect(),
            &broker,
            &grant,
            &approval,
        )
        .await
        .unwrap();
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), ProviderEvent::OutputDelta(x) if x=="pi-deterministic:hello")
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), ProviderEvent::ToolCallDelta{name:Some(x),..} if x=="read")
    );
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), ProviderEvent::Completed(u) if u.total_tokens==Some(8))
    );
}

#[tokio::test]
async fn denied_tool_never_crosses_preflight() {
    let tool = FixtureTool {
        name: "shell".into(),
        capability: "process.exec".into(),
        arguments: json!({}),
    };
    let gateway = gateway();
    let task_id = Uuid::new_v4();
    let (grant, approval) = authorization(&gateway, task_id);
    let store = Store::open(":memory:").unwrap();
    let broker = EffectBroker {
        store: &store,
        adapter: SystemAdapter,
    };
    let result = gateway
        .run_deterministic(
            input(Some(tool)),
            &BTreeSet::new(),
            &broker,
            &grant,
            &approval,
        )
        .await;
    let error = match result {
        Ok(_) => panic!("denied tool unexpectedly passed preflight"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("capability_denied"));
}

#[tokio::test]
async fn installed_pi_runtime_is_discovered_when_available() {
    let modules = installed_modules();
    assert!(
        modules
            .join("@mariozechner/pi-agent-core/package.json")
            .exists(),
        "installed Pi package is required for concrete gateway acceptance"
    );
    let gateway = gateway().with_node_modules(modules);
    let task_id = Uuid::new_v4();
    let (grant, approval) = authorization(&gateway, task_id);
    let store = Store::open(":memory:").unwrap();
    let broker = EffectBroker {
        store: &store,
        adapter: SystemAdapter,
    };
    let discovered = gateway.discover(&broker, &grant, &approval).await.unwrap();
    assert_eq!(discovered.protocol, PI_PROTOCOL_VERSION);
    assert_eq!(discovered.agent_core, "0.73.0");
    assert_eq!(discovered.ai, "0.73.0");
    assert!(discovered.capabilities.contains("abort"));
}

#[tokio::test]
async fn denied_pi_launch_is_recorded_without_starting_sidecar() {
    let gateway = gateway();
    let task_id = Uuid::new_v4();
    let (mut grant, approval) = authorization(&gateway, task_id);
    grant.capabilities.clear();
    let store = Store::open(":memory:").unwrap();
    let broker = EffectBroker {
        store: &store,
        adapter: SystemAdapter,
    };
    let error = gateway
        .discover(&broker, &grant, &approval)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("capability denied"));
    let operations = store.operations_for(task_id).unwrap();
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].state, aster::domain::OperationState::Failed);
    assert!(
        store
            .effect_authorizations(operations[0].id)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn pi_launch_approval_rejects_argument_and_environment_mutation() {
    let gateway = gateway();
    let task_id = Uuid::new_v4();
    let (grant, approval) = authorization(&gateway, task_id);
    let mutated = gateway
        .clone()
        .with_node_modules(installed_modules().join("mutated"));
    let store = Store::open(":memory:").unwrap();
    let broker = EffectBroker {
        store: &store,
        adapter: SystemAdapter,
    };
    let error = mutated
        .discover(&broker, &grant, &approval)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("approval is not bound"));
    let operation = store.operations_for(task_id).unwrap().pop().unwrap();
    assert_eq!(operation.state, aster::domain::OperationState::Failed);
    assert!(
        store
            .effect_authorizations(operation.id)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn pi_launch_authorization_and_operation_survive_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pi.db");
    let gateway = gateway();
    let task_id = Uuid::new_v4();
    let (grant, approval) = authorization(&gateway, task_id);
    let operation_id = {
        let store = Store::open(&path).unwrap();
        let broker = EffectBroker {
            store: &store,
            adapter: SystemAdapter,
        };
        gateway.discover(&broker, &grant, &approval).await.unwrap();
        store.operations_for(task_id).unwrap()[0].id
    };

    let restarted = Store::open(&path).unwrap();
    let operation = restarted.operation(operation_id).unwrap().unwrap();
    assert_eq!(operation.state, aster::domain::OperationState::Succeeded);
    let authorizations = restarted.effect_authorizations(operation_id).unwrap();
    assert_eq!(authorizations.len(), 1);
    assert_eq!(authorizations[0].approval_id, Some(approval.id));
    assert_eq!(authorizations[0].request_hash, approval.request_hash);
}
