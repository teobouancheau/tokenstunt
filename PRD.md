# Token Stunt PRD: The Ultimate MCP Code Intelligence Plugin

## Problem

AI coding assistants waste 70-90% of their token budget reading entire files to find one function. Grep finds the file, Read loads the whole thing. For a 500-line file where you need a 20-line function, that's 96% waste. Multiply by every lookup in a conversation and you're burning thousands of tokens on content the AI never uses.

Token Stunt solves this by returning exact symbol bodies (functions, classes, types) ranked by relevance, with dependency graphs and impact analysis. It's an MCP server that gives the AI structured code intelligence instead of raw file contents.

## Current State (Honest Assessment)

### What works well
- BM25 keyword search via SQLite FTS5 (proven, fast)
- Exact symbol lookup by name
- Dependency graph visualization (for extracted symbols)
- Blast radius analysis with BFS traversal
- Project structure overview with language breakdown
- Real-time file watching and incremental reindex
- 10 languages built-in (TS, JS, Python, Rust, Go, Java, C, C++, Ruby, TSX)
- 3 optional languages (Swift, Kotlin, Dart)
- Background indexing with state tracking (idle/running/ready/failed)
- Token efficiency: 70-90% savings vs Grep+Read

### What works with caveats
- Semantic search requires external embedding provider (Ollama/OpenAI)
- Dependency extraction limited to import statements and basic function calls
- "Public API" detection is heuristic (export keyword, naming conventions)
- Entry point detection is filename-prefix based only
- Impact analysis capped at depth 5

