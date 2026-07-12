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
        [
            "context.total_tokens",
            "memory.enabled",
            "providers.enabled",
            "providers.default_provider",
            "providers.auth_ref",
            "providers.endpoint",
            "models.enabled",
            "models.default_model",
            "models.reasoning_effort",
            "models.allow",
            "roles.enabled",
            "roles.default_role",
            "roles.available",
            "routing.enabled",
            "routing.policy_path",
            "routing.fallback_provider",
            "routing.fallback_model",
            "budgets.enabled",
            "budgets.token_budget",
            "budgets.timeout_ms",
            "budgets.cost_limit_usd",
            "permissions.enabled",
            "permissions.default_allow",
            "permissions.approval_ttl_secs",
            "permissions.policy_path",
            "tools_mcp.enabled",
            "tools_mcp.servers",
            "tools_mcp.allow_tools",
            "tools_mcp.command_timeout_ms",
            "skills_rules.enabled",
            "skills_rules.skill_paths",
            "skills_rules.rule_paths",
            "skills_rules.max_context_tokens",
            "hooks_plugins.enabled",
            "hooks_plugins.hook_paths",
            "hooks_plugins.plugin_paths",
            "hooks_plugins.fail_closed",
            "persistence.enabled",
            "persistence.database_path",
            "persistence.memory_path",
            "persistence.artifacts_path",
            "tui.enabled",
            "tui.refresh_ms",
            "tui.compact",
            "tui.color",
            "verification.enabled",
            "verification.commands",
            "verification.require_clean_tree",
            "verification.timeout_ms",
            "lifecycle.enabled",
            "lifecycle.concurrency",
            "lifecycle.retry_limit",
            "lifecycle.task_timeout_ms",
            "lifecycle.shutdown_grace_ms",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }
    pub fn edit_required(&mut self, field: &str, value: &str) -> Result<()> {
        if !Self::editable_fields()
            .iter()
            .any(|candidate| candidate == field)
        {
            bail!("field is not editable in the TUI: {field}")
        }
        let mut root = toml::Value::try_from(&self.config)?;
        let (section, property) = field
            .split_once('.')
            .context("configuration field must be nested")?;
        let table = root
            .as_table_mut()
            .context("configuration root must be a table")?;
        let section = table
            .entry(section)
            .or_insert_with(|| toml::Value::Table(Default::default()));
        let section = section
            .as_table_mut()
            .context("configuration section must be a table")?;
        let parsed: toml::Value = toml::from_str::<toml::Value>(&format!("value = {value}"))?
            .get("value")
            .cloned()
            .context("configuration value is missing")?;
        section.insert(property.to_owned(), parsed);
        let candidate: Config = root.try_into()?;
        candidate.validate()?;
        self.config = candidate;
        Ok(())
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
    let default = Config {
        version: CURRENT_CONFIG_VERSION,
        ..Config::default()
    };
    let mut out = toml::Value::try_from(default)?;
    for p in paths {
        if p.exists() {
            let text = fs::read_to_string(p).with_context(|| format!("read {}", p.display()))?;
            let parsed = toml::from_str::<toml::Value>(&text)?;
            let migrated = if parsed.get("version").is_some() {
                migrate(&text)?
            } else {
                text
            };
            merge_toml(&mut out, toml::from_str(&migrated)?);
        }
    }
    let out: Config = out.try_into()?;
    out.validate()?;
    Ok(out)
}

fn merge_toml(base: &mut toml::Value, patch: toml::Value) {
    match (base, patch) {
        (toml::Value::Table(base), toml::Value::Table(patch)) => {
            for (key, value) in patch {
                match base.get_mut(&key) {
                    Some(existing) => merge_toml(existing, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (base, patch) => *base = patch,
    }
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
