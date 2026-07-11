use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub const JSONRPC_VERSION: &str = "2.0";
pub const MCP_PROTOCOL_VERSION: &str = "2025-03-26";
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Request {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Response {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}
pub trait Transport {
    fn round_trip(&mut self, request: &str) -> Result<String>;
}

/// A capability decision made before any MCP network bytes are sent. The
/// disclosure is deliberately complete enough for a UI/audit record without
/// exposing request contents accidentally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkDisclosure {
    pub destination: String,
    pub context_classes: Vec<String>,
    pub operation: String,
}

pub trait NetworkMediator {
    fn authorize(&self, disclosure: &NetworkDisclosure) -> Result<()>;
}

pub struct EffectBrokerMediator<'a, 'b, A: crate::effects::EffectAdapter> {
    pub broker: &'a crate::effects::EffectBroker<'b, A>,
    pub grant: &'a crate::effects::ScopedGrant,
}
impl<A: crate::effects::EffectAdapter> NetworkMediator for EffectBrokerMediator<'_, '_, A> {
    fn authorize(&self, disclosure: &NetworkDisclosure) -> Result<()> {
        let destination = reqwest::Url::parse(&disclosure.destination)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_else(|| disclosure.destination.clone());
        self.broker.authorize_network(self.grant, &destination)
    }
}

