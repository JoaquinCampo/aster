use aster::{
    pi_gateway::{FixtureTool, PI_PROTOCOL_VERSION, PiGateway, PiRunInput},
    provider::{ProviderEvent, ReasoningEffort},
};
use futures_util::StreamExt;
use serde_json::json;
use std::{collections::BTreeSet, path::PathBuf};

fn installed_modules() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("node_modules")
}

fn gateway() -> PiGateway {
    PiGateway::new(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/pi-sidecar.mjs"))
        .with_node_modules(installed_modules())
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
    let mut stream = gateway()
        .run_deterministic(
            input(Some(tool)),
            &["fs.read".to_string()].into_iter().collect(),
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
    let result = gateway()
        .run_deterministic(input(Some(tool)), &BTreeSet::new())
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
    let discovered = gateway()
        .with_node_modules(modules)
        .discover()
        .await
        .unwrap();
    assert_eq!(discovered.protocol, PI_PROTOCOL_VERSION);
    assert_eq!(discovered.agent_core, "0.73.0");
    assert_eq!(discovered.ai, "0.73.0");
    assert!(discovered.capabilities.contains("abort"));
}
