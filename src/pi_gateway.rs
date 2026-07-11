use crate::{
    domain::{Effort, Route},
    provider::{
        EventStream, ExecutionResult, PiAdapter, ProviderError, ProviderEvent, ReasoningEffort,
        Usage,
    },
};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

pub const PI_PROTOCOL_VERSION: u8 = 1;

#[derive(Debug, Clone)]
pub struct PiGateway {
    pub node: PathBuf,
    pub sidecar: PathBuf,
    pub node_modules: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PiRunInput {
    pub prompt: String,
    pub model: String,
    pub effort: ReasoningEffort,
    pub context: Option<String>,
    pub fixture_tool: Option<FixtureTool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FixtureTool {
    pub name: String,
    pub capability: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiDiscovery {
    pub protocol: u8,
    pub node: String,
    pub agent_core: String,
    pub ai: String,
    pub capabilities: BTreeSet<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireEvent {
    v: u8,
    #[allow(dead_code)]
    id: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    protocol: Option<u8>,
    node: Option<String>,
    versions: Option<Versions>,
    capabilities: Option<Vec<String>>,
    text: Option<String>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    call_id: Option<String>,
    name: Option<String>,
    arguments: Option<Value>,
    capability: Option<String>,
    error: Option<WireError>,
}
#[derive(Debug, Deserialize)]
struct Versions {
    #[serde(rename = "agentCore")]
    agent_core: String,
    ai: String,
}
#[derive(Debug, Deserialize)]
struct WireError {
    code: String,
    message: String,
}

fn resolve_executable(executable: &Path) -> Result<PathBuf, ProviderError> {
    if executable.components().count() > 1 {
        return executable.canonicalize().map_err(|error| {
            ProviderError::Transport(format!("invalid Pi node executable: {error}"))
        });
    }
    let path = std::env::var_os("PATH")
        .ok_or_else(|| ProviderError::Transport("PATH is unavailable".into()))?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(executable))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| ProviderError::Transport("Pi node executable not found on PATH".into()))?
        .canonicalize()
        .map_err(|error| ProviderError::Transport(error.to_string()))
}

impl PiGateway {
    pub fn new(sidecar: impl Into<PathBuf>) -> Self {
        Self {
            node: "node".into(),
            sidecar: sidecar.into(),
            node_modules: None,
        }
    }
    pub fn with_node_modules(mut self, path: impl Into<PathBuf>) -> Self {
        self.node_modules = Some(path.into());
        self
    }

    pub fn launch_request(&self) -> Result<crate::effects::EffectRequest, ProviderError> {
        let node = resolve_executable(&self.node)?;
        let sidecar = self.sidecar.canonicalize().map_err(|error| {
            ProviderError::Transport(format!("invalid Pi sidecar path: {error}"))
        })?;
        let cwd = sidecar
            .parent()
            .ok_or_else(|| ProviderError::Transport("Pi sidecar has no parent directory".into()))?
            .to_owned();
        let mut env = BTreeMap::new();
        env.insert("PATH".into(), std::env::var("PATH").unwrap_or_default());
        if let Some(path) = &self.node_modules {
            env.insert(
                "ASTER_PI_NODE_MODULES".into(),
                path.to_string_lossy().into_owned(),
            );
        }
        Ok(crate::effects::EffectRequest::Exec {
            program: node,
            args: vec![sidecar.to_string_lossy().into_owned()],
            env,
            cwd,
        })
    }

    async fn request<A: crate::effects::EffectAdapter>(
        &self,
        request: Value,
        broker: &crate::effects::EffectBroker<'_, A>,
        grant: &crate::effects::ScopedGrant,
        approval: &crate::effects::Approval,
    ) -> Result<Vec<WireEvent>, ProviderError> {
        let (_, mut child) = broker
            .spawn_authorized_interactive(grant, Some(approval), self.launch_request()?)
            .map_err(|error| ProviderError::Transport(format!("Pi launch denied: {error}")))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| ProviderError::Transport("sidecar stdin unavailable".into()))?;
        writeln!(stdin, "{request}")
            .map_err(|error| ProviderError::Transport(error.to_string()))?;
        drop(stdin);
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ProviderError::Transport("sidecar stdout unavailable".into()))?;
        let mut events = Vec::new();
        for line in BufReader::new(stdout).lines() {
            let line = line.map_err(|error| ProviderError::Transport(error.to_string()))?;
            let event: WireEvent = serde_json::from_str(&line).map_err(|error| {
                ProviderError::Protocol(format!("invalid sidecar event: {error}"))
            })?;
            if event.v != PI_PROTOCOL_VERSION {
                let _ = child.kill();
                return Err(ProviderError::Protocol(
                    "unsupported sidecar protocol".into(),
                ));
            }
            if let Some(error) = &event.error {
                let _ = child.kill();
                return Err(ProviderError::Response {
                    code: error.code.clone(),
                    message: error.message.clone(),
                });
            }
            events.push(event);
        }
        let status = child
            .wait()
            .map_err(|error| ProviderError::Transport(error.to_string()))?;
        if !status.success() {
            return Err(ProviderError::Transport(format!(
                "Pi sidecar exited with {status}"
            )));
        }
        Ok(events)
    }

