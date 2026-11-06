//! Model Context Protocol server that lets AI agents drive the running editor.
//!
//! Speaks the Streamable HTTP transport with plain JSON responses on
//! `http://127.0.0.1:<port>/mcp`. Only loopback connections are accepted, and requests
//! carrying a non-local `Origin` header are refused to block DNS-rebinding attacks.
//!
//! Each `tools/call` runs on this server's thread through the handler given to [`start`],
//! which works on the editor state the window shares, so agent edits go through the same
//! undo history as edits made by hand. A panicking tool becomes an error result instead of
//! stopping the server.

use std::io::{self, Read};
use std::panic::{self, AssertUnwindSafe};
use std::thread;

use base64::Engine;
use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server};

/// Protocol revisions this server understands, newest first.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
/// Largest request body accepted.
const MAX_BODY_BYTES: u64 = 16 * 1024 * 1024;

const INSTRUCTIONS: &str = "Mallet is a Hammer-style editor for Source engine VMF maps. \
Use get_status first to see which map is open. Edits made through these tools are undoable \
in the editor. Use screenshot to see the camera, top, front and side viewports.";

/// One piece of a tool result.
pub enum Content {
    Text(String),
    Png(Vec<u8>),
}

/// The answer to a tool call.
pub struct ToolResult {
    pub content: Vec<Content>,
    pub is_error: bool,
}

impl ToolResult {
    /// A successful text result.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![Content::Text(text.into())],
            is_error: false,
        }
    }

    /// A successful result holding pretty-printed JSON.
    pub fn json(value: &Value) -> Self {
        Self::text(serde_json::to_string_pretty(value).unwrap_or_default())
    }

    /// A failed tool call with a message the agent can act on.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![Content::Text(message.into())],
            is_error: true,
        }
    }

    /// A PNG image.
    pub fn png(bytes: Vec<u8>) -> Self {
        Self {
            content: vec![Content::Png(bytes)],
            is_error: false,
        }
    }

    fn to_json(&self) -> Value {
        let content: Vec<Value> = self
            .content
            .iter()
            .map(|item| match item {
                Content::Text(text) => json!({ "type": "text", "text": text }),
                Content::Png(bytes) => json!({
                    "type": "image",
                    "mimeType": "image/png",
                    "data": base64::engine::general_purpose::STANDARD.encode(bytes),
                }),
            })
            .collect();
        json!({ "content": content, "isError": self.is_error })
    }
}

/// Runs a tool by name with its JSON arguments.
pub trait ToolHandler: Fn(&str, &Value) -> ToolResult + Send + 'static {}

impl<F: Fn(&str, &Value) -> ToolResult + Send + 'static> ToolHandler for F {}

/// Starts the server on a background thread and returns its endpoint URL.
///
/// `tools` is the JSON array returned from `tools/list`; `handler` runs each tool call.
pub fn start(port: u16, tools: Value, handler: impl ToolHandler) -> io::Result<String> {
    let server = Server::http(("127.0.0.1", port)).map_err(io::Error::other)?;
    thread::Builder::new().name("mcp".to_owned()).spawn(move || {
        for request in server.incoming_requests() {
            handle(request, &handler, &tools);
        }
    })?;
    Ok(format!("http://127.0.0.1:{port}/mcp"))
}

fn handle(mut request: Request, handler: &dyn ToolHandler, tools: &Value) {
    let path = request.url().split('?').next().unwrap_or("");
    let response = if path != "/mcp" {
        Response::from_string("not found").with_status_code(404)
    } else if !origin_allowed(&request) {
        Response::from_string("origin not allowed").with_status_code(403)
    } else if *request.method() != Method::Post {
        Response::from_string("this server only accepts POST")
            .with_status_code(405)
            .with_header(header("Allow", "POST"))
    } else {
        let mut body = String::new();
        let read = request
            .as_reader()
            .take(MAX_BODY_BYTES)
            .read_to_string(&mut body);
        match read {
            Ok(_) => answer(&body, handler, tools),
            Err(error) => Response::from_string(error.to_string()).with_status_code(400),
        }
    };
    let _ = request.respond(response);
}

fn answer(body: &str, handler: &dyn ToolHandler, tools: &Value) -> Response<io::Cursor<Vec<u8>>> {
    let message: Value = match serde_json::from_str(body) {
        Ok(message) => message,
        Err(error) => return rpc_response(Value::Null, Err((-32700, format!("parse error: {error}")))),
    };
    let Some(id) = message.get("id").cloned() else {
        return Response::from_data(Vec::new()).with_status_code(202);
    };
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    rpc_response(id, dispatch(method, &params, handler, tools))
}

fn dispatch(method: &str, params: &Value, handler: &dyn ToolHandler, tools: &Value) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => {
            let requested = params.get("protocolVersion").and_then(Value::as_str);
            let version = requested
                .filter(|version| PROTOCOL_VERSIONS.contains(version))
                .unwrap_or(PROTOCOL_VERSIONS[0]);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "mallet", "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools })),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or((-32602, "missing tool name".to_owned()))?;
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let result = panic::catch_unwind(AssertUnwindSafe(|| handler(name, &arguments)))
                .unwrap_or_else(|_| ToolResult::error(format!("the \"{name}\" tool crashed")));
            Ok(result.to_json())
        }
        _ => Err((-32601, format!("method not found: {method}"))),
    }
}

fn rpc_response(id: Value, result: Result<Value, (i64, String)>) -> Response<io::Cursor<Vec<u8>>> {
    let body = match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        }
    };
    Response::from_data(body.to_string().into_bytes()).with_header(header("Content-Type", "application/json"))
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header is valid")
}

/// Allows requests without an `Origin` header and requests from loopback origins only.
fn origin_allowed(request: &Request) -> bool {
    let Some(origin) = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Origin"))
    else {
        return true;
    };
    let origin = origin.value.as_str();
    let host = origin.split_once("://").map_or(origin, |(_, rest)| rest);
    let host = if host.starts_with('[') {
        host.split_inclusive(']').next().unwrap_or("")
    } else {
        host.split([':', '/']).next().unwrap_or("")
    };
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}
