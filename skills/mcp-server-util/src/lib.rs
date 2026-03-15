use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    #[allow(dead_code)]
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

impl JsonRpcResponse {
    pub fn success(id: u64, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: u64, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }

    pub fn method_not_found(id: u64, method: &str) -> Self {
        Self::error(id, -32601, format!("Method not found: {method}"))
    }
}

/// Standard MCP initialize response.
pub fn initialize_result(server_name: &str, tools: &[ToolDef]) -> serde_json::Value {
    let has_tools = !tools.is_empty();
    serde_json::json!({
        "protocolVersion": "2025-03-26",
        "capabilities": {
            "tools": if has_tools { serde_json::json!({}) } else { serde_json::Value::Null }
        },
        "serverInfo": {
            "name": server_name,
            "version": "0.1.0"
        }
    })
}

/// Standard MCP tools/list response.
pub fn tools_list_result(tools: &[ToolDef]) -> serde_json::Value {
    let tools_json: Vec<serde_json::Value> = tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema
            })
        })
        .collect();
    serde_json::json!({ "tools": tools_json })
}

/// Wrap tool call output into standard MCP response format.
pub fn tool_call_result(text: &str, is_error: bool) -> serde_json::Value {
    serde_json::json!({
        "content": [{"type": "text", "text": text}],
        "is_error": is_error
    })
}

/// Definition of a tool exposed by an MCP server.
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: serde_json::Value,
}
