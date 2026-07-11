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
