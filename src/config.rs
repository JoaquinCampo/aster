use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub const CURRENT_CONFIG_VERSION: u32 = 2;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub context: ContextConfig,
    pub memory: MemoryConfig,
    pub providers: ProvidersConfig,
    pub models: ModelsConfig,
    pub roles: RolesConfig,
    pub routing: RoutingConfig,
    pub budgets: BudgetsConfig,
    pub permissions: PermissionsConfig,
    pub tools_mcp: ToolsMcpConfig,
    pub skills_rules: SkillsRulesConfig,
    pub hooks_plugins: HooksPluginsConfig,
    pub persistence: PersistenceConfig,
    pub tui: TuiConfig,
    pub verification: VerificationConfig,
    pub lifecycle: LifecycleConfig,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, toml::Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ContextConfig {
    pub total_tokens: u32,
    pub category_tokens: BTreeMap<String, u32>,
}
impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            total_tokens: 32_000,
            category_tokens: BTreeMap::new(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MemoryConfig {
    pub enabled: bool,
}
impl Default for MemoryConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}
macro_rules! domain_config {
    ($name:ident { $($field:ident : $ty:ty = $default:expr),* $(,)? }) => {
        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
        #[serde(default)]
        pub struct $name {
            pub enabled: bool,
            $(pub $field: $ty,)*
            #[serde(flatten)]
            pub extensions: BTreeMap<String, toml::Value>,
        }
        impl Default for $name {
            fn default() -> Self { Self { enabled: false, $($field: $default,)* extensions: BTreeMap::new() } }
        }
    };
}

