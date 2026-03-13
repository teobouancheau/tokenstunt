# TokenStunt

Smart code search MCP server for Claude Code. Indexes your codebase into searchable symbols and dependency graphs, served over the Model Context Protocol.

## What it does

TokenStunt parses your source code with tree-sitter, extracts every function, class, interface, trait, enum, and constant, stores them in a SQLite FTS5 index, and serves them through 6 MCP tools.

### `ts_search`

Search code by concept or keyword. Returns ranked symbol bodies with scores.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `query` | yes | Natural language or keyword query |
| `scope` | no | Restrict to a directory path |
| `language` | no | Filter by language (`typescript`, `python`, etc.) |
| `symbol_kind` | no | Filter by kind (`function`, `class`, `interface`, etc.) |
| `limit` | no | Max results, default 10 |

### `ts_symbol`

Look up a symbol by exact name. Returns the full definition with location.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `name` | yes | Exact symbol name |
| `kind` | no | Filter by symbol kind |

### `ts_context`

Show a symbol's definition, what it calls, and what calls it.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `symbol` | yes | Symbol name |
| `direction` | no | `dependencies`, `dependents`, or `both` (default `both`) |

### `ts_overview`

Project structure at a glance: files, languages, modules, public API, entry points.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `scope` | no | Restrict to a directory path (e.g. `src/`) |
| `depth` | no | Directory depth for module tree, default 1 |

### `ts_setup`

Index health, languages detected, embeddings coverage. No parameters.

### `ts_impact`

Show every symbol and file affected by changing a given symbol.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `symbol` | yes | Symbol name to analyze |
| `max_depth` | no | Max traversal depth, default 3, max 5 |

## Supported languages

**Built in:** TypeScript, TSX, JavaScript, Python, Rust, Go, Java, C, C++, Ruby

**Optional (feature gated):** Swift (`lang-swift`), Kotlin (`lang-kotlin`), Dart (`lang-dart`)

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

All CLI output uses colored, compact formatting. Indexing shows a live progress bar. Status and serve display structured summaries.

## Features

### Native Claude Code output

MCP tool responses use Unicode compact blocks instead of markdown. Box drawing characters, score bars, tree connectors, and aligned columns. Renders cleanly in Claude Code's terminal.

### Live reactivity

TokenStunt watches your filesystem and re indexes changed files in real time (500ms debounce). No manual re indexing needed.

### Dependency graph

Import statements are extracted and stored. The dependency table tracks what each symbol references, enabling `ts_context` to show callers and callees.

### Semantic search (optional)

Configure a local embedding model (Ollama, LM Studio, or any OpenAI compatible endpoint) for hybrid BM25 + cosine ranking. Run `/tokenstunt:configure` in Claude Code, or create the config manually:

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

On every `serve` startup, TokenStunt compares file hashes against the stored index and only re indexes what changed. Cold starts are fast.

### Transparent storage

All data (index database, config) is stored in `~/.cache/tokenstunt/`. Nothing is created in your project directory.

## Architecture

Cargo workspace with 7 crates:

```
tokenstunt              CLI binary (clap)
tokenstunt-server       MCP server (rmcp, stdio)
tokenstunt-index        Indexer orchestrator, file walker, file watcher
tokenstunt-search       BM25 keyword search + hybrid cosine ranking
tokenstunt-parser       Tree-sitter AST extraction (13 languages)
tokenstunt-embeddings   Embedding providers (Ollama, OpenAI compat)
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