### Known gaps
- No phrase search ("exact phrase" queries)
- No boolean search operators (AND, NOT)
- Silent embedding fallback (user doesn't know if result is keyword or semantic)
- Single monolithic write transaction for full indexing (large repos block for minutes)
- No schema migration (version bump = delete DB and re-index)
- No graceful shutdown (SIGTERM kills mid-transaction, progress lost)
- No `tools/list_changed` MCP notification when indexing completes
- Fixed column widths truncate long symbol names and paths

### Extraction gaps by area
- No property/field extraction (class fields not indexed)
- No decorator/annotation extraction as searchable entities
- No generic/template parameter capture
- No nested function extraction in JS/TS
- Constant detection relies on naming conventions (ALL_CAPS) for Python/Ruby
- No data flow or taint tracking
- No cross-file type inference

## What "Ultimate" Means

The goal is not to build an IDE. Token Stunt is a code intelligence layer for AI agents. "Ultimate" means:

1. **Zero friction**: install the plugin, it works on any project, any language, immediately
2. **Always accurate**: tools never lie, never return misleading results, always explain their state
3. **Maximally efficient**: every token in the response carries information the AI needs
4. **Complete for AI workflows**: the AI can orient, search, understand coupling, assess risk, and decide what to read

## Changes Required

### P0: Tool Naming (Breaking Change)

**Problem**: Tools use `ts_` prefix (`ts_search`, `ts_symbol`, etc.). Claude Code already namespaces MCP tools as `mcp__plugin_tokenstunt_tokenstunt__<tool_name>`. The prefix is redundant, wastes tokens in every tool call, and doesn't follow the convention set by official MCP reference servers (which use descriptive verb-noun names without project prefixes).

Context7 uses `query-docs` and `resolve-library-id`. The official MCP filesystem server uses `read_file`, `write_file`, `list_directory`. No prefixes.

**Change**: Rename all tools to descriptive verb-noun snake_case:

| Current | New | Rationale |
|---------|-----|-----------|
| `ts_search` | `search_code` | Verb + object, self-documenting |
| `ts_symbol` | `lookup_symbol` | Action-oriented, matches what it does |
| `ts_context` | `show_context` | Consistent verb prefix |
| `ts_overview` | `show_overview` | Consistent verb prefix |
| `ts_setup` | `run_diagnostics` | Describes actual function, not a lifecycle step |
| `ts_impact` | `analyze_impact` | Verb matches the analysis nature |

**Files**: `crates/tokenstunt-server/src/tools.rs` (tool definitions), all 7 `skills/*/SKILL.md` files (references), `CLAUDE.md` (documented tool names)

**Migration**: This is a breaking change for existing users. Since the plugin auto-updates, the rename takes effect on next server restart. Skills and instructions reference the new names immediately.

### P0: Tool Descriptions (Token Efficiency)

**Problem**: Tool descriptions are the AI's first signal for when to use a tool. Current descriptions mix marketing ("saves 95% of tokens") with usage guidance. The AI doesn't need persuading, it needs clear affordance descriptions.

**Change**: Rewrite all tool descriptions to follow the pattern: `{What it returns} -- {When to use it instead of alternatives}`.

| Tool | Current Description | New Description |
|------|-------------------|-----------------|
| `search_code` | "Semantic code search -- returns exact function/class/type bodies ranked by relevance. Use instead of Grep+Read when searching by concept or keyword. Saves 95% tokens vs reading full files." | "Returns ranked function/class/type bodies matching a query. Use instead of Grep+Read for any code lookup." |
| `lookup_symbol` | "Exact symbol lookup by name -- returns the full definition with file path and line numbers. Faster than Grep for known symbol names." | "Returns the full definition of a symbol by exact name. Use when you know the symbol name." |
| `show_context` | "Symbol definition + dependency graph -- shows what this symbol calls and what calls it. Use to understand coupling before modifying code." | "Returns a symbol's definition, what it calls, and what calls it. Use before modifying code." |
| `show_overview` | "Project structure overview -- module tree, language breakdown, public API surface, and entry points. Start here to orient in an unfamiliar codebase." | "Returns project structure: modules, languages, public API, entry points. Use first in unfamiliar codebases." |
| `run_diagnostics` | "Project diagnostics: index health, languages, embeddings status, and configuration guidance." | "Returns index health, indexing state, languages, and embeddings status. Use when tools report empty index." |
| `analyze_impact` | "Blast radius analysis: shows all symbols and files affected by changing a given symbol. Use before refactoring." | "Returns all symbols and files affected by changing a symbol. Use before refactoring." |

**Principle**: Descriptions start with "Returns" (tells the AI what it gets). Second sentence starts with "Use" (tells the AI when). No marketing. No metrics. No comparisons.

### P0: Server Instructions

**Problem**: Current instructions are a wall of text. The AI scans instructions at conversation start. Critical routing logic ("when to use this vs Grep") needs to be front and center.

**Change**:
```
Token Stunt indexes code into searchable symbols. Tools return exact function/class bodies instead of full files.

If any tool returns "Index is empty" or "Indexing is in progress", call run_diagnostics.
If a query returns no results, try broader terms or check the index with run_diagnostics.

Workflow: show_overview -> search_code -> lookup_symbol -> show_context/analyze_impact -> Read (only for files you will edit).

Use search_code instead of Grep+Read for any code lookup. Use Read only for files you intend to modify.
```

### P1: Chunked Indexing Transactions

**Problem**: `index_directory` runs one giant `write_transaction` for all files. A 50k-file repo holds the write lock for the entire duration. If the process crashes mid-index, all progress is lost.

**Change**: Chunk the indexing into batches of 1000 files per transaction. After each batch, commit and start a new transaction. This means:
- WAL checkpointing can happen between batches
- Crashes lose at most 1000 files of progress
- Memory for pending embedding work is bounded per batch
- Tools can see partial results while indexing is in progress (files from committed batches are queryable)

**Files**: `crates/tokenstunt-index/src/indexer.rs` (`index_directory` method)

**Constraint**: The dependency resolution pass (resolving forward references) currently runs at the end of the single transaction. With chunked transactions, unresolved dependencies from early batches get resolved when the referenced symbol is indexed in a later batch. The existing `resolve_unresolved_dependencies` pass at the end handles this.

### P1: MCP Notifications on Index Complete

**Problem**: When indexing finishes in the background, the AI has no way to know unless it calls `run_diagnostics`. If the AI called `search_code` during indexing and got "Indexing is in progress", it falls back to Grep. But indexing may finish seconds later and the AI never retries.

**Change**: Send an MCP `notifications/tools/list_changed` notification when indexing state transitions to `READY` or `FAILED`. This tells the client that tool capabilities have changed (they now return real results instead of "indexing in progress").

**Dependency**: Requires checking if rmcp supports sending server-initiated notifications. The `rmcp::ServiceExt` trait may have a notification method.

**Files**: `crates/tokenstunt/src/main.rs` (background task), `crates/tokenstunt-server/src/tools.rs` (notification sender)

### P1: Search Quality Improvements

**Problem**: FTS5 query construction is naive. All terms become `term*` OR'd together. "authentication middleware" finds anything with "auth*" OR "middleware*" which is too broad.

**Changes**:
1. **AND by default**: Join terms with AND instead of OR. "authentication middleware" should require both terms. Users can still get OR behavior by searching each term separately.
2. **Phrase search**: Detect quoted strings and pass them as FTS5 phrase queries. `"error handler"` searches for the exact phrase.
3. **Result scoring display**: Show whether each result came from keyword match, semantic match, or both. This helps the AI understand result quality and decide whether to trust or verify.

**Files**: `crates/tokenstunt-search/src/lib.rs` (`build_fts_query`, `SearchEngine::search`)

### P1: Graceful Shutdown

**Problem**: SIGTERM kills the background indexing task mid-transaction. SQLite rolls back, all indexing progress is lost.

**Change**: 
1. Add a `CancellationToken` (from `tokio_util`) threaded through the indexer
2. Check the token between file processing iterations in `index_directory`
3. When cancelled, commit the current transaction (preserving partial progress) and exit
4. Store the `JoinHandle` from `tokio::spawn` and await it on shutdown

**Files**: `crates/tokenstunt-index/src/indexer.rs`, `crates/tokenstunt/src/main.rs`

### P2: Schema Migration

**Problem**: Schema version bump = delete database and re-index from scratch. When Token Stunt ships a schema change, users with large indexes lose everything.

**Change**: Add versioned migration functions in `schema.rs`. When `initialize()` detects a version mismatch, it runs the migration chain (v2 -> v3, v3 -> v4, etc.) instead of failing.

**Files**: `crates/tokenstunt-store/src/schema.rs`

**Constraint**: Migrations must be backwards-compatible. A v3 schema should still work if the user downgrades to a v2 binary (the extra columns are ignored). If that's not possible, the migration should log a clear warning.

### P2: Property and Field Extraction

**Problem**: Class fields, struct fields, and object properties are not indexed. `search_code "userId"` won't find `class User { userId: string }` because the field isn't extracted as a symbol.

**Change**: Add field/property extraction to the language extractors that support it:
- TypeScript/JavaScript: class fields, object properties in type literals
- Python: `__init__` assignments as class fields
- Rust: struct fields
- Go: struct fields
- Java: class fields

**Files**: `crates/tokenstunt-parser/src/extract/*.rs` (per-language extractors)

### P2: Nested Symbol Extraction

**Problem**: Nested functions in JavaScript/TypeScript, inner classes in Java, and closures in Python are not extracted. The AI can't find `const handler = () => { ... }` inside a React component.

**Change**: Walk the AST recursively for nested function/class definitions. Store parent-child relationship via `parent_id` (already exists in the schema but underused).

**Files**: `crates/tokenstunt-parser/src/extract/typescript.rs`, `crates/tokenstunt-parser/src/extract/python.rs`, `crates/tokenstunt-parser/src/extract/java.rs`

### P3: Multi-Match Symbol Resolution

**Problem**: When multiple symbols share the same name (common with `handle`, `process`, `init`), `ts_context` and `ts_impact` silently pick the first one. The AI doesn't know there are alternatives.

**Change**: When a lookup returns multiple matches, include all of them with disambiguation info (file path, kind). Let the AI pick.

**Files**: `crates/tokenstunt-server/src/tools.rs` (context and impact handlers)

### P3: Incremental Dependency Resolution

**Problem**: Dependency resolution only happens at the end of `index_directory`. If a file is reindexed by the file watcher, its dependencies are resolved against whatever symbols exist at that point. Forward references to files not yet indexed stay unresolved.

**Change**: Run a targeted resolution pass after each `reindex_files` call. Only resolve dependencies for the newly indexed blocks.

**Files**: `crates/tokenstunt-index/src/indexer.rs` (`reindex_files` method)

## Out of Scope

These are not on the roadmap. They would change what Token Stunt is:

- **IDE features** (hover, completion, inlay hints): Token Stunt is for AI agents, not human editors
- **Refactoring support** (rename, extract): That's the language server's job
- **Data flow analysis**: Requires type inference engine, dramatically increases complexity
- **Multi-repo support**: Would need Postgres or distributed store
- **Custom query language**: MCP tools take natural language, not DSLs

## Verification Plan

For each change:

1. `cargo build` and `cargo test` (full workspace) pass
2. `cargo clippy` clean
3. Manual test: install plugin on a fresh project, verify immediate server start
4. Manual test: call each tool during indexing, verify state messages
5. Manual test: call each tool after indexing, verify correct results
6. Manual test: modify a file, verify file watcher triggers reindex
7. Measure: compare token count of search results vs equivalent Grep+Read for 5 common queries
