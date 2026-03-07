# EdgeClaw Development Guide

## Build & Check Commands

```bash
# Check both crates
cargo check -p agent-core
cargo check -p agent-core --target wasm32-unknown-unknown
cargo check -p edgeclaw-worker --target wasm32-unknown-unknown

# Run unit tests
cargo test -p agent-core

# Clippy
cargo clippy -p agent-core -- -D warnings
cargo clippy -p edgeclaw-worker --target wasm32-unknown-unknown -- -D warnings

# Format
cargo fmt --all -- --check

# Build worker for deployment/integration tests
cargo install worker-build && worker-build --release crates/edgeclaw-worker

# Integration tests
npm test
```

## Architecture Rules

- **agent-core** must have zero workers-rs dependency. It compiles to both wasm32 and native targets.
- **edgeclaw-worker** is the only crate that depends on `worker`. It's a `cdylib` targeting wasm32-unknown-unknown.
- The `HttpBackend` trait in agent-core abstracts HTTP calls — worker implements it with `worker::Fetch`, tests use `MockHttpBackend`.

## worker crate v0.7 Quirks

- `#[durable_object]` goes on the struct only, NOT on the impl block.
- `DurableObject::fetch` takes `&self` not `&mut self`. Use `Cell<bool>` for interior mutability (e.g., `initialized` flag).
- `sql.exec()` takes 2 args: `(query, impl Into<Option<Vec<SqlStorageValue>>>)` — pass `None` for no bindings.
- `SqlCursorRawIterator` yields `Result<Vec<SqlStorageValue>>` — match on `SqlStorageValue::{String, Integer, Float}`.
- `Request::path()` returns `String`; use `.as_str()` for match arms.
- `async-trait(?Send)` is required for WASM compatibility.

## Testing Patterns

- **Unit tests**: `MockHttpBackend` with `RefCell<VecDeque<Vec<u8>>>` for pre-recorded API responses. Fixtures in `tests/fixtures/`.
- **Integration tests**: Miniflare + Vitest in `tests/integration/`. Requires `worker-build --release crates/edgeclaw-worker` first.

## Durable Object Identity

Always use `id_from_name()` for deterministic agent identity, never `new_unique_id()`. Format: `agent:{user_id}`.

## Deployment

```bash
npx wrangler deploy
npx wrangler secret put ANTHROPIC_API_KEY
```

Model configurable via `CLAUDE_MODEL` env var in wrangler.toml `[vars]` or `.dev.vars`.
