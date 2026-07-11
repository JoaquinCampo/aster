use crate::domain::{Effort, ExecutionDimensions, Role, Route};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateEvidence {
    pub model: String,
    pub reliable: bool,
    pub estimated_cost_micros: u64,
    pub rejected_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    pub route: Route,
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

pub fn built_in_roles() -> Vec<(Role, &'static str)> {
    vec![
        (Role::Orchestrator, "decompose, route, and synthesize"),
        (Role::Implementer, "change code and verify it"),
        (Role::Reviewer, "find correctness and security defects"),
        (Role::Researcher, "gather and cite evidence"),
        (Role::Tester, "design and execute deterministic tests"),
    ]
}

pub struct Router {
    pub models: Vec<ModelProfile>,
}
impl Default for Router {
    fn default() -> Self {
        Self {
            models: vec![
                ModelProfile {
                    id: "fake-luna".into(),
                    quality: 60,
                    cost_per_million: 100,
                    latency_ms: 100,
                    context_tokens: 8_000,
                },
                ModelProfile {
                    id: "fake-terra".into(),
                    quality: 85,
                    cost_per_million: 500,
                    latency_ms: 300,
                    context_tokens: 32_000,
                },
                ModelProfile {
                    id: "fixed-strong".into(),
                    quality: 95,
                    cost_per_million: 2_000,
                    latency_ms: 700,
                    context_tokens: 128_000,
                },
            ],
        }
    }
}
impl Router {
    pub fn route(&self, prompt: &str) -> Route {
        self.decide(RoutingRequest {
            prompt: prompt.into(),
            required_quality: 0,
            estimated_tokens: prompt.len() as u32 * 2,
            overrides: UserOverrides::default(),
        })
        .route
    }

    /// Hybrid routing: deterministic policy establishes requirements, then measured model
    /// profiles select the cheapest candidate that satisfies every reliability constraint.
    pub fn decide(&self, request: RoutingRequest) -> RoutingDecision {
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
        let mut eligible: Vec<(&ModelProfile, u64)> = self
            .models
            .iter()
            .filter_map(|m| {
                let cost = request.estimated_tokens as u64 * m.cost_per_million as u64 / 1_000_000;
                let reason = if m.quality < required {
                    Some("quality threshold")
                } else if m.context_tokens < request.estimated_tokens {
                    Some("context budget")
                } else if request.overrides.max_cost_micros.is_some_and(|b| cost > b) {
                    Some("cost budget")
                } else if request
                    .overrides
                    .max_latency_ms
                    .is_some_and(|b| m.latency_ms > b)
                {
                    Some("latency budget")
                } else {
                    None
                };
                evidence.push(CandidateEvidence {
                    model: m.id.clone(),
                    reliable: reason.is_none(),
                    estimated_cost_micros: cost,
                    rejected_reason: reason.map(str::to_owned),
                });
                reason.is_none().then_some((m, cost))
            })
            .collect();
        eligible.sort_by_key(|(m, cost)| (*cost, m.latency_ms, m.id.as_str()));
        let selected = request
            .overrides
            .model
            .as_ref()
            .and_then(|id| self.models.iter().find(|m| &m.id == id))
            .or_else(|| eligible.first().map(|v| v.0))
            .unwrap_or_else(|| {
                self.models
                    .iter()
                    .max_by_key(|m| m.quality)
                    .expect("built-in model catalog is non-empty")
            });
        let effort = request.overrides.effort.unwrap_or(if required >= 90 {
            Effort::High
        } else if substantive {
            Effort::Medium
        } else {
            Effort::Low
        });
        let context = request
            .overrides
            .context_tokens
            .unwrap_or(
                request
                    .estimated_tokens
                    .max(if substantive { 16_000 } else { 4_000 }),
            )
            .min(selected.context_tokens);
        let decision_id = format!(
            "v1:{}:{}:{}:{}",
            role, selected.id, required, request.estimated_tokens
        );
        RoutingDecision {
            route: Route {
                role,
                model: selected.id.clone(),
                dimensions: ExecutionDimensions {
                    effort,
                    context_tokens: context,
                    output_tokens: 4_000,
                    max_latency_ms: request
                        .overrides
                        .max_latency_ms
                        .unwrap_or(selected.latency_ms),
                    capabilities: vec!["workspace:read".into()],
                    isolation: vec![
                        "process:none".into(),
                        "network:denied".into(),
                        "credentials:none".into(),
                    ],
                    verification: "deterministic-output-check".into(),
                },
                rationale: format!(
                    "cheapest reliable model meeting quality {required}; overrides are explicit"
                ),
                decision_id,
            },
            evidence,
        }
    }

    pub fn delegation_benefit(&self, request: &RoutingRequest) -> DelegationBenefit {
        let complex = request.prompt.len() > 120;
        DelegationBenefit {
            delegated: complex,
            expected_quality_gain: if complex { 15 } else { 0 },
            expected_cost_delta_micros: if complex { 2 } else { 0 },
            expected_latency_delta_ms: if complex { 200 } else { 0 },
            context_savings_tokens: if complex {
                request.estimated_tokens as i64 / 2
            } else {
                0
            },
            reason: if complex {
                "specialist quality and context savings exceed overhead"
            } else {
                "delegation overhead exceeds expected benefit"
            }
            .into(),
        }
    }

    /// Escalate only after failed evidence; de-escalate after repeated verified success.
    pub fn adapt_quality(required: u8, verification_history: &[bool]) -> u8 {
        if verification_history.last() == Some(&false) {
            required.saturating_add(10).min(100)
        } else if verification_history.len() >= 3
            && verification_history.iter().rev().take(3).all(|v| *v)
        {
            required.saturating_sub(10)
        } else {
            required
        }
    }
}
