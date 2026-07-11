use aster::{
    config::{ConfigDocument, load_layered},
    context::{discover, manifest_from_assets},
    memory::{MemoryScope, MemoryStore},
};
use std::fs;
#[test]
fn config_unknown_roundtrip_and_conflict() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("aster.toml");
    fs::write(
        &p,
        "version=1\nfuture='kept'\n[context]\ntotal_tokens=100\n",
    )
    .unwrap();
    let mut doc = ConfigDocument::load(&p).unwrap();
    doc.config.memory.enabled = false;
    doc.save_atomic().unwrap();
    assert!(
        fs::read_to_string(&p)
            .unwrap()
            .contains("future = \"kept\"")
    );
    let mut conflict = ConfigDocument::load(&p).unwrap();
    fs::write(&p, "version=1\n").unwrap();
    assert!(
        conflict
            .save_atomic()
            .unwrap_err()
            .to_string()
            .contains("conflict")
    );
}
#[test]
fn layered_validation() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a.toml");
    let b = d.path().join("b.toml");
    fs::write(&a, "version=1\n[context]\ntotal_tokens=100\n").unwrap();
    fs::write(&b, "version=1\n[memory]\nenabled=false\n").unwrap();
    assert!(!load_layered(&[a, b]).unwrap().memory.enabled);
}
#[test]
fn nested_discovery_claude_wins_and_reports_unsupported() {
    let d = tempfile::tempdir().unwrap();
    let nested = d.path().join("a/b");
    fs::create_dir_all(nested.join(".agents/skills/x")).unwrap();
    fs::create_dir_all(nested.join(".claude/skills/x")).unwrap();
    fs::write(nested.join(".agents/skills/x/SKILL.md"), "agents").unwrap();
    fs::write(nested.join(".claude/skills/x/SKILL.md"), "claude").unwrap();
    fs::write(nested.join(".claude/plugin.json"), "{}").unwrap();
    let assets = discover(d.path(), &nested).unwrap();
    assert!(
        assets
            .iter()
            .any(|a| a.supported && a.ecosystem == ".claude")
    );
    assert!(!assets.iter().any(|a| a.supported
        && a.ecosystem == ".agents"
        && a.relative_key.ends_with("x/SKILL.md")));
    assert!(assets.iter().any(|a| !a.supported));
    let m = manifest_from_assets(&assets, 100).unwrap();
    assert_eq!(m.items[0].provenance.ecosystem, ".claude");
}
#[test]
fn memory_dedup_contradiction_and_delete_erases_payload() {
    let d = tempfile::tempdir().unwrap();
    let s = MemoryStore::open(d.path().join("m.db")).unwrap();
    let a = s
        .add(MemoryScope::UserPreference, "theme", "dark", "user:1")
        .unwrap();
    assert_eq!(
        a,
        s.add(MemoryScope::UserPreference, "theme", " DARK ", "user:2")
            .unwrap()
    );
    assert_eq!(
        s.contradictions(&MemoryScope::UserPreference, "theme", "light")
            .unwrap()
            .len(),
        1
    );
    s.delete(a).unwrap();
    assert!(s.active().unwrap().is_empty());
    assert_eq!(s.tombstone_count().unwrap(), 1);
}
