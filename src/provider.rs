use crate::domain::{OperationState, Route};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fmt, pin::Pin, sync::Arc};
use thiserror::Error;
use tokio_stream::Stream;

/// Canonical provider-independent reasoning allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    None,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl ReasoningEffort {
    pub fn normalize(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "none" | "off" => Self::None,
            "minimal" | "light" | "low" => Self::Low,
            "high" => Self::High,
            "extra_high" | "extrahigh" | "xhigh" | "ultra" => Self::XHigh,
            "max" | "maximum" => Self::Max,
            _ => Self::Medium,
        }
    }
    fn wire(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderEvent {
    OutputDelta(String),
    ReasoningDelta(String),
    ToolCallDelta {
        call_id: Option<String>,
        name: Option<String>,
        arguments: String,
    },
    Completed(Usage),
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("invalid provider configuration: {0}")]
    Configuration(String),
    #[error("provider transport failed: {0}")]
    Transport(String),
    #[error("provider returned HTTP {status}: {code}: {message}")]
    Http {
        status: StatusCode,
        code: String,
        message: String,
    },
    #[error("provider stream protocol error: {0}")]
    Protocol(String),
    #[error("provider response failed: {code}: {message}")]
    Response { code: String, message: String },
}

pub type EventStream = Pin<Box<dyn Stream<Item = Result<ProviderEvent, ProviderError>> + Send>>;

#[derive(Debug, Clone)]
pub struct ProviderRequest {
    pub model: String,
    pub prompt: String,
    pub effort: ReasoningEffort,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkDisclosure {
    pub destination: String,
    pub purpose: String,
    pub context: Vec<String>,
    pub classification: String,
}

impl NetworkDisclosure {
    pub fn audit_detail(&self) -> String {
        format!(
            "destination={} purpose={} classification={} context={}",
            self.destination,
            self.purpose,
            self.classification,
            self.context.join(",")
        )
    }

    pub fn audit_event(&self, task_id: uuid::Uuid) -> crate::domain::AuditEvent {
        crate::domain::AuditEvent {
            id: uuid::Uuid::new_v4(),
            task_id,
            kind: "network.destination_disclosed".into(),
            detail: self.audit_detail(),
            at: chrono::Utc::now(),
        }
    }
}

pub trait NetworkAuthorizer: Send + Sync {
    fn authorize(&self, disclosure: &NetworkDisclosure) -> Result<(), ProviderError>;
}

/// Explicit fail-closed authorizer for callers that have no network grant.
#[derive(Debug, Default, Clone, Copy)]
pub struct DenyNetwork;

impl NetworkAuthorizer for DenyNetwork {
    fn authorize(&self, _: &NetworkDisclosure) -> Result<(), ProviderError> {
        Err(ProviderError::Transport(
            "network authorization denied".into(),
        ))
    }
}

pub struct EffectBrokerNetworkAuthorizer<'a, A: crate::effects::EffectAdapter> {
    grant: &'a crate::effects::ScopedGrant,
    _adapter: std::marker::PhantomData<A>,
}

impl<'a, A: crate::effects::EffectAdapter> EffectBrokerNetworkAuthorizer<'a, A> {
    pub fn new(
        _: &crate::effects::EffectBroker<'_, A>,
        grant: &'a crate::effects::ScopedGrant,
    ) -> Self {
        Self {
            grant,
            _adapter: std::marker::PhantomData,
        }
    }
}

impl<A: crate::effects::EffectAdapter> NetworkAuthorizer for EffectBrokerNetworkAuthorizer<'_, A> {
    fn authorize(&self, disclosure: &NetworkDisclosure) -> Result<(), ProviderError> {
        crate::effects::Policy::evaluate(
            self.grant,
            &crate::effects::EffectRequest::Network {
                destination: disclosure.destination.clone(),
                payload: Vec::new(),
            },
        )
        .map_err(|error| ProviderError::Transport(format!("network authorization denied: {error}")))
    }
}

