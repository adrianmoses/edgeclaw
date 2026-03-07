# EdgeClaw

Stateful, edge-native AI agent runtime on Cloudflare Workers + Durable Objects. Each user gets a dedicated agent instance with its own SQLite database, running a ReAct loop against the Anthropic API.

## Architecture

```
HTTP / WebSocket ──> Dispatcher Worker ──> AgentDO (Rust WASM)
                                               │
                         ┌─────────┬───────────┴──────────┐
                         ▼         ▼                      ▼
                   MemorySkill  WebSearch             HttpFetch
                   (Phase 2)   (Phase 2)             (Phase 2)
```

**Two-crate workspace:**
- `crates/agent-core` — Pure Rust: ReAct agent loop, LLM client, domain types. Zero workers-rs dependency, compiles to both `wasm32-unknown-unknown` and native.
- `crates/edgeclaw-worker` — workers-rs glue: Durable Object with SQLite persistence, HTTP dispatcher, WebSocket hibernation.

## Prerequisites

- Rust (stable) with `wasm32-unknown-unknown` target: `rustup target add wasm32-unknown-unknown`
- Node.js 20+
- [wrangler](https://developers.cloudflare.com/workers/wrangler/) (`npm install`)

## Local Development

```bash
# Set your API key
echo "ANTHROPIC_API_KEY=sk-ant-..." > .dev.vars

# Start local dev server
npx wrangler dev
```

## Testing

```bash
# Rust unit tests
cargo test --workspace

# Integration tests (requires worker build)
cargo install worker-build
worker-build --release crates/edgeclaw-worker
npm install
npm test
```

## Deployment

```bash
# Deploy to Cloudflare
npx wrangler deploy

# Set the API key secret
npx wrangler secret put ANTHROPIC_API_KEY
```

## API Reference

All endpoints require user identity via `X-User-Id` header or `?user_id=` query param.

### POST /message
Send a message to the agent.

```json
// Request
{ "message": "Hello, what can you do?" }

// Response
{ "answer": "I can help you with...", "pending_tool_calls": [] }
```

### GET /history
Retrieve conversation history (last 50 messages).

### GET / (WebSocket Upgrade)
Connect via WebSocket for real-time interaction. Send `Upgrade: websocket` header.

```json
// Send
{ "message": "Hello" }

// Receive
{ "answer": "Hi there!", "pending_tool_calls": [] }
```

### POST /orchestrate
Fan out a task to multiple named agents.

```json
// Request
{ "task": "Research Rust WASM", "agents": ["researcher", "writer"] }

// Response
{ "researcher": { "answer": "..." }, "writer": { "answer": "..." } }
```

## Project Structure

```
edgeclaw/
├── crates/
│   ├── agent-core/src/
│   │   ├── agent.rs      # ReAct loop (run + resume)
│   │   ├── llm.rs        # Anthropic API client + HttpBackend trait
│   │   ├── types.rs      # Domain types (Message, ContentBlock, ToolCall, etc.)
│   │   └── error.rs      # AgentError enum
│   └── edgeclaw-worker/src/
│       └── lib.rs         # Dispatcher, AgentDO, WebSocket, orchestration
├── tests/
│   ├── fixtures/          # JSON fixtures for unit tests
│   └── integration/       # Miniflare integration tests
├── wrangler.toml          # Cloudflare Workers config
└── CLAUDE.md              # Development conventions
```
