use crate::{
    domain::{Effort, Role},
    routing::ModelProfile,
};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutingPolicy {
    pub schema_version: u32,
    pub revision: u64,
    pub reviewed: bool,
    pub models: Vec<ModelProfile>,
    pub defaults: RouteDefaults,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouteDefaults {
    pub effort: Effort,
    pub output_tokens: u32,
    pub tools: Vec<String>,
    pub capabilities: Vec<String>,
    pub isolation: Vec<String>,
    pub lifecycle: String,
    pub verification: String,
}
impl RoutingPolicy {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let policy: Self = toml::from_str(&fs::read_to_string(path)?)?;
        policy.validate()?;
        Ok(policy)
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            bail!("unsupported routing policy schema {}", self.schema_version)
        }
        if self.revision == 0 || self.models.is_empty() {
            bail!("routing policy requires a revision and models")
        }
        if self
            .models
            .iter()
            .any(|m| m.id.is_empty() || m.quality > 100)
        {
            bail!("invalid model profile")
        }
        if !self.reviewed {
            bail!("active routing policy revision must be explicitly reviewed")
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutcomeAggregate {
    pub policy_revision: u64,
    pub role: Role,
    pub model: String,
    pub attempts: u64,
    pub verified_successes: u64,
    pub failures: u64,
    pub total_cost_micros: u64,
    pub total_latency_ms: u64,
}
impl OutcomeAggregate {
    pub fn success_rate_millis(&self) -> u16 {
        self.verified_successes
            .saturating_mul(1000)
            .checked_div(self.attempts)
            .unwrap_or(0) as u16
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyRecommendation {
    pub based_on_revision: u64,
    pub proposed_revision: u64,
    pub model: String,
    pub reason: String,
    pub evidence_attempts: u64,
    pub reviewed: bool,
}

/// Produces advisory evidence only. It never mutates the active policy.
pub fn recommend(
    policy: &RoutingPolicy,
    outcomes: &[OutcomeAggregate],
) -> Vec<PolicyRecommendation> {
    let mut by_model: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    for o in outcomes
        .iter()
        .filter(|o| o.policy_revision == policy.revision)
    {
        let e = by_model.entry(&o.model).or_default();
        e.0 += o.attempts;
        e.1 += o.verified_successes;
    }
    by_model
        .into_iter()
        .filter(|(_, (n, _))| *n >= 3)
        .map(|(model, (n, ok))| PolicyRecommendation {
            based_on_revision: policy.revision,
            proposed_revision: policy.revision + 1,
            model: model.into(),
            reason: format!("historical verified success {ok}/{n}; review before activation"),
            evidence_attempts: n,
            reviewed: false,
        })
        .collect()
}

pub fn apply_reviewed_revision(
    current: &RoutingPolicy,
    candidate: RoutingPolicy,
    recommendation: &PolicyRecommendation,
) -> Result<RoutingPolicy> {
    if !recommendation.reviewed || !candidate.reviewed {
        bail!("learned recommendation requires explicit review")
    }
    if recommendation.based_on_revision != current.revision
        || candidate.revision != recommendation.proposed_revision
        || candidate.revision <= current.revision
    {
        bail!("policy revision does not match reviewed recommendation")
    }
    candidate.validate()?;
    Ok(candidate)
}
