use aster::{
    config::{ConfigDocument, load_layered},
    context::{discover, manifest_from_assets},
    memory::{MemoryScope, MemoryStore},
};
use sha2::Digest;
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
    let bytes = fs::read(d.path().join("m.db")).unwrap();
    assert!(!bytes.windows(b"dark".len()).any(|window| window == b"dark"));
    let plain_digest = format!("{:x}", sha2::Sha256::digest(b"dark"));
    assert!(!String::from_utf8_lossy(&bytes).contains(&plain_digest));
}

#[test]
fn sparse_layers_only_override_fields_they_set() {
    let d = tempfile::tempdir().unwrap();
    let base = d.path().join("base.toml");
    let patch = d.path().join("patch.toml");
    fs::write(&base, "version=1\n[context]\ntotal_tokens=123\n[context.category_tokens]\nrules=7\n[memory]\nenabled=false\n").unwrap();
    fs::write(&patch, "[context]\ntotal_tokens=456\n").unwrap();
    let config = load_layered(&[base, patch]).unwrap();
    assert_eq!(config.context.total_tokens, 456);
    assert_eq!(config.context.category_tokens["rules"], 7);
    assert!(!config.memory.enabled);
}

#[test]
fn required_configuration_domains_are_typed_and_round_trip() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("all.toml");
    fs::write(&path, "version=1\n[providers]\nenabled=true\n[models]\nenabled=true\n[roles]\nenabled=true\n[routing]\nenabled=true\n[budgets]\nenabled=true\n[permissions]\nenabled=true\n[tools_mcp]\nenabled=true\n[skills_rules]\nenabled=true\n[hooks_plugins]\nenabled=true\n[persistence]\nenabled=true\n[tui]\nenabled=true\n[verification]\nenabled=true\n[lifecycle]\nenabled=true\n").unwrap();
    let mut doc = ConfigDocument::load(&path).unwrap();
    assert!(doc.config.providers.enabled && doc.config.models.enabled);
    assert!(doc.config.roles.enabled && doc.config.routing.enabled);
    assert!(doc.config.budgets.enabled && doc.config.permissions.enabled);
    assert!(doc.config.tools_mcp.enabled && doc.config.skills_rules.enabled);
    assert!(doc.config.hooks_plugins.enabled && doc.config.persistence.enabled);
    assert!(doc.config.tui.enabled && doc.config.verification.enabled);
    assert!(doc.config.lifecycle.enabled);
    doc.save_atomic().unwrap();
    assert!(ConfigDocument::load(path).unwrap().config.lifecycle.enabled);
}

#[test]
fn discovery_is_contained_hierarchical_and_project_content_is_untrusted() {
    let d = tempfile::tempdir().unwrap();
    let nested = d.path().join("a/b");
    fs::create_dir_all(nested.join(".agents/skills/x")).unwrap();
    fs::create_dir_all(d.path().join(".claude/skills/x")).unwrap();
    fs::write(d.path().join(".claude/skills/x/SKILL.md"), "root").unwrap();
    fs::write(nested.join(".agents/skills/x/SKILL.md"), "nested").unwrap();
    let assets = discover(d.path(), &nested).unwrap();
    assert_eq!(assets.iter().filter(|a| a.supported).count(), 2);
    let manifest = manifest_from_assets(&assets, 100).unwrap();
    assert!(
        manifest
            .items
            .iter()
            .all(|item| item.provenance.trust == aster::context::Trust::UntrustedContent)
    );

    let outside = tempfile::tempdir().unwrap();
    assert!(discover(d.path(), outside.path()).is_err());
}

#[test]
fn tui_required_field_edits_are_validated_atomic_and_conflict_aware() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("edit.toml");
    fs::write(&path, "version=1\n[context]\ntotal_tokens=100\n").unwrap();
    let mut doc = ConfigDocument::load(&path).unwrap();
    doc.edit_required("context.total_tokens", "200").unwrap();
    doc.edit_required("routing.enabled", "true").unwrap();
    doc.edit_required("verification.enabled", "true").unwrap();
    doc.edit_required("lifecycle.enabled", "true").unwrap();
    assert!(doc.edit_required("providers.secret", "x").is_err());
    doc.save_atomic().unwrap();
    let loaded = ConfigDocument::load(&path).unwrap();
    assert_eq!(loaded.config.context.total_tokens, 200);
    assert!(
        loaded.config.routing.enabled
            && loaded.config.verification.enabled
            && loaded.config.lifecycle.enabled
    );
}
