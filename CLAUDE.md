# Token Stunt

## Commands
- Build: `cargo build`
- Build all features: `cargo build --features lang-swift,lang-kotlin,lang-dart`
- Test: `cargo test`
- Test all features: `cargo test --features lang-swift,lang-kotlin,lang-dart`
- Run: `cargo run -- serve`, `cargo run -- index --root .`, `cargo run -- status`
- Single crate: `cargo test -p tokenstunt-store`, `cargo test -p tokenstunt-parser`, etc.

## Architecture
Cargo workspace, 7 crates:
- `tokenstunt` — CLI binary (clap), config loading
- `tokenstunt-server` — MCP server (rmcp, stdio), 6 tools: `ts_search`, `ts_symbol`, `ts_context`, `ts_overview`, `ts_setup`, `ts_impact`
- `tokenstunt-index` — indexer orchestrator, file walker, file watcher (notify), startup reconciliation
- `tokenstunt-search` — BM25 keyword search + optional hybrid cosine ranking
- `tokenstunt-parser` — tree-sitter AST extraction, `LanguageExtractor` trait, per-language modules in `extract/`
- `tokenstunt-embeddings` — `EmbeddingProvider` trait, Ollama + OpenAI-compat clients
- `tokenstunt-store` — SQLite persistence (rusqlite, FTS5, WAL mode), read/write connection split

## Key patterns
- Store uses separate `read_conn` / `write_conn` for WAL concurrency
- `write_transaction(|conn| { ... })` holds the mutex for the entire transaction
- `_with_conn` variants (`pub`) allow store methods inside transactions
- Parser uses `LanguageExtractor` trait with one module per language in `extract/`
- Adding a language: create `extract/<lang>.rs`, implement `LanguageExtractor`, wire in `extract/mod.rs` + `languages.rs`
- Feature-gated languages (Swift/Kotlin/Dart) use `#[cfg(feature = "lang-X")]`
- Config and database live in `~/.cache/tokenstunt/<project-name>-<hash>/`

## Rules
- Rust edition 2024, strict mode
- No `unwrap()` or `expect()` in library code — use `anyhow::Result`
- No `unsafe` unless absolutely required
- All public functions must have tests
- Conventional commits: feat/fix/chore/refactor
