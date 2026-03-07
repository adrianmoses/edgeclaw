use std::cell::Cell;

use agent_core::{Agent, AgentContext, HttpBackend, LlmClient, LlmConfig, Message};
use serde::Deserialize;
use worker::*;

// --- HttpBackend implementation for worker::Fetch ---

struct WorkerFetchBackend;

#[async_trait::async_trait(?Send)]
impl HttpBackend for WorkerFetchBackend {
    async fn post(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Result<Vec<u8>, agent_core::AgentError> {
        let mut init = RequestInit::new();
        init.method = Method::Post;

        let body_str = String::from_utf8(body.to_vec())
            .map_err(|e| agent_core::AgentError::Http(e.to_string()))?;
        init.body = Some(wasm_bindgen::JsValue::from_str(&body_str));

        for (key, value) in headers {
            init.headers
                .set(key, value)
                .map_err(|e| agent_core::AgentError::Http(format!("{e:?}")))?;
        }

        let request = Request::new_with_init(url, &init)
            .map_err(|e| agent_core::AgentError::Http(format!("{e:?}")))?;

        let mut response = Fetch::Request(request)
            .send()
            .await
            .map_err(|e| agent_core::AgentError::Http(format!("{e:?}")))?;

        let bytes = response
            .bytes()
            .await
            .map_err(|e| agent_core::AgentError::Http(format!("{e:?}")))?;

        Ok(bytes)
    }
}

// --- Dispatcher Worker ---

#[event(fetch)]
async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let url = req.url()?;
    let path = req.path();

    // POST /orchestrate — multi-agent fan-out (M1.7)
    if req.method() == Method::Post && path.as_str() == "/orchestrate" {
        return handle_orchestrate(req, &env).await;
    }

    // Extract user ID from X-User-Id header or query param for local testing
    let user_id = req.headers().get("X-User-Id").ok().flatten().or_else(|| {
        url.query_pairs()
            .find(|(k, _)| k == "user_id")
            .map(|(_, v)| v.to_string())
    });

    let user_id = match user_id {
        Some(id) if !id.is_empty() => id,
        _ => {
            return Response::error(
                "Missing user identity (X-User-Id header or user_id query param)",
                400,
            )
        }
    };

    // Get the AgentDO namespace and derive a deterministic ID
    let namespace = env.durable_object("AGENT_DO")?;
    let stub = namespace
        .id_from_name(&format!("agent:{user_id}"))?
        .get_stub()?;

    // Forward the request to the DO
    stub.fetch_with_request(req).await
}

// --- Multi-Agent Orchestration (M1.7) ---

#[derive(Deserialize)]
struct OrchestrateRequest {
    task: String,
    agents: Vec<String>,
}

async fn handle_orchestrate(mut req: Request, env: &Env) -> Result<Response> {
    let body: OrchestrateRequest = req.json().await?;
    let namespace = env.durable_object("AGENT_DO")?;

    let mut results = serde_json::Map::new();

    // Sequential fan-out: send the task to each named agent
    for agent_name in &body.agents {
        let stub = namespace
            .id_from_name(&format!("agent:{agent_name}"))?
            .get_stub()?;

        let message_body = serde_json::json!({ "message": body.task });

        let mut init = RequestInit::new();
        init.method = Method::Post;
        init.body = Some(wasm_bindgen::JsValue::from_str(
            &serde_json::to_string(&message_body).map_err(|e| Error::RustError(e.to_string()))?,
        ));
        init.headers
            .set("content-type", "application/json")
            .map_err(|e| Error::RustError(format!("{e:?}")))?;

        let inner_req = Request::new_with_init("https://fake-host/message", &init)?;
        let mut resp = stub.fetch_with_request(inner_req).await?;
        let value: serde_json::Value = resp.json().await?;
        results.insert(agent_name.clone(), value);
    }

    Response::from_json(&results)
}

// --- AgentDO Durable Object ---

#[durable_object]
pub struct AgentDo {
    state: State,
    env: Env,
    initialized: Cell<bool>,
}

