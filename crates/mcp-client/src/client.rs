use agent_core::{AgentError, HttpBackend, ToolDefinition, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;

use crate::protocol::{JsonRpcRequest, JsonRpcResponse};

pub struct McpClient<H: HttpBackend> {
    backend: H,
    server_url: String,
    next_id: std::sync::atomic::AtomicU64,
    extra_headers: Vec<(String, String)>,
    /// MCP session ID returned by the server in the `mcp-session-id` response header.
    session_id: std::sync::Mutex<Option<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerCapabilities {
    #[serde(default)]
    pub tools: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InitializeResult {
    pub capabilities: ServerCapabilities,
    #[allow(dead_code)]
    #[serde(default)]
    pub server_info: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct ToolsListResult {
    tools: Vec<McpToolDef>,
}

#[derive(Debug, Clone, Deserialize)]
struct McpToolDef {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "inputSchema")]
    input_schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct ToolCallResult {
    content: Vec<ToolCallContent>,
    #[serde(default)]
    #[allow(dead_code)]
    is_error: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ToolCallContent {
    Text { text: String },
}

/// Extract JSON from a response that may be plain JSON or SSE format.
/// SSE responses contain lines like `event: message\ndata: {...}\n\n`.
fn extract_json_from_response(response: &str) -> &str {
    let trimmed = response.trim();
    // If it starts with '{', it's already plain JSON
    if trimmed.starts_with('{') {
        return trimmed;
    }
    // Otherwise look for "data: " prefix in SSE format
    for line in trimmed.lines() {
        let line = line.trim();
        if let Some(json) = line.strip_prefix("data: ") {
            return json;
        }
        if let Some(json) = line.strip_prefix("data:") {
            return json.trim();
        }
    }
    // Fallback: return as-is and let JSON parsing surface the error
    trimmed
}

impl<H: HttpBackend> McpClient<H> {
    pub fn new(backend: H, server_url: String, extra_headers: Vec<(String, String)>) -> Self {
        Self {
            backend,
            server_url,
            next_id: std::sync::atomic::AtomicU64::new(1),
            extra_headers,
            session_id: std::sync::Mutex::new(None),
        }
    }

    fn next_id(&self) -> u64 {
        self.next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    async fn send_request(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, AgentError> {
        let request = JsonRpcRequest::new(self.next_id(), method, params);
        let body = serde_json::to_vec(&request).map_err(AgentError::Serialization)?;

        let url = format!("{}/mcp", self.server_url);
        let session_id = self.session_id.lock().unwrap().clone();
        let mut headers: Vec<(&str, &str)> = vec![
            ("content-type", "application/json"),
            ("accept", "application/json, text/event-stream"),
        ];
        if let Some(ref sid) = session_id {
            headers.push(("mcp-session-id", sid.as_str()));
        }
        for (k, v) in &self.extra_headers {
            headers.push((k.as_str(), v.as_str()));
        }

        let response = self.backend.post(&url, &headers, &body).await?;

        // Capture session ID from response headers
        for (k, v) in &response.headers {
            if k.eq_ignore_ascii_case("mcp-session-id") {
                *self.session_id.lock().unwrap() = Some(v.clone());
                break;
            }
        }

        let response_text = String::from_utf8_lossy(&response.body);

        // Extract JSON from response — may be plain JSON or SSE format (data: {...})
        let json_str = extract_json_from_response(&response_text);

        let rpc_response: JsonRpcResponse = serde_json::from_str(json_str).map_err(|e| {
            AgentError::McpError(format!("Invalid JSON-RPC response: {e}: {}", response_text))
        })?;

        if let Some(err) = rpc_response.error {
            return Err(AgentError::McpError(err.to_string()));
        }

        rpc_response
            .result
            .ok_or_else(|| AgentError::McpError("Missing result in JSON-RPC response".to_string()))
    }

    pub async fn initialize(&self) -> Result<ServerCapabilities, AgentError> {
        let params = serde_json::json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {
                "name": "edgeclaw",
                "version": "0.1.0"
            }
        });

        let result = self.send_request("initialize", Some(params)).await?;
        let init: InitializeResult = serde_json::from_value(result)
            .map_err(|e| AgentError::McpError(format!("Failed to parse initialize result: {e}")))?;

        Ok(init.capabilities)
    }

    pub async fn list_tools(&self) -> Result<Vec<ToolDefinition>, AgentError> {
        let result = self.send_request("tools/list", None).await?;
        let tools_result: ToolsListResult = serde_json::from_value(result)
            .map_err(|e| AgentError::McpError(format!("Failed to parse tools/list result: {e}")))?;

        Ok(tools_result
            .tools
            .into_iter()
            .map(|t| ToolDefinition {
                name: t.name,
                description: t.description.unwrap_or_default(),
                input_schema: t
                    .input_schema
                    .unwrap_or(serde_json::json!({"type": "object"})),
            })
            .collect())
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolResult, AgentError> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments,
        });

        let result = self.send_request("tools/call", Some(params)).await?;
        let call_result: ToolCallResult = serde_json::from_value(result)
            .map_err(|e| AgentError::McpError(format!("Failed to parse tools/call result: {e}")))?;

        let content = call_result
            .content
            .into_iter()
            .map(|c| match c {
                ToolCallContent::Text { text } => text,
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(ToolResult {
            tool_use_id: String::new(), // Caller sets this
            content,
            is_error: call_result.is_error,
        })
    }
}

/// Convenience: allow `&McpClient` to act as a `ToolExecutor` when tool names
/// are already resolved (no namespacing). The SkillRegistry layer handles
/// namespacing and delegates to this.
#[cfg(feature = "native")]
#[async_trait]
impl<H: HttpBackend> agent_core::ToolExecutor for McpClient<H> {
    async fn execute(&self, tool_call: &agent_core::ToolCall) -> Result<ToolResult, AgentError> {
        let mut result = self
            .call_tool(&tool_call.name, tool_call.input.clone())
            .await?;
        result.tool_use_id = tool_call.id.clone();
        Ok(result)
    }
}

#[cfg(not(feature = "native"))]
#[async_trait(?Send)]
impl<H: HttpBackend> agent_core::ToolExecutor for McpClient<H> {
    async fn execute(&self, tool_call: &agent_core::ToolCall) -> Result<ToolResult, AgentError> {
        let mut result = self
            .call_tool(&tool_call.name, tool_call.input.clone())
            .await?;
        result.tool_use_id = tool_call.id.clone();
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::HttpBackend;
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    struct MockHttpBackend {
        responses: Mutex<VecDeque<Vec<u8>>>,
        captured_headers: Arc<Mutex<Vec<Vec<(String, String)>>>>,
    }

    impl MockHttpBackend {
        fn new(responses: Vec<&str>) -> Self {
            Self {
                responses: Mutex::new(
                    responses
                        .into_iter()
                        .map(|s| s.as_bytes().to_vec())
                        .collect(),
                ),
                captured_headers: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[cfg_attr(feature = "native", async_trait)]
    #[cfg_attr(not(feature = "native"), async_trait(?Send))]
    impl HttpBackend for MockHttpBackend {
        async fn post(
            &self,
            _url: &str,
            headers: &[(&str, &str)],
            _body: &[u8],
        ) -> Result<agent_core::llm::HttpResponse, AgentError> {
            self.captured_headers.lock().unwrap().push(
                headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            );
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .map(agent_core::llm::HttpResponse::body_only)
                .ok_or_else(|| AgentError::Http("No more mock responses".to_string()))
        }
    }

    #[tokio::test]
    async fn test_initialize() {
        let response = r#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{"tools":{}},"serverInfo":{"name":"test","version":"1.0"}}}"#;
        let client = McpClient::new(
            MockHttpBackend::new(vec![response]),
            "http://localhost:8787".to_string(),
            vec![],
        );
        let caps = client.initialize().await.unwrap();
        assert!(caps.tools.is_some());
    }

    #[tokio::test]
    async fn test_list_tools() {
        let response = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[{"name":"web_search","description":"Search the web","inputSchema":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}}]}}"#;
        let client = McpClient::new(
            MockHttpBackend::new(vec![response]),
            "http://localhost:8787".to_string(),
            vec![],
        );
        let tools = client.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "web_search");
        assert_eq!(tools[0].description, "Search the web");
    }

    #[tokio::test]
    async fn test_call_tool() {
        let response = r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"Result data"}],"is_error":false}}"#;
        let client = McpClient::new(
            MockHttpBackend::new(vec![response]),
            "http://localhost:8787".to_string(),
            vec![],
        );
        let result = client
            .call_tool("web_search", serde_json::json!({"query": "test"}))
            .await
            .unwrap();
        assert_eq!(result.content, "Result data");
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_rpc_error() {
        let response =
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"Invalid Request"}}"#;
        let client = McpClient::new(
            MockHttpBackend::new(vec![response]),
            "http://localhost:8787".to_string(),
            vec![],
        );
        let err = client.list_tools().await.unwrap_err();
        match err {
            AgentError::McpError(msg) => assert!(msg.contains("Invalid Request")),
            other => panic!("Expected McpError, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_extra_headers_forwarded() {
        let response = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;
        let backend = MockHttpBackend::new(vec![response]);
        let captured = backend.captured_headers.clone();
        let client = McpClient {
            backend,
            server_url: "http://localhost:8787".to_string(),
            next_id: std::sync::atomic::AtomicU64::new(1),
            extra_headers: vec![(
                "authorization".to_string(),
                "Bearer sk-test-123".to_string(),
            )],
            session_id: std::sync::Mutex::new(None),
        };
        client.list_tools().await.unwrap();
        let headers = captured.lock().unwrap();
        assert_eq!(headers.len(), 1);
        let req_headers = &headers[0];
        assert!(req_headers
            .iter()
            .any(|(k, v)| k == "authorization" && v == "Bearer sk-test-123"));
    }
}
