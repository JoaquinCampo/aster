use crate::domain::{Effort, ExecutionDimensions, Role, Route};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelProfile {
    pub id: String,
    pub quality: u8,
    pub cost_per_million: u32,
    pub latency_ms: u64,
    pub context_tokens: u32,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserOverrides {
    pub role: Option<Role>,
    pub model: Option<String>,
    pub effort: Option<Effort>,
    pub context_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub capabilities: Option<Vec<String>>,
    pub tools: Option<Vec<String>>,
    pub isolation: Option<Vec<String>>,
    pub lifecycle: Option<String>,
    pub verification: Option<String>,
    pub max_cost_micros: Option<u64>,
    pub max_latency_ms: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRequest {
    pub prompt: String,
    pub required_quality: u8,
    pub estimated_tokens: u32,
    pub overrides: UserOverrides,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConstraintKind {
    Hard,
    Soft,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintEvidence {
    pub kind: ConstraintKind,
    pub constraint: String,
    pub satisfied: bool,
    pub detail: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateEvidence {
    pub model: String,
    pub reliable: bool,
    pub estimated_cost_micros: u64,
    pub constraints: Vec<ConstraintEvidence>,
    pub rejected_reason: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    pub route: Route,
    pub evidence: Vec<CandidateEvidence>,
}
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
#[error("no eligible route: {reason}")]
pub struct NoEligibleRoute {
    pub reason: String,
    pub evidence: Vec<CandidateEvidence>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationBenefit {
    pub delegated: bool,
    pub expected_quality_gain: i16,
    pub expected_cost_delta_micros: i64,
    pub expected_latency_delta_ms: i64,
    pub context_savings_tokens: i64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RoleContract {
    pub role: Role,
    pub purpose: &'static str,
    pub boundaries: &'static str,
    pub expected_inputs: &'static [&'static str],
    pub expected_outputs: &'static [&'static str],
    pub default_context_policy: &'static str,
    pub default_capabilities: &'static [&'static str],
    pub allowed_tools: &'static [&'static str],
    pub verification: &'static str,
    pub fallback: &'static str,
    pub isolation: &'static str,
    pub completion: &'static str,
}
macro_rules! role {
    ($role:expr,$purpose:expr,$boundaries:expr,$inputs:expr,$outputs:expr,$caps:expr,$tools:expr,$verify:expr,$fallback:expr,$isolation:expr,$completion:expr) => {
        RoleContract {
            role: $role,
            purpose: $purpose,
            boundaries: $boundaries,
            expected_inputs: $inputs,
            expected_outputs: $outputs,
            default_context_policy: "minimum task-relevant context; compact before expanding",
            default_capabilities: $caps,
            allowed_tools: $tools,
            verification: $verify,
            fallback: $fallback,
            isolation: $isolation,
            completion: $completion,
        }
    };
}
pub fn built_in_roles() -> Vec<RoleContract> {
    vec![
        role!(
            Role::Orchestrator,
            "decompose, route, and synthesize",
            "does not perform specialist changes",
            &["goal", "constraints"],
            &["task graph", "synthesis"],
            &["workspace:read"],
            &["task", "route"],
            "dependency and acceptance checks",
            "advisor when decomposition is unsafe",
            "read-only control plane",
            "accepted work is synthesized"
        ),
        role!(
            Role::Explorer,
            "locate relevant facts and code",
            "does not modify the workspace",
            &["question", "search scope"],
            &["evidence", "locations"],
            &["workspace:read"],
            &["search", "read"],
            "source-backed findings",
            "advisor when evidence is inconclusive",
            "read-only workspace",
            "findings cite sources"
        ),
        role!(
            Role::Planner,
            "produce an executable plan",
            "does not implement the plan",
            &["goal", "evidence"],
            &["ordered plan", "risks"],
            &["workspace:read"],
            &["read", "task"],
            "coverage of requirements and risks",
            "explorer when facts are missing",
            "read-only workspace",
            "steps have verification"
        ),
        role!(
            Role::Implementer,
            "make scoped changes",
            "does not approve its own correctness",
            &["plan", "workspace"],
            &["changes", "test evidence"],
            &["workspace:read", "workspace:write"],
            &["read", "edit", "test"],
            "tests and deterministic checks",
            "fixer after failed verification",
            "isolated worktree, network denied",
            "requested change passes checks"
        ),
        role!(
            Role::Reviewer,
            "identify correctness and security defects",
            "does not silently alter reviewed work",
            &["diff", "requirements"],
            &["findings", "verdict"],
            &["workspace:read"],
            &["read", "search", "test"],
            "independent evidence per finding",
            "verifier for disputed findings",
            "read-only isolated view",
            "verdict addresses requirements"
        ),
        role!(
            Role::Verifier,
            "evaluate acceptance evidence",
            "does not implement fixes",
            &["artifact", "criteria"],
            &["pass/fail evidence"],
            &["workspace:read", "process:test"],
            &["read", "test"],
            "deterministic acceptance checks",
            "reviewer when checks are ambiguous",
            "read-only except test outputs",
            "every criterion has a result"
        ),
        role!(
            Role::Fixer,
            "repair a verified defect",
            "does not broaden scope",
            &["finding", "failing evidence"],
            &["minimal fix", "regression test"],
            &["workspace:read", "workspace:write"],
            &["read", "edit", "test"],
            "regression plus affected gates",
            "implementer for architectural repair",
            "isolated worktree, network denied",
            "failure is reproduced then resolved"
        ),
        role!(
            Role::Advisor,
            "provide bounded recommendations",
            "does not execute decisions",
            &["decision", "context"],
            &["options", "trade-offs"],
            &["workspace:read"],
            &["read", "search"],
            "claims linked to supplied evidence",
            "explorer when more evidence is needed",
            "read-only, no credentials",
            "recommendation states uncertainty"
        ),
        role!(
            Role::LearningCapture,
            "capture reusable verified learning",
            "does not store secrets or unverified claims",
            &["verified outcome", "provenance"],
            &["memory candidate"],
            &["memory:write"],
            &["memory"],
            "provenance and sensitivity checks",
            "advisor when generality is unclear",
            "no workspace write or network",
            "entry is scoped, sourced, and redactable"
        ),
    ]
}

pub struct Router {
    pub models: Vec<ModelProfile>,
    pub policy_revision: u64,
    pub defaults: crate::routing_policy::RouteDefaults,
}
impl Default for Router {
    fn default() -> Self {
        let policy: crate::routing_policy::RoutingPolicy =
            toml::from_str(include_str!("../config/routing-policy-v1.toml"))
                .expect("embedded routing policy must be valid TOML");
        policy
            .validate()
            .expect("embedded routing policy must be reviewed and valid");
        Self::from_policy(policy)
    }
}
impl Router {
    pub fn from_policy(policy: crate::routing_policy::RoutingPolicy) -> Self {
        Self {
            models: policy.models,
            policy_revision: policy.revision,
            defaults: policy.defaults,
        }
    }
    pub fn from_policy_path(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        Ok(Self::from_policy(
            crate::routing_policy::RoutingPolicy::load(path)?,
        ))
    }
    pub fn route(&self, prompt: &str) -> Route {
        self.decide(RoutingRequest {
            prompt: prompt.into(),
            required_quality: 0,
            estimated_tokens: prompt.len() as u32 * 2,
            overrides: UserOverrides::default(),
        })
        .expect("default catalog must provide an eligible route")
        .route
    }
    pub fn validate_route(&self, route: &Route) -> Result<(), NoEligibleRoute> {
        let request = RoutingRequest {
            prompt: String::new(),
            required_quality: 0,
            estimated_tokens: route.dimensions.context_tokens,
            overrides: UserOverrides {
                role: Some(route.role.clone()),
                model: Some(route.model.clone()),
                effort: Some(route.dimensions.effort),
                context_tokens: Some(route.dimensions.context_tokens),
                output_tokens: Some(route.dimensions.output_tokens),
                capabilities: Some(route.dimensions.capabilities.clone()),
                tools: Some(route.dimensions.tools.clone()),
                isolation: Some(route.dimensions.isolation.clone()),
                lifecycle: Some(route.dimensions.lifecycle.clone()),
                verification: Some(route.dimensions.verification.clone()),
                max_cost_micros: None,
                max_latency_ms: Some(route.dimensions.max_latency_ms),
            },
        };
        self.decide(request).map(|_| ())
    }
    pub fn decide(&self, request: RoutingRequest) -> Result<RoutingDecision, NoEligibleRoute> {
        let lower = request.prompt.to_lowercase();
        let substantive = request.prompt.len() > 120
            || ["implement", "refactor", "review", "debug"]
                .iter()
                .any(|w| lower.contains(w));
        let role = request.overrides.role.clone().unwrap_or(if substantive {
            Role::Implementer
        } else {
            Role::Orchestrator
        });
        let required = request
            .required_quality
            .max(if substantive { 75 } else { 50 });
        let mut evidence = Vec::new();
        let mut eligible = Vec::new();
        for m in &self.models {
            let cost = request.estimated_tokens as u64 * m.cost_per_million as u64 / 1_000_000;
            let checks = vec![
                ConstraintEvidence {
                    kind: ConstraintKind::Hard,
                    constraint: "quality".into(),
                    satisfied: m.quality >= required,
                    detail: format!("{} >= {}", m.quality, required),
                },
                ConstraintEvidence {
                    kind: ConstraintKind::Hard,
                    constraint: "context".into(),
                    satisfied: m.context_tokens >= request.estimated_tokens,
                    detail: format!("{} >= {}", m.context_tokens, request.estimated_tokens),
                },
                ConstraintEvidence {
                    kind: ConstraintKind::Hard,
                    constraint: "cost".into(),
                    satisfied: request.overrides.max_cost_micros.is_none_or(|b| cost <= b),
                    detail: format!("estimated {cost}"),
                },
                ConstraintEvidence {
                    kind: ConstraintKind::Hard,
                    constraint: "latency".into(),
                    satisfied: request
                        .overrides
                        .max_latency_ms
                        .is_none_or(|b| m.latency_ms <= b),
                    detail: format!("profile {}ms", m.latency_ms),
                },
                ConstraintEvidence {
                    kind: ConstraintKind::Soft,
                    constraint: "least-cost".into(),
                    satisfied: true,
                    detail: format!("estimated {cost}"),
                },
            ];
            let reliable = checks
                .iter()
                .filter(|c| c.kind == ConstraintKind::Hard)
                .all(|c| c.satisfied);
            let rejected_reason = checks
                .iter()
                .find(|c| c.kind == ConstraintKind::Hard && !c.satisfied)
                .map(|c| format!("{}: {}", c.constraint, c.detail));
            evidence.push(CandidateEvidence {
                model: m.id.clone(),
                reliable,
                estimated_cost_micros: cost,
                constraints: checks,
                rejected_reason,
            });
            if reliable {
                eligible.push((m, cost));
            }
        }
        if let Some(id) = &request.overrides.model {
            let Some(index) = self.models.iter().position(|m| &m.id == id) else {
                return Err(NoEligibleRoute {
                    reason: format!("unknown model override `{id}`"),
                    evidence,
                });
            };
            if !evidence[index].reliable {
                return Err(NoEligibleRoute {
                    reason: format!("model override `{id}` violates hard constraints"),
                    evidence,
                });
            }
            eligible.retain(|(m, _)| &m.id == id);
        }
        eligible.sort_by_key(|(m, c)| (*c, m.latency_ms, m.id.as_str()));
        let Some(selected) = eligible.first().map(|v| v.0) else {
            return Err(NoEligibleRoute {
                reason: "all candidates violate hard constraints".into(),
                evidence,
            });
        };
        let effort = request.overrides.effort.unwrap_or(if required >= 90 {
            Effort::High
        } else if substantive {
            Effort::Medium
        } else {
            Effort::Low
        });
        let requested_context = request.overrides.context_tokens.unwrap_or(
            request
                .estimated_tokens
                .max(if substantive { 16_000 } else { 4_000 }),
        );
        if requested_context > selected.context_tokens {
            return Err(NoEligibleRoute {
                reason: format!(
                    "requested context {requested_context} exceeds model capacity {}",
                    selected.context_tokens
                ),
                evidence,
            });
        }
        let decision_id = format!(
            "v{}:{role}:{}:{required}:{}",
            self.policy_revision, selected.id, request.estimated_tokens
        );
        Ok(RoutingDecision {
            route: Route {
                role,
                model: selected.id.clone(),
                dimensions: ExecutionDimensions {
                    effort,
                    context_tokens: requested_context,
                    output_tokens: request
                        .overrides
                        .output_tokens
                        .unwrap_or(self.defaults.output_tokens),
                    max_latency_ms: request
                        .overrides
                        .max_latency_ms
                        .unwrap_or(selected.latency_ms),
                    capabilities: request
                        .overrides
                        .capabilities
                        .clone()
                        .unwrap_or_else(|| self.defaults.capabilities.clone()),
                    tools: request
                        .overrides
                        .tools
                        .clone()
                        .unwrap_or_else(|| self.defaults.tools.clone()),
                    isolation: request
                        .overrides
                        .isolation
                        .clone()
                        .unwrap_or_else(|| self.defaults.isolation.clone()),
                    lifecycle: request
                        .overrides
                        .lifecycle
                        .clone()
                        .unwrap_or_else(|| self.defaults.lifecycle.clone()),
                    verification: request
                        .overrides
                        .verification
                        .clone()
                        .unwrap_or_else(|| self.defaults.verification.clone()),
                },
                rationale: format!(
                    "deterministic policy; cheapest eligible profile meeting hard quality {required}; no persisted outcome history used"
                ),
                decision_id,
            },
            evidence,
        })
    }
    pub fn delegation_benefit(&self, r: &RoutingRequest) -> DelegationBenefit {
        let c = r.prompt.len() > 120;
        DelegationBenefit {
            delegated: c,
            expected_quality_gain: if c { 15 } else { 0 },
            expected_cost_delta_micros: if c { 2 } else { 0 },
            expected_latency_delta_ms: if c { 200 } else { 0 },
            context_savings_tokens: if c { r.estimated_tokens as i64 / 2 } else { 0 },
            reason: if c {
                "specialist quality and context savings exceed overhead"
            } else {
                "delegation overhead exceeds expected benefit"
            }
            .into(),
        }
    }
    pub fn adapt_quality(required: u8, history: &[bool]) -> u8 {
        if history.last() == Some(&false) {
            required.saturating_add(10).min(100)
        } else if history.len() >= 3 && history.iter().rev().take(3).all(|v| *v) {
            required.saturating_sub(10)
        } else {
            required
        }
    }
}
