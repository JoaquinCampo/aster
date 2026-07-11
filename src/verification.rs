use crate::effects::{Approval, EffectAdapter, EffectBroker, EffectRequest, ScopedGrant};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf, time::Duration};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum VerificationStatus {
    Passed,
    Failed,
    Inconclusive,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Artifact {
    pub path: PathBuf,
    pub media_type: String,
    pub digest: String,
    pub size: u64,
}

impl Artifact {
    pub async fn from_path(path: PathBuf, media_type: impl Into<String>) -> Result<Self> {
        let bytes = tokio::fs::read(&path).await?;
        Ok(Self {
            path,
            media_type: media_type.into(),
            digest: digest(&bytes),
            size: bytes.len() as u64,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeterministicCheck {
    pub name: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
    pub timeout_ms: u64,
    pub artifacts: Vec<(PathBuf, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckEvidence {
    pub check: String,
    pub status: VerificationStatus,
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_digest: String,
    pub stderr_digest: String,
    pub artifacts: Vec<Artifact>,
    pub detail: Option<String>,
}

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub async fn run_check<A: EffectAdapter>(
    broker: &EffectBroker<'_, A>,
    operation_id: Uuid,
    grant: &ScopedGrant,
    approval: Option<&Approval>,
    check: &DeterministicCheck,
) -> CheckEvidence {
    let request = EffectRequest::Exec {
        program: check.program.clone(),
        args: check.args.clone(),
        env: check.env.clone(),
        cwd: check.cwd.clone(),
    };
    let execution = tokio::time::timeout(
        Duration::from_millis(check.timeout_ms),
        broker.execute_process(operation_id, grant, approval, request),
    )
    .await;
    let (status, code, stdout, stderr, detail) = match execution {
        Err(_) => (
            VerificationStatus::TimedOut,
            None,
            vec![],
            vec![],
            Some("check deadline exceeded".into()),
        ),
        Ok(Err(e)) => (
            VerificationStatus::Inconclusive,
            None,
            vec![],
            vec![],
            Some(e.to_string()),
        ),
        Ok(Ok(o)) if o.cancelled => (
            VerificationStatus::Cancelled,
            o.exit_code,
            o.stdout,
            o.stderr,
            None,
        ),
        Ok(Ok(o)) if o.exit_code == Some(0) => (
            VerificationStatus::Passed,
            o.exit_code,
            o.stdout,
            o.stderr,
            None,
        ),
        Ok(Ok(o)) => (
            VerificationStatus::Failed,
            o.exit_code,
            o.stdout,
            o.stderr,
            None,
        ),
    };
    let mut artifacts = Vec::new();
    for (path, media) in &check.artifacts {
        match Artifact::from_path(path.clone(), media.clone()).await {
            Ok(a) => artifacts.push(a),
            Err(e) if status == VerificationStatus::Passed => {
                return evidence(
                    check,
                    VerificationStatus::Inconclusive,
                    code,
                    stdout,
                    stderr,
                    artifacts,
                    Some(format!("artifact unavailable: {e}")),
                );
            }
            Err(_) => {}
        }
    }
    evidence(check, status, code, stdout, stderr, artifacts, detail)
}

fn evidence(
    check: &DeterministicCheck,
    status: VerificationStatus,
    exit_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    artifacts: Vec<Artifact>,
    detail: Option<String>,
) -> CheckEvidence {
    CheckEvidence {
        check: check.name.clone(),
        status,
        exit_code,
        stdout_digest: digest(&stdout),
        stderr_digest: digest(&stderr),
        stdout,
        stderr,
        artifacts,
        detail,
    }
}

pub fn require_passed(items: &[CheckEvidence]) -> Result<()> {
    if items.iter().all(|e| e.status == VerificationStatus::Passed) {
        Ok(())
    } else {
        bail!("verification contains non-passing evidence")
    }
}
