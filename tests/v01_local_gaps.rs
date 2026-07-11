use aster::{
    domain::Route,
    mcp::{Client, Loopback, MCP_PROTOCOL_VERSION, Server},
    memory::{MemoryScope, MemoryStore},
    orchestration::DelegationPolicy,
    provider::{ExecutionResult, FakePiAdapter, PiAdapter},
    runtime::Runtime,
    store::Store,
    workflow::{Risk, VerificationPolicy},
};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde_json::json;

#[test]
fn memory_search_merge_expire_and_export() {
    let m = MemoryStore::open(":memory:").unwrap();
    let a = m
        .add(MemoryScope::ProjectKnowledge, "language", "Rust", "repo")
        .unwrap();
    let b = m
        .add(
            MemoryScope::ProjectKnowledge,
            "edition",
            "2024",
            "Cargo.toml",
        )
        .unwrap();
    assert_eq!(m.search("rust", None).unwrap()[0].id, a);
    let merged = m
        .merge(&[a, b], "toolchain", "Rust 2024", "reviewed")
        .unwrap();
    assert_eq!(
        m.search("rust", Some(&MemoryScope::ProjectKnowledge))
            .unwrap()[0]
            .id,
        merged
    );
    m.add_expiring(
        MemoryScope::Session,
        "temp",
        "gone",
        "test",
        Some(Utc::now() - Duration::seconds(1)),
    )
    .unwrap();
    assert_eq!(m.expire(Utc::now()).unwrap(), 1);
    let export = m.export().unwrap();
    assert_eq!(export.schema_version, 1);
    assert_eq!(export.memories.len(), 1);
}
#[test]
fn deterministic_mcp_client_server_conformance() {
    let server = Server::new("fixture", "1").tool("echo", json!({"type":"object"}), Ok);
    let mut client = Client::new(Loopback(&server));
    let init = client.initialize().unwrap();
    assert_eq!(init["protocolVersion"], MCP_PROTOCOL_VERSION);
    assert_eq!(client.list_tools().unwrap()["tools"][0]["name"], "echo");
    let result = client.call_tool("echo", json!({"value":7})).unwrap();
    assert!(result["content"][0]["text"].as_str().unwrap().contains("7"));
    let parse: serde_json::Value = serde_json::from_str(&server.handle("{")).unwrap();
    assert_eq!(parse["error"]["code"], -32700);
}
#[test]
fn delegation_is_bounded() {
    let p = DelegationPolicy {
        max_depth: 2,
        max_fanout: 3,
    };
    assert!(p.validate(0, 0, 3).is_ok());
    assert!(p.validate(2, 0, 1).is_err());
    assert!(p.validate(0, 2, 2).is_err());
}
struct FailingChecker;
#[async_trait]
impl PiAdapter for FailingChecker {
    async fn execute(&self, prompt: &str, _: &Route) -> anyhow::Result<ExecutionResult> {
        let output = if prompt.starts_with("deterministic checker")
            || prompt.starts_with("independent checker")
        {
            json!({"status":"Failed","rationale":"fixture failure"}).to_string()
        } else {
            format!("completed: {prompt}")
        };
        Ok(ExecutionResult {
            output,
            usage_tokens: 1,
        })
    }
}

#[tokio::test]
async fn checker_verdict_conditionally_schedules_one_bounded_fixer() {
    let rt = Runtime::new(Store::open(":memory:").unwrap(), FailingChecker);
    let run = rt
        .run_maker_checker_fixer(
            "repair",
            VerificationPolicy::proportional(Risk::Medium),
            DelegationPolicy::default(),
        )
        .await
        .unwrap();
    assert!(
        run.checker_verdicts
            .iter()
            .all(|(_, verdict)| verdict.requires_fix())
    );
    assert_eq!(
        run.dag
            .nodes
            .iter()
            .filter(|node| node.role == aster::workflow::DagRole::Fixer)
            .count(),
        1
    );
    assert!(
        run.results
            .iter()
            .any(|task| task.prompt.starts_with("fixer"))
    );
}

#[tokio::test]
async fn integrated_maker_checker_fixer_dag_executes() {
    let rt = Runtime::new(Store::open(":memory:").unwrap(), FakePiAdapter);
    let run = rt
        .run_maker_checker_fixer(
            "ship safely",
            VerificationPolicy::proportional(Risk::Medium),
            DelegationPolicy::default(),
        )
        .await
        .unwrap();
    assert_eq!(run.results.len(), run.task_ids.len());
    assert!(run.results.iter().all(|t| t.state.is_terminal()));
    assert!(run.results.iter().all(|t| t.output.is_some()));
}
