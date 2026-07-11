use aster::{
    effects::Capability,
    mcp::{Server, serve_stdio},
    plugin::BrokerRequest,
};
use serde_json::{Value, json};
use std::{
    io::{self, Read, Write},
    thread,
    time::Duration,
};

fn main() -> anyhow::Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("mcp") => {
            let server = Server::new("fixture", "1").tool("echo", json!({"type":"object"}), Ok);
            serve_stdio(&server, io::BufReader::new(io::stdin()), io::stdout())
        }
        Some("hook-ok") => hook(json!({"result": {"ok": true}})),
        Some("hook-effect") => {
            let effect = BrokerRequest {
                capability: Capability::FileRead,
                operation: "read".into(),
                arguments: json!({"path":"README.md"}),
            };
            hook(json!({"effect": effect}))
        }
        Some("hook-error") => hook(json!({"error":"fixture rejected"})),
        Some("hook-sleep") => {
            hook_after_ready(json!({"result":null}), Some(Duration::from_secs(2)))
        }
        _ => anyhow::bail!("fixture mode required"),
    }
}
fn hook(response: Value) -> anyhow::Result<()> {
    hook_after_ready(response, None)
}
fn hook_after_ready(response: Value, delay: Option<Duration>) -> anyhow::Result<()> {
    println!(r#"{{"protocol":"aster-hook-v1","ready":true}}"#);
    io::stdout().flush()?;
    if let Some(delay) = delay {
        thread::sleep(delay);
    }
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let _: Value = serde_json::from_str(&input)?;
    println!("{response}");
    Ok(())
}
