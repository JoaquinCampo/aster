use anyhow::Result;
use aster::plugin::{BrokerRequest, EffectBroker, Health, PluginHost, Registry, discover};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
struct Broker(Arc<Mutex<Vec<BrokerRequest>>>);
impl EffectBroker for Broker {
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
    assert_eq!(log.lock().unwrap()[0].capability, "workspace.read");
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
