# TokenStunt

AST-level code intelligence MCP server for Claude Code. Indexes your codebase into searchable symbols, dependency graphs, and project overviews, all served over the Model Context Protocol.

## What it does

TokenStunt parses your source code with tree-sitter, extracts every function, class, interface, trait, enum, and constant, stores them in a SQLite FTS5 index, and serves them through 6 MCP tools.

### `ts_search`

Semantic code search. Returns ranked code blocks with score bars, not full files.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `query` | yes | Search query (natural language or keyword) |
| `scope` | no | Scope to a directory path |
| `language` | no | Filter by language (e.g. `typescript`, `python`) |
| `symbol_kind` | no | Filter by symbol kind (`function`, `class`, `interface`, etc.) |
| `limit` | no | Max results (default: 10) |

### `ts_symbol`

Exact symbol lookup by name. Returns the full definition with file path and line numbers.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `name` | yes | Exact symbol name |
| `kind` | no | Filter by symbol kind |

### `ts_context`

Symbol definition + dependency graph. Shows what a symbol calls and what calls it.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `symbol` | yes | Symbol name |
| `direction` | no | `dependencies`, `dependents`, or `both` (default: `both`) |

### `ts_overview`

Project structure: module tree, language breakdown, public API surface, entry points.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `scope` | no | Scope to a directory path (e.g. `src/`) |
| `depth` | no | Directory depth for module tree (default: 1) |

### `ts_setup`

Project diagnostics: index health, languages, embeddings status and configuration guidance. No parameters.

### `ts_impact`

Blast radius analysis: all symbols and files affected by changing a given symbol.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `symbol` | yes | Symbol name to analyze |
| `max_depth` | no | Max traversal depth (default: 3, max: 5) |

## Supported languages

**Built-in:** TypeScript, TSX, JavaScript, Python, Rust, Go, Java, C, C++, Ruby

**Optional (feature-gated):** Swift (`lang-swift`), Kotlin (`lang-kotlin`), Dart (`lang-dart`)

## Install

### Claude Code plugin (recommended)

```
/plugin install teobouancheau/tokenstunt
```

### Manual

```bash
cargo build --release
tokenstunt serve --root /path/to/your/project
```

## CLI commands

```bash
tokenstunt index --root .    # Index a directory with progress bar
tokenstunt status            # Show index health at a glance
tokenstunt serve --root .    # Start MCP server (used by Claude Code plugin)
```

All CLI output uses colored, compact formatting with an orange accent palette. Indexing shows a live progress bar. Status and serve display structured summaries.

## Features

### Native Claude Code output

MCP tool responses use Unicode compact blocks instead of markdown — box-drawing characters, score bars (`▓░`), tree connectors (`├─ └─`), and aligned columns. Renders cleanly in Claude Code's terminal without formatting artifacts.

### Live reactivity

TokenStunt watches your filesystem and re-indexes changed files in real-time (500ms debounce). No manual re-indexing needed.

### Dependency graph

Import statements are extracted from TypeScript and Python files. The dependency table tracks what each symbol references, enabling `ts_context` to show callers and callees.

### Semantic search (optional)

Configure a local embedding model (Ollama, LM Studio, or any OpenAI-compatible endpoint) for hybrid BM25 + cosine ranking. Run `/tokenstunt:configure` in Claude Code, or create the config manually:

```toml
# ~/.cache/tokenstunt/<project>/config.toml
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

### Transparent storage

All data (index database, config) is stored in `~/.cache/tokenstunt/` — nothing is created in your project directory.

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
