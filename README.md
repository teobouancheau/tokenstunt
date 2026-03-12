# TokenStunt

AST-level code intelligence MCP server for Claude Code. Indexes your codebase into searchable symbols, dependency graphs, and project overviews -- all served over the Model Context Protocol.

## What it does

TokenStunt parses your source code with tree-sitter, extracts every function, class, interface, trait, enum, and constant, stores them in a SQLite FTS5 index, and serves them through 4 MCP tools:

| Tool | Description |
|------|-------------|
| `ts_search` | Code search across indexed symbols. Returns ranked code blocks, not full files. |
| `ts_symbol` | Exact symbol lookup by name. Returns the full definition. |
| `ts_context` | Symbol definition + dependency graph. Shows what a symbol calls and what calls it. |
| `ts_overview` | Project structure: module tree, language breakdown, public API surface, entry points. |

## Supported languages

**Built-in:** TypeScript, TSX, JavaScript, Python, Rust, Go, Java, C, C++, Ruby

**Optional (feature-gated):** Swift (`lang-swift`), Kotlin (`lang-kotlin`), Dart (`lang-dart`)

## Quick start

```bash
# Build
cargo build --release

# Index a project
tokenstunt index --root /path/to/your/project

# Start the MCP server
tokenstunt serve --root /path/to/your/project
```

### Claude Code integration

Add to your Claude Code MCP config (`~/.claude/claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "tokenstunt": {
      "command": "/path/to/tokenstunt",
      "args": ["serve", "--root", "/path/to/your/project"]
    }
  }
}
```

## Features

### Live reactivity

TokenStunt watches your filesystem and re-indexes changed files in real-time (500ms debounce). No manual re-indexing needed.

### Dependency graph

Import statements are extracted from TypeScript and Python files. The dependency table tracks what each symbol references, enabling `ts_context` to show callers and callees.

### Semantic search (optional)

Configure a local embedding model (Ollama, LM Studio, or any OpenAI-compatible endpoint) for hybrid BM25 + cosine ranking:

```toml
# .tokenstunt/config.toml
[embeddings]
enabled = true
provider = "ollama"           # or "openai-compat"
model = "nomic-embed-text"
endpoint = "http://localhost:11434"
dimensions = 768
```

Without embeddings, search uses pure BM25 keyword ranking.

### Startup reconciliation

On every `serve` startup, TokenStunt compares file hashes against the stored index and only re-indexes what changed. Cold starts are fast.

## Architecture

Cargo workspace with 7 crates:

```
tokenstunt              CLI binary (clap)
tokenstunt-server       MCP server (rmcp, stdio)
tokenstunt-index        Indexer orchestrator, file walker, file watcher
tokenstunt-search       BM25 keyword search + hybrid cosine ranking
tokenstunt-parser       Tree-sitter AST extraction (13 languages)
tokenstunt-embeddings   Embedding providers (Ollama, OpenAI-compat)
tokenstunt-store        SQLite persistence (rusqlite, FTS5, WAL)
```

## Building with optional languages

```bash
# Default (10 languages)
cargo build --release

# With Swift, Kotlin, and Dart
cargo build --release --features lang-swift,lang-kotlin,lang-dart
```

## License

MIT
