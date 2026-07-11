use anyhow::Result;
use aster::{
    effects::{
        Approval, Capability, EffectBroker as CoreEffectBroker, EffectRequest, FilesystemIsolation,
        IsolationProfile, NetworkIsolation, ProcessIsolation, ScopedGrant, SecretIsolation,
        SystemAdapter,
    },
    hooks::{HookFailurePolicy, HookOutcome, HookRunner, HookSpec, HookTrigger, LifecycleHooks},
    mcp::{Client, StdioTransport},
    plugin::{BrokerRequest, EffectBroker},
    provider::FakePiAdapter,
    runtime::Runtime,
    store::Store,
};
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
struct Broker(Arc<Mutex<Vec<BrokerRequest>>>);
impl EffectBroker for Broker {
    fn execute(&self, _: &str, request: BrokerRequest) -> Result<Value> {
        self.0.lock().unwrap().push(request);
        Ok(json!({"mediated":true}))
    }
}
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fixture_process"))
}
fn hook(mode: &str, policy: HookFailurePolicy) -> HookSpec {
    HookSpec {
        id: "fixture".into(),
        trigger: HookTrigger::BeforeTask,
        executable: fixture(),
        args: vec![mode.into()],
        timeout_ms: 500,
        failure_policy: policy,
        capabilities: BTreeSet::new(),
    }
}

fn hook_process_authorization(
    spec: &HookSpec,
    task_id: uuid::Uuid,
) -> Result<(ScopedGrant, Approval)> {
    let cwd = spec.executable.parent().unwrap().to_path_buf();
    let grant = ScopedGrant {
        id: uuid::Uuid::new_v4(),
        task_id,
        capabilities: [Capability::ProcessExec].into_iter().collect(),
        workspace: cwd.clone(),
        worktrees: vec![],
        executable_allowlist: [spec.executable.clone()].into_iter().collect(),
        network_allowlist: BTreeSet::new(),
        external_allowlist: BTreeSet::new(),
        secret_destinations: BTreeMap::new(),
        isolation: IsolationProfile {
            filesystem: FilesystemIsolation::None,
            process: ProcessIsolation::ScrubbedEnvironment,
            network: NetworkIsolation::Denied,
            secrets: SecretIsolation::Denied,
        },
        expires_at: None,
    };
    let request = EffectRequest::Exec {
        program: spec.executable.clone(),
        args: spec.args.clone(),
        env: BTreeMap::new(),
        cwd,
    };
    let approval = Approval::for_request(
        task_id,
        grant.id,
        &request,
        Utc::now() + Duration::minutes(1),
    )?;
    Ok((grant, approval))
}

fn run_hook(
    runner: &HookRunner<Broker>,
    spec: &HookSpec,
    context: Value,
    store: &Store,
) -> Result<HookOutcome> {
    let (grant, approval) = hook_process_authorization(spec, uuid::Uuid::new_v4())?;
    let process_broker = CoreEffectBroker {
        store,
        adapter: SystemAdapter,
    };
    runner.run(spec, context, &process_broker, &grant, &approval)
}

#[test]
fn typed_hook_success_and_failure_policies() -> Result<()> {
    let store = Store::open(":memory:")?;
    let runner = HookRunner::new(Broker::default());
    assert_eq!(
        run_hook(
            &runner,
            &hook("hook-ok", HookFailurePolicy::FailExecution),
            json!({"task":"x"}),
            &store,
        )?,
        HookOutcome::Completed(json!({"ok":true}))
    );
    assert!(matches!(
        run_hook(
            &runner,
            &hook("hook-error", HookFailurePolicy::Continue),
            json!({}),
            &store,
        )?,
        HookOutcome::Continued(_)
    ));
    assert!(
        run_hook(
            &runner,
            &hook("hook-error", HookFailurePolicy::FailExecution),
            json!({}),
            &store,
        )
        .is_err()
    );
    assert!(matches!(
        run_hook(
            &runner,
            &hook("hook-sleep", HookFailurePolicy::Continue),
            json!({}),
            &store,
        )?,
        HookOutcome::Continued(_)
    ));
    Ok(())
}
#[test]
fn hook_protocol_synchronization_is_stable_under_repeated_startup() -> Result<()> {
    let store = Store::open(":memory:")?;
    let runner = HookRunner::new(Broker::default());
    for iteration in 0..100 {
        assert_eq!(
            run_hook(
                &runner,
                &hook("hook-ok", HookFailurePolicy::FailExecution),
                json!({"iteration": iteration}),
                &store,
            )?,
            HookOutcome::Completed(json!({"ok":true}))
        );
    }
    Ok(())
}

