use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Trust {
    TrustedInstruction,
    UntrustedContent,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub path: PathBuf,
    pub ecosystem: String,
    pub trust: Trust,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextItem {
    pub category: String,
    pub content: String,
    pub estimated_tokens: u32,
    pub provenance: Provenance,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextManifest {
    pub budget: u32,
    pub used: u32,
    pub items: Vec<ContextItem>,
    pub excluded: Vec<PathBuf>,
}
impl ContextManifest {
    pub fn build(budget: u32, items: Vec<ContextItem>) -> Result<Self> {
        let used = items.iter().map(|x| x.estimated_tokens).sum();
        if used > budget {
            bail!("context budget exceeded: {used}/{budget}")
        }
        Ok(Self {
            budget,
            used,
            items,
            excluded: vec![],
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredAsset {
    pub path: PathBuf,
    pub relative_key: PathBuf,
    pub ecosystem: String,
    pub supported: bool,
    pub reason: Option<String>,
}

pub fn discover(project_root: &Path, working_dir: &Path) -> Result<Vec<DiscoveredAsset>> {
    let project_root = project_root.canonicalize()?;
    let working_dir = working_dir.canonicalize()?;
    if !working_dir.starts_with(&project_root) {
        bail!("working directory is outside project root")
    }
    let mut dirs = vec![];
    let mut cur = working_dir;
    loop {
        dirs.push(cur.clone());
        if cur == project_root {
            break;
        }
        cur = cur
            .parent()
            .ok_or_else(|| anyhow::anyhow!("working directory is outside project root"))?
            .to_path_buf();
    }
    dirs.reverse();
    let mut merged: BTreeMap<PathBuf, DiscoveredAsset> = BTreeMap::new();
    for dir in dirs {
        let scope = dir.strip_prefix(&project_root).unwrap_or(Path::new(""));
        for eco in [".agents", ".claude"] {
            let base = dir.join(eco);
            for name in ["AGENTS.md", "CLAUDE.md"] {
                let p = base.join(name);
                if p.is_file() {
                    insert(&mut merged, scope, &base, p, eco, true, None)
                }
            }
            let skills = base.join("skills");
            if skills.is_dir() {
                walk_skills(&skills, scope, &base, eco, &mut merged)?
            }
            if base.is_dir() {
                for entry in fs::read_dir(&base)? {
                    let p = entry?.path();
                    if p.is_file()
                        && !matches!(
                            p.file_name().and_then(|x| x.to_str()),
                            Some("AGENTS.md" | "CLAUDE.md")
                        )
                    {
                        insert(
                            &mut merged,
                            scope,
                            &base,
                            p,
                            eco,
                            false,
                            Some("unsupported asset type".into()),
                        )
                    }
                }
            }
        }
    }
    Ok(merged.into_values().collect())
}
fn insert(
    map: &mut BTreeMap<PathBuf, DiscoveredAsset>,
    scope: &Path,
    base: &Path,
    path: PathBuf,
    eco: &str,
    supported: bool,
    reason: Option<String>,
) {
    let relative = path.strip_prefix(base).unwrap_or(&path);
    let key = scope.join(relative);
    let asset = DiscoveredAsset {
        path,
        relative_key: key.clone(),
        ecosystem: eco.into(),
        supported,
        reason,
    };
    if eco == ".claude" || !map.contains_key(&key) {
        map.insert(key, asset);
    }
}
fn walk_skills(
    dir: &Path,
    scope: &Path,
    base: &Path,
    eco: &str,
    map: &mut BTreeMap<PathBuf, DiscoveredAsset>,
) -> Result<()> {
    for e in fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            walk_skills(&p, scope, base, eco, map)?
        } else if p.file_name().and_then(|x| x.to_str()) == Some("SKILL.md") {
            insert(map, scope, base, p, eco, true, None)
        }
    }
    Ok(())
}
pub fn manifest_from_assets(assets: &[DiscoveredAsset], budget: u32) -> Result<ContextManifest> {
    let mut items = vec![];
    for a in assets.iter().filter(|a| a.supported) {
        let content = fs::read_to_string(&a.path)?;
        items.push(ContextItem {
            category: "project_rules".into(),
            estimated_tokens: (content.len() as u32).div_ceil(4),
            content,
            provenance: Provenance {
                path: a.path.clone(),
                ecosystem: a.ecosystem.clone(),
                trust: Trust::UntrustedContent,
            },
        })
    }
    ContextManifest::build(budget, items)
}