impl AgentDo {
    fn ensure_schema(&self) {
        if self.initialized.get() {
            return;
        }
        let sql = self.state.storage().sql();
        let none: Option<Vec<SqlStorageValue>> = None;
        let _ = sql.exec(
            "CREATE TABLE IF NOT EXISTS messages (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                role       TEXT    NOT NULL,
                content    TEXT    NOT NULL,
                created_at INTEGER NOT NULL
            )",
            none.clone(),
        );
        let _ = sql.exec(
            "CREATE TABLE IF NOT EXISTS skills (
                name       TEXT PRIMARY KEY,
                url        TEXT NOT NULL,
                tools      TEXT NOT NULL,
                added_at   INTEGER NOT NULL
            )",
            none.clone(),
        );
        let _ = sql.exec(
            "CREATE TABLE IF NOT EXISTS pending_approvals (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                tool_call  TEXT    NOT NULL,
                created_at INTEGER NOT NULL
            )",
            none.clone(),
        );
        let _ = sql.exec(
            "CREATE TABLE IF NOT EXISTS prefs (
                key        TEXT PRIMARY KEY,
                value      TEXT NOT NULL
            )",
            none,
        );
        self.initialized.set(true);
    }

    fn build_llm_config(&self) -> LlmConfig {
        let api_key = self
            .env
            .secret("ANTHROPIC_API_KEY")
            .map(|s| s.to_string())
            .unwrap_or_default();

        let model = self
            .env
            .var("CLAUDE_MODEL")
            .map(|v| v.to_string())
            .unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string());

        let base_url = self
            .env
            .var("ANTHROPIC_BASE_URL")
            .map(|v| v.to_string())
            .unwrap_or_else(|_| "https://api.anthropic.com".to_string());

        LlmConfig {
            api_key,
            model,
            base_url,
            max_tokens: 4096,
        }
    }

    fn load_messages(&self, limit: u32) -> Vec<Message> {
        let sql = self.state.storage().sql();
        let cursor = match sql.exec(
            "SELECT role, content, created_at FROM messages ORDER BY id DESC LIMIT ?",
            Some(vec![SqlStorageValue::Integer(limit as i64)]),
        ) {
            Ok(c) => c,
            Err(_) => return vec![],
        };

        let mut messages: Vec<Message> = cursor
            .raw()
            .filter_map(|row| {
                let values = row.ok()?;
                let role_str = match &values[0] {
                    SqlStorageValue::String(s) => s.clone(),
                    _ => return None,
                };
                let content_json = match &values[1] {
                    SqlStorageValue::String(s) => s.clone(),
                    _ => return None,
                };
                let created_at = match &values[2] {
                    SqlStorageValue::Integer(i) => *i,
                    SqlStorageValue::Float(f) => *f as i64,
                    _ => return None,
                };

                let role = match role_str.as_str() {
                    "user" => agent_core::Role::User,
                    "assistant" => agent_core::Role::Assistant,
                    _ => return None,
                };
                let content: Vec<agent_core::ContentBlock> =
                    serde_json::from_str(&content_json).ok()?;

                Some(Message {
                    role,
                    content,
                    created_at,
                })
            })
            .collect();

        messages.reverse(); // DESC -> chronological order
        messages
    }

    fn persist_messages(&self, messages: &[Message]) {
        let sql = self.state.storage().sql();
        let now = js_sys::Date::now() as i64;
        for msg in messages {
            let role = match msg.role {
                agent_core::Role::User => "user",
                agent_core::Role::Assistant => "assistant",
            };
            let content_json = serde_json::to_string(&msg.content).unwrap_or_default();
            let bindings: Vec<SqlStorageValue> = vec![
                role.into(),
                content_json.into(),
                SqlStorageValue::Integer(now),
            ];
            let _ = sql.exec(
                "INSERT INTO messages (role, content, created_at) VALUES (?, ?, ?)",
                Some(bindings),
            );
        }
    }

    fn load_system_prompt(&self) -> String {
        let sql = self.state.storage().sql();
        let none: Option<Vec<SqlStorageValue>> = None;
        let cursor = match sql.exec("SELECT value FROM prefs WHERE key = 'system_prompt'", none) {
            Ok(c) => c,
            Err(_) => return "You are a helpful AI assistant.".to_string(),
        };

        cursor
            .raw()
            .filter_map(|row| {
                let values = row.ok()?;
                match &values[0] {
                    SqlStorageValue::String(s) => Some(s.clone()),
                    _ => None,
                }
            })
            .next()
            .unwrap_or_else(|| "You are a helpful AI assistant.".to_string())
    }

    /// Shared agent turn logic used by both HTTP and WebSocket handlers.
    async fn run_agent_turn(&self, user_message: &str) -> Result<serde_json::Value> {
        let config = self.build_llm_config();
        let llm = LlmClient::new(config, WorkerFetchBackend);
        let agent = Agent::new(llm);

        let messages = self.load_messages(50);
        let system_prompt = self.load_system_prompt();

        let ctx = AgentContext {
            system_prompt,
            messages,
            tools: vec![], // Phase 1: no tools
        };

        let result = agent
            .run(ctx, user_message)
            .await
            .map_err(|e| Error::RustError(e.to_string()))?;

        self.persist_messages(&result.new_messages);

        Ok(serde_json::json!({
            "answer": result.answer,
            "pending_tool_calls": result.pending_tool_calls,
        }))
    }

    async fn handle_message(&self, mut req: Request) -> Result<Response> {
        #[derive(Deserialize)]
        struct MessageRequest {
            message: String,
        }

        let body: MessageRequest = req.json().await?;
        let response_body = self.run_agent_turn(&body.message).await?;
        Response::from_json(&response_body)
    }

    fn handle_history(&self) -> Result<Response> {
        let messages = self.load_messages(50);
        Response::from_json(&messages)
    }

    fn handle_websocket_upgrade(&self) -> Result<Response> {
        let pair = WebSocketPair::new()?;
        self.state.accept_web_socket(&pair.server);
        Response::from_websocket(pair.client)
    }
}