#[async_trait]
pub trait Provider: Send + Sync {
    async fn stream(
        &self,
        request: ProviderRequest,
        authorizer: &dyn NetworkAuthorizer,
    ) -> Result<EventStream, ProviderError>;
    fn network_disclosure(&self) -> Option<NetworkDisclosure> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Support {
    Supported,
    Unsupported,
    ModelDependent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub reasoning: Support,
    pub tools: Support,
    pub streaming: Support,
    pub usage: Support,
    pub structured_errors: Support,
    pub cancellation: Support,
}

impl ProviderCapabilities {
    pub const RESPONSES: Self = Self {
        reasoning: Support::ModelDependent,
        tools: Support::Supported,
        streaming: Support::Supported,
        usage: Support::Supported,
        structured_errors: Support::Supported,
        cancellation: Support::Supported,
    };
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatus {
    NotRequired,
    ReferenceAvailable,
    ReferenceMissing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeStatus {
    Fixture,
    Unchecked,
    Healthy,
    Unhealthy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderStatus {
    pub id: String,
    pub display_name: String,
    pub adapter: String,
    pub models: Vec<String>,
    pub capabilities: ProviderCapabilities,
    pub auth_reference: Option<String>,
    pub auth_status: AuthStatus,
    pub probe_status: ProbeStatus,
    pub diagnostic: String,
}

impl ProviderStatus {
    pub fn negotiate(
        &self,
        model: &str,
        effort: ReasoningEffort,
        needs_tools: bool,
    ) -> Result<(), ProviderError> {
        if !self.models.is_empty()
            && !self.models.iter().any(|advertised| {
                advertised == model
                    || (advertised.ends_with('*')
                        && model.starts_with(advertised.trim_end_matches('*')))
            })
        {
            return Err(ProviderError::Configuration(format!(
                "provider {} does not advertise model {model}",
                self.id
            )));
        }
        if effort != ReasoningEffort::None && self.capabilities.reasoning == Support::Unsupported {
            return Err(ProviderError::Configuration(format!(
                "provider {} does not support reasoning",
                self.id
            )));
        }
        if needs_tools && self.capabilities.tools == Support::Unsupported {
            return Err(ProviderError::Configuration(format!(
                "provider {} does not support tools",
                self.id
            )));
        }
        if self.auth_status == AuthStatus::ReferenceMissing {
            return Err(ProviderError::Configuration(format!(
                "provider {} auth reference is unavailable",
                self.id
            )));
        }
        Ok(())
    }
}

#[derive(Default, Clone)]
pub struct ProviderRegistry {
    providers: BTreeMap<String, (Arc<dyn Provider>, ProviderStatus)>,
}

impl ProviderRegistry {
    pub fn register(
        &mut self,
        status: ProviderStatus,
        provider: Arc<dyn Provider>,
    ) -> Result<(), ProviderError> {
        if status.id.trim().is_empty() {
            return Err(ProviderError::Configuration("provider ID is empty".into()));
        }
        if self
            .providers
            .insert(status.id.clone(), (provider, status.clone()))
            .is_some()
        {
            return Err(ProviderError::Configuration(format!(
                "duplicate provider ID {}",
                status.id
            )));
        }
        Ok(())
    }

    pub fn statuses(&self) -> Vec<ProviderStatus> {
        self.providers
            .values()
            .map(|(_, status)| status.clone())
            .collect()
    }

    pub fn resolve(
        &self,
        id: &str,
        model: &str,
        effort: ReasoningEffort,
        needs_tools: bool,
    ) -> Result<Arc<dyn Provider>, ProviderError> {
        let (provider, status) = self
            .providers
            .get(id)
            .ok_or_else(|| ProviderError::Configuration(format!("unknown provider {id}")))?;
        status.negotiate(model, effort, needs_tools)?;
        Ok(provider.clone())
    }
}

pub fn builtin_statuses(
    xai_auth_ref: Option<&str>,
    generic_auth_ref: Option<&str>,
    live: bool,
) -> Vec<ProviderStatus> {
    let probe_status = if live {
        ProbeStatus::Unchecked
    } else {
        ProbeStatus::Fixture
    };
    let auth = |reference: Option<&str>| {
        if reference.is_some() {
            AuthStatus::ReferenceAvailable
        } else {
            AuthStatus::ReferenceMissing
        }
    };
    vec![
        ProviderStatus {
            id: "codex".into(),
            display_name: "Codex bridge".into(),
            adapter: "codex_bridge".into(),
            models: vec![
                "gpt-5.6-luna".into(),
                "gpt-5.6-terra".into(),
                "gpt-5.6-sol".into(),
            ],
            capabilities: ProviderCapabilities::RESPONSES,
            auth_reference: Some("CODEX_AUTH_PATH or ~/.codex/auth.json (bridge-owned)".into()),
            auth_status: AuthStatus::NotRequired,
            probe_status: probe_status.clone(),
            diagnostic: if live {
                "not probed; bridge health and Codex session are external"
            } else {
                "deterministic fixture; no live bridge or credential used"
            }
            .into(),
        },
        ProviderStatus {
            id: "xai".into(),
            display_name: "xAI / Grok".into(),
            adapter: "xai_responses".into(),
            models: vec!["grok-*".into()],
            capabilities: ProviderCapabilities::RESPONSES,
            auth_reference: xai_auth_ref.map(str::to_owned),
            auth_status: auth(xai_auth_ref),
            probe_status: probe_status.clone(),
            diagnostic: if live {
                "not probed; status does not claim credential validity"
            } else {
                "deterministic fixture; no live xAI request"
            }
            .into(),
        },
        ProviderStatus {
            id: "openai-compatible".into(),
            display_name: "OpenAI-compatible".into(),
            adapter: "openai_responses".into(),
            models: vec![],
            capabilities: ProviderCapabilities::RESPONSES,
            auth_reference: generic_auth_ref.map(str::to_owned),
            auth_status: auth(generic_auth_ref),
            probe_status,
            diagnostic: if live {
                "not probed; endpoint semantics vary by deployment"
            } else {
                "deterministic fixture; no live provider request"
            }
            .into(),
        },
    ]
}

/// Generic OpenAI Responses API adapter. Dropping the returned stream drops the
/// response body, which cancels the in-flight HTTP transfer.
#[derive(Clone)]
pub struct OpenAiResponsesProvider {
    client: Client,
    endpoint: Url,
    authorization: Option<String>,
}

impl fmt::Debug for OpenAiResponsesProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiResponsesProvider")
            .field("endpoint", &self.endpoint)
            .field(
                "authorization",
                &self.authorization.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl OpenAiResponsesProvider {
    pub fn new(endpoint: Url, authorization: Option<String>) -> Self {
        Self {
            client: Client::new(),
            endpoint,
            authorization,
        }
    }
}

#[async_trait]
impl Provider for OpenAiResponsesProvider {
    async fn stream(
        &self,
        request: ProviderRequest,
        authorizer: &dyn NetworkAuthorizer,
    ) -> Result<EventStream, ProviderError> {
        if request.model.trim().is_empty() || !request.model.contains('-') {
            return Err(ProviderError::Configuration(
                "model must be a canonical full model ID".into(),
            ));
        }
        let body = json!({"model": request.model, "input": request.prompt, "reasoning": {"effort": request.effort.wire()}, "stream": true, "store": false});
        let mut builder = self
            .client
            .post(self.endpoint.clone())
            .header("accept", "text/event-stream")
            .json(&body);
        if let Some(auth) = &self.authorization {
            builder = builder.bearer_auth(auth);
        }
        let disclosure = self
            .network_disclosure()
            .expect("OpenAI Responses providers always disclose their destination");
        authorizer.authorize(&disclosure)?;
        let response = builder
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        if !response.status().is_success() {
            let status = response.status();
            let value: Value = response.json().await.unwrap_or(Value::Null);
            let code = value
                .pointer("/error/code")
                .and_then(Value::as_str)
                .unwrap_or("http_error")
                .to_owned();
            let message = value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("provider request failed")
                .to_owned();
            return Err(ProviderError::Http {
                status,
                code,
                message,
            });
        }
        let mut bytes = response.bytes_stream();
        let output = async_stream::try_stream! {
            let mut buffer = Vec::new();
            while let Some(chunk) = bytes.next().await {
                buffer.extend_from_slice(&chunk.map_err(|e| ProviderError::Transport(e.to_string()))?);
                while let Some(end) = buffer.windows(2).position(|w| w == b"\n\n") {
                    let frame = buffer.drain(..end + 2).collect::<Vec<_>>();
                    let text = String::from_utf8(frame).map_err(|_| ProviderError::Protocol("SSE was not UTF-8".into()))?;
                    let data = text.lines().filter_map(|l| l.strip_prefix("data:")).map(str::trim_start).collect::<Vec<_>>().join("\n");
                    if data.is_empty() || data == "[DONE]" { continue; }
                    let value: Value = serde_json::from_str(&data).map_err(|e| ProviderError::Protocol(format!("invalid SSE JSON: {e}")))?;
                    if let Some(event) = parse_event(&value)? { yield event; }
                }
            }
            if !buffer.iter().all(u8::is_ascii_whitespace) { Err(ProviderError::Protocol("truncated SSE frame".into()))?; }
        };
        Ok(Box::pin(output))
    }

    fn network_disclosure(&self) -> Option<NetworkDisclosure> {
        Some(NetworkDisclosure {
            destination: self.endpoint.origin().ascii_serialization(),
            purpose: "task provider response generation".into(),
            context: vec![
                "model identifier".into(),
                "task prompt/context".into(),
                "reasoning effort".into(),
            ],
            classification: "task_communication_not_product_telemetry".into(),
        })
    }
}

fn parse_event(value: &Value) -> Result<Option<ProviderEvent>, ProviderError> {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| ProviderError::Protocol("event missing type".into()))?;
    Ok(match kind {
        "response.output_text.delta" => Some(ProviderEvent::OutputDelta(
            value
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
        )),
        "response.reasoning_summary_text.delta" => Some(ProviderEvent::ReasoningDelta(
            value
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
        )),
        "response.function_call_arguments.delta" => Some(ProviderEvent::ToolCallDelta {
            call_id: value
                .get("call_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            name: value.get("name").and_then(Value::as_str).map(str::to_owned),
            arguments: value
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
        }),
        "response.completed" => {
            let usage = value.pointer("/response/usage").unwrap_or(&Value::Null);
            Some(ProviderEvent::Completed(Usage {
                input_tokens: usage.get("input_tokens").and_then(Value::as_u64),
                output_tokens: usage.get("output_tokens").and_then(Value::as_u64),
                total_tokens: usage.get("total_tokens").and_then(Value::as_u64),
            }))
        }
        "response.failed" => {
            return Err(ProviderError::Response {
                code: value
                    .pointer("/response/error/code")
                    .and_then(Value::as_str)
                    .unwrap_or("response_failed")
                    .into(),
                message: value
                    .pointer("/response/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("provider response failed")
                    .into(),
            });
        }
        _ => None,
    })
}

#[derive(Clone, Debug)]
pub struct CodexBridgeProvider(OpenAiResponsesProvider);
impl CodexBridgeProvider {
    pub fn local() -> Self {
        Self(OpenAiResponsesProvider::new(
            Url::parse("http://127.0.0.1:18474/v1/responses").expect("constant URL"),
            None,
        ))
    }
    pub fn at(endpoint: Url) -> Self {
        Self(OpenAiResponsesProvider::new(endpoint, None))
    }
}
#[async_trait]
impl Provider for CodexBridgeProvider {
    async fn stream(
        &self,
        r: ProviderRequest,
        authorizer: &dyn NetworkAuthorizer,
    ) -> Result<EventStream, ProviderError> {
        if !matches!(
            r.model.as_str(),
            "gpt-5.6-luna" | "gpt-5.6-terra" | "gpt-5.6-sol"
        ) {
            return Err(ProviderError::Configuration(
                "unsupported Codex bridge model ID".into(),
            ));
        }
        self.0.stream(r, authorizer).await
    }
    fn network_disclosure(&self) -> Option<NetworkDisclosure> {
        self.0.network_disclosure()
    }
}

#[derive(Clone, Debug)]
pub struct XaiProvider(OpenAiResponsesProvider);
impl XaiProvider {
    pub fn new(endpoint: Url, api_key: String) -> Self {
        Self(OpenAiResponsesProvider::new(endpoint, Some(api_key)))
    }
}
#[async_trait]
impl Provider for XaiProvider {
    async fn stream(
        &self,
        r: ProviderRequest,
        authorizer: &dyn NetworkAuthorizer,
    ) -> Result<EventStream, ProviderError> {
        if !r.model.starts_with("grok-") {
            return Err(ProviderError::Configuration(
                "xAI model must use its full grok-* ID".into(),
            ));
        }
        self.0.stream(r, authorizer).await
    }
    fn network_disclosure(&self) -> Option<NetworkDisclosure> {
        self.0.network_disclosure()
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub output: String,
    pub usage_tokens: u64,
}
#[async_trait]
pub trait PiAdapter: Send + Sync {
    async fn execute(&self, prompt: &str, route: &Route) -> anyhow::Result<ExecutionResult>;
    /// A live adapter must opt in before the runtime asks it to prove a lost outcome.
    fn supports_reconciliation(&self) -> bool {
        false
    }
    async fn reconcile(&self, _operation_id: uuid::Uuid) -> anyhow::Result<Option<OperationState>> {
        Ok(None)
    }
    /// Cooperative in-flight cancellation is optional; safe-boundary cancellation is universal.
    fn supports_cancellation(&self) -> bool {
        false
    }
}

/// Process boundary for a future Pi child-process protocol; it deliberately
/// contains no process spawning policy or TUI/runtime coupling.
#[async_trait]
pub trait PiProcess: Send + Sync {
    async fn execute_process(&self, request: ProviderRequest)
    -> Result<EventStream, ProviderError>;
}

#[derive(Default)]
pub struct FakePiAdapter;
#[async_trait]
impl PiAdapter for FakePiAdapter {
    async fn execute(&self, prompt: &str, route: &Route) -> anyhow::Result<ExecutionResult> {
        if prompt.contains("cenario:timeout") {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        if prompt.contains("cenario:permission-denied") {
            anyhow::bail!("permission denied by deterministic effect broker fixture")
        }
        if prompt.contains("cenario:injected-crash") {
            std::process::exit(86);
        }
        let output = if prompt.starts_with("deterministic checker")
            || prompt.starts_with("independent checker")
        {
            serde_json::json!({"status":"Passed","rationale":"deterministic fake checker pass"})
                .to_string()
        } else {
            format!("Fake Pi execution completed as {}: {}", route.role, prompt)
        };
        Ok(ExecutionResult {
            output,
            usage_tokens: prompt.split_whitespace().count() as u64 + 12,
        })
    }
}

#[derive(Debug, Clone)]
pub struct DeterministicFakeProvider {
    events: Vec<ProviderEvent>,
}
impl DeterministicFakeProvider {
    pub fn new(events: Vec<ProviderEvent>) -> Self {
        Self { events }
    }
}
#[async_trait]
impl Provider for DeterministicFakeProvider {
    async fn stream(
        &self,
        _: ProviderRequest,
        _: &dyn NetworkAuthorizer,
    ) -> Result<EventStream, ProviderError> {
        Ok(Box::pin(tokio_stream::iter(
            self.events.clone().into_iter().map(Ok),
        )))
    }
}
