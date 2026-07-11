use anyhow::{Result, bail};
use aster::mcp::{Client, NetworkDisclosure, NetworkMediator, StreamableHttpTransport};
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

#[derive(Clone)]
struct Mediator(Arc<Mutex<Vec<NetworkDisclosure>>>);
impl NetworkMediator for Mediator {
    fn authorize(&self, disclosure: &NetworkDisclosure) -> Result<()> {
        if !disclosure.destination.starts_with("http://127.0.0.1:") {
            bail!("destination denied")
        }
        self.0.lock().unwrap().push(disclosure.clone());
        Ok(())
    }
}

#[test]
fn local_streamable_http_session_disclosure_discovery_and_invocation() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let endpoint = format!("http://{}/mcp", listener.local_addr()?);
    let server = thread::spawn(move || -> Result<()> {
        for sequence in 0..3 {
            let (mut stream, _) = listener.accept()?;
            let mut bytes = Vec::new();
            let mut buf = [0; 4096];
            loop {
                let n = stream.read(&mut buf)?;
                bytes.extend_from_slice(&buf[..n]);
                if let Some(split) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..split]);
                    let len = headers
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>())
                        })
                        .transpose()?
                        .unwrap_or(0);
                    if bytes.len() >= split + 4 + len {
                        break;
                    }
                }
            }
            let split = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
            let headers = String::from_utf8_lossy(&bytes[..split]).to_ascii_lowercase();
            if sequence > 0 {
                assert!(headers.contains("mcp-session-id: local-session"));
            }
            let request: serde_json::Value = serde_json::from_slice(&bytes[split + 4..])?;
            let result = match request["method"].as_str().unwrap() {
                "initialize" => {
                    json!({"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"local","version":"1"}})
                }
                "tools/list" => json!({"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}),
                "tools/call" => json!({"content":[{"type":"text","text":"ok"}]}),
                _ => unreachable!(),
            };
            let body = json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nmcp-session-id: local-session\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            )?;
        }
        Ok(())
    });
    let disclosures = Arc::new(Mutex::new(Vec::new()));
    let transport = StreamableHttpTransport::new(
        &endpoint,
        Duration::from_secs(2),
        Mediator(disclosures.clone()),
        vec!["tool arguments".into()],
    )?;
    let mut client = Client::new(transport);
    client.initialize()?;
    assert_eq!(client.list_tools()?["tools"][0]["name"], "echo");
    assert_eq!(
        client.call_tool("echo", json!({"value":"hi"}))?["content"][0]["text"],
        "ok"
    );
    server.join().unwrap()?;
    assert_eq!(disclosures.lock().unwrap().len(), 3);
    assert_eq!(
        disclosures.lock().unwrap()[0].context_classes,
        ["tool arguments"]
    );
    Ok(())
}

#[test]
fn built_in_streamable_server_enforces_initialization_sessions_and_errors() -> Result<()> {
    use aster::mcp::{CancellationToken, Server, serve_streamable_http};
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let endpoint = format!("http://{}/mcp", listener.local_addr()?);
    let shutdown = CancellationToken::default();
    let stop = shutdown.clone();
    let server = thread::spawn(move || {
        let fixture = Server::new("local", "1").tool("echo", json!({"type":"object"}), Ok);
        serve_streamable_http(listener, &fixture, stop)
    });
    let disclosures = Arc::new(Mutex::new(Vec::new()));
    let transport = StreamableHttpTransport::new(
        &endpoint,
        Duration::from_secs(2),
        Mediator(disclosures),
        vec!["selected tool arguments".into()],
    )?;
    let mut client = Client::new(transport);
    client.initialize()?;
    assert_eq!(client.list_tools()?["tools"][0]["name"], "echo");
    assert!(
        client
            .call_tool("missing", json!({}))
            .unwrap_err()
            .to_string()
            .contains("unknown tool")
    );
    client.transport_mut().terminate_session()?;
    assert!(
        client
            .list_tools()
            .unwrap_err()
            .to_string()
            .contains("initialize required")
    );
    shutdown.cancel();
    server.join().unwrap()?;
    Ok(())
}

#[test]
fn timeout_and_in_flight_cancellation_are_bounded() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let endpoint = format!("http://{}/mcp", listener.local_addr()?);
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (_stream, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_millis(250));
        }
    });
    let disclosures = Arc::new(Mutex::new(Vec::new()));
    let transport = StreamableHttpTransport::new(
        &endpoint,
        Duration::from_millis(30),
        Mediator(disclosures.clone()),
        vec![],
    )?;
    let mut client = Client::new(transport);
    assert!(
        client
            .initialize()
            .unwrap_err()
            .to_string()
            .contains("timed out")
    );
    let transport = StreamableHttpTransport::new(
        &endpoint,
        Duration::from_secs(2),
        Mediator(disclosures),
        vec![],
    )?;
    let token = transport.cancellation_token();
    let cancel = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        token.cancel();
    });
    let mut client = Client::new(transport);
    assert!(
        client
            .initialize()
            .unwrap_err()
            .to_string()
            .contains("cancelled in flight")
    );
    cancel.join().unwrap();
    server.join().unwrap();
    Ok(())
}

#[test]
fn cancellation_prevents_authorization_and_network_io() -> Result<()> {
    let disclosures = Arc::new(Mutex::new(Vec::new()));
    let transport = StreamableHttpTransport::new(
        "http://127.0.0.1:9/mcp",
        Duration::from_millis(50),
        Mediator(disclosures.clone()),
        vec![],
    )?;
    let token = transport.cancellation_token();
    token.cancel();
    let mut client = Client::new(transport);
    assert!(
        client
            .initialize()
            .unwrap_err()
            .to_string()
            .contains("cancelled")
    );
    assert!(disclosures.lock().unwrap().is_empty());
    Ok(())
}