#[test]
fn hook_effects_require_declaration_and_use_broker() -> Result<()> {
    let store = Store::open(":memory:")?;
    let broker = Broker::default();
    let log = broker.0.clone();
    let runner = HookRunner::new(broker);
    assert!(
        run_hook(
            &runner,
            &hook("hook-effect", HookFailurePolicy::FailExecution),
            json!({}),
            &store,
        )
        .is_err()
    );
    let mut declared = hook("hook-effect", HookFailurePolicy::FailExecution);
    declared.capabilities.insert(Capability::FileRead);
    assert_eq!(
        run_hook(&runner, &declared, json!({}), &store)?,
        HookOutcome::Completed(json!({"mediated":true}))
    );
    assert_eq!(log.lock().unwrap()[0].capability, Capability::FileRead);
    Ok(())
}
#[test]
fn denied_and_mutated_hook_launches_fail_before_process_authorization() -> Result<()> {
    let store = Store::open(":memory:")?;
    let runner = HookRunner::new(Broker::default());
    let spec = hook("hook-ok", HookFailurePolicy::FailExecution);
    let task_id = uuid::Uuid::new_v4();
    let (mut denied_grant, approval) = hook_process_authorization(&spec, task_id)?;
    denied_grant.capabilities.clear();
    let process_broker = CoreEffectBroker {
        store: &store,
        adapter: SystemAdapter,
    };
    let error = runner
        .run(&spec, json!({}), &process_broker, &denied_grant, &approval)
        .unwrap_err();
    assert!(format!("{error:#}").contains("capability denied"));
    let denied = store.operations_for(task_id)?[0].clone();
    assert_eq!(denied.state, aster::domain::OperationState::Failed);
    assert!(store.effect_authorizations(denied.id)?.is_empty());

    let mutated_task = uuid::Uuid::new_v4();
    let (grant, approval) = hook_process_authorization(&spec, mutated_task)?;
    let mut mutated = spec.clone();
    mutated.args.push("unexpected".into());
    let error = runner
        .run(&mutated, json!({}), &process_broker, &grant, &approval)
        .unwrap_err();
    assert!(format!("{error:#}").contains("approval is not bound"));
    let mutation = store.operations_for(mutated_task)?[0].clone();
    assert_eq!(mutation.state, aster::domain::OperationState::Failed);
    assert!(store.effect_authorizations(mutation.id)?.is_empty());
    Ok(())
}

