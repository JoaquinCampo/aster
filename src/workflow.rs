use crate::verification::{Artifact, CheckEvidence, VerificationStatus};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
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
    pub fn gate(&self, checks: &[CheckEvidence], reviews: &[ReviewEvidence]) -> Result<()> {
        self.validate()?;
        if checks.len() < self.deterministic_checks || reviews.len() < self.independent_checkers {
            bail!("required verification gate evidence is missing")
        }
        if self.risk == Risk::High
            && (checks
                .iter()
                .any(|x| x.status != VerificationStatus::Passed)
                || reviews
                    .iter()
                    .any(|x| x.status != VerificationStatus::Passed))
        {
            bail!("high-risk verification gate did not pass")
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
            checks.push(id)
        }
        for _ in 0..policy.independent_checkers {
            let id = Uuid::new_v4();
            nodes.push(DagNode {
                id,
                role: DagRole::IndependentChecker,
                dependencies: vec![maker],
                attempt: 1,
            });
            checks.push(id)
        }
        nodes.push(DagNode {
            id: Uuid::new_v4(),
            role: DagRole::Finalizer,
            dependencies: checks,
            attempt: 1,
        });
        Ok(Self {
            maker,
            nodes,
            policy,
        })
    }
    pub fn append_fixer_round(&mut self, failed_checks: &[Uuid]) -> Result<Uuid> {
        let attempt = self
            .nodes
            .iter()
            .filter(|n| n.role == DagRole::Fixer)
            .count() as u32
            + 1;
        if failed_checks.is_empty() {
            bail!("fixer requires a failing checker verdict")
        }
        if attempt > self.policy.max_fixer_rounds {
            bail!("fixer loop bound exceeded")
        }
        let id = Uuid::new_v4();
        let finalizer = self
            .nodes
            .pop()
            .ok_or_else(|| anyhow::anyhow!("missing finalizer"))?;
        self.nodes.push(DagNode {
            id,
            role: DagRole::Fixer,
            dependencies: failed_checks.to_vec(),
            attempt,
        });
        self.nodes.push(DagNode {
            dependencies: vec![id],
            ..finalizer
        });
        Ok(id)
    }
    pub fn validate_checker_attempts(&self, ids: &[Uuid]) -> Result<()> {
        if ids.len() < self.policy.independent_checkers {
            bail!("insufficient independent checker attempts")
        }
        let unique: BTreeSet<_> = ids.iter().copied().collect();
        if unique.len() != ids.len() || unique.contains(&self.maker) {
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
    policy.gate(&checks, &reviews)?;
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Handoff {
    pub objective: String,
    pub summary: String,
    pub constraints: Vec<String>,
    pub decisions: Vec<String>,
    pub open_issues: Vec<String>,
    pub artifacts: Vec<Artifact>,
}
impl Handoff {
    pub fn validate(&self, max_bytes: usize) -> Result<()> {
        let bytes = serde_json::to_vec(self)?;
        if self.objective.trim().is_empty() || self.summary.trim().is_empty() {
            bail!("handoff omits required context")
        }
        if bytes.len() > max_bytes {
            bail!("handoff exceeds compact size bound")
        }
        let mut paths = BTreeSet::new();
        if self.artifacts.iter().any(|a| !paths.insert(&a.path)) {
            bail!("handoff contains duplicate artifacts")
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EscalationEvidence {
    pub reason: String,
    pub repeated_fingerprint: String,
    pub observations: u32,
    pub handoff: Handoff,
}
#[derive(Debug, Clone)]
pub struct ProgressDetector {
    window: usize,
    history: BTreeMap<Uuid, VecDeque<String>>,
}
impl ProgressDetector {
    pub fn new(window: usize) -> Result<Self> {
        if window < 2 {
            bail!("stall window must be at least two")
        }
        Ok(Self {
            window,
            history: BTreeMap::new(),
        })
    }
    pub fn observe(
        &mut self,
        task: Uuid,
        fingerprint: impl Into<String>,
        handoff: Handoff,
    ) -> Result<Option<EscalationEvidence>> {
        handoff.validate(16 * 1024)?;
        let f = fingerprint.into();
        let h = self.history.entry(task).or_default();
        h.push_back(f.clone());
        while h.len() > self.window {
            h.pop_front();
        }
        if h.len() == self.window && h.iter().all(|x| x == &f) {
            Ok(Some(EscalationEvidence {
                reason: "agent stalled or looping without observable progress".into(),
                repeated_fingerprint: f,
                observations: self.window as u32,
                handoff,
            }))
        } else {
            Ok(None)
        }
    }
}
