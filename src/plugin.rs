use crate::effects::Capability;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

pub const HOST_PROTOCOL: &str = "aster-plugin";
pub const HOST_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProtocolRequirement {
    pub name: String,
    pub min_version: u32,
    pub max_version: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpEndpoint {
    pub name: String,
    pub description: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolContract {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub version: String,
    pub executable: PathBuf,
    pub protocol: ProtocolRequirement,
    #[serde(default)]
    pub capabilities: BTreeSet<Capability>,
    #[serde(default)]
    pub mcp_endpoints: Vec<McpEndpoint>,
    #[serde(default)]
    pub tools: Vec<ToolContract>,
    #[serde(default)]
    pub skill: Option<PathBuf>,
    #[serde(default)]
    pub rules: Vec<PathBuf>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(skip)]
    install_root: PathBuf,
}
fn default_timeout() -> u64 {
    5_000
}

impl PluginManifest {
    pub fn load(path: &Path) -> Result<Self> {
        let install_root = path
            .parent()
            .unwrap_or(Path::new("."))
            .canonicalize()
            .context("canonicalize plugin install root")?;
        Self::load_with_root(path, &install_root)
    }
    fn load_with_root(path: &Path, install_root: &Path) -> Result<Self> {
        let mut value: Self = toml::from_str(&fs::read_to_string(path)?)?;
        let root = path.parent().unwrap_or(Path::new("."));
        if value.executable.is_relative() {
            value.executable = root.join(&value.executable);
        }
        if let Some(skill) = &mut value.skill
            && skill.is_relative()
        {
            *skill = root.join(&*skill);
        }
        for rule in &mut value.rules {
            if rule.is_relative() {
                *rule = root.join(&*rule);
            }
        }
        value.install_root = install_root.to_path_buf();
        value.executable = confined_path(install_root, &value.executable, "executable")?;
        if let Some(skill) = &mut value.skill {
            *skill = confined_path(install_root, skill, "skill")?;
        }
        for rule in &mut value.rules {
            *rule = confined_path(install_root, rule, "rule")?;
        }
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty()
            || self
                .id
                .chars()
                .any(|c| !(c.is_ascii_alphanumeric() || ".-_".contains(c)))
        {
            bail!("invalid plugin id")
        }
        if self.protocol.name != HOST_PROTOCOL
            || self.protocol.min_version > HOST_PROTOCOL_VERSION
            || self.protocol.max_version < HOST_PROTOCOL_VERSION
        {
            bail!("incompatible plugin protocol requirement")
        }
        if !self.executable.is_file() {
            bail!(
                "plugin executable does not exist: {}",
                self.executable.display()
            )
        }
        for tool in &self.tools {
            if !tool.input_schema.is_object() {
                bail!("tool {} schema must be an object", tool.name)
            }
        }
        Ok(())
    }
    pub fn instructions(&self) -> Result<String> {
        let mut out = String::new();
        if let Some(p) = &self.skill {
            out.push_str(&fs::read_to_string(p).with_context(|| format!("read {}", p.display()))?);
        }
        for p in &self.rules {
            out.push('\n');
            out.push_str(&fs::read_to_string(p).with_context(|| format!("read {}", p.display()))?);
        }
        Ok(out)
    }
}

fn confined_path(install_root: &Path, path: &Path, kind: &str) -> Result<PathBuf> {
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        bail!("plugin {kind} contains path traversal")
    }
    let canonical = path
        .canonicalize()
        .with_context(|| format!("canonicalize plugin {kind} {}", path.display()))?;
    if !canonical.starts_with(install_root) {
        bail!("plugin {kind} escapes install root")
    }
    Ok(canonical)
}