domain_config!(ProvidersConfig { default_provider: Option<String> = None, auth_ref: Option<String> = None, endpoint: Option<String> = None });
domain_config!(ModelsConfig { default_model: Option<String> = None, reasoning_effort: Option<String> = None, allow: Vec<String> = Vec::new() });
domain_config!(RolesConfig { default_role: Option<String> = None, available: Vec<String> = Vec::new() });
domain_config!(RoutingConfig { policy_path: Option<PathBuf> = None, fallback_provider: Option<String> = None, fallback_model: Option<String> = None });
domain_config!(BudgetsConfig { token_budget: Option<u64> = None, timeout_ms: Option<u64> = None, cost_limit_usd: Option<f64> = None });
domain_config!(PermissionsConfig { default_allow: bool = false, approval_ttl_secs: u64 = 300, policy_path: Option<PathBuf> = None });
domain_config!(ToolsMcpConfig { servers: Vec<String> = Vec::new(), allow_tools: Vec<String> = Vec::new(), command_timeout_ms: u64 = 30_000 });
domain_config!(SkillsRulesConfig { skill_paths: Vec<PathBuf> = Vec::new(), rule_paths: Vec<PathBuf> = Vec::new(), max_context_tokens: Option<u32> = None });
domain_config!(HooksPluginsConfig { hook_paths: Vec<PathBuf> = Vec::new(), plugin_paths: Vec<PathBuf> = Vec::new(), fail_closed: bool = true });
domain_config!(PersistenceConfig { database_path: Option<PathBuf> = None, memory_path: Option<PathBuf> = None, artifacts_path: Option<PathBuf> = None });
domain_config!(TuiConfig {
    refresh_ms: u64 = 250,
    compact: bool = false,
    color: bool = true
});
domain_config!(VerificationConfig { commands: Vec<String> = Vec::new(), require_clean_tree: bool = false, timeout_ms: u64 = 120_000 });
domain_config!(LifecycleConfig { concurrency: usize = 1, retry_limit: u32 = 0, task_timeout_ms: Option<u64> = None, shutdown_grace_ms: u64 = 5_000 });
#[derive(Debug, Clone, Default, Deserialize)]
struct ConfigPatch {
    version: Option<u32>,
    context: Option<ContextPatch>,
    memory: Option<MemoryPatch>,
    providers: Option<ProvidersConfig>,
    models: Option<ModelsConfig>,
    roles: Option<RolesConfig>,
    routing: Option<RoutingConfig>,
    budgets: Option<BudgetsConfig>,
    permissions: Option<PermissionsConfig>,
    tools_mcp: Option<ToolsMcpConfig>,
    skills_rules: Option<SkillsRulesConfig>,
    hooks_plugins: Option<HooksPluginsConfig>,
    persistence: Option<PersistenceConfig>,
    tui: Option<TuiConfig>,
    verification: Option<VerificationConfig>,
    lifecycle: Option<LifecycleConfig>,
    #[serde(flatten)]
    extensions: BTreeMap<String, toml::Value>,
}
#[derive(Debug, Clone, Default, Deserialize)]
struct ContextPatch {
    total_tokens: Option<u32>,
    category_tokens: Option<BTreeMap<String, u32>>,
}
#[derive(Debug, Clone, Default, Deserialize)]
struct MemoryPatch {
    enabled: Option<bool>,
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        if self.version != CURRENT_CONFIG_VERSION {
            bail!(
                "unsupported config version {} (current {})",
                self.version,
                CURRENT_CONFIG_VERSION
            )
        }
        if self.context.total_tokens == 0 {
            bail!("context.total_tokens must be greater than zero")
        }
        let sum: u32 = self.context.category_tokens.values().sum();
        if sum > self.context.total_tokens {
            bail!("category budgets exceed total context budget")
        }
        if self.lifecycle.concurrency == 0 {
            bail!("lifecycle.concurrency must be greater than zero")
        }
        if self.tui.refresh_ms == 0
            || self.verification.timeout_ms == 0
            || self.tools_mcp.command_timeout_ms == 0
        {
            bail!("configured timeouts and refresh intervals must be greater than zero")
        }
        if let Some(limit) = self.budgets.cost_limit_usd
            && (!limit.is_finite() || limit < 0.0)
        {
            bail!("budgets.cost_limit_usd must be finite and non-negative")
        }
        if let Some(auth_ref) = &self.providers.auth_ref {
            let valid = auth_ref.starts_with("env:")
                || auth_ref.starts_with("keychain:")
                || auth_ref.starts_with("file:");
            if !valid
                || auth_ref
                    .split_once(':')
                    .is_none_or(|(_, value)| value.trim().is_empty())
            {
                bail!(
                    "providers.auth_ref must be a non-empty env:, keychain:, or file: secret reference"
                )
            }
        }
        if self.providers.endpoint.as_deref().is_some_and(|endpoint| {
            !(endpoint.starts_with("https://")
                || endpoint.starts_with("http://localhost")
                || endpoint.starts_with("http://127.0.0.1"))
        }) {
            bail!("providers.endpoint must use https (except loopback endpoints)")
        }
        Ok(())
    }
    fn apply(&mut self, patch: ConfigPatch) {
        if let Some(version) = patch.version {
            self.version = version;
        }
        if let Some(context) = patch.context {
            if let Some(total_tokens) = context.total_tokens {
                self.context.total_tokens = total_tokens;
            }
            if let Some(category_tokens) = context.category_tokens {
                self.context.category_tokens = category_tokens;
            }
        }
        if let Some(memory) = patch.memory
            && let Some(enabled) = memory.enabled
        {
            self.memory.enabled = enabled;
        }
        macro_rules! apply_domain {
            ($field:ident) => {
                if let Some(value) = patch.$field {
                    self.$field = value;
                }
            };
        }
        apply_domain!(providers);
        apply_domain!(models);
        apply_domain!(roles);
        apply_domain!(routing);
        apply_domain!(budgets);
        apply_domain!(permissions);
        apply_domain!(tools_mcp);
        apply_domain!(skills_rules);
        apply_domain!(hooks_plugins);
        apply_domain!(persistence);
        apply_domain!(tui);
        apply_domain!(verification);
        apply_domain!(lifecycle);
        self.extensions.extend(patch.extensions);
    }
}
#[derive(Debug, Clone)]
pub struct ConfigDocument {
    pub path: PathBuf,
    pub config: Config,
    baseline_hash: String,
}
impl ConfigDocument {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        let text = std::str::from_utf8(&bytes)?;
        let migrated = migrate(text)?;
        let config: Config = toml::from_str(&migrated)?;
        config.validate()?;
        Ok(Self {
            path,
            config,
            baseline_hash: hash(&bytes),
        })
    }
    pub fn editable_fields() -> Vec<String> {
        let mut fields = vec!["context.total_tokens".into(), "memory.enabled".into()];
        for domain in [
            "providers",
            "models",
            "roles",
            "routing",
            "budgets",
            "permissions",
            "tools_mcp",
            "skills_rules",
            "hooks_plugins",
            "persistence",
            "tui",
            "verification",
            "lifecycle",
        ] {
            fields.push(format!("{domain}.enabled"));
        }
        fields
    }
    pub fn edit_required(&mut self, field: &str, value: &str) -> Result<()> {
        match field {
            "context.total_tokens" => self.config.context.total_tokens = value.parse()?,
            "memory.enabled" => self.config.memory.enabled = value.parse()?,
            _ => {
                let (domain, property) = field
                    .split_once('.')
                    .ok_or_else(|| anyhow::anyhow!("invalid configuration field: {field}"))?;
                if property != "enabled" {
                    bail!("field is not editable in the TUI: {field}")
                }
                let enabled: bool = value.parse()?;
                match domain {
                    "providers" => self.config.providers.enabled = enabled,
                    "models" => self.config.models.enabled = enabled,
                    "roles" => self.config.roles.enabled = enabled,
                    "routing" => self.config.routing.enabled = enabled,
                    "budgets" => self.config.budgets.enabled = enabled,
                    "permissions" => self.config.permissions.enabled = enabled,
                    "tools_mcp" => self.config.tools_mcp.enabled = enabled,
                    "skills_rules" => self.config.skills_rules.enabled = enabled,
                    "hooks_plugins" => self.config.hooks_plugins.enabled = enabled,
                    "persistence" => self.config.persistence.enabled = enabled,
                    "tui" => self.config.tui.enabled = enabled,
                    "verification" => self.config.verification.enabled = enabled,
                    "lifecycle" => self.config.lifecycle.enabled = enabled,
                    _ => bail!("field is not editable in the TUI: {field}"),
                }
            }
        }
        self.config.validate()
    }
    pub fn save_atomic(&mut self) -> Result<()> {
        let current = fs::read(&self.path).unwrap_or_default();
        if hash(&current) != self.baseline_hash {
            bail!("configuration conflict: file changed since load")
        }
        self.config.validate()?;
        let bytes = toml::to_string_pretty(&self.config)?.into_bytes();
        let parent = self.path.parent().unwrap_or(Path::new("."));
        let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
        use std::io::Write;
        tmp.write_all(&bytes)?;
        tmp.as_file().sync_all()?;
        tmp.persist(&self.path).map_err(|e| e.error)?;
        self.baseline_hash = hash(&bytes);
        Ok(())
    }
}
pub type ConfigService = ConfigDocument;

