use crate::effects::{Approval, Capability, EffectAdapter, EffectRequest, ScopedGrant};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout},
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
    #[serde(default)]
    pub destination: Option<String>,
    #[serde(default)]
    pub context_classes: Vec<String>,
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
    /// Persists legacy hook spawn intent. Plugin process launches use the core
    /// effect broker instead.
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
            health: Health::Disabled,
        }
    }
    pub fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
    pub fn health(&self) -> &Health {
        &self.health
    }
    pub fn enable<A: EffectAdapter>(
        &mut self,
        process_broker: &crate::effects::EffectBroker<'_, A>,
        grant: &ScopedGrant,
        approval: &Approval,
    ) -> Result<()> {
        self.enabled = true;
        self.start(process_broker, grant, approval)
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
    pub fn start<A: EffectAdapter>(
        &mut self,
        process_broker: &crate::effects::EffectBroker<'_, A>,
        grant: &ScopedGrant,
        approval: &Approval,
    ) -> Result<()> {
        if !self.enabled {
            bail!("plugin is disabled")
        }
        self.kill();
        let request = EffectRequest::Exec {
            program: self.manifest.executable.clone(),
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: self.manifest.install_root.clone(),
        };
        let (_, mut child) = process_broker
            .spawn_authorized_interactive(grant, Some(approval), request)
            .context("spawn plugin")?;
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompatibilityDiagnostic {
    pub compatible: bool,
    pub plugin_id: Option<String>,
    pub plugin_version: Option<String>,
    pub messages: Vec<String>,
}

pub fn diagnose(source: &Path) -> CompatibilityDiagnostic {
    match PluginManifest::load(&source.join("plugin.toml")) {
        Ok(manifest) => CompatibilityDiagnostic {
            compatible: true,
            plugin_id: Some(manifest.id),
            plugin_version: Some(manifest.version),
            messages: vec![format!(
                "compatible with {HOST_PROTOCOL} protocol v{HOST_PROTOCOL_VERSION}"
            )],
        },
        Err(error) => CompatibilityDiagnostic {
            compatible: false,
            plugin_id: None,
            plugin_version: None,
            messages: vec![error.to_string()],
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallAction {
    Installed,
    Upgraded { from: String },
    Uninstalled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReceipt {
    pub plugin_id: String,
    pub version: String,
    pub action: InstallAction,
}

pub struct PluginInstaller {
    root: PathBuf,
}
impl PluginInstaller {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn install(&self, source: &Path) -> Result<InstallReceipt> {
        fs::create_dir_all(&self.root)?;
        let candidate = PluginManifest::load(&source.join("plugin.toml"))?;
        let destination = self.root.join(&candidate.id);
        let staging = self.root.join(format!(".{}.staging", candidate.id));
        let backup = self.root.join(format!(".{}.rollback", candidate.id));
        remove_any(&staging)?;
        copy_tree(source, &staging).context("stage plugin")?;
        let staged =
            PluginManifest::load_with_root(&staging.join("plugin.toml"), &staging.canonicalize()?)
                .context("validate staged plugin")?;
        if staged.id != candidate.id || staged.version != candidate.version {
            bail!("staged plugin identity changed")
        }
        let previous = if destination.exists() {
            Some(PluginManifest::load(&destination.join("plugin.toml"))?.version)
        } else {
            None
        };
        remove_any(&backup)?;
        if destination.exists() {
            fs::rename(&destination, &backup)?;
        }
        if let Err(error) = fs::rename(&staging, &destination) {
            if backup.exists() {
                let _ = fs::rename(&backup, &destination);
            }
            return Err(error).context("activate plugin; previous installation restored");
        }
        if let Err(error) = PluginManifest::load(&destination.join("plugin.toml")) {
            remove_any(&destination)?;
            if backup.exists() {
                fs::rename(&backup, &destination)?;
            }
            return Err(error)
                .context("post-install validation failed; previous installation restored");
        }
        remove_any(&backup)?;
        Ok(InstallReceipt {
            plugin_id: candidate.id,
            version: candidate.version,
            action: previous.map_or(InstallAction::Installed, |from| InstallAction::Upgraded {
                from,
            }),
        })
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let destination = self.root.join(id);
        PluginManifest::load(&destination.join("plugin.toml"))
            .context("plugin is not installed")?;
        let state = destination.join(".enabled");
        if enabled {
            fs::write(state, "enabled\n")?;
        } else if state.exists() {
            fs::remove_file(state)?;
        }
        Ok(())
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        self.root.join(id).join(".enabled").is_file()
    }

    pub fn diagnostics(&self) -> Result<Vec<CompatibilityDiagnostic>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let path = entry?.path();
            if path.is_dir()
                && !path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            {
                out.push(diagnose(&path));
            }
        }
        out.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
        Ok(out)
    }

    pub fn uninstall(&self, id: &str) -> Result<InstallReceipt> {
        if id.is_empty()
            || id
                .chars()
                .any(|c| !(c.is_ascii_alphanumeric() || ".-_".contains(c)))
        {
            bail!("invalid plugin id")
        }
        let destination = self.root.join(id);
        let manifest = PluginManifest::load(&destination.join("plugin.toml"))
            .context("plugin is not installed")?;
        let quarantine = self.root.join(format!(".{id}.uninstalling"));
        remove_any(&quarantine)?;
        fs::rename(&destination, &quarantine)?;
        if let Err(error) = remove_any(&quarantine) {
            let _ = fs::rename(&quarantine, &destination);
            return Err(error).context("uninstall failed; plugin restored");
        }
        Ok(InstallReceipt {
            plugin_id: manifest.id,
            version: manifest.version,
            action: InstallAction::Uninstalled,
        })
    }
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    if !source.is_dir() {
        bail!("plugin source is not a directory")
    }
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if entry.file_type()?.is_file() {
            fs::copy(entry.path(), target)?;
        } else {
            bail!("plugin source contains unsupported filesystem entry")
        }
    }
    Ok(())
}
fn remove_any(path: &Path) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
