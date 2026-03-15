use std::cell::Cell;

use mcp_server_util::{
    initialize_result, tool_call_result, tools_list_result, JsonRpcRequest, JsonRpcResponse,
    ToolDef,
};
use serde::Deserialize;
use worker::*;

fn tool_definitions() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "memory_store",
            description: "Store a key-value memory with optional tags",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Unique key for the memory" },
                    "value": { "type": "string", "description": "Content to store" },
                    "tags": { "type": "string", "description": "Comma-separated tags (optional)" }
                },
                "required": ["key", "value"]
            }),
        },
        ToolDef {
            name: "memory_retrieve",
            description: "Retrieve a memory by key",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Key to look up" }
                },
                "required": ["key"]
            }),
        },
        ToolDef {
            name: "memory_list",
            description: "List memories, optionally filtered by tag",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "tag": { "type": "string", "description": "Filter by tag (optional)" }
                }
            }),
        },
        ToolDef {
            name: "memory_delete",
            description: "Delete a memory by key",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Key to delete" }
                },
                "required": ["key"]
            }),
        },
    ]
}

// --- Dispatcher ---

#[event(fetch)]
async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    if req.method() != Method::Post || req.path().as_str() != "/mcp" {
        return Response::error("POST /mcp only", 404);
    }

    let user_id = req
        .headers()
        .get("X-User-Id")
        .ok()
        .flatten()
        .unwrap_or_else(|| "default".to_string());

    let namespace = env.durable_object("MEMORY_DO")?;
    let stub = namespace
        .id_from_name(&format!("memory:{user_id}"))?
        .get_stub()?;

    stub.fetch_with_request(req).await
}

// --- MemoryDo Durable Object ---

#[durable_object]
pub struct MemoryDo {
    state: State,
    #[allow(dead_code)]
    env: Env,
    initialized: Cell<bool>,
}