    pub async fn discover<A: crate::effects::EffectAdapter>(
        &self,
        broker: &crate::effects::EffectBroker<'_, A>,
        grant: &crate::effects::ScopedGrant,
        approval: &crate::effects::Approval,
    ) -> Result<PiDiscovery, ProviderError> {
        let events = self
            .request(
                serde_json::json!({"v":1,"id":"discover","type":"discover"}),
                broker,
                grant,
                approval,
            )
            .await?;
        let event = events
            .into_iter()
            .find(|e| e.kind == "ready")
            .ok_or_else(|| ProviderError::Protocol("sidecar omitted ready event".into()))?;
        let versions = event
            .versions
            .ok_or_else(|| ProviderError::Protocol("ready omitted versions".into()))?;
        Ok(PiDiscovery {
            protocol: event.protocol.unwrap_or_default(),
            node: event.node.unwrap_or_default(),
            agent_core: versions.agent_core,
            ai: versions.ai,
            capabilities: event.capabilities.unwrap_or_default().into_iter().collect(),
        })
    }

    pub async fn run_deterministic<A: crate::effects::EffectAdapter>(
        &self,
        input: PiRunInput,
        allowed_capabilities: &BTreeSet<String>,
        broker: &crate::effects::EffectBroker<'_, A>,
        grant: &crate::effects::ScopedGrant,
        approval: &crate::effects::Approval,
    ) -> Result<EventStream, ProviderError> {
        let events = self
            .request(
                serde_json::json!({"v":1,"id":"run-1","type":"run","mode":"deterministic","input":input}),
                broker,
                grant,
                approval,
            )
            .await?;
        let mut normalized = Vec::new();
        for event in events {
            match event.kind.as_str() {
                "message_delta" => normalized.push(Ok(ProviderEvent::OutputDelta(
                    event.text.unwrap_or_default(),
                ))),
                "tool_preflight" => {
                    let cap = event.capability.unwrap_or_default();
                    if !allowed_capabilities.contains(&cap) {
                        return Err(ProviderError::Response {
                            code: "capability_denied".into(),
                            message: format!(
                                "tool {} requires capability {cap}",
                                event.name.unwrap_or_default()
                            ),
                        });
                    }
                    normalized.push(Ok(ProviderEvent::ToolCallDelta {
                        call_id: event.call_id,
                        name: event.name,
                        arguments: event.arguments.unwrap_or(Value::Null).to_string(),
                    }));
                }
                "usage" => normalized.push(Ok(ProviderEvent::Completed(Usage {
                    input_tokens: event.input_tokens,
                    output_tokens: event.output_tokens,
                    total_tokens: event.total_tokens,
                }))),
                _ => {}
            }
        }
        Ok(Box::pin(tokio_stream::iter(normalized)))
    }
}

