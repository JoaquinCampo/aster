use aster::config::{ConfigDocument, ConfigService, migrate};
use std::fs;

#[test]
fn v1_migrates_sequentially_and_idempotently_preserving_unknowns() {
    let v1 = include_str!("fixtures/config/v1.toml");
    let first = migrate(v1).unwrap();
    assert!(first.contains("version = 2") && first.contains("future = \"preserved\""));
    assert_eq!(migrate(&first).unwrap(), first);
}

#[test]
fn current_roundtrip_uses_conflict_aware_service() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("aster.toml");
    fs::write(&p, include_str!("fixtures/config/v2.toml")).unwrap();
    let mut service: ConfigService = ConfigDocument::load(&p).unwrap();
    service.save_atomic().unwrap();
    assert!(
        fs::read_to_string(&p)
            .unwrap()
            .contains("future = \"preserved\"")
    );
}

#[test]
fn malformed_old_and_future_versions_are_rejected() {
    assert!(migrate("version='two'").is_err());
    assert!(migrate("version=0").is_err());
    assert!(
        migrate("version=999")
            .unwrap_err()
            .to_string()
            .contains("future")
    );
}

#[test]
fn typed_domains_validate_secrets_ranges_and_preserve_nested_unknowns() {
    let valid = r#"version=2
[providers]
enabled=true
auth_ref="env:XAI_API_KEY"
future_provider_option="kept"
[lifecycle]
concurrency=4
[tui]
refresh_ms=10
"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("typed.toml");
    fs::write(&path, valid).unwrap();
    let mut service = ConfigService::load(&path).unwrap();
    assert_eq!(service.config.lifecycle.concurrency, 4);
    assert_eq!(
        service.config.providers.extensions["future_provider_option"].as_str(),
        Some("kept")
    );
    service.save_atomic().unwrap();
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("future_provider_option")
    );

    fs::write(&path, "version=2\n[providers]\nauth_ref='literal-secret'\n").unwrap();
    assert!(
        ConfigService::load(&path)
            .unwrap_err()
            .to_string()
            .contains("secret reference")
    );
    fs::write(&path, "version=2\n[lifecycle]\nconcurrency=0\n").unwrap();
    assert!(
        ConfigService::load(&path)
            .unwrap_err()
            .to_string()
            .contains("concurrency")
    );
}

#[test]
fn conflict_leaves_external_write_intact_and_no_temp_artifact() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("aster.toml");
    fs::write(&p, "version=2\n").unwrap();
    let mut service = ConfigService::load(&p).unwrap();
    fs::write(&p, "version=2\nexternal='intact'\n").unwrap();
    assert!(service.save_atomic().is_err());
    assert!(fs::read_to_string(&p).unwrap().contains("external"));
    assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
}
