# EdgeClaw — Prototype Specification

> A stateless, edge-native, WASM-isolated personal AI agent with MCP-native skill routing.

---

## Overview

EdgeClaw is a personal AI agent runtime built on three core principles derived from the lessons of OpenClaw and NanoClaw:

1. **Isolation by architecture, not policy** — WASM sandboxing is stronger than container-level security because there is no syscall surface and no kernel exposure. A compromised skill cannot touch the host or any other skill.
2. **Stateless and edge-native** — the agent core runs as a WASM module on Cloudflare Workers or Fermyon Spin. There is no persistent server to maintain, patch, or compromise.
3. **Skills as first-class citizens** — tools are not hardcoded features. They are MCP-compatible modules discovered and invoked at runtime, each isolated from the others.

---

## Repository Structure

```
edgeclaw/
├── crates/
│   ├── agent-core/          # Phase 1 — ReAct loop, LLM client, state machine
│   ├── mcp-client/          # Phase 2 — MCP protocol client (WASM-compatible)
│   ├── skill-registry/      # Phase 2 — runtime skill discovery & dispatch
│   └── edgeclaw-runtime/    # Entrypoint — wires everything together
├── skills/                  # Phase 2 — bundled reference skills
│   ├── web-search/
│   ├── memory/
│   └── http-fetch/
├── deploy/
│   ├── cloudflare/          # wrangler.toml, worker entrypoint
│   └── spin/                # spin.toml, Fermyon entrypoint
├── tests/
│   ├── integration/
│   └── fixtures/
└── docs/
    └── architecture.md
```

---

## Phase 1 — Rust Agent Core

### Goal

A minimal, auditable, WASM-compilable agent loop that can conduct multi-turn LLM conversations, execute tool calls, and manage conversation state. No skills, no MCP yet — just the core reasoning engine proven to work in a WASM target.

### Deliverables

- `crates/agent-core` compiles cleanly to `wasm32-wasip1` and `wasm32-unknown-unknown`
- A working ReAct loop against the Anthropic API (via `wasm-compatible` HTTP)
- Conversation state serialized to/from JSON (for stateless edge deployments)
- A local CLI harness for development and testing
- Integration test suite with fixture-based LLM responses

---

### 1.1 — Data Model

```rust
// crates/agent-core/src/types.rs

/// A single turn in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: Value },
    ToolResult { tool_use_id: String, content: String },
}

/// Full serializable agent state — passed in and out of each edge invocation
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentState {
    pub conversation: Vec<Message>,
    pub system_prompt: String,
    pub tool_results_pending: Vec<ToolResult>,
}

/// A tool definition exposed to the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value, // JSON Schema
}

/// A request to invoke a tool, parsed from the LLM response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// The result of a tool invocation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
}
```

---

### 1.2 — LLM Client

The LLM client must be WASM-compatible. No `tokio` features that don't compile to WASM. Use `wasm-bindgen-futures` for browser targets or `wasi-http` for WASI targets.

```rust
// crates/agent-core/src/llm.rs

#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub api_key: String,
    pub model: String,        // e.g. "claude-opus-4-6"
    pub max_tokens: u32,
    pub base_url: String,     // overridable for testing
}

pub struct LlmClient {
    config: LlmConfig,
    http: HttpClient,         // abstracted — different impls for native vs WASM
}

impl LlmClient {
    /// Send a conversation turn and return the raw response
    pub async fn complete(
        &self,
        state: &AgentState,
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse, LlmError>;
}

/// Parsed response from the LLM
pub struct LlmResponse {
    pub stop_reason: StopReason,
    pub content: Vec<ContentBlock>,
}

pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
}
```

**HTTP abstraction** — this is the key to WASM portability:

```rust
// crates/agent-core/src/http.rs

/// Trait over HTTP — implemented differently per target
#[async_trait(?Send)]
pub trait HttpBackend {
    async fn post(&self, url: &str, headers: &Headers, body: &[u8])
        -> Result<Response, HttpError>;
}

// Native (dev/test): reqwest
// Cloudflare Workers: worker::Fetch
// WASI/Spin: wasi-http outbound-handler
```

---

### 1.3 — ReAct Agent Loop

The core loop follows the standard Reason → Act → Observe cycle:

