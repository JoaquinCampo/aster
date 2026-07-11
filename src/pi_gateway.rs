use crate::provider::{EventStream, ProviderError, ProviderEvent, ReasoningEffort, Usage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeSet, path::PathBuf, process::Stdio};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
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

    async fn request(&self, request: Value) -> Result<Vec<WireEvent>, ProviderError> {
        let mut command = Command::new(&self.node);
        command
            .arg(&self.sidecar)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default());
        if let Some(path) = &self.node_modules {
            command.env("ASTER_PI_NODE_MODULES", path);
        }
        let mut child = command
            .spawn()
            .map_err(|e| ProviderError::Transport(format!("failed to start Pi sidecar: {e}")))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| ProviderError::Transport("sidecar stdin unavailable".into()))?;
        stdin
            .write_all(format!("{}\n", request).as_bytes())
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        drop(stdin);
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ProviderError::Transport("sidecar stdout unavailable".into()))?;
        let mut lines = BufReader::new(stdout).lines();
        let mut events = Vec::new();
        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?
        {
            let event: WireEvent = serde_json::from_str(&line)
                .map_err(|e| ProviderError::Protocol(format!("invalid sidecar event: {e}")))?;
            if event.v != PI_PROTOCOL_VERSION {
                return Err(ProviderError::Protocol(
                    "unsupported sidecar protocol".into(),
                ));
            }
            if let Some(error) = &event.error {
                return Err(ProviderError::Response {
                    code: error.code.clone(),
                    message: error.message.clone(),
                });
            }
            events.push(event);
        }
        let status = child
            .wait()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        if !status.success() {
            return Err(ProviderError::Transport(format!(
                "Pi sidecar exited with {status}"
            )));
        }
        Ok(events)
    }

    pub async fn discover(&self) -> Result<PiDiscovery, ProviderError> {
        let events = self
            .request(serde_json::json!({"v":1,"id":"discover","type":"discover"}))
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

    pub async fn run_fixture(
        &self,
        input: PiRunInput,
        allowed_capabilities: &BTreeSet<String>,
    ) -> Result<EventStream, ProviderError> {
        let events = self
            .request(
                serde_json::json!({"v":1,"id":"run-1","type":"run","mode":"fixture","input":input}),
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
