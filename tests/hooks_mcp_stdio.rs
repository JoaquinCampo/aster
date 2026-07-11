use anyhow::Result;
use aster::{
    effects::Capability,
    hooks::{HookFailurePolicy, HookOutcome, HookRunner, HookSpec, HookTrigger},
    mcp::{Client, StdioTransport},
    plugin::{BrokerRequest, EffectBroker},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
struct Broker(Arc<Mutex<Vec<BrokerRequest>>>);
impl EffectBroker for Broker {
    fn begin_spawn(&self, _: &str, _: &Path) -> Result<uuid::Uuid> {
        Ok(uuid::Uuid::new_v4())
    }
    fn finish_spawn(&self, _: uuid::Uuid, _: bool) -> Result<()> {
        Ok(())
    }
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

#[test]
fn typed_hook_success_and_failure_policies() -> Result<()> {
    let runner = HookRunner::new(Broker::default());
    assert_eq!(
        runner.run(
            &hook("hook-ok", HookFailurePolicy::FailExecution),
            json!({"task":"x"})
        )?,
        HookOutcome::Completed(json!({"ok":true}))
    );
    assert!(matches!(
        runner.run(&hook("hook-error", HookFailurePolicy::Continue), json!({}))?,
        HookOutcome::Continued(_)
    ));
    assert!(
        runner
            .run(
                &hook("hook-error", HookFailurePolicy::FailExecution),
                json!({})
            )
            .is_err()
    );
    assert!(matches!(
        runner.run(&hook("hook-sleep", HookFailurePolicy::Continue), json!({}))?,
        HookOutcome::Continued(_)
    ));
    Ok(())
}
#[test]
fn hook_effects_require_declaration_and_use_broker() -> Result<()> {
    let broker = Broker::default();
    let log = broker.0.clone();
    let runner = HookRunner::new(broker);
    assert!(
        runner
            .run(
                &hook("hook-effect", HookFailurePolicy::FailExecution),
                json!({})
            )
            .is_err()
    );
    let mut declared = hook("hook-effect", HookFailurePolicy::FailExecution);
    declared.capabilities.insert(Capability::FileRead);
    assert_eq!(
        runner.run(&declared, json!({}))?,
        HookOutcome::Completed(json!({"mediated":true}))
    );
    assert_eq!(log.lock().unwrap()[0].capability, Capability::FileRead);
    Ok(())
}
#[test]
fn mcp_stdio_client_and_server_interoperate() -> Result<()> {
    let transport = StdioTransport::spawn(&fixture(), &["mcp".into()])?;
    let mut client = Client::new(transport);
    assert_eq!(client.initialize()?["serverInfo"]["name"], "fixture");
    assert_eq!(client.list_tools()?["tools"][0]["name"], "echo");
    let called = client.call_tool("echo", json!({"value":7}))?;
    assert!(called["content"][0]["text"].as_str().unwrap().contains("7"));
    Ok(())
}