impl MemoryDo {
    fn ensure_schema(&self) {
        if self.initialized.get() {
            return;
        }
        let sql = self.state.storage().sql();
        let none: Option<Vec<SqlStorageValue>> = None;
        match sql.exec(
            "CREATE TABLE IF NOT EXISTS memories (
                key        TEXT PRIMARY KEY,
                value      TEXT NOT NULL,
                tags       TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )",
            none,
        ) {
            Ok(_) => self.initialized.set(true),
            Err(e) => {
                console_error!("Failed to initialize memories schema: {:?}", e);
            }
        }
    }

    fn handle_tool_call(&self, name: &str, params: &serde_json::Value) -> serde_json::Value {
        match name {
            "memory_store" => self.tool_store(params),
            "memory_retrieve" => self.tool_retrieve(params),
            "memory_list" => self.tool_list(params),
            "memory_delete" => self.tool_delete(params),
            _ => tool_call_result(&format!("Unknown tool: {name}"), true),
        }
    }

    fn tool_store(&self, params: &serde_json::Value) -> serde_json::Value {
        let key = params["key"].as_str().unwrap_or("");
        let value = params["value"].as_str().unwrap_or("");
        let tags = params["tags"].as_str().unwrap_or("");
        let now = js_sys::Date::now() as i64;

        let sql = self.state.storage().sql();
        let bindings: Vec<SqlStorageValue> = vec![
            key.into(),
            value.into(),
            tags.into(),
            SqlStorageValue::Integer(now),
            SqlStorageValue::Integer(now),
        ];
        match sql.exec(
            "INSERT OR REPLACE INTO memories (key, value, tags, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
            Some(bindings),
        ) {
            Ok(_) => tool_call_result(&format!("Stored memory with key '{key}'"), false),
            Err(e) => tool_call_result(&format!("Failed to store: {e:?}"), true),
        }
    }

    fn tool_retrieve(&self, params: &serde_json::Value) -> serde_json::Value {
        let key = params["key"].as_str().unwrap_or("");
        let sql = self.state.storage().sql();

        let cursor = match sql.exec(
            "SELECT value, tags FROM memories WHERE key = ?",
            Some(vec![key.into()]),
        ) {
            Ok(c) => c,
            Err(e) => return tool_call_result(&format!("Query failed: {e:?}"), true),
        };

        let row = cursor.raw().filter_map(|r| r.ok()).next();
        match row {
            Some(values) => {
                let value = match &values[0] {
                    SqlStorageValue::String(s) => s.clone(),
                    _ => String::new(),
                };
                let tags = match &values[1] {
                    SqlStorageValue::String(s) => s.clone(),
                    _ => String::new(),
                };
                let result = serde_json::json!({"key": key, "value": value, "tags": tags});
                tool_call_result(&result.to_string(), false)
            }
            None => tool_call_result(&format!("No memory found with key '{key}'"), true),
        }
    }

    fn tool_list(&self, params: &serde_json::Value) -> serde_json::Value {
        let tag = params.get("tag").and_then(|v| v.as_str());
        let sql = self.state.storage().sql();

        let cursor = if let Some(tag) = tag {
            let pattern = format!("%{tag}%");
            sql.exec(
                "SELECT key, value, tags FROM memories WHERE tags LIKE ?",
                Some(vec![pattern.into()]),
            )
        } else {
            let none: Option<Vec<SqlStorageValue>> = None;
            sql.exec("SELECT key, value, tags FROM memories", none)
        };

        let cursor = match cursor {
            Ok(c) => c,
            Err(e) => return tool_call_result(&format!("Query failed: {e:?}"), true),
        };

        let entries: Vec<serde_json::Value> = cursor
            .raw()
            .filter_map(|r| {
                let values = r.ok()?;
                let key = match &values[0] {
                    SqlStorageValue::String(s) => s.clone(),
                    _ => return None,
                };
                let value = match &values[1] {
                    SqlStorageValue::String(s) => s.clone(),
                    _ => return None,
                };
                let tags = match &values[2] {
                    SqlStorageValue::String(s) => s.clone(),
                    _ => String::new(),
                };
                Some(serde_json::json!({"key": key, "value": value, "tags": tags}))
            })
            .collect();

        tool_call_result(&serde_json::to_string(&entries).unwrap_or_default(), false)
    }

    fn tool_delete(&self, params: &serde_json::Value) -> serde_json::Value {
        let key = params["key"].as_str().unwrap_or("");
        let sql = self.state.storage().sql();

        match sql.exec("DELETE FROM memories WHERE key = ?", Some(vec![key.into()])) {
            Ok(_) => tool_call_result(&format!("Deleted memory with key '{key}'"), false),
            Err(e) => tool_call_result(&format!("Failed to delete: {e:?}"), true),
        }
    }
}

#[derive(Deserialize)]
struct ToolCallParams {
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

impl DurableObject for MemoryDo {
    fn new(state: State, env: Env) -> Self {
        Self {
            state,
            env,
            initialized: Cell::new(false),
        }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        self.ensure_schema();

        let body: JsonRpcRequest = req
            .json()
            .await
            .map_err(|e| Error::RustError(format!("Invalid JSON-RPC: {e:?}")))?;

        let tools = tool_definitions();
        let response = match body.method.as_str() {
            "initialize" => {
                JsonRpcResponse::success(body.id, initialize_result("skill-memory", &tools))
            }
            "tools/list" => JsonRpcResponse::success(body.id, tools_list_result(&tools)),
            "tools/call" => {
                let params: ToolCallParams = body
                    .params
                    .as_ref()
                    .and_then(|p| serde_json::from_value(p.clone()).ok())
                    .ok_or_else(|| Error::RustError("Missing tools/call params".to_string()))?;

                let result = self.handle_tool_call(&params.name, &params.arguments);
                JsonRpcResponse::success(body.id, result)
            }
            _ => JsonRpcResponse::method_not_found(body.id, &body.method),
        };

        Response::from_json(&response)
    }
}
