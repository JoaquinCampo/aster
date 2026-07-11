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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalCandidate {
    pub item: ContextItem,
    pub relevance: f32,
    pub content_version: String,
    pub fresh: bool,
    pub critical: bool,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextMetrics {
    pub input_tokens: u32,
    pub selected_tokens: u32,
    pub duplicate_tokens_avoided: u32,
    pub compressed_tokens_avoided: u32,
    pub omitted_relevant_items: u32,
    pub omission_rework_count: u32,
    pub stale_items_invalidated: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalResult {
    pub manifest: ContextManifest,
    pub metrics: ContextMetrics,
}

pub fn retrieve_relevant(
    total_budget: u32,
    category_budgets: &BTreeMap<String, u32>,
    mut candidates: Vec<RetrievalCandidate>,
) -> Result<RetrievalResult> {
    use std::collections::{BTreeSet, HashMap};
    let input_tokens = candidates.iter().map(|c| c.item.estimated_tokens).sum();
    let stale_items_invalidated = candidates.iter().filter(|c| !c.fresh).count() as u32;
    candidates.retain(|c| c.fresh);
    candidates.sort_by(|a, b| {
        b.critical
            .cmp(&a.critical)
            .then_with(|| b.relevance.total_cmp(&a.relevance))
            .then_with(|| a.item.provenance.path.cmp(&b.item.provenance.path))
    });
    let mut used_category: HashMap<String, u32> = HashMap::new();
    let mut fingerprints = BTreeSet::new();
    let mut selected = Vec::new();
    let mut used = 0;
    let mut metrics = ContextMetrics {
        input_tokens,
        stale_items_invalidated,
        ..Default::default()
    };
    for c in candidates {
        let fingerprint = format!("{}:{}", c.content_version, c.item.content.trim());
        if !fingerprints.insert(fingerprint) {
            metrics.duplicate_tokens_avoided += c.item.estimated_tokens;
            continue;
        }
        let cap = category_budgets
            .get(&c.item.category)
            .copied()
            .unwrap_or(total_budget);
        let cat_used = used_category.get(&c.item.category).copied().unwrap_or(0);
        if used + c.item.estimated_tokens > total_budget || cat_used + c.item.estimated_tokens > cap
        {
            if c.critical {
                bail!(
                    "critical constraint cannot fit context/category budget: {}",
                    c.item.provenance.path.display()
                )
            }
            if c.relevance > 0.0 {
                metrics.omitted_relevant_items += 1;
            }
            continue;
        }
        used += c.item.estimated_tokens;
        *used_category.entry(c.item.category.clone()).or_default() += c.item.estimated_tokens;
        selected.push(c.item);
    }
    metrics.selected_tokens = used;
    Ok(RetrievalResult {
        manifest: ContextManifest::build(total_budget, selected)?,
        metrics,
    })
}

pub fn record_omission_rework(metrics: &mut ContextMetrics) {
    metrics.omission_rework_count += 1;
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
