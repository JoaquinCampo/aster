use aster::{
    domain::Route,
    mcp::{Client, Loopback, MCP_PROTOCOL_VERSION, Server},
    memory::{MemoryScope, MemoryStore},
    orchestration::{DelegationPolicy, direct_result},
    pi_gateway::PiGateway,
    provider::{ExecutionResult, FakePiAdapter, PiAdapter},
    runtime::Runtime,
    store::Store,
    workflow::{DagRole, Risk, VerificationPolicy},
};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde_json::json;
use std::path::PathBuf;

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

#[tokio::test]
async fn v01_integrated_acceptance_covers_all_thirteen_steps() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("acceptance.db");
    let objective = "implement a durable repository change with independent verification, isolated execution, deterministic checks, lifecycle controls, and restart recovery";
    let pi_modules = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository parent")
        .join("autopoiesis/node_modules");
    assert!(
        pi_modules
            .join("@mariozechner/pi-agent-core/package.json")
            .exists(),
        "installed Pi package is required for integrated acceptance"
    );
    let adapter = || {
        PiGateway::new(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/pi-sidecar.mjs"))
            .with_node_modules(&pi_modules)
    };
    let mut runtime = Runtime::new(Store::open(&path).unwrap(), adapter());

    // Submission, visible routing, override, and every safe-boundary lifecycle control.
    let queued = runtime.submit(objective.into()).unwrap();
    assert!(!queued.route.rationale.is_empty());
    let paused = runtime.pause(queued.id).unwrap();
    assert_eq!(paused.state, aster::domain::TaskState::Paused);
    runtime.resume(queued.id).unwrap();
    runtime.cancel(queued.id).unwrap();
    runtime.retry(queued.id).unwrap();
    let mut overridden = runtime.store.task(queued.id).unwrap().unwrap().route;
    overridden.rationale = "operator acceptance override".into();
    let overridden = runtime.override_route(queued.id, overridden).unwrap();
    assert_eq!(overridden.route.rationale, "operator acceptance override");
    let completed_probe = runtime.run(overridden).await.unwrap();
    assert!(completed_probe.state.is_terminal());

    // Durable state is observed through a fresh runtime after the first instance is dropped.
    drop(runtime);
    let runtime = Runtime::new(Store::open(&path).unwrap(), adapter());
    assert_eq!(
        runtime.store.task(queued.id).unwrap().unwrap().state,
        completed_probe.state
    );

    let run = runtime
        .run_maker_checker_fixer(
            objective,
            VerificationPolicy::proportional(Risk::Medium),
            DelegationPolicy::default(),
        )
        .await
        .unwrap();
    let lifecycle = vec![
        "paused",
        "resumed",
        "cancelled",
        "retried",
        "route-overridden",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let final_result = runtime
        .finalize_workflow(objective, &run, lifecycle, true)
        .unwrap();

    assert!(final_result.delegated);
    assert_eq!(final_result.durable_task_ids, run.task_ids);
    assert!(run.dag.nodes.iter().any(|node| node.role == DagRole::Maker));
    assert!(
        run.dag
            .nodes
            .iter()
            .any(|node| node.role == DagRole::DeterministicChecker)
    );
    assert!(
        run.dag
            .nodes
            .iter()
            .any(|node| node.role == DagRole::IndependentChecker)
    );
    assert!(final_result.isolated_implementer);
    assert!(final_result.recovered_after_restart);
    assert_eq!(final_result.lifecycle_events.len(), 5);
    assert!(!final_result.artifacts.is_empty());
    assert!(!final_result.verification_evidence.is_empty());
    assert_eq!(final_result.routing_trace.len(), run.results.len());
    assert!(
        final_result
            .audit
            .iter()
            .any(|event| event.kind == "route.selected")
    );
    assert!(final_result.context.execution_budget_tokens > 0);
    assert!(final_result.usage.tokens > 0);
    assert!(final_result.usage.attempts > 0);
    assert!(run.results.iter().all(|task| task.output.is_some()));
}

#[test]
fn v01_trivial_request_is_direct_without_subagent() {
    let result = direct_result("remember: use tabs");
    assert!(!result.delegated);
    assert!(result.durable_task_ids.is_empty());
    assert!(result.routing_trace.is_empty());
    assert_eq!(result.context.executions, 0);
    assert_eq!(result.usage.tokens, 0);
    assert_eq!(result.lifecycle_events, ["handled.directly"]);
}
