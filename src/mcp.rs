use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

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
                Err(e) => self.err(r.id, -32601, &e.to_string()),
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
pub struct Loopback<'a>(pub &'a Server);
impl Transport for Loopback<'_> {
    fn round_trip(&mut self, request: &str) -> Result<String> {
        Ok(self.0.handle(request))
    }
}
