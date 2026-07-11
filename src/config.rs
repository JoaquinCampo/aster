use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub context: ContextConfig,
    pub memory: MemoryConfig,
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
#[derive(Debug, Clone, Default, Deserialize)]
struct ConfigPatch {
    version: Option<u32>,
    context: Option<ContextPatch>,
    memory: Option<MemoryPatch>,
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
        if self.version != 1 {
            bail!("unsupported config version {}", self.version)
        }
        let sum: u32 = self.context.category_tokens.values().sum();
        if sum > self.context.total_tokens {
            bail!("category budgets exceed total context budget")
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
        let config: Config = toml::from_str(std::str::from_utf8(&bytes)?)?;
        config.validate()?;
        Ok(Self {
            path,
            config,
            baseline_hash: hash(&bytes),
        })
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
pub fn load_layered(paths: &[PathBuf]) -> Result<Config> {
    let mut out = Config {
        version: 1,
        ..Config::default()
    };
    for p in paths {
        if p.exists() {
            let bytes = fs::read(p).with_context(|| format!("read {}", p.display()))?;
            let patch: ConfigPatch = toml::from_str(std::str::from_utf8(&bytes)?)?;
            out.apply(patch);
        }
    }
    out.validate()?;
    Ok(out)
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