```rust
// crates/agent-core/src/agent.rs

pub struct Agent {
    llm: LlmClient,
    tools: Vec<ToolDefinition>,
    config: AgentConfig,
}

pub struct AgentConfig {
    pub max_iterations: u8,   // prevent infinite loops, default: 10
    pub system_prompt: String,
}

impl Agent {
    /// Run one full agent turn, returning updated state and final response.
    /// State is passed in and out — the agent itself is stateless.
    pub async fn run(
        &self,
        state: AgentState,
        user_message: &str,
        tool_executor: &dyn ToolExecutor,
    ) -> Result<AgentRunResult, AgentError> {
        let mut state = state;
        state.conversation.push(user_message_turn(user_message));

        for iteration in 0..self.config.max_iterations {
            let response = self.llm.complete(&state, &self.tools).await?;

            match response.stop_reason {

                // LLM is done — return the final answer
                StopReason::EndTurn => {
                    let text = extract_text(&response.content);
                    state.conversation.push(assistant_turn(response.content));
                    return Ok(AgentRunResult { state, answer: text });
                }

                // LLM wants to call tools — execute them all, then loop
                StopReason::ToolUse => {
                    let tool_calls = extract_tool_calls(&response.content);
                    state.conversation.push(assistant_turn(response.content));

                    // Execute tool calls concurrently where possible
                    let results = execute_tools(tool_calls, tool_executor).await;

                    // Append tool results as a user turn
                    state.conversation.push(tool_results_turn(results));
                }

                StopReason::MaxTokens => return Err(AgentError::MaxTokensReached),
                StopReason::StopSequence => break,
            }
        }

        Err(AgentError::MaxIterationsReached)
    }
}

/// Result of a full agent run
pub struct AgentRunResult {
    pub state: AgentState,    // caller persists this for next turn
    pub answer: String,
}

/// Abstraction over tool execution — Phase 1 uses a no-op or fixture impl
#[async_trait(?Send)]
pub trait ToolExecutor {
    async fn execute(&self, call: &ToolCall) -> ToolResult;
}
```

---

### 1.4 — Edge Entrypoint (Cloudflare Workers)

```rust
// crates/edgeclaw-runtime/src/lib.rs  (Cloudflare Workers target)

use worker::*;

#[event(fetch)]
pub async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let body: IncomingMessage = req.json().await?;

    // Load state from KV (keyed by session/user ID)
    let kv = env.kv("EDGECLAW_STATE")?;
    let state: AgentState = load_state(&kv, &body.session_id).await?;

    // Build agent — tools empty in Phase 1
    let agent = Agent::new(build_llm_client(&env)?, vec![], default_config());
    let executor = NoopExecutor; // replaced in Phase 2

    // Run the agent loop
    let result = agent.run(state, &body.message, &executor).await?;

    // Persist updated state back to KV
    save_state(&kv, &body.session_id, &result.state).await?;

    Response::from_json(&OutgoingMessage { reply: result.answer })
}
```

---

### 1.5 — Phase 1 Milestones

| Milestone | Description | Done When |
|---|---|---|
| M1.1 | `agent-core` compiles to `wasm32-wasip1` with zero errors | `cargo build --target wasm32-wasip1` passes |
| M1.2 | LLM client makes real Anthropic API calls from native target | Single-turn completion test passes |
| M1.3 | ReAct loop handles multi-turn conversation with mock tools | Fixture-based integration tests pass |
| M1.4 | State round-trips cleanly through JSON serialization | Serde round-trip tests pass |
| M1.5 | Cloudflare Worker deploys and responds to a real message | `wrangler dev` end-to-end smoke test passes |
| M1.6 | CLI harness for local development | `cargo run -- --message "hello"` works |

---

### 1.6 — Phase 1 Dependencies

```toml
# Cargo.toml (agent-core)
[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
async-trait = "0.1"
thiserror = "1"

# HTTP — feature-flagged per target
[target.'cfg(target_arch = "wasm32")'.dependencies]
wasm-bindgen = "0.2"
wasm-bindgen-futures = "0.4"
js-sys = "0.3"
web-sys = { version = "0.3", features = ["Request", "Response", "Headers"] }

[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
reqwest = { version = "0.12", features = ["json"] }
tokio = { version = "1", features = ["rt", "macros"] }

[dev-dependencies]
wiremock = "0.6"   # HTTP mocking for LLM fixture tests
```

---

## Phase 2 — Skills & MCP Support

### Goal

Replace the `NoopExecutor` from Phase 1 with a fully functional `SkillRegistry` that discovers, routes, and invokes tools via the MCP protocol. Each skill is either a remote MCP server (HTTP/SSE) or a local WASM module. Skills are isolated from each other — a buggy or malicious skill cannot affect other skills or the agent core.

### Deliverables

