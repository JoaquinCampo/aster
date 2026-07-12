use aster::{
    config::{ConfigService, load_layered},
    provider::FakePiAdapter,
    runtime::Runtime,
    store::Store,
};
use std::fs;

#[test]
fn layered_config_drives_runtime_routing_budgets_and_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.toml");
    let project = dir.path().join("project.toml");
    let local = dir.path().join("local.toml");
    fs::write(
        &base,
        r#"version=2
[routing]
enabled=true
[budgets]
enabled=true
token_budget=1000
timeout_ms=9000
[lifecycle]
enabled=true
concurrency=2
retry_limit=1
[providers]
enabled=true
default_provider="base"
future_key="preserved"
"#,
    )
    .unwrap();
    fs::write(
        &project,
        "[budgets]\ntoken_budget=2000\n[providers]\ndefault_provider='project'\n",
    )
    .unwrap();
    fs::write(&local, "[lifecycle]\nconcurrency=3\n").unwrap();

    let config = load_layered(&[base, project, local]).unwrap();
    assert_eq!(
        config.providers.default_provider.as_deref(),
        Some("project")
    );
    assert_eq!(
        config.providers.extensions["future_key"].as_str(),
        Some("preserved")
    );
    assert_eq!(config.budgets.token_budget, Some(2000));
    assert_eq!(config.budgets.timeout_ms, Some(9000));

    let store = Store::open(dir.path().join("state.db")).unwrap();
    let runtime = Runtime::from_config(store, FakePiAdapter, &config).unwrap();
    assert_eq!(runtime.concurrency(), 3);
    let task = runtime.submit("configured task".into()).unwrap();
    assert_eq!(task.token_budget, Some(2000));
    assert_eq!(task.timeout_ms, Some(9000));
    assert_eq!(task.retry.max_attempts, 2);
}

#[test]
fn nested_tui_edits_round_trip_all_value_shapes_and_reject_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    fs::write(&path, "version=2\n[providers]\nunknown='keep'\n").unwrap();
    let mut service = ConfigService::load(&path).unwrap();
    service
        .edit_required("providers.auth_ref", "\"env:XAI_API_KEY\"")
        .unwrap();
    service
        .edit_required("models.allow", "[\"grok-4\", \"grok-4-fast\"]")
        .unwrap();
    service
        .edit_required("persistence.database_path", "\"configured.db\"")
        .unwrap();
    service
        .edit_required("verification.commands", "[\"cargo test --locked\"]")
        .unwrap();
    service
        .edit_required("tools_mcp.command_timeout_ms", "42")
        .unwrap();
    service.save_atomic().unwrap();

    let loaded = ConfigService::load(&path).unwrap();
    assert_eq!(loaded.config.models.allow.len(), 2);
    assert_eq!(loaded.config.tools_mcp.command_timeout_ms, 42);
    assert_eq!(
        loaded.config.providers.extensions["unknown"].as_str(),
        Some("keep")
    );
    assert!(
        loaded
            .config
            .providers
            .auth_ref
            .as_deref()
            .unwrap()
            .starts_with("env:")
    );
    assert!(loaded.config.verification.commands[0].contains("--locked"));

    let error = loaded.config.clone();
    let text = toml::to_string(&error)
        .unwrap()
        .replace("env:XAI_API_KEY", "plaintext");
    fs::write(&path, text).unwrap();
    assert!(ConfigService::load(&path).is_err());
}
