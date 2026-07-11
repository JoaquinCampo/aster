use crate::domain::Route;
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fmt, pin::Pin};
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

#[async_trait]
pub trait Provider: Send + Sync {
    async fn stream(&self, request: ProviderRequest) -> Result<EventStream, ProviderError>;
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
    async fn stream(&self, request: ProviderRequest) -> Result<EventStream, ProviderError> {
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
    async fn stream(&self, r: ProviderRequest) -> Result<EventStream, ProviderError> {
        if !matches!(
            r.model.as_str(),
            "gpt-5.6-luna" | "gpt-5.6-terra" | "gpt-5.6-sol"
        ) {
            return Err(ProviderError::Configuration(
                "unsupported Codex bridge model ID".into(),
            ));
        }
        self.0.stream(r).await
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
    async fn stream(&self, r: ProviderRequest) -> Result<EventStream, ProviderError> {
        if !r.model.starts_with("grok-") {
            return Err(ProviderError::Configuration(
                "xAI model must use its full grok-* ID".into(),
            ));
        }
        self.0.stream(r).await
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
        Ok(ExecutionResult {
            output: format!("Fake Pi execution completed as {}: {}", route.role, prompt),
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
    async fn stream(&self, _: ProviderRequest) -> Result<EventStream, ProviderError> {
        Ok(Box::pin(tokio_stream::iter(
            self.events.clone().into_iter().map(Ok),
        )))
    }
}