#[async_trait(?Send)]
impl PiAdapter for PiGateway {
    fn launch_isolation(&self, _route: &Route) -> Vec<crate::domain::ExecutionIsolation> {
        use crate::domain::{ExecutionIsolation, IsolationDimension};
        IsolationDimension::ALL.into_iter().map(|dimension| {
            let (active, enforced, mechanism, limitation) = match dimension {
                IsolationDimension::Process => (true, true, "dedicated kill-on-drop child process", "no PID namespace or syscall sandbox"),
                IsolationDimension::Credentials => (true, true, "environment cleared; only PATH and optional ASTER_PI_NODE_MODULES injected", "child may access credentials available through host files or services"),
                IsolationDimension::WorkspaceWorktree => (false, false, "inherits runtime working directory", "no separate worktree or workspace boundary"),
                IsolationDimension::Filesystem => (false, false, "host filesystem access", "no OS filesystem sandbox"),
                IsolationDimension::Network => (false, false, "host network stack", "no network namespace or destination filter at sidecar launch"),
                IsolationDimension::ExternalServices => (false, false, "sidecar/provider protocol only", "external-service access is not independently sandboxed"),
            };
            ExecutionIsolation { task_id: uuid::Uuid::nil(), attempt: 0, operation_id: uuid::Uuid::nil(), dimension, active, enforced, mechanism: mechanism.into(), limitation: limitation.into(), recorded_at: chrono::Utc::now() }
        }).collect()
    }

    async fn execute_controlled(
        &self,
        prompt: &str,
        route: &Route,
        store: &crate::store::Store,
        task_id: uuid::Uuid,
    ) -> anyhow::Result<ExecutionResult> {
        use crate::effects::{
            Approval, Capability, EffectBroker, FilesystemIsolation, IsolationProfile,
            NetworkIsolation, ProcessIsolation, ScopedGrant, SecretIsolation, SystemAdapter,
        };
        let request = self.launch_request()?;
        let (program, cwd) = match &request {
            crate::effects::EffectRequest::Exec { program, cwd, .. } => {
                (program.clone(), cwd.clone())
            }
            _ => unreachable!(),
        };
        let grant = ScopedGrant {
            id: uuid::Uuid::new_v4(),
            task_id,
            capabilities: [Capability::ProcessExec].into_iter().collect(),
            workspace: cwd,
            worktrees: vec![],
            executable_allowlist: [program].into_iter().collect(),
            network_allowlist: BTreeSet::new(),
            external_allowlist: BTreeSet::new(),
            secret_destinations: BTreeMap::new(),
            isolation: IsolationProfile {
                filesystem: FilesystemIsolation::WorkspaceReadOnly,
                process: ProcessIsolation::ScrubbedEnvironment,
                network: NetworkIsolation::Denied,
                secrets: SecretIsolation::Denied,
            },
            expires_at: Some(chrono::Utc::now() + chrono::Duration::minutes(5)),
        };
        let approval = Approval::for_request(
            task_id,
            grant.id,
            &request,
            chrono::Utc::now() + chrono::Duration::minutes(5),
        )?;
        let broker = EffectBroker {
            store,
            adapter: SystemAdapter,
        };
        let effort = match route.dimensions.effort {
            Effort::Low => ReasoningEffort::Low,
            Effort::Medium => ReasoningEffort::Medium,
            Effort::High => ReasoningEffort::High,
        };
        let input = PiRunInput {
            prompt: prompt.to_owned(),
            model: route.model.clone(),
            effort,
            context: Some(
                serde_json::json!({
                    "role": route.role,
                    "decision_id": route.decision_id,
                    "execution_dimensions": route.dimensions,
                })
                .to_string(),
            ),
            fixture_tool: None,
        };
        let allowed = route.dimensions.capabilities.iter().cloned().collect();
        let mut stream = self
            .run_deterministic(input, &allowed, &broker, &grant, &approval)
            .await?;
        let mut output = String::new();
        let mut usage_tokens = 0;
        while let Some(event) = stream.next().await {
            match event? {
                ProviderEvent::OutputDelta(delta) => output.push_str(&delta),
                ProviderEvent::Completed(usage) => {
                    usage_tokens = usage.total_tokens.unwrap_or_else(|| {
                        usage.input_tokens.unwrap_or_default()
                            + usage.output_tokens.unwrap_or_default()
                    });
                }
                ProviderEvent::ReasoningDelta(_) | ProviderEvent::ToolCallDelta { .. } => {}
            }
        }
        Ok(ExecutionResult {
            output,
            usage_tokens,
        })
    }

    async fn execute(&self, _prompt: &str, _route: &Route) -> anyhow::Result<ExecutionResult> {
        anyhow::bail!("Pi execution requires trusted control-plane authorization")
    }
}