impl DurableObject for AgentDo {
    fn new(state: State, env: Env) -> Self {
        Self {
            state,
            env,
            initialized: Cell::new(false),
        }
    }

    async fn fetch(&self, req: Request) -> Result<Response> {
        self.ensure_schema();

        let path = req.path();
        let method = req.method();

        match (method, path.as_str()) {
            (Method::Post, "/message") => self.handle_message(req).await,
            (Method::Get, "/history") => self.handle_history(),
            (Method::Get, "/") => {
                // Check for WebSocket upgrade
                let upgrade = req.headers().get("Upgrade").ok().flatten();
                if upgrade.as_deref() == Some("websocket") {
                    self.handle_websocket_upgrade()
                } else {
                    Response::error("Expected WebSocket upgrade", 426)
                }
            }
            _ => Response::error("Not Found", 404),
        }
    }

    async fn websocket_message(
        &self,
        ws: WebSocket,
        message: WebSocketIncomingMessage,
    ) -> Result<()> {
        self.ensure_schema();

        let text = match message {
            WebSocketIncomingMessage::String(s) => s,
            WebSocketIncomingMessage::Binary(_) => {
                ws.send_with_str(r#"{"error":"binary messages not supported"}"#)?;
                return Ok(());
            }
        };

        // Parse as JSON: { "message": "..." }
        #[derive(Deserialize)]
        struct WsMessage {
            message: String,
        }

        let parsed: WsMessage = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                let err = serde_json::json!({ "error": format!("invalid JSON: {e}") });
                ws.send_with_str(serde_json::to_string(&err).unwrap_or_default())?;
                return Ok(());
            }
        };

        match self.run_agent_turn(&parsed.message).await {
            Ok(result) => {
                ws.send_with_str(serde_json::to_string(&result).unwrap_or_default())?;
            }
            Err(e) => {
                let err = serde_json::json!({ "error": e.to_string() });
                ws.send_with_str(serde_json::to_string(&err).unwrap_or_default())?;
            }
        }

        Ok(())
    }

    async fn websocket_close(
        &self,
        _ws: WebSocket,
        _code: usize,
        _reason: String,
        _was_clean: bool,
    ) -> Result<()> {
        Ok(())
    }

    async fn websocket_error(&self, _ws: WebSocket, _error: Error) -> Result<()> {
        Ok(())
    }
}
