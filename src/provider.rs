use crate::domain::Route;
use anyhow::Result;
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub output: String,
    pub usage_tokens: u64,
}

#[async_trait]
pub trait PiAdapter: Send + Sync {
    async fn execute(&self, prompt: &str, route: &Route) -> Result<ExecutionResult>;
}

#[derive(Default)]
pub struct FakePiAdapter;

#[async_trait]
impl PiAdapter for FakePiAdapter {
    async fn execute(&self, prompt: &str, route: &Route) -> Result<ExecutionResult> {
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        Ok(ExecutionResult {
            output: format!("Fake Pi execution completed as {}: {}", route.role, prompt),
            usage_tokens: prompt.split_whitespace().count() as u64 + 12,
        })
    }
}
