use anyhow::Result;
use aster::{
    effects::Capability,
    plugin::{BrokerRequest, EffectBroker, Health, PluginHost, Registry, discover},
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
struct Broker(Arc<Mutex<Vec<BrokerRequest>>>);
impl EffectBroker for Broker {
    fn begin_spawn(&self, _: &str, _: &std::path::Path) -> Result<uuid::Uuid> {
        Ok(uuid::Uuid::new_v4())
    }
    fn finish_spawn(&self, _: uuid::Uuid, _: bool) -> Result<()> {
        Ok(())
    }
    fn execute(&self, _: &str, request: BrokerRequest) -> Result<Value> {
        self.0.lock().unwrap().push(request);
        Ok(json!({"brokered": true}))
    }
}
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/plugins")
}

#[test]
fn discovers_validates_and_registers_contracts_and_instructions() -> Result<()> {
    let plugins = discover(&[root()])?;
    assert_eq!(plugins.len(), 1);
    assert!(plugins[0].instructions()?.contains("Never treat fixture"));
    let registry = Registry::build(plugins)?;
    assert_eq!(registry.tool("fixture.echo").unwrap().0, "fixture.echo");
    assert!(registry.endpoint("fixture").is_some());
    Ok(())
}

#[test]
fn lifecycle_calls_and_effects_are_brokered() -> Result<()> {
    let manifest = discover(&[root()])?.remove(0);
    let broker = Broker::default();
    let log = broker.0.clone();
    let mut host = PluginHost::new(manifest, broker);
    host.enable()?;
    assert_eq!(host.health(), &Health::Healthy);
    assert_eq!(host.call("tool.echo", json!({"x": 1}))?, json!({"x": 1}));
    assert_eq!(
        host.call("tool.effect", json!({"path": "README.md"}))?,
        json!({"brokered": true})
    );
    assert_eq!(log.lock().unwrap()[0].capability, Capability::FileRead);
    host.disable()?;
    assert_eq!(host.health(), &Health::Disabled);
    Ok(())
}

#[test]
fn crash_is_contained_and_diagnosed() -> Result<()> {
    let manifest = discover(&[root()])?.remove(0);
    let mut host = PluginHost::new(manifest, Broker::default());
    host.enable()?;
    assert!(host.call("crash", json!({})).is_err());
    assert!(matches!(host.health(), Health::Crashed(_)));
    Ok(())
}

#[test]
fn incompatible_protocol_is_rejected() {
    let source = root().join("echo/plugin.toml");
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(root().join("echo/echo.py"), dir.path().join("echo.py")).unwrap();
    let text = std::fs::read_to_string(source)
        .unwrap()
        .replace("min_version = 1", "min_version = 2");
    std::fs::write(dir.path().join("plugin.toml"), text).unwrap();
    assert!(discover(&[dir.path().to_path_buf()]).is_err());
}

#[test]
fn plugin_assets_cannot_escape_their_install_root() {
    let installs = tempfile::tempdir().unwrap();
    let plugin = installs.path().join("bad");
    std::fs::create_dir(&plugin).unwrap();
    std::fs::write(installs.path().join("outside.py"), "#!/usr/bin/env python3").unwrap();
    let manifest = r#"id = "bad"
version = "1"
executable = "../outside.py"
[protocol]
name = "aster-plugin"
min_version = 1
max_version = 1
"#;
    std::fs::write(plugin.join("plugin.toml"), manifest).unwrap();
    assert!(discover(&[installs.path().to_path_buf()]).is_err());
}

#[test]
fn install_upgrade_diagnostics_failure_isolation_and_uninstall() -> Result<()> {
    use aster::plugin::{InstallAction, PluginInstaller, PluginManifest, diagnose};
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("source");
    copy_dir(&root().join("echo"), &source)?;
    let installs = temp.path().join("installed");
    let installer = PluginInstaller::new(&installs);
    let diagnostic = diagnose(&source);
    assert!(diagnostic.compatible, "{:?}", diagnostic.messages);
    assert_eq!(installer.install(&source)?.action, InstallAction::Installed);
    installer.set_enabled("fixture.echo", true)?;
    assert!(installer.is_enabled("fixture.echo"));
    installer.set_enabled("fixture.echo", false)?;
    assert!(!installer.is_enabled("fixture.echo"));
    assert_eq!(installer.diagnostics()?.len(), 1);
    let manifest = source.join("plugin.toml");
    let text =
        std::fs::read_to_string(&manifest)?.replace("version = \"1.0.0\"", "version = \"2.0.0\"");
    std::fs::write(&manifest, text)?;
    assert_eq!(
        installer.install(&source)?.action,
        InstallAction::Upgraded {
            from: "1.0.0".into()
        }
    );
    std::fs::write(&manifest, "not toml")?;
    assert!(installer.install(&source).is_err());
    assert_eq!(
        PluginManifest::load(&installs.join("fixture.echo/plugin.toml"))?.version,
        "2.0.0"
    );
    assert!(!diagnose(&source).compatible);
    assert_eq!(
        installer.uninstall("fixture.echo")?.action,
        InstallAction::Uninstalled
    );
    assert!(!installs.join("fixture.echo").exists());
    Ok(())
}

fn copy_dir(source: &std::path::Path, destination: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