#[test]
fn hook_crash_timeout_and_authorization_restart_are_durable() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("hooks.db");
    let runner = HookRunner::new(Broker::default());

    let crash = hook("hook-crash", HookFailurePolicy::Continue);
    let crash_task = uuid::Uuid::new_v4();
    let (crash_grant, crash_approval) = hook_process_authorization(&crash, crash_task)?;
    let timeout = hook("hook-sleep", HookFailurePolicy::Continue);
    let timeout_task = uuid::Uuid::new_v4();
    let (timeout_grant, timeout_approval) = hook_process_authorization(&timeout, timeout_task)?;
    let operation_ids = {
        let store = Store::open(&path)?;
        let process_broker = CoreEffectBroker {
            store: &store,
            adapter: SystemAdapter,
        };
        assert!(matches!(
            runner.run(
                &crash,
                json!({}),
                &process_broker,
                &crash_grant,
                &crash_approval
            )?,
            HookOutcome::Continued(_)
        ));
        assert!(matches!(
            runner.run(
                &timeout,
                json!({}),
                &process_broker,
                &timeout_grant,
                &timeout_approval,
            )?,
            HookOutcome::Continued(_)
        ));
        vec![
            (
                store.operations_for(crash_task)?[0].id,
                crash_approval.clone(),
            ),
            (
                store.operations_for(timeout_task)?[0].id,
                timeout_approval.clone(),
            ),
        ]
    };

    let restarted = Store::open(&path)?;
    for (operation_id, approval) in operation_ids {
        assert!(restarted.operation(operation_id)?.is_some());
        let authorizations = restarted.effect_authorizations(operation_id)?;
        assert_eq!(authorizations.len(), 1);
        assert_eq!(authorizations[0].approval_id, Some(approval.id));
        assert_eq!(authorizations[0].request_hash, approval.request_hash);
    }
    Ok(())
}

#[derive(Default)]
struct Recorder(Mutex<Vec<HookTrigger>>);
impl LifecycleHooks for Recorder {
    fn invoke(&self, trigger: HookTrigger, _: Value) -> Result<Vec<HookOutcome>> {
        self.0.lock().unwrap().push(trigger);
        Ok(vec![])
    }
}

#[tokio::test]
async fn runtime_invokes_task_tool_and_checkpoint_hooks_at_real_boundaries() -> Result<()> {
    let recorder = Arc::new(Recorder::default());
    let runtime =
        Runtime::new(Store::open(":memory:")?, FakePiAdapter).with_hooks(recorder.clone());
    let task = runtime.submit("hook integration".into())?;
    runtime.run(task).await?;
    assert_eq!(
        *recorder.0.lock().unwrap(),
        vec![
            HookTrigger::BeforeTask,
            HookTrigger::BeforeTool,
            HookTrigger::AfterTool,
            HookTrigger::OnCheckpoint,
            HookTrigger::AfterTask
        ]
    );
    Ok(())
}

#[test]
fn mcp_stdio_client_and_server_interoperate() -> Result<()> {
    let root = tempfile::tempdir()?;
    let store = Store::open(root.path().join("db"))?;
    let executable = fixture();
    let grant = ScopedGrant {
        id: uuid::Uuid::new_v4(),
        task_id: uuid::Uuid::new_v4(),
        capabilities: [Capability::ProcessExec].into_iter().collect(),
        workspace: root.path().to_owned(),
        worktrees: vec![],
        executable_allowlist: [executable.clone()].into_iter().collect(),
        network_allowlist: BTreeSet::new(),
        external_allowlist: BTreeSet::new(),
        secret_destinations: BTreeMap::new(),
        isolation: IsolationProfile {
            filesystem: FilesystemIsolation::None,
            process: ProcessIsolation::ScrubbedEnvironment,
            network: NetworkIsolation::Denied,
            secrets: SecretIsolation::Denied,
        },
        expires_at: None,
    };
    let args = vec!["mcp".into()];
    let env = BTreeMap::new();
    let request = EffectRequest::Exec {
        program: executable.clone(),
        args: args.clone(),
        env: env.clone(),
        cwd: root.path().to_owned(),
    };
    let approval = Approval::for_request(
        grant.task_id,
        grant.id,
        &request,
        Utc::now() + Duration::minutes(1),
    )?;
    let broker = CoreEffectBroker {
        store: &store,
        adapter: SystemAdapter,
    };
    let transport = StdioTransport::spawn_authorized(
        &broker,
        &grant,
        &approval,
        &executable,
        &args,
        &env,
        root.path(),
    )?;
    let mut client = Client::new(transport);
    assert_eq!(client.initialize()?["serverInfo"]["name"], "fixture");
    assert_eq!(client.list_tools()?["tools"][0]["name"], "echo");
    let called = client.call_tool("echo", json!({"value":7}))?;
    assert!(called["content"][0]["text"].as_str().unwrap().contains("7"));
    Ok(())
}