- `crates/mcp-client` — MCP protocol client compilable to WASM
- `crates/skill-registry` — runtime skill registration, discovery, and dispatch
- Reference skills: `web-search`, `memory`, `http-fetch`
- Dynamic skill addition — user provides an MCP URL, tools are discovered automatically
- Skill sandboxing guarantees documented and tested

---

### 2.1 — MCP Client

The MCP client implements the [Model Context Protocol](https://modelcontextprotocol.io) spec over HTTP+SSE, fully compatible with WASM targets.

```rust
// crates/mcp-client/src/lib.rs

/// A connection to a single MCP server
pub struct McpClient {
    base_url: String,
    http: Box<dyn HttpBackend>,
    capabilities: ServerCapabilities,
}

impl McpClient {
    /// Connect to an MCP server and fetch its capabilities
    pub async fn connect(url: &str, http: impl HttpBackend) -> Result<Self, McpError>;

    /// List all tools exposed by this server
    pub async fn list_tools(&self) -> Result<Vec<ToolDefinition>, McpError>;

    /// Invoke a tool by name with JSON arguments
    pub async fn call_tool(
        &self,
        name: &str,
        args: Value,
    ) -> Result<ToolCallResult, McpError>;
}

pub struct ServerCapabilities {
    pub server_name: String,
    pub server_version: String,
    pub tools: bool,
    pub resources: bool,
    pub prompts: bool,
}

pub struct ToolCallResult {
    pub content: Vec<McpContent>,
    pub is_error: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpContent {
    Text { text: String },
    Image { data: String, mime_type: String },
    Resource { uri: String, text: Option<String> },
}
```

---

### 2.2 — Skill Registry

```rust
// crates/skill-registry/src/lib.rs

/// A registered skill — wraps an MCP client with metadata
pub struct Skill {
    pub name: String,
    pub description: String,
    pub source: SkillSource,
    client: McpClient,
}

pub enum SkillSource {
    Remote { url: String },      // remote MCP server
    Bundled { module_name: String }, // compiled-in WASM module (future)
}

pub struct SkillRegistry {
    skills: HashMap<String, Skill>,
    // tool_name → skill_name mapping (tools are namespaced per skill)
    tool_index: HashMap<String, String>,
}

impl SkillRegistry {
    /// Register a skill from a remote MCP server URL.
    /// Connects, discovers tools, and adds them to the tool index.
    pub async fn register_remote(
        &mut self,
        name: &str,
        url: &str,
    ) -> Result<Vec<ToolDefinition>, RegistryError>;

    /// Get all tool definitions across all registered skills
    /// (passed to the LLM as available tools)
    pub fn all_tools(&self) -> Vec<ToolDefinition>;

    /// Dispatch a tool call to the correct skill
    pub async fn dispatch(
        &self,
        call: &ToolCall,
    ) -> ToolResult;

    /// Serialize registry config for persistence
    pub fn to_config(&self) -> RegistryConfig;

    /// Restore registry from persisted config (reconnects on startup)
    pub async fn from_config(config: RegistryConfig) -> Result<Self, RegistryError>;
}

/// Implements the ToolExecutor trait from Phase 1
#[async_trait(?Send)]
impl ToolExecutor for SkillRegistry {
    async fn execute(&self, call: &ToolCall) -> ToolResult {
        self.dispatch(call).await
    }
}
```

---

### 2.3 — Skill Isolation Model

Each skill communicates with the agent core only through the MCP protocol over HTTP. This means:

```
┌─────────────────────────────────────────────────────┐
│  Agent Core (WASM sandbox)                          │
│                                                     │
│  SkillRegistry                                      │
│       │                                             │
│       │  HTTP / SSE only — no shared memory        │
│       │                                             │
│  ┌────▼──────┐  ┌────────────┐  ┌───────────────┐  │
│  │web-search │  │  memory    │  │  http-fetch   │  │
│  │MCP Server │  │ MCP Server │  │  MCP Server   │  │
│  │           │  │            │  │               │  │
│  │(separate  │  │(separate   │  │(separate      │  │
│  │ process / │  │ process /  │  │ process /     │  │
│  │ worker)   │  │ worker)    │  │ worker)       │  │
│  └───────────┘  └────────────┘  └───────────────┘  │
└─────────────────────────────────────────────────────┘
```

Isolation guarantees:
- **No shared memory** between skills — all communication is over the network boundary
- **No filesystem access** — skills running as Cloudflare Workers or WASM modules have no disk
- **Capability-limited** — each skill server only has the outbound network access it needs
- **Blast radius contained** — a compromised `web-search` skill cannot read `memory` skill data

---

### 2.4 — Reference Skills

#### `skill-memory`
Persistent key-value memory scoped to a session. Backed by Cloudflare KV or D1.

Tools exposed:
- `memory_store(key, value)` — save a fact
- `memory_retrieve(key)` — fetch a fact
- `memory_search(query)` — semantic search over stored facts (optional, Phase 3)

#### `skill-web-search`
Wraps a search API (Brave Search, SearXNG, or Tavily).

Tools exposed:
- `web_search(query, max_results?)` — returns titles, URLs, snippets

#### `skill-http-fetch`
Allows the agent to fetch URL contents — with a user-controlled allowlist.

Tools exposed:
- `http_fetch(url)` — returns page text content (sanitized)

---

### 2.5 — Dynamic Skill Discovery

Users can add new MCP servers at runtime without redeploying:

```
User: "Add my Notion workspace as a skill"
Agent: "Sure. What's your Notion MCP server URL?"
User: "https://notion-mcp.myserver.com"

Agent (internally):
  1. registry.register_remote("notion", "https://notion-mcp.myserver.com").await
  2. Discovers tools: [notion_search, notion_create_page, notion_update_page]
  3. Saves updated RegistryConfig to KV
  4. Replies: "Done. I can now search and edit your Notion workspace."
```

---

### 2.6 — Updated Edge Entrypoint

```rust
// Phase 2 entrypoint — adds skill registry

#[event(fetch)]
pub async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let body: IncomingMessage = req.json().await?;
    let kv = env.kv("EDGECLAW_STATE")?;

    // Load agent state (conversation history)
    let state: AgentState = load_state(&kv, &body.session_id).await?;

    // Load and reconnect skill registry from persisted config
    let registry_config = load_registry_config(&kv, &body.session_id).await?;
    let mut registry = SkillRegistry::from_config(registry_config).await?;

    // Build agent with live tool list
    let tools = registry.all_tools();
    let agent = Agent::new(build_llm_client(&env)?, tools, default_config());

    // Run — registry now dispatches real tool calls
    let result = agent.run(state, &body.message, &registry).await?;

    // Persist updated state and registry config
    save_state(&kv, &body.session_id, &result.state).await?;
    save_registry_config(&kv, &body.session_id, &registry.to_config()).await?;

    Response::from_json(&OutgoingMessage { reply: result.answer })
}
```

---

### 2.7 — Phase 2 Milestones

| Milestone | Description | Done When |
|---|---|---|
| M2.1 | MCP client connects to a real MCP server and lists tools | Integration test against a local MCP server |
| M2.2 | MCP client invokes a tool and returns a result | Tool call round-trip test passes |
| M2.3 | SkillRegistry dispatches tool calls to correct skill | Multi-skill dispatch test passes |
| M2.4 | `skill-memory` deployed and integrated | Agent can store and retrieve facts across turns |
| M2.5 | `skill-web-search` deployed and integrated | Agent can answer questions using live search |
| M2.6 | Dynamic skill registration works end-to-end | User adds MCP URL via chat, tools become available |
| M2.7 | RegistryConfig persists and restores across cold starts | Skills survive Worker restart |
| M2.8 | Full end-to-end test: multi-turn conversation using 2+ skills | Demo scenario passes |

---

## Security Model Summary

| Threat | Phase 1 Mitigation | Phase 2 Mitigation |
|---|---|---|
| Prompt injection via tool output | Max iterations cap, structured tool result parsing | Same + skill output sanitization |
| Runaway agent (inbox deletion, etc.) | No tools in Phase 1 | Tool allowlist per session; user confirms destructive tools |
| Skill escaping its sandbox | N/A | HTTP boundary — no shared memory |
| Compromised skill reading another skill's data | N/A | Skills never share state — KV scoped per skill |
| Malicious MCP server | N/A | URL allowlist; skills declared by user only |
| Conversation state tampering | State is server-side in KV, not client-controlled | Same |
| API key exposure | Key in Worker env secret, never in state | Same |

---

## Open Questions for Phase 3+

- **Authentication** — how does EdgeClaw authenticate to user-provided MCP servers? OAuth? Bearer token stored in KV?
- **Streaming responses** — Cloudflare Workers supports streaming; the current design is request/response. Worth adding for long agent runs.
- **Messaging frontends** — Telegram webhook is the obvious first integration. WhatsApp (Twilio or Meta API) second.
- **Local model support** — Cloudflare Workers AI or Fermyon Wasm-native inference for privacy-sensitive deployments.
- **Skill marketplace** — a registry of community MCP servers users can add with one command, similar to NanoClaw's skills model.