pub fn discover(roots: &[PathBuf]) -> Result<Vec<PluginManifest>> {
    let mut found = Vec::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        let install_root = root.canonicalize()?;
        for entry in fs::read_dir(&install_root)? {
            let path = entry?.path();
            let manifest = if path.is_dir() {
                path.join("plugin.toml")
            } else {
                path
            };
            if manifest.file_name().is_some_and(|n| n == "plugin.toml") {
                let plugin_root = manifest.parent().unwrap_or(&install_root).canonicalize()?;
                if !plugin_root.starts_with(&install_root) {
                    bail!("plugin manifest escapes install root")
                }
                found.push(PluginManifest::load_with_root(&manifest, &plugin_root)?);
            }
        }
    }
    found.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(found)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerRequest {
    pub capability: Capability,
    pub operation: String,
    pub arguments: Value,
}
pub trait EffectBroker: Send + Sync {
    /// Persists spawn intent and returns its durable operation id before the
    /// plugin process may be created.
    fn begin_spawn(&self, plugin: &str, executable: &Path) -> Result<uuid::Uuid>;
    fn finish_spawn(&self, operation_id: uuid::Uuid, succeeded: bool) -> Result<()>;
    fn execute(&self, plugin: &str, request: BrokerRequest) -> Result<Value>;
}

#[derive(Debug, Serialize, Deserialize)]
struct Request {
    id: u64,
    method: String,
    params: Value,
}
#[derive(Debug, Serialize, Deserialize)]
struct Response {
    id: u64,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    effect: Option<BrokerRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    Disabled,
    Stopped,
    Healthy,
    Unhealthy(String),
    Crashed(String),
    TimedOut,
}