#[derive(Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);
impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// MCP Streamable HTTP transport. It retains the server-issued session id,
/// uses protocol-correct Accept headers, enforces a finite timeout, and sends
/// every destination through the capability mediator before connecting.
pub struct StreamableHttpTransport<M> {
    endpoint: String,
    client: reqwest::blocking::Client,
    mediator: M,
    session_id: Option<String>,
    context_classes: Vec<String>,
    cancellation: CancellationToken,
}
impl<M: NetworkMediator> StreamableHttpTransport<M> {
    pub fn new(
        endpoint: &str,
        timeout: Duration,
        mediator: M,
        context_classes: Vec<String>,
    ) -> Result<Self> {
        let parsed = reqwest::Url::parse(endpoint)?;
        if !matches!(parsed.scheme(), "http" | "https") {
            bail!("MCP endpoint must use http or https")
        }
        if !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.host_str().is_none()
        {
            bail!("MCP endpoint must not contain credentials and must have a host")
        }
        if timeout.is_zero() {
            bail!("MCP timeout must be non-zero")
        }
        Ok(Self {
            endpoint: parsed.to_string(),
            client: reqwest::blocking::Client::builder()
                .timeout(timeout)
                .build()?,
            mediator,
            session_id: None,
            context_classes,
            cancellation: CancellationToken::default(),
        })
    }
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }
    pub fn disclosure(&self) -> NetworkDisclosure {
        NetworkDisclosure {
            destination: self.endpoint.clone(),
            context_classes: self.context_classes.clone(),
            operation: "mcp.streamable-http".into(),
        }
    }
    pub fn terminate_session(&mut self) -> Result<()> {
        let Some(session) = self.session_id.take() else {
            return Ok(());
        };
        self.mediator.authorize(&self.disclosure())?;
        let response = self
            .client
            .delete(&self.endpoint)
            .header("mcp-session-id", session)
            .send()?;
        if !response.status().is_success() && response.status() != reqwest::StatusCode::NOT_FOUND {
            bail!("MCP session termination failed: {}", response.status())
        }
        Ok(())
    }
}
impl<M: NetworkMediator> Transport for StreamableHttpTransport<M> {
    fn round_trip(&mut self, request: &str) -> Result<String> {
        if self.cancellation.is_cancelled() {
            bail!("MCP request cancelled")
        }
        self.mediator.authorize(&self.disclosure())?;
        let client = self.client.clone();
        let endpoint = self.endpoint.clone();
        let session = self.session_id.clone();
        let request = request.to_owned();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut builder = client
                .post(endpoint)
                .header("accept", "application/json, text/event-stream")
                .header("content-type", "application/json")
                .body(request);
            if let Some(session) = session {
                builder = builder.header("mcp-session-id", session);
            }
            let result = builder.send().and_then(|response| {
                let session = response
                    .headers()
                    .get("mcp-session-id")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned);
                let status = response.status();
                let content_type = response
                    .headers()
                    .get("content-type")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .to_owned();
                response
                    .text()
                    .map(|body| (session, status, content_type, body))
            });
            let _ = tx.send(result);
        });
        let (session, status, content_type, body) = loop {
            if self.cancellation.is_cancelled() {
                let _ = self.terminate_session();
                bail!("MCP request cancelled in flight")
            }
            match rx.recv_timeout(Duration::from_millis(10)) {
                Ok(result) => {
                    break result.map_err(|error| {
                        anyhow::anyhow!(if error.is_timeout() {
                            "MCP request timed out".to_owned()
                        } else {
                            format!("MCP HTTP transport: {error}")
                        })
                    })?;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(error) => bail!("MCP HTTP worker failed: {error}"),
            }
        };
        if session.is_some() {
            self.session_id = session;
        }
        if !status.is_success() {
            bail!("MCP HTTP status {}: {}", status.as_u16(), body)
        }
        if content_type.starts_with("text/event-stream") {
            return body
                .lines()
                .find_map(|line| line.strip_prefix("data:").map(str::trim))
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| anyhow::anyhow!("MCP event stream contained no data"));
        }
        Ok(body)
    }
}
pub struct Client<T> {
    transport: T,
    next_id: u64,
}
impl<T: Transport> Client<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            next_id: 1,
        }
    }
    pub fn transport(&self) -> &T {
        &self.transport
    }
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let req = Request {
            jsonrpc: JSONRPC_VERSION.into(),
            id,
            method: method.into(),
            params,
        };
        let wire = serde_json::to_string(&req)?;
        let response: Response = serde_json::from_str(&self.transport.round_trip(&wire)?)?;
        if response.jsonrpc != JSONRPC_VERSION || response.id != id {
            bail!("invalid JSON-RPC response correlation")
        }
        if let Some(e) = response.error {
            bail!("MCP error {}: {}", e.code, e.message)
        }
        response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing JSON-RPC result"))
    }
    pub fn initialize(&mut self) -> Result<Value> {
        self.call("initialize",json!({"protocolVersion":MCP_PROTOCOL_VERSION,"capabilities":{},"clientInfo":{"name":"aster","version":env!("CARGO_PKG_VERSION")}}))
    }
    pub fn list_tools(&mut self) -> Result<Value> {
        self.call("tools/list", json!({}))
    }
    pub fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value> {
        self.call("tools/call", json!({"name":name,"arguments":arguments}))
    }
}
type Handler = Box<dyn Fn(Value) -> Result<Value> + Send + Sync>;
pub struct Server {
    name: String,
    version: String,
    tools: BTreeMap<String, (Value, Handler)>,
}
impl Server {
    pub fn new(name: &str, version: &str) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            tools: BTreeMap::new(),
        }
    }
    pub fn tool(
        mut self,
        name: &str,
        schema: Value,
        handler: impl Fn(Value) -> Result<Value> + Send + Sync + 'static,
    ) -> Self {
        self.tools.insert(name.into(), (schema, Box::new(handler)));
        self
    }
    pub fn handle(&self, wire: &str) -> String {
        let parsed = serde_json::from_str::<Request>(wire);
        let response = match parsed {
            Err(e) => Response {
                jsonrpc: JSONRPC_VERSION.into(),
                id: 0,
                result: None,
                error: Some(RpcError {
                    code: -32700,
                    message: e.to_string(),
                }),
            },
            Ok(r) if r.jsonrpc != JSONRPC_VERSION => self.err(r.id, -32600, "invalid request"),
            Ok(r) => match self.dispatch(&r.method, r.params) {
                Ok(v) => Response {
                    jsonrpc: JSONRPC_VERSION.into(),
                    id: r.id,
                    result: Some(v),
                    error: None,
                },
                Err(e) => self.err(
                    r.id,
                    if matches!(
                        r.method.as_str(),
                        "initialize" | "tools/list" | "tools/call"
                    ) {
                        -32602
                    } else {
                        -32601
                    },
                    &e.to_string(),
                ),
            },
        };
        serde_json::to_string(&response).expect("response serializes")
    }
    fn err(&self, id: u64, code: i64, message: &str) -> Response {
        Response {
            jsonrpc: JSONRPC_VERSION.into(),
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
        }
    }
    fn dispatch(&self, method: &str, params: Value) -> Result<Value> {
        match method {
            "initialize" => Ok(
                json!({"protocolVersion":MCP_PROTOCOL_VERSION,"capabilities":{"tools":{}},"serverInfo":{"name":self.name,"version":self.version}}),
            ),
            "tools/list" => Ok(
                json!({"tools":self.tools.iter().map(|(name,(schema,_))|json!({"name":name,"inputSchema":schema})).collect::<Vec<_>>() }),
            ),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing tool name"))?;
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let (_, handler) = self
                    .tools
                    .get(name)
                    .ok_or_else(|| anyhow::anyhow!("unknown tool: {name}"))?;
                Ok(
                    json!({"content":[{"type":"text","text":serde_json::to_string(&handler(args)?)?}]}),
                )
            }
            _ => bail!("method not found: {method}"),
        }
    }
}
pub struct StdioTransport {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}
impl StdioTransport {
    pub fn spawn(executable: &Path, args: &[String]) -> Result<Self> {
        let mut child = Command::new(executable)
            .args(args)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("MCP stdin unavailable"))?;
        let output = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| anyhow::anyhow!("MCP stdout unavailable"))?,
        );
        Ok(Self {
            child,
            input,
            output,
        })
    }
}
impl Transport for StdioTransport {
    fn round_trip(&mut self, request: &str) -> Result<String> {
        writeln!(self.input, "{request}")?;
        self.input.flush()?;
        let mut line = String::new();
        if self.output.read_line(&mut line)? == 0 {
            bail!("MCP server closed stdout")
        }
        Ok(line)
    }
}
impl Drop for StdioTransport {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn serve_stdio<R: BufRead, W: Write>(
    server: &Server,
    mut input: R,
    mut output: W,
) -> Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(());
        }
        writeln!(output, "{}", server.handle(line.trim_end()))?;
        output.flush()?;
    }
}

