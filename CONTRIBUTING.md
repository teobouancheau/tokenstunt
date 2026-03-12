# Contributing to TokenStunt

Thanks for your interest in contributing to TokenStunt.

## Getting Started

1. Fork the repository
2. Clone your fork: `git clone https://github.com/YOUR_USERNAME/tokenstunt.git`
3. Create a branch: `git checkout -b feat/your-feature`
4. Make your changes
5. Run tests: `cargo test`
6. Push and open a Pull Request

## Development

### Prerequisites

- Rust (edition 2024)
- tree-sitter grammars are vendored, no extra setup needed

### Build

```bash
cargo build
cargo build --features lang-swift,lang-kotlin,lang-dart  # all languages
```

### Test

```bash
cargo test
cargo test --features lang-swift,lang-kotlin,lang-dart  # all languages
```

### Project Structure

Cargo workspace with 7 crates:

- `tokenstunt` -- CLI binary
- `tokenstunt-server` -- MCP server
- `tokenstunt-index` -- indexer, file watcher
- `tokenstunt-search` -- BM25 keyword search + hybrid ranking
- `tokenstunt-parser` -- tree-sitter AST extraction
- `tokenstunt-embeddings` -- embedding providers (Ollama, OpenAI-compat)
- `tokenstunt-store` -- SQLite persistence

## Guidelines

- Conventional commits: `feat/fix/chore/refactor/docs`
- No `unwrap()` or `expect()` in library code, use `anyhow::Result`
- All public functions must have tests
- Keep PRs focused on a single change
- Run `cargo test` before submitting

## Adding a Language

1. Create `crates/tokenstunt-parser/src/extract/<lang>.rs`
2. Implement the `LanguageExtractor` trait
3. Wire it in `extract/mod.rs` and `languages.rs`
4. Add tests
5. Feature-gate if the tree-sitter grammar is heavy (see Swift/Kotlin/Dart)

## Reporting Issues

Use [GitHub Issues](https://github.com/teobouancheau/tokenstunt/issues). Include:

- OS and architecture
- Rust version (`rustc --version`)
- Steps to reproduce
- Expected vs actual behavior

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
