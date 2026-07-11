use crate::{
    effects::Capability,
    plugin::{BrokerRequest, EffectBroker},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader, Write},
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

pub trait LifecycleHooks: Send + Sync {
    fn invoke(&self, trigger: HookTrigger, context: Value) -> Result<Vec<HookOutcome>>;
}

pub struct HookSet<B: EffectBroker> {
    runner: HookRunner<B>,
    specs: Vec<HookSpec>,
}
impl<B: EffectBroker> HookSet<B> {
    pub fn new(broker: B, specs: Vec<HookSpec>) -> Self {
        Self {
            runner: HookRunner::new(broker),
            specs,
        }
    }
}
impl<B: EffectBroker + Send + Sync> LifecycleHooks for HookSet<B> {
    fn invoke(&self, trigger: HookTrigger, context: Value) -> Result<Vec<HookOutcome>> {
        self.specs
            .iter()
            .filter(|spec| spec.trigger == trigger)
            .map(|spec| self.runner.run(spec, context.clone()))
            .collect()
    }
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
        let stdout = BufReader::new(child.stdout.take().context("hook stdout unavailable")?);
        let (sender, receiver) = std::sync::mpsc::sync_channel(2);
        thread::spawn(move || {
            let mut stdout = stdout;
            let mut ready = String::new();
            if let Err(error) = stdout.read_line(&mut ready) {
                let _ = sender.send(Err(error));
                return;
            }
            if sender.send(Ok(ready.into_bytes())).is_err() {
                return;
            }
            let mut response = Vec::new();
            let result = std::io::Read::read_to_end(&mut stdout, &mut response).map(|_| response);
            let _ = sender.send(result);
        });
        let ready = match receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(ready) => String::from_utf8(ready?)?,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                self.broker.finish_spawn(operation, false)?;
                return self.failure(spec, "hook readiness handshake timed out");
            }
        };
        if ready.trim_end() != r#"{"protocol":"aster-hook-v1","ready":true}"# {
            let _ = child.kill();
            let _ = child.wait();
            self.broker.finish_spawn(operation, false)?;
            return self.failure(spec, "hook readiness handshake failed");
        }
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
        let output = match receiver.recv_timeout(timeout) {
            Ok(output) => output?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let _ = child.kill();
                let _ = child.wait();
                self.broker.finish_spawn(operation, false)?;
                return self.failure(spec, "hook timed out");
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                bail!("hook response reader disconnected")
            }
        };
        let status = child.wait()?;
        if !status.success() {
            self.broker.finish_spawn(operation, false)?;
            return self.failure(spec, "hook process failed");
        }
        let response: HookResponse =
            serde_json::from_slice(&output).context("invalid hook response")?;
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
