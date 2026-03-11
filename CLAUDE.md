# TokenStunt

## Commands
- Build: `cargo build`
- Test: `cargo test`
- Run: `cargo run -- serve`, `cargo run -- index --root .`, `cargo run -- status`

## Architecture
Cargo workspace, 6 crates:
- `tokenstunt` — CLI binary (clap)
- `tokenstunt-server` — MCP server (rmcp, stdio)
- `tokenstunt-index` — indexer orchestrator + file walker
- `tokenstunt-search` — BM25 keyword search
- `tokenstunt-parser` — tree-sitter AST extraction
- `tokenstunt-store` — SQLite persistence (rusqlite, FTS5)

## Rules
- Rust edition 2024, strict mode
- No `unwrap()` or `expect()` in library code — use `anyhow::Result`
- No `unsafe` unless absolutely required
- All public functions must have tests
- Conventional commits: feat/fix/chore/refactor
