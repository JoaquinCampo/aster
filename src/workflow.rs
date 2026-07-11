use crate::verification::{Artifact, CheckEvidence, VerificationStatus};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Risk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationPolicy {
    pub risk: Risk,
    pub deterministic_checks: usize,
    pub independent_checkers: usize,
    pub max_fixer_rounds: u32,
}
impl VerificationPolicy {
    pub fn proportional(risk: Risk) -> Self {
        match risk {
            Risk::Low => Self {
                risk,
                deterministic_checks: 1,
                independent_checkers: 0,
                max_fixer_rounds: 0,
            },
            Risk::Medium => Self {
                risk,
                deterministic_checks: 1,
                independent_checkers: 1,
                max_fixer_rounds: 2,
            },
            Risk::High => Self {
                risk,
                deterministic_checks: 2,
                independent_checkers: 2,
                max_fixer_rounds: 3,
            },
        }
    }
    pub fn validate(&self) -> Result<()> {
        let floor = Self::proportional(self.risk);
        if self.deterministic_checks < floor.deterministic_checks
            || self.independent_checkers < floor.independent_checkers
            || self.max_fixer_rounds > 10
        {
            bail!("verification policy is not proportional or bounded")
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DagRole {
    Maker,
    DeterministicChecker,
    IndependentChecker,
    Fixer,
    Finalizer,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DagNode {
    pub id: Uuid,
    pub role: DagRole,
    pub dependencies: Vec<Uuid>,
    pub attempt: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MakerCheckerFixerDag {
    pub maker: Uuid,
    pub nodes: Vec<DagNode>,
    pub policy: VerificationPolicy,
}

impl MakerCheckerFixerDag {
    pub fn template(policy: VerificationPolicy) -> Result<Self> {
        policy.validate()?;
        let maker = Uuid::new_v4();
        let mut nodes = vec![DagNode {
            id: maker,
            role: DagRole::Maker,
            dependencies: vec![],
            attempt: 1,
        }];
        let mut checks = vec![];
        for _ in 0..policy.deterministic_checks {
            let id = Uuid::new_v4();
            nodes.push(DagNode {
                id,
                role: DagRole::DeterministicChecker,
                dependencies: vec![maker],
                attempt: 1,
            });
            checks.push(id);
        }
        for _ in 0..policy.independent_checkers {
            let id = Uuid::new_v4();
            nodes.push(DagNode {
                id,
                role: DagRole::IndependentChecker,
                dependencies: vec![maker],
                attempt: 1,
            });
            checks.push(id);
        }
        let mut deps = checks;
        for attempt in 1..=policy.max_fixer_rounds {
            let id = Uuid::new_v4();
            nodes.push(DagNode {
                id,
                role: DagRole::Fixer,
                dependencies: deps.clone(),
                attempt,
            });
            deps = vec![id];
        }
        nodes.push(DagNode {
            id: Uuid::new_v4(),
            role: DagRole::Finalizer,
            dependencies: deps,
            attempt: 1,
        });
        Ok(Self {
            maker,
            nodes,
            policy,
        })
    }
    pub fn validate_checker_attempts(&self, checker_actor_ids: &[Uuid]) -> Result<()> {
        if checker_actor_ids.len() < self.policy.independent_checkers {
            bail!("insufficient independent checker attempts")
        }
        let unique: BTreeSet<_> = checker_actor_ids.iter().copied().collect();
        if unique.len() != checker_actor_ids.len() || unique.contains(&self.maker) {
            bail!("checker attempts must be independent")
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewEvidence {
    pub checker_id: Uuid,
    pub attempt: u32,
    pub status: VerificationStatus,
    pub rationale: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinalEvidence {
    pub status: VerificationStatus,
    pub checks: Vec<CheckEvidence>,
    pub reviews: Vec<ReviewEvidence>,
    pub artifacts: Vec<Artifact>,
    pub fixer_rounds: u32,
}

pub fn assemble_final(
    policy: &VerificationPolicy,
    checks: Vec<CheckEvidence>,
    reviews: Vec<ReviewEvidence>,
    fixer_rounds: u32,
) -> Result<FinalEvidence> {
    policy.validate()?;
    if checks.len() < policy.deterministic_checks || reviews.len() < policy.independent_checkers {
        bail!("required evidence is missing")
    }
    if fixer_rounds > policy.max_fixer_rounds {
        bail!("fixer loop bound exceeded")
    }
    let ids: BTreeSet<_> = reviews.iter().map(|r| r.checker_id).collect();
    if ids.len() != reviews.len() {
        bail!("checker attempts are not independent")
    }
    let status = checks
        .iter()
        .map(|x| x.status)
        .chain(reviews.iter().map(|x| x.status))
        .find(|s| *s != VerificationStatus::Passed)
        .unwrap_or(VerificationStatus::Passed);
    let artifacts = checks.iter().flat_map(|x| x.artifacts.clone()).collect();
    Ok(FinalEvidence {
        status,
        checks,
        reviews,
        artifacts,
        fixer_rounds,
    })
}
