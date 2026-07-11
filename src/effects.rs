use crate::store::Store;
use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    #[serde(rename = "workspace.read")]
    FileRead,
    #[serde(rename = "workspace.write")]
    FileWrite,
    #[serde(rename = "process.exec")]
    ProcessExec,
    #[serde(rename = "network")]
    Network,
    #[serde(rename = "secret.read")]
    SecretRead,
    #[serde(rename = "external")]
    External,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IsolationProfile {
    pub filesystem: FilesystemIsolation,
    pub process: ProcessIsolation,
    pub network: NetworkIsolation,
    pub secrets: SecretIsolation,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FilesystemIsolation {
    None,
    WorkspaceReadOnly,
    WorkspaceReadWrite,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProcessIsolation {
    Denied,
    ScrubbedEnvironment,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum NetworkIsolation {
    Denied,
    Allowlisted,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SecretIsolation {
    Denied,
    DestinationScoped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScopedGrant {
    pub id: Uuid,
    pub task_id: Uuid,
    pub capabilities: BTreeSet<Capability>,
    pub workspace: PathBuf,
    pub worktrees: Vec<PathBuf>,
    pub executable_allowlist: BTreeSet<PathBuf>,
    pub network_allowlist: BTreeSet<String>,
    pub external_allowlist: BTreeSet<String>,
    pub secret_destinations: BTreeMap<String, BTreeSet<String>>,
    pub isolation: IsolationProfile,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EffectRequest {
    ReadFile {
        path: PathBuf,
    },
    WriteFile {
        path: PathBuf,
        data: Vec<u8>,
    },
    Exec {
        program: PathBuf,
        args: Vec<String>,
        env: BTreeMap<String, String>,
        cwd: PathBuf,
    },
    Network {
        destination: String,
        payload: Vec<u8>,
    },
    Secret {
        name: String,
        destination: String,
    },
    External {
        service: String,
        action: String,
        payload: Vec<u8>,
    },
}
impl EffectRequest {
    fn capability(&self) -> Capability {
        match self {
            Self::ReadFile { .. } => Capability::FileRead,
            Self::WriteFile { .. } => Capability::FileWrite,
            Self::Exec { .. } => Capability::ProcessExec,
            Self::Network { .. } => Capability::Network,
            Self::Secret { .. } => Capability::SecretRead,
            Self::External { .. } => Capability::External,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Approval {
    pub id: Uuid,
    pub task_id: Uuid,
    pub grant_id: Uuid,
    /// Digest of the complete serialized request. Any destination, payload,
    /// argument, environment, or path mutation invalidates the approval.
    pub request_hash: String,
    pub expires_at: DateTime<Utc>,
}

impl Approval {
    pub fn for_request(
        task_id: Uuid,
        grant_id: Uuid,
        request: &EffectRequest,
        expires_at: DateTime<Utc>,
    ) -> Result<Self> {
        Ok(Self {
            id: Uuid::new_v4(),
            task_id,
            grant_id,
            request_hash: request_hash(request)?,
            expires_at,
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationAuthorization {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub task_id: Uuid,
    pub grant_id: Uuid,
    pub approval_id: Option<Uuid>,
    pub request_hash: String,
    pub issued_at: DateTime<Utc>,
}

pub fn request_hash(request: &EffectRequest) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(request)?)
    ))
}

pub struct Policy;
impl Policy {
    pub fn evaluate(grant: &ScopedGrant, request: &EffectRequest) -> Result<()> {
        if grant.task_id.is_nil() || grant.expires_at.is_some_and(|x| x <= Utc::now()) {
            bail!("grant is invalid or expired")
        }
        if !grant.capabilities.contains(&request.capability()) {
            bail!("capability denied")
        }
        match request {
            EffectRequest::ReadFile { path } => {
                if grant.isolation.filesystem == FilesystemIsolation::None {
                    bail!("filesystem denied")
                }
                validate_path(grant, path, false)?;
            }
            EffectRequest::WriteFile { path, .. } => {
                if grant.isolation.filesystem != FilesystemIsolation::WorkspaceReadWrite {
                    bail!("filesystem write denied")
                }
                validate_path(grant, path, true)?;
            }
            EffectRequest::Exec { program, cwd, .. } => {
                if grant.isolation.process != ProcessIsolation::ScrubbedEnvironment
                    || !grant.executable_allowlist.contains(program)
                {
                    bail!("process denied")
                }
                validate_path(grant, cwd, false)?;
            }
            EffectRequest::Network { destination, .. } => {
                if grant.isolation.network != NetworkIsolation::Allowlisted
                    || !grant.network_allowlist.contains(destination)
                {
                    bail!("network destination denied")
                }
            }
            EffectRequest::Secret { name, destination } => {
                if grant.isolation.secrets != SecretIsolation::DestinationScoped
                    || !grant
                        .secret_destinations
                        .get(name)
                        .is_some_and(|d| d.contains(destination))
                {
                    bail!("secret destination denied")
                }
            }
            EffectRequest::External { service, .. } => {
                if !grant.external_allowlist.contains(service) {
                    bail!("external service denied")
                }
            }
        }
        Ok(())
    }
}

fn validate_path(grant: &ScopedGrant, path: &Path, allow_missing_leaf: bool) -> Result<()> {
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        bail!("path traversal denied")
    }
    let candidate = if allow_missing_leaf && !path.exists() {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow!("missing parent"))?
            .canonicalize()?;
        parent.join(
            path.file_name()
                .ok_or_else(|| anyhow!("missing filename"))?,
        )
    } else {
        path.canonicalize()?
    };
    let mut roots = vec![grant.workspace.canonicalize()?];
    for w in &grant.worktrees {
        roots.push(w.canonicalize()?);
    }
    if !roots.iter().any(|r| candidate.starts_with(r)) {
        bail!("path outside granted workspace")
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessOutput {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub cancelled: bool,
}

#[async_trait]
pub trait EffectAdapter: Send + Sync {
    async fn read_file(&self, path: &Path) -> Result<Vec<u8>>;
    async fn write_file(&self, path: &Path, data: &[u8]) -> Result<()>;
    async fn exec(
        &self,
        program: &Path,
        args: &[String],
        env: &BTreeMap<String, String>,
        cwd: &Path,
    ) -> Result<Vec<u8>>;
    async fn exec_process(
        &self,
        program: &Path,
        args: &[String],
        env: &BTreeMap<String, String>,
        cwd: &Path,
    ) -> Result<ProcessOutput> {
        self.exec(program, args, env, cwd)
            .await
            .map(|stdout| ProcessOutput {
                exit_code: Some(0),
                stdout,
                stderr: vec![],
                cancelled: false,
            })
    }
    async fn network(&self, destination: &str, payload: &[u8]) -> Result<Vec<u8>>;
    async fn secret(&self, name: &str) -> Result<Vec<u8>>;
    async fn external(&self, service: &str, action: &str, payload: &[u8]) -> Result<Vec<u8>>;
}
pub struct SystemAdapter;
#[async_trait]
impl EffectAdapter for SystemAdapter {
    async fn read_file(&self, p: &Path) -> Result<Vec<u8>> {
        Ok(tokio::fs::read(p).await?)
    }
    async fn write_file(&self, p: &Path, d: &[u8]) -> Result<()> {
        Ok(tokio::fs::write(p, d).await?)
    }
    async fn exec(
        &self,
        p: &Path,
        a: &[String],
        e: &BTreeMap<String, String>,
        cwd: &Path,
    ) -> Result<Vec<u8>> {
        let o = tokio::process::Command::new(p)
            .args(a)
            .env_clear()
            .envs(e)
            .current_dir(cwd)
            .output()
            .await?;
        if !o.status.success() {
            bail!("process failed: {}", o.status)
        }
        Ok(o.stdout)
    }
    async fn exec_process(
        &self,
        p: &Path,
        a: &[String],
        e: &BTreeMap<String, String>,
        cwd: &Path,
    ) -> Result<ProcessOutput> {
        let o = tokio::process::Command::new(p)
            .args(a)
            .env_clear()
            .envs(e)
            .current_dir(cwd)
            .output()
            .await?;
        Ok(ProcessOutput {
            exit_code: o.status.code(),
            stdout: o.stdout,
            stderr: o.stderr,
            cancelled: false,
        })
    }
    async fn network(&self, _: &str, _: &[u8]) -> Result<Vec<u8>> {
        bail!("no network transport configured")
    }
    async fn secret(&self, n: &str) -> Result<Vec<u8>> {
        Ok(std::env::var(n)?.into_bytes())
    }
    async fn external(&self, _: &str, _: &str, _: &[u8]) -> Result<Vec<u8>> {
        bail!("no external transport configured")
    }
}

fn authorize_approval(
    grant: &ScopedGrant,
    request: &EffectRequest,
    approval: Option<&Approval>,
) -> Result<String> {
    let hash = request_hash(request)?;
    let requires_approval = !matches!(request, EffectRequest::ReadFile { .. });
    if requires_approval && approval.is_none() {
        bail!("explicit approval required for mutating or external effect")
    }
    if let Some(a) = approval
        && (a.task_id != grant.task_id
            || a.grant_id != grant.id
            || a.request_hash != hash
            || a.expires_at <= Utc::now())
    {
        bail!("approval is not bound to this request")
    }
    Ok(hash)
}

pub struct EffectBroker<'a, A: EffectAdapter> {
    pub store: &'a Store,
    pub adapter: A,
}
impl<'a, A: EffectAdapter> EffectBroker<'a, A> {
    /// Capability-only authorization for transports that perform their own I/O.
    /// The destination is evaluated by the same scoped policy as brokered
    /// effects; no socket may be opened before this succeeds.
    pub fn authorize_network(&self, grant: &ScopedGrant, destination: &str) -> Result<()> {
        Policy::evaluate(
            grant,
            &EffectRequest::Network {
                destination: destination.to_owned(),
                payload: Vec::new(),
            },
        )
    }
    /// Creates and owns the durable operation lifecycle for an effect. The
    /// intent is committed before authorization or adapter dispatch.
    pub async fn execute_owned(
        &self,
        grant: &ScopedGrant,
        approval: Option<&Approval>,
        request: EffectRequest,
    ) -> Result<(Uuid, Vec<u8>)> {
        use crate::domain::{Operation, OperationState};
        let mut operation = Operation {
            id: Uuid::new_v4(),
            task_id: grant.task_id,
            attempt: self.store.operations_for(grant.task_id)?.len() as u32 + 1,
            state: OperationState::IntentRecorded,
            retry_safe: false,
            started_at: Utc::now(),
            completed_at: None,
        };
        self.store.create_operation(&operation)?;
        operation.state = OperationState::Running;
        self.store.save_operation(&operation)?;
        let result = self.execute(operation.id, grant, approval, request).await;
        operation.completed_at = Some(Utc::now());
        operation.state = if result.is_ok() {
            OperationState::Succeeded
        } else {
            OperationState::Failed
        };
        self.store.save_operation(&operation)?;
        result.map(|output| (operation.id, output))
    }

    /// Authorizes and records a process launch performed by a transport that
    /// must retain the child for interactive I/O. The exact exec request is
    /// bound to the approval before `launch` can run.
    pub fn launch_process_owned<T>(
        &self,
        grant: &ScopedGrant,
        approval: Option<&Approval>,
        request: EffectRequest,
        launch: impl FnOnce(&Path, &[String], &BTreeMap<String, String>, &Path) -> Result<T>,
    ) -> Result<(Uuid, T)> {
        use crate::domain::{Operation, OperationState};
        let mut operation = Operation {
            id: Uuid::new_v4(),
            task_id: grant.task_id,
            attempt: self.store.operations_for(grant.task_id)?.len() as u32 + 1,
            state: OperationState::IntentRecorded,
            retry_safe: false,
            started_at: Utc::now(),
            completed_at: None,
        };
        self.store.create_operation(&operation)?;
        operation.state = OperationState::Running;
        self.store.save_operation(&operation)?;

        let result = (|| {
            Policy::evaluate(grant, &request)?;
            let hash = authorize_approval(grant, &request, approval)?;
            let auth = OperationAuthorization {
                id: Uuid::new_v4(),
                operation_id: operation.id,
                task_id: grant.task_id,
                grant_id: grant.id,
                approval_id: approval.map(|approval| approval.id),
                request_hash: hash,
                issued_at: Utc::now(),
            };
            self.store.authorize_effect(&auth)?;
            match &request {
                EffectRequest::Exec {
                    program,
                    args,
                    env,
                    cwd,
                } => launch(program, args, env, cwd),
                _ => bail!("process launch requires an exec request"),
            }
        })();

        operation.completed_at = Some(Utc::now());
        operation.state = if result.is_ok() {
            OperationState::Succeeded
        } else {
            OperationState::Failed
        };
        self.store.save_operation(&operation)?;
        result.map(|value| (operation.id, value))
    }

    pub async fn execute_process_owned(
        &self,
        grant: &ScopedGrant,
        approval: Option<&Approval>,
        request: EffectRequest,
    ) -> Result<(Uuid, ProcessOutput)> {
        use crate::domain::{Operation, OperationState};
        let mut operation = Operation {
            id: Uuid::new_v4(),
            task_id: grant.task_id,
            attempt: self.store.operations_for(grant.task_id)?.len() as u32 + 1,
            state: OperationState::IntentRecorded,
            retry_safe: false,
            started_at: Utc::now(),
            completed_at: None,
        };
        self.store.create_operation(&operation)?;
        operation.state = OperationState::Running;
        self.store.save_operation(&operation)?;
        let result = self
            .execute_process(operation.id, grant, approval, request)
            .await;
        operation.completed_at = Some(Utc::now());
        operation.state = match &result {
            Ok(output) if output.cancelled => OperationState::Cancelled,
            Ok(_) => OperationState::Succeeded,
            Err(_) => OperationState::Failed,
        };
        self.store.save_operation(&operation)?;
        result.map(|output| (operation.id, output))
    }

    pub async fn execute_process(
        &self,
        operation_id: Uuid,
        grant: &ScopedGrant,
        approval: Option<&Approval>,
        request: EffectRequest,
    ) -> Result<ProcessOutput> {
        Policy::evaluate(grant, &request)?;
        let hash = authorize_approval(grant, &request, approval)?;
        let auth = OperationAuthorization {
            id: Uuid::new_v4(),
            operation_id,
            task_id: grant.task_id,
            grant_id: grant.id,
            approval_id: approval.map(|a| a.id),
            request_hash: hash,
            issued_at: Utc::now(),
        };
        self.store.authorize_effect(&auth)?;
        match request {
            EffectRequest::Exec {
                program,
                args,
                env,
                cwd,
            } => self.adapter.exec_process(&program, &args, &env, &cwd).await,
            _ => bail!("process execution requires an exec request"),
        }
    }

    pub async fn execute(
        &self,
        operation_id: Uuid,
        grant: &ScopedGrant,
        approval: Option<&Approval>,
        request: EffectRequest,
    ) -> Result<Vec<u8>> {
        Policy::evaluate(grant, &request)?;
        let hash = authorize_approval(grant, &request, approval)?;
        let auth = OperationAuthorization {
            id: Uuid::new_v4(),
            operation_id,
            task_id: grant.task_id,
            grant_id: grant.id,
            approval_id: approval.map(|a| a.id),
            request_hash: hash,
            issued_at: Utc::now(),
        };
        self.store.authorize_effect(&auth)?;
        match request {
            EffectRequest::ReadFile { path } => self.adapter.read_file(&path).await,
            EffectRequest::WriteFile { path, data } => {
                self.adapter.write_file(&path, &data).await?;
                Ok(vec![])
            }
            EffectRequest::Exec {
                program,
                args,
                env,
                cwd,
            } => self.adapter.exec(&program, &args, &env, &cwd).await,
            EffectRequest::Network {
                destination,
                payload,
            } => self.adapter.network(&destination, &payload).await,
            EffectRequest::Secret { name, .. } => self.adapter.secret(&name).await,
            EffectRequest::External {
                service,
                action,
                payload,
            } => self.adapter.external(&service, &action, &payload).await,
        }
    }
}
