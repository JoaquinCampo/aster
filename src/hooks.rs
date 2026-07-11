use crate::{
    effects::Capability,
    plugin::{BrokerRequest, EffectBroker},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum HookTrigger {
    BeforeTask,
    AfterTask,
    BeforeTool,
    AfterTool,
    OnFailure,
    OnCheckpoint,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookFailurePolicy {
    Continue,
    FailExecution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookSpec {
    pub id: String,
    pub trigger: HookTrigger,
    pub executable: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    pub failure_policy: HookFailurePolicy,
    #[serde(default)]
    pub capabilities: BTreeSet<Capability>,
}
fn default_timeout_ms() -> u64 {
    5_000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookInvocation {
    pub trigger: HookTrigger,
    pub context: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HookResponse {
    #[serde(default)]
    result: Value,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    effect: Option<BrokerRequest>,
}
#[derive(Debug, Clone, PartialEq)]
pub enum HookOutcome {
    Completed(Value),
    Continued(String),
}

pub struct HookRunner<B: EffectBroker> {
    broker: B,
}
impl<B: EffectBroker> HookRunner<B> {
    pub fn new(broker: B) -> Self {
        Self { broker }
    }
    pub fn run(&self, spec: &HookSpec, context: Value) -> Result<HookOutcome> {
        if spec.id.is_empty() || spec.timeout_ms == 0 || !spec.executable.is_file() {
            bail!("invalid hook specification")
        }
        let operation = self
            .broker
            .begin_spawn(&format!("hook:{}", spec.id), &spec.executable)?;
        let mut child = Command::new(&spec.executable)
            .args(&spec.args)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("spawn hook")?;
        let payload = serde_json::to_vec(&HookInvocation {
            trigger: spec.trigger,
            context,
        })?;
        child
            .stdin
            .take()
            .context("hook stdin unavailable")?
            .write_all(&payload)?;
        let timeout = Duration::from_millis(spec.timeout_ms);
        let started = std::time::Instant::now();
        let output = loop {
            if child.try_wait()?.is_some() {
                break child.wait_with_output()?;
            }
            if started.elapsed() >= timeout {
                let _ = child.kill();
                let _ = child.wait();
                self.broker.finish_spawn(operation, false)?;
                return self.failure(spec, "hook timed out");
            }
            thread::sleep(Duration::from_millis(5));
        };
        if !output.status.success() {
            self.broker.finish_spawn(operation, false)?;
            return self.failure(spec, "hook process failed");
        }
        let response: HookResponse =
            serde_json::from_slice(&output.stdout).context("invalid hook response")?;
        if let Some(error) = response.error {
            self.broker.finish_spawn(operation, false)?;
            return self.failure(spec, &error);
        }
        let result = if let Some(effect) = response.effect {
            if !spec.capabilities.contains(&effect.capability) {
                self.broker.finish_spawn(operation, false)?;
                return self.failure(spec, "hook requested undeclared capability");
            }
            self.broker.execute(&format!("hook:{}", spec.id), effect)?
        } else {
            response.result
        };
        self.broker.finish_spawn(operation, true)?;
        Ok(HookOutcome::Completed(result))
    }
    fn failure(&self, spec: &HookSpec, message: &str) -> Result<HookOutcome> {
        match spec.failure_policy {
            HookFailurePolicy::Continue => Ok(HookOutcome::Continued(message.into())),
            HookFailurePolicy::FailExecution => bail!(message.to_owned()),
        }
    }
}