/// Migrates a document one released schema at a time. Migration operates on
/// TOML values so fields unknown to this binary survive semantic round trips.
pub fn migrate(input: &str) -> Result<String> {
    let mut value: toml::Value = toml::from_str(input).context("parse configuration")?;
    let table = value
        .as_table_mut()
        .context("configuration root must be a table")?;
    let mut version = table
        .get("version")
        .and_then(toml::Value::as_integer)
        .context("configuration version must be an integer")?;
    if version < 1 {
        bail!("unsupported config version {version}")
    }
    if version > i64::from(CURRENT_CONFIG_VERSION) {
        bail!("unsupported future config version {version}")
    }
    while version < i64::from(CURRENT_CONFIG_VERSION) {
        match version {
            1 => {
                // v2 makes lifecycle explicit while retaining v1 behavior.
                table.entry("lifecycle").or_insert_with(|| {
                    let mut domain = toml::map::Map::new();
                    domain.insert("enabled".into(), toml::Value::Boolean(false));
                    toml::Value::Table(domain)
                });
                version = 2;
                table.insert("version".into(), toml::Value::Integer(version));
            }
            _ => bail!("no migration from config version {version}"),
        }
    }
    Ok(toml::to_string_pretty(&value)?)
}

pub fn load_layered(paths: &[PathBuf]) -> Result<Config> {
    let mut out = Config {
        version: CURRENT_CONFIG_VERSION,
        ..Config::default()
    };
    for p in paths {
        if p.exists() {
            let bytes = fs::read(p).with_context(|| format!("read {}", p.display()))?;
            let text = std::str::from_utf8(&bytes)?;
            let migrated = if toml::from_str::<toml::Value>(text)?
                .get("version")
                .is_some()
            {
                migrate(text)?
            } else {
                text.to_owned()
            };
            let patch: ConfigPatch = toml::from_str(&migrated)?;
            out.apply(patch);
        }
    }
    out.validate()?;
    Ok(out)
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