/// Deterministic, dependency-light Streamable HTTP conformance server. It is
/// intended for local harness integration and tests: no external endpoint is
/// required. POST carries JSON-RPC, DELETE cancels a session, and session ids
/// are stable monotonic values scoped to this server process.
pub fn serve_streamable_http(
    listener: TcpListener,
    server: &Server,
    shutdown: CancellationToken,
) -> Result<()> {
    listener.set_nonblocking(true)?;
    let mut next_session = 1_u64;
    let mut sessions = std::collections::BTreeSet::new();
    while !shutdown.is_cancelled() {
        let (mut stream, _) = match listener.accept() {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut request_line = String::new();
        reader.read_line(&mut request_line)?;
        let mut content_length = 0_usize;
        let mut session = None;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line)?;
            if line == "\r\n" || line.is_empty() {
                break;
            }
            let (name, value) = line.split_once(':').unwrap_or(("", ""));
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => content_length = value.trim().parse()?,
                "mcp-session-id" => session = Some(value.trim().to_owned()),
                _ => {}
            }
        }
        let method = request_line.split_whitespace().next().unwrap_or("");
        if method == "DELETE" {
            let status = if session.as_ref().is_some_and(|id| sessions.remove(id)) {
                "200 OK"
            } else {
                "404 Not Found"
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            )?;
            continue;
        }
        if method != "POST" {
            write!(
                stream,
                "HTTP/1.1 405 Method Not Allowed\r\nallow: POST, DELETE\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            )?;
            continue;
        }
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body)?;
        let initialized = serde_json::from_slice::<Request>(&body)
            .ok()
            .is_some_and(|r| r.method == "initialize");
        let session = match session {
            Some(id) if sessions.contains(&id) => id,
            Some(_) => {
                let message = "unknown or cancelled MCP session";
                write!(
                    stream,
                    "HTTP/1.1 404 Not Found\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{message}",
                    message.len()
                )?;
                continue;
            }
            None if initialized => {
                let id = format!("aster-{next_session}");
                next_session += 1;
                sessions.insert(id.clone());
                id
            }
            None => {
                let message = "initialize required before session requests";
                write!(
                    stream,
                    "HTTP/1.1 400 Bad Request\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{message}",
                    message.len()
                )?;
                continue;
            }
        };
        let response = server.handle(std::str::from_utf8(&body)?);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nmcp-session-id: {session}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}",
            response.len()
        )?;
    }
    Ok(())
}

pub struct Loopback<'a>(pub &'a Server);
impl Transport for Loopback<'_> {
    fn round_trip(&mut self, request: &str) -> Result<String> {
        Ok(self.0.handle(request))
    }
}