pub struct PluginHost<B: EffectBroker> {
    manifest: PluginManifest,
    broker: B,
    enabled: bool,
    child: Option<Child>,
    input: Option<ChildStdin>,
    output: Option<BufReader<ChildStdout>>,
    next_id: u64,
    spawn_operation: Option<uuid::Uuid>,
    health: Health,
}
impl<B: EffectBroker> PluginHost<B> {
    pub fn new(manifest: PluginManifest, broker: B) -> Self {
        Self {
            manifest,
            broker,
            enabled: false,
            child: None,
            input: None,
            output: None,
            next_id: 1,
            spawn_operation: None,
            health: Health::Disabled,
        }
    }
    pub fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
    pub fn health(&self) -> &Health {
        &self.health
    }
    pub fn enable(&mut self) -> Result<()> {
        self.enabled = true;
        self.start()
    }
    pub fn disable(&mut self) -> Result<()> {
        if self.child.is_some() {
            let _ = self.call("lifecycle.stop", json!({}));
        }
        self.kill();
        self.enabled = false;
        self.health = Health::Disabled;
        Ok(())
    }
    pub fn start(&mut self) -> Result<()> {
        if !self.enabled {
            bail!("plugin is disabled")
        }
        self.kill();
        let operation_id = self
            .broker
            .begin_spawn(&self.manifest.id, &self.manifest.executable)?;
        self.spawn_operation = Some(operation_id);
        let child_result = Command::new(&self.manifest.executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env_clear()
            .spawn();
        let mut child = match child_result {
            Ok(child) => child,
            Err(error) => {
                self.spawn_operation = None;
                self.broker.finish_spawn(operation_id, false)?;
                return Err(error).context("spawn plugin");
            }
        };
        self.input = child.stdin.take();
        self.output = child.stdout.take().map(BufReader::new);
        self.child = Some(child);
        match self.call("initialize", json!({"protocol": HOST_PROTOCOL, "version": HOST_PROTOCOL_VERSION, "capabilities": self.manifest.capabilities})) { Ok(_) => { self.health = Health::Healthy; Ok(()) }, Err(e) => { self.health = Health::Unhealthy(e.to_string()); self.kill(); Err(e) } }
    }
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        if !self.enabled {
            bail!("plugin is disabled")
        }
        let id = self.next_id;
        self.next_id += 1;
        let request = serde_json::to_string(&Request {
            id,
            method: method.into(),
            params,
        })?;
        writeln!(
            self.input.as_mut().context("plugin is not running")?,
            "{request}"
        )?;
        self.input.as_mut().unwrap().flush()?;
        let mut reader = self.output.take().context("plugin output unavailable")?;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut line = String::new();
            let result = reader.read_line(&mut line).map(|_| (reader, line));
            let _ = tx.send(result);
        });
        let (returned, line) =
            match rx.recv_timeout(Duration::from_millis(self.manifest.timeout_ms)) {
                Ok(v) => v?,
                Err(_) => {
                    self.health = Health::TimedOut;
                    self.kill();
                    bail!("plugin request timed out")
                }
            };
        self.output = Some(returned);
        if line.is_empty() {
            self.health = Health::Crashed("unexpected EOF".into());
            self.kill();
            bail!("plugin crashed")
        }
        let response: Response = serde_json::from_str(&line).context("invalid plugin response")?;
        if response.id != id {
            bail!("plugin response id mismatch")
        }
        if let Some(error) = response.error {
            bail!("plugin error: {error}")
        }
        if let Some(effect) = response.effect {
            if !self.manifest.capabilities.contains(&effect.capability) {
                bail!("undeclared capability: {:?}", effect.capability)
            }
            return self.broker.execute(&self.manifest.id, effect);
        }
        Ok(response.result.unwrap_or(Value::Null))
    }
    pub fn check_health(&mut self) -> Health {
        if !self.enabled {
            return Health::Disabled;
        }
        if self
            .child
            .as_mut()
            .and_then(|c| c.try_wait().ok())
            .flatten()
            .is_some()
        {
            self.health = Health::Crashed("process exited".into());
        } else if self.call("health", json!({})).is_err()
            && !matches!(self.health, Health::TimedOut)
        {
            self.health = Health::Unhealthy("health check failed".into());
        }
        self.health.clone()
    }
    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(operation_id) = self.spawn_operation.take() {
            let succeeded = !matches!(
                self.health,
                Health::Crashed(_) | Health::TimedOut | Health::Unhealthy(_)
            );
            let _ = self.broker.finish_spawn(operation_id, succeeded);
        }
        self.input = None;
        self.output = None;
        if self.enabled && !matches!(self.health, Health::TimedOut | Health::Crashed(_)) {
            self.health = Health::Stopped;
        }
    }
}
impl<B: EffectBroker> Drop for PluginHost<B> {
    fn drop(&mut self) {
        self.kill();
    }
}

pub struct Registry {
    plugins: BTreeMap<String, PluginManifest>,
    tools: BTreeMap<String, (String, ToolContract)>,
    endpoints: BTreeMap<String, (String, McpEndpoint)>,
}
impl Registry {
    pub fn build(manifests: Vec<PluginManifest>) -> Result<Self> {
        let mut r = Self {
            plugins: BTreeMap::new(),
            tools: BTreeMap::new(),
            endpoints: BTreeMap::new(),
        };
        for p in manifests {
            if r.plugins.contains_key(&p.id) {
                bail!("duplicate plugin id: {}", p.id)
            }
            for t in &p.tools {
                if r.tools
                    .insert(t.name.clone(), (p.id.clone(), t.clone()))
                    .is_some()
                {
                    bail!("duplicate tool: {}", t.name)
                }
            }
            for e in &p.mcp_endpoints {
                if r.endpoints
                    .insert(e.name.clone(), (p.id.clone(), e.clone()))
                    .is_some()
                {
                    bail!("duplicate endpoint: {}", e.name)
                }
            }
            r.plugins.insert(p.id.clone(), p);
        }
        Ok(r)
    }
    pub fn tool(&self, name: &str) -> Option<&(String, ToolContract)> {
        self.tools.get(name)
    }
    pub fn endpoint(&self, name: &str) -> Option<&(String, McpEndpoint)> {
        self.endpoints.get(name)
    }
}
