# TokenStunt v1 -- Design Specification

The ultimate MCP code intelligence server for Claude Code. Automatic, progressive, real-time indexing. Token-optimized responses. Never stale, never lossy.

## Problem

Claude Code wastes tokens reading entire files, running grep, and manually exploring codebases. A single "how does auth work?" question can cost 50k+ tokens across multiple tool calls. TokenStunt replaces that with AST-level semantic search that returns exact function bodies, dependency chains, and project structure in one call.

## Current State (What Exists)

- 4 MCP tools: `ts_search`, `ts_symbol`, `ts_context`, `ts_overview`
- BM25 keyword search via SQLite FTS5
- AST extraction for TypeScript, TSX, JavaScript, Python
- Incremental indexing with content hash comparison
- SQLite persistence with WAL mode
- `tokenstunt-embeddings` crate exists as an empty placeholder

### What's Broken

| Feature | Claim | Reality |
|---------|-------|---------|
| `ts_context` | "dependency traversal" | `dependencies` table is never populated by indexer |
| `ts_overview` | "modules, public APIs, entry points" | Returns file count and block count |
| Tool descriptions | "hybrid semantic + keyword" | Pure BM25, no semantic search |
| `overview_cache` table | Exists in schema | Never read or written |
| `embedding_id` column | Exists in schema | Never populated |
| Reactivity | None | Manual `index` command required |
| `Store::transaction` | Transaction isolation | Drops mutex between BEGIN and closure -- concurrent access can interleave |

## Pre-Requisite: Fix Concurrency Model

Before any pillar work, the `Store` concurrency model must be fixed. The current `Store::transaction` drops the `MutexGuard` between `BEGIN TRANSACTION` and the closure execution, allowing another thread to interleave SQL within the transaction. With a file watcher running concurrently to MCP tool calls, this becomes a data corruption risk.

**Fix:**

```rust
pub fn transaction<F, T>(&self, f: F) -> Result<T>
where
    F: FnOnce(&Connection) -> Result<T>,
{
    let conn = self.lock()?;
    conn.execute_batch("BEGIN TRANSACTION")?;
    match f(&conn) {
        Ok(val) => {
            conn.execute_batch("COMMIT")?;
            Ok(val)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}
```

The closure receives `&Connection` directly, and the `MutexGuard` is held for the entire transaction. All store methods that are called within transactions need `_with_conn(&self, conn: &Connection, ...)` variants that skip the `self.lock()` call.

**Methods requiring `_with_conn` variants** (used within transactions during indexing/reconciliation):
- `get_file_hash_with_conn`
- `upsert_file_with_conn`
- `delete_file_blocks_with_conn`
- `insert_code_block_with_conn`
- `insert_dependency_with_conn`
- `delete_block_dependencies_with_conn`
- `delete_stale_files_with_conn`
- `lookup_symbol_with_conn` (for dependency resolution within transaction)

Read-only methods (`search_fts`, `get_block_by_id`, `get_dependencies`, `get_dependents`, `file_count`, `block_count`, overview queries) do not need `_with_conn` variants -- they use the read connection.

**Pattern:** Each method has the public locking version and a `pub(crate)` `_with_conn` variant:

```rust
pub fn upsert_file(&self, repo_id: i64, path: &str, ...) -> Result<i64> {
    let conn = self.write_lock()?;
    self.upsert_file_with_conn(&conn, repo_id, path, ...)
}

pub(crate) fn upsert_file_with_conn(&self, conn: &Connection, repo_id: i64, path: &str, ...) -> Result<i64> {
    // actual implementation
}
```

**Read/write separation:** SQLite WAL mode allows concurrent readers with one writer. Split into:
- `read_conn: Mutex<Connection>` -- for tool queries (ts_search, ts_symbol, etc.)
- `write_conn: Mutex<Connection>` -- for indexing, file watcher updates

This prevents the file watcher from blocking MCP tool responses during re-indexing.

## Schema Migration Strategy

Current behavior: `schema.rs` hard-fails on version mismatch with "Delete the database and re-index." This is acceptable for v1 since the product is pre-release.

**Strategy:**
- Bump `SCHEMA_VERSION` to 2 when adding `embeddings` table
- On version mismatch: delete DB and re-index automatically (no user prompt)
- Log: "Schema upgraded from v1 to v2, re-indexing..."
- Future: proper migrations when the schema stabilizes

## Design: 7 Pillars

### Pillar 1: Dependency Graph Extraction

**Goal**: `ts_context` returns real caller/callee chains from production code.

**Parser output change:**

```rust
pub struct ParseResult {
    pub symbols: Vec<ParsedSymbol>,
    pub references: Vec<RawReference>,
}

pub struct RawReference {
    pub source_symbol: String,
    pub target_name: String,
    pub kind: &'static str,  // "call", "import", "type_ref"
    pub line: u32,
}
```

**Priority tiers for reference extraction:**

| Priority | Reference kind | Complexity | Languages |
|----------|---------------|------------|-----------|
| P0 | Imports | Low (structural nodes) | All 13 |
| P1 | Function/method calls | Medium (name matching) | TS/JS, Python, Rust, Go, Java |
| P2 | Type references | High (generics, trait bounds) | TS/JS, Rust, Java |

P0 ships first. P1 and P2 are follow-up passes.

**Resolution during indexing:**

1. Extract raw references per symbol during parsing
2. After all files in a transaction are indexed, resolve `target_name` to `target_block_id` via `lookup_symbol`
3. Unresolved references stored with `resolved = 0`, `target_block_id = NULL` (external dependencies, stdlib)
4. Re-resolution triggered on incremental re-index when affected files change

**Indexer reference processing flow:**

After all symbols for a file are inserted, the indexer processes references in a second pass within the same transaction:

```rust
// In Indexer::index_file_with_conn
// 1. Delete old blocks (CASCADE deletes old deps)
self.store.delete_file_blocks_with_conn(conn, file_id)?;

// 2. Insert new symbols, collecting (symbol_name -> block_id) mapping
let mut name_to_id: HashMap<String, i64> = HashMap::new();
for symbol in &result.symbols {
    let block_id = self.insert_symbol_with_conn(conn, file_id, symbol, None)?;
    name_to_id.insert(symbol.name.clone(), block_id);
}

// 3. Insert references as dependencies
for reference in &result.references {
    let source_id = match name_to_id.get(&reference.source_symbol) {
        Some(id) => *id,
        None => continue, // orphaned reference, skip
    };
    // Try to resolve target within the same repo
    let target = self.store.lookup_symbol_with_conn(conn, &reference.target_name, None)?;
    let target_id = target.first().map(|b| b.id);
    self.store.insert_dependency_with_conn(
        conn, source_id, target_id, &reference.target_name, reference.kind,
    )?;
}
```

**Store changes:**

- `insert_dependency` signature changed to accept `Option<i64>` for target_block_id:
  ```rust
  pub fn insert_dependency(
      &self,
      source_block_id: i64,
      target_block_id: Option<i64>,
      target_name: &str,
      kind: &str,
  ) -> Result<()>
  ```
- Add `delete_block_dependencies(block_id: i64)` -- cleanup before re-index
- Add `get_unresolved_dependencies() -> Vec<(i64, String, String)>` -- (source_id, target_name, kind)
- Add `resolve_dependency(source_block_id: i64, target_name: &str, target_block_id: i64)` -- update resolved deps

---

### Pillar 2: Real Overview

**Goal**: `ts_overview` returns module structure, public API surface, entry points. Cached and invalidated on change.

**New output format:**

```markdown
## Project Overview

- **Root**: /path/to/project
- **Languages**: TypeScript (25 files), Python (12 files), Rust (5 files)
- **Total**: 42 files, 200 code blocks

### Module Structure
src/
  auth/         3 files, 12 symbols
  api/          5 files, 28 symbols
  utils/        2 files, 8 symbols
tests/          4 files, 15 symbols

### Public API (exported symbols)
- authenticateUser (function) -- src/auth/index.ts:1
- UserService (class) -- src/auth/service.ts:5
- ApiRouter (class) -- src/api/router.ts:1

### Entry Points
- src/main.ts (default export)
- src/server.ts (main function)
```

**New `TsOverviewParams`:**

```rust
struct TsOverviewParams {
    /// Scope to a subdirectory (optional)
    scope: Option<String>,
    /// Detail level: 0 = counts, 1 = module tree, 2 = full with public API (default: 2)
    depth: Option<i32>,
}
```

Depth is `i32` to match the existing `overview_cache` table schema which uses `INTEGER`.

**Store additions:**

- `get_language_stats() -> Vec<(String, i64)>` -- language name, file count
- `get_directory_stats(scope: Option<&str>) -> Vec<(String, i64, i64)>` -- dir prefix, file count, block count
- `get_exported_symbols(scope: Option<&str>) -> Vec<CodeBlock>` -- top-level symbols (no parent_id)
- `get_overview_cache(scope: &str, depth: i32) -> Option<String>` -- cached content
- `set_overview_cache(scope: &str, depth: i32, content: &str)` -- write cache
- `invalidate_overview_cache(scope: &str)` -- delete entries matching scope prefix

**Cache strategy:**

- `overview_cache` table already exists in schema: `(scope TEXT, depth INTEGER, content TEXT, computed_at INTEGER)`
- On query: check cache. Cache is valid indefinitely -- only invalidated explicitly by file changes, never by time
- Invalidation: any file change in scope deletes cached entries for that scope and all parent scopes
- File watcher triggers invalidation (see Pillar 5)
- On startup reconciliation: if any files changed, invalidate all overview cache entries

---

### Pillar 3: All 13 Languages

**Goal**: Full symbol extraction for every language TokenStunt recognizes.

**Priority tiers:**

| Tier | Languages | Rationale |
|------|-----------|-----------|
| P0 (must ship) | Rust, Go, Java | Most requested by Claude Code users |
| P1 (ship if stable) | C, C++, Ruby | Stable grammars, common languages |
| P2 (best effort) | Swift, Kotlin, Dart | Community grammars, may have rough edges |

All 13 ship, but P2 languages get `#[cfg(feature = "lang-swift")]` etc. feature flags to keep default compile times manageable. Default features include P0 + P1. P2 opt-in.

**New tree-sitter dependencies:**

| Language | Crate | Grammar maturity |
|----------|-------|-----------------|
| Rust | `tree-sitter-rust` | Stable |
| Go | `tree-sitter-go` | Stable |
| Java | `tree-sitter-java` | Stable |
| C | `tree-sitter-c` | Stable |
| C++ | `tree-sitter-cpp` | Stable |
| Ruby | `tree-sitter-ruby` | Stable |
| Swift | `tree-sitter-swift` | Community |
| Kotlin | `tree-sitter-kotlin` | Community |
| Dart | `tree-sitter-dart` | Community |

**File structure:**

Current `extract.rs` is ~408 lines of production code (527 with tests). Adding 9 languages would push it far past the 200-line limit. Split into per-language modules:

```
crates/tokenstunt-parser/src/
  extract/
    mod.rs              -- SymbolExtractor, ParseResult, dispatch, shared helpers
    helpers.rs          -- child_text_by_field, node_text, node_text_str (extracted from current SymbolExtractor)
    typescript.rs       -- TS/TSX/JS (moved from extract.rs)
    python.rs           -- Python (moved from extract.rs)
    rust_lang.rs        -- Rust (new)
    go.rs               -- Go (new)
    java.rs             -- Java (new)
    c_lang.rs           -- C/C++ (new, shared extractor)
    ruby.rs             -- Ruby (new)
    swift.rs            -- Swift (new, feature-gated)
    kotlin.rs           -- Kotlin (new, feature-gated)
    dart.rs             -- Dart (new, feature-gated)
```

Note: `rust.rs` and `c.rs` are avoided as module names because they shadow std lib paths. Use `rust_lang.rs` and `c_lang.rs`.

**Extraction trait:**

```rust
// In extract/mod.rs
pub(crate) trait LanguageExtractor {
    fn extract_symbols(&self, root: Node, source: &[u8]) -> Vec<ParsedSymbol>;
    fn extract_references(&self, root: Node, source: &[u8]) -> Vec<RawReference>;
}
```

**Migration from current architecture:**

The current `SymbolExtractor` has private helper methods (`child_text_by_field`, `node_text`, `node_text_str`) that all language extractors need. These become free functions in `helpers.rs`:

```rust
// extract/helpers.rs
pub(crate) fn child_text_by_field(node: Node, field: &str, source: &[u8]) -> Option<String> { ... }
pub(crate) fn node_text(node: Node, source: &[u8]) -> String { ... }
```

Each language module is a struct implementing `LanguageExtractor`. The `SymbolExtractor::extract` method dispatches to the appropriate implementation:

```rust
pub fn extract(&self, source: &str, language: Language) -> Result<ParseResult> {
    let ts_lang = self.registry.get_ts_language(language)?;
    let tree = parse(source, &ts_lang)?;
    let root = tree.root_node();

    let (symbols, references) = match language {
        Language::TypeScript | Language::Tsx | Language::JavaScript => {
            let ext = TypeScriptExtractor;
            (ext.extract_symbols(root, source.as_bytes()),
             ext.extract_references(root, source.as_bytes()))
        }
        Language::Python => {
            let ext = PythonExtractor;
            (ext.extract_symbols(root, source.as_bytes()),
             ext.extract_references(root, source.as_bytes()))
        }
        // ... etc
    };

    Ok(ParseResult { symbols, references })
}
```

**Per-language symbol extraction targets:**

| Language | Functions | Classes/Structs | Interfaces/Traits | Enums | Constants | Imports |
|----------|-----------|----------------|-------------------|-------|-----------|---------|
| Rust | fn, pub fn | struct, impl | trait | enum | const, static | use |
| Go | func | struct | interface | - | const, var | import |
| Java | methods | class | interface | enum | static final | import |
| C/C++ | functions | struct, class | - | enum | #define, const | #include |
| Ruby | def | class | module | - | CONSTANTS | require |
| Swift | func | class, struct | protocol | enum | let | import |
| Kotlin | fun | class, data class | interface | enum | val, const val | import |
| Dart | functions | class | abstract class | enum | const, final | import |

**Feature gates** (in `tokenstunt-parser/Cargo.toml`):

```toml
[features]
default = ["lang-rust", "lang-go", "lang-java", "lang-c", "lang-cpp", "lang-ruby"]
lang-rust = ["dep:tree-sitter-rust"]
lang-go = ["dep:tree-sitter-go"]
lang-java = ["dep:tree-sitter-java"]
lang-c = ["dep:tree-sitter-c"]
lang-cpp = ["dep:tree-sitter-cpp"]
lang-ruby = ["dep:tree-sitter-ruby"]
lang-swift = ["dep:tree-sitter-swift"]
lang-kotlin = ["dep:tree-sitter-kotlin"]
lang-dart = ["dep:tree-sitter-dart"]
all-languages = ["lang-rust", "lang-go", "lang-java", "lang-c", "lang-cpp", "lang-ruby", "lang-swift", "lang-kotlin", "lang-dart"]
```

**LanguageRegistry changes:**

Add grammar loading for all new languages. `get_ts_language` and `is_supported` updated to cover all 13. Feature-gated languages return `bail!("language not enabled -- compile with feature lang-{name}")` when the feature is off.

**Testing**: Each language gets a test with a non-trivial snippet (30+ lines). At minimum: function, class/struct, import extraction, and one edge case per language.

---

### Pillar 4: Semantic Search

**Goal**: Hybrid BM25 + vector search when embeddings configured. Pure BM25 fallback otherwise.

**Configuration** (`.tokenstunt/config.toml`):

```toml
[embeddings]
enabled = true
provider = "ollama"              # or "openai-compat"
model = "nomic-embed-text"       # user's choice
endpoint = "http://localhost:11434"
# api_key = "sk-..."            # for openai-compat only
dimensions = 768
batch_size = 64
```

**Config struct and loading:**

```rust
// In tokenstunt (CLI crate)
#[derive(Debug, Deserialize, Default)]
pub struct Config {
    pub embeddings: Option<EmbeddingsConfig>,
}

#[derive(Debug, Deserialize)]
pub struct EmbeddingsConfig {
    pub enabled: bool,
    pub provider: String,        // "ollama" or "openai-compat"
    pub model: String,
    pub endpoint: String,
    pub api_key: Option<String>,
    pub dimensions: usize,
    pub batch_size: Option<usize>,
}

impl Config {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(".tokenstunt/config.toml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }
}
```

Config is loaded in the CLI, threaded through to `Indexer::new()` and `TokenStuntServer::new()`.

**Updated constructor signatures:**

```rust
// tokenstunt-index
impl Indexer {
    pub fn new(store: Store, embedder: Option<Arc<dyn EmbeddingProvider>>) -> Result<Self>
}

// tokenstunt-server
impl TokenStuntServer {
    pub fn new(indexer: Arc<Indexer>, root: PathBuf, has_embeddings: bool) -> Self
}
```

The CLI's `serve` command:
1. Loads `Config` from `.tokenstunt/config.toml`
2. If embeddings enabled: creates provider via `load_provider(&config.embeddings)`
3. Creates `Indexer::new(store, embedder)`
4. Creates `TokenStuntServer::new(Arc::new(indexer), root, embedder.is_some())`

**Crate structure** (`tokenstunt-embeddings`, already exists as placeholder):

```
src/
  lib.rs          -- EmbeddingProvider trait, Config re-export, load_provider()
  ollama.rs       -- Ollama /api/embed client
  openai.rs       -- OpenAI-compat /v1/embeddings client
```

**Provider trait:**

```rust
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    fn dimensions(&self) -> usize;
    async fn health_check(&self) -> Result<()>;
}
```

**Storage** (new table in schema v2):

```sql
CREATE TABLE IF NOT EXISTS embeddings (
    id INTEGER PRIMARY KEY,
    block_id INTEGER NOT NULL UNIQUE REFERENCES code_blocks(id) ON DELETE CASCADE,
    vector BLOB NOT NULL,
    model TEXT NOT NULL,
    computed_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_embeddings_block ON embeddings(block_id);
```

Store methods:
- `insert_embedding(block_id: i64, vector: &[f32], model: &str)`
- `get_embedding(block_id: i64) -> Option<Vec<f32>>`
- `get_all_embeddings() -> Vec<(i64, Vec<f32>)>` -- for brute-force cosine search
- `delete_stale_embeddings(model: &str)` -- delete embeddings from a different model
- `get_blocks_without_embeddings(limit: usize) -> Vec<i64>` -- for incremental embedding

**Cosine similarity** (computed in Rust, not SQL):

```rust
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 { return 0.0; }
    dot / (norm_a * norm_b)
}
```

**Scale limits:** Brute-force cosine is O(n) over all embeddings. For 10k blocks with 768-dim vectors: ~30MB memory, ~10ms scan. For 100k blocks: ~300MB, ~100ms. This is acceptable for v1. If repos exceed 100k blocks, ANN (approximate nearest neighbor) becomes a future optimization.

**Hybrid ranking:**

```rust
let alpha = if embeddings_available { 0.4 } else { 1.0 };
let score = (alpha * bm25_normalized) + ((1.0 - alpha) * cosine_score);
```

- BM25 scores normalized to [0, 1] range (divide by max score in result set)
- Alpha = 0.4 gives semantic search 60% weight (better for natural language queries)
- Alpha = 1.0 when no embeddings (pure BM25, exact same behavior as today)

**Embedding lifecycle during indexing:**

1. Parse file, extract symbols, insert code blocks
2. If embeddings enabled and provider healthy: collect new/changed blocks
3. Batch embed (groups of `batch_size`, default 64)
4. Store embeddings in DB
5. On re-index: only re-embed blocks whose content hash changed
6. On model change (detected via `model` column mismatch): re-embed all blocks

**Graceful degradation:**

- Provider unreachable at startup: log warning, disable embeddings for this session
- Provider fails mid-batch: log error, skip remaining embeddings, continue with BM25
- Retry with exponential backoff: 1s, 2s, 4s, max 3 retries per batch
- Health check on startup: single embed call with test text, verify dimensions match config

**Search filter optimization:**

Current search applies language/kind/scope filters post-query in Rust, which means FTS results can be wasted. Push filters into SQL:

```sql
SELECT cb.id, cb.file_id, cb.name, cb.kind, cb.start_line, cb.end_line,
       cb.content, cb.signature, cb.parent_id, f.path, f.language
FROM code_blocks_fts fts
JOIN code_blocks cb ON cb.id = fts.rowid
JOIN files f ON f.id = cb.file_id
WHERE code_blocks_fts MATCH ?1
  AND (?2 IS NULL OR f.language = ?2)
  AND (?3 IS NULL OR cb.kind = ?3)
  AND (?4 IS NULL OR f.path LIKE ?4 || '%')
ORDER BY rank
LIMIT ?5
```

This ensures the LIMIT returns `limit` matching results, not `limit` results minus filtered-out ones.

---

### Pillar 5: Live Reactivity

**Goal**: Index stays current in real-time. Zero manual intervention. Sub-second staleness window.

**File watcher** (`notify` crate, lives in `tokenstunt-index` crate):

The `FileWatcher` is owned by `tokenstunt-index` because it needs direct access to `Indexer` for re-indexing. The CLI's `serve` command creates the watcher and passes it the `Arc<Indexer>`:

```rust
// In tokenstunt-index/src/watcher.rs
pub struct FileWatcher {
    watcher: RecommendedWatcher,
    pending: Arc<Mutex<HashSet<PathBuf>>>,
    debounce_handle: tokio::task::JoinHandle<()>,
}

impl FileWatcher {
    pub fn start(indexer: Arc<Indexer>, root: PathBuf) -> Result<Self> { ... }
    pub fn stop(self) { ... }
}
```

The `Indexer` gains a public `reindex_files(&self, paths: &[PathBuf]) -> Result<ReindexStats>` method that the watcher calls after debouncing.

**Lifecycle during `serve`:**

```
1. Server starts
2. Full reconciliation (diff fs vs DB, re-index only changed files)
3. Start MCP server on stdio
4. Start file watcher on root directory (background tokio task)
5. File change detected
   -> debounce 500ms (batch rapid saves)
   -> collect changed paths
   -> filter to supported languages + gitignore rules
   -> acquire write_conn lock
   -> re-index only changed files (delete old blocks, re-extract, re-insert)
   -> re-extract dependencies for changed files
   -> re-resolve affected dependency edges
   -> invalidate overview cache for affected scopes
   -> release write_conn lock
   -> re-embed changed blocks (if embeddings enabled, async, non-blocking)
   -> log: "re-indexed 2 files (3 blocks updated)"
6. Claude's next tool call reads from read_conn -- never blocked by write
```

**Startup reconciliation:**

```rust
pub fn reconcile(&self, root: &Path, repo_id: i64) -> Result<ReconcileStats> {
    let entries = walker::walk_directory(root)?;
    let mut stats = ReconcileStats::default();
    let mut current_paths = Vec::with_capacity(entries.len());

    // Single transaction for all updates
    self.store.write_transaction(|conn| {
        for entry in &entries {
            let rel_path = entry.path.strip_prefix(root)?.to_string_lossy().to_string();
            current_paths.push(rel_path.clone());

            let source = match std::fs::read_to_string(&entry.path) {
                Ok(s) => s,
                Err(_) => { stats.errors += 1; continue; }
            };
            let content_hash = xxh3_64(source.as_bytes());

            if let Some(existing) = self.store.get_file_hash_with_conn(conn, repo_id, &rel_path)? {
                if existing == content_hash {
                    stats.unchanged += 1;
                    continue;
                }
            }

            self.index_file_with_conn(conn, repo_id, &entry, &source, content_hash)?;
            stats.updated += 1;
        }

        stats.deleted = self.store.delete_stale_files_with_conn(conn, repo_id, &current_paths)?;
        Ok(stats)
    })
}
```

**Dirty propagation:**

When file B changes:
1. B's blocks are deleted (CASCADE deletes their dependencies and embeddings)
2. New blocks for B are inserted with fresh content
3. New dependencies for B's blocks are extracted and inserted
4. Dependencies in OTHER files that pointed to B's old block IDs are now dangling (`target_block_id` is NULL due to ON DELETE SET NULL)
5. Re-resolution pass: for all dependencies where `target_block_id IS NULL AND resolved = 1`, attempt to re-resolve `target_name` against current symbols
6. Overview cache entries for B's directory scope (and parent scopes) are invalidated

**Watcher configuration:**

- Respects `.gitignore` (reuses `ignore` crate's gitignore logic)
- Ignores `.tokenstunt/` directory (avoid self-triggering on DB writes)
- Ignores non-supported file extensions
- Debounce window: 500ms
- Recursive watch on root directory

**Edge cases:**

- File created then immediately deleted: debounce catches both events, net no-op
- File moved/renamed: detected as delete + create, re-indexed under new path
- Branch switch (`git checkout`): many files change at once, batch re-index in single transaction
- Large repo initial index: progress logging every 100 files
- Another tokenstunt instance: SQLite WAL handles this -- second writer will get SQLITE_BUSY, retry with backoff
- Process crash mid-write: WAL recovery handles automatically on next startup

**`delete_stale_files` scaling fix:**

For large repos, the current `NOT IN (?)` with individually bound parameters hits SQLite's default parameter limit (999). Fix: batch into chunks of 500 parameters:

```rust
pub fn delete_stale_files_with_conn(
    &self, conn: &Connection, repo_id: i64, current_paths: &[String],
) -> Result<u64> {
    if current_paths.is_empty() {
        return Ok(0);
    }

    // Collect all file IDs that should be kept
    let mut keep_ids: HashSet<i64> = HashSet::new();
    for chunk in current_paths.chunks(500) {
        let placeholders: String = chunk.iter().enumerate()
            .map(|(i, _)| format!("?{}", i + 2))
            .collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT id FROM files WHERE repo_id = ?1 AND path IN ({})", placeholders
        );
        let mut stmt = conn.prepare(&sql)?;
        // bind repo_id + chunk paths, collect IDs
        // ...
    }

    // Delete files NOT in keep_ids
    let deleted = conn.execute(
        "DELETE FROM files WHERE repo_id = ?1 AND id NOT IN (SELECT value FROM json_each(?2))",
        params![repo_id, serde_json::to_string(&keep_ids.iter().collect::<Vec<_>>())?],
    )?;

    Ok(deleted as u64)
}
```

Alternative (simpler): use SQLite's `json_each()` to pass all paths as a single JSON array parameter, avoiding the parameter limit entirely.

---

### Pillar 6: Honest Tool Descriptions

**Goal**: Every MCP tool description matches exactly what it does.

**Problem with current approach:** The `#[tool(description = "...")]` macro in `rmcp` generates descriptions at compile time. Dynamic descriptions based on config are not possible through the macro.

**Solution:** Do not use the `#[tool]` macro for description. Instead, register tools manually via `ToolRouter::builder()` with runtime-generated descriptions:

```rust
impl TokenStuntServer {
    pub fn new(indexer: Arc<Indexer>, root: PathBuf, config: &Config) -> Self {
        let has_embeddings = config.embeddings.as_ref().is_some_and(|e| e.enabled);

        let search_desc = if has_embeddings {
            "Hybrid semantic + keyword code search. Returns ranked code blocks."
        } else {
            "Keyword code search (BM25). Returns ranked code blocks."
        };

        // Build tool router with dynamic descriptions
        let tool_router = ToolRouter::builder()
            .tool("ts_search", search_desc, Self::ts_search)
            .tool("ts_symbol", "Exact symbol lookup by name.", Self::ts_symbol)
            .tool("ts_context", "Symbol + dependency graph traversal.", Self::ts_context)
            .tool("ts_overview", "Project structure: modules, APIs, entry points.", Self::ts_overview)
            .build();

        Self { indexer, root, tool_router }
    }
}
```

If `rmcp` does not support this builder pattern, fall back to: use accurate static descriptions that don't claim capabilities that may not exist. The description should say "code search" generically, and the response text itself can mention whether semantic search was used.

**Accurate static descriptions (fallback):**

```
ts_search:   "Code search across indexed symbols. Returns ranked code blocks
              (exact function/class/type bodies), not full files."
ts_symbol:   "Exact symbol lookup by name. Returns definition, signature, and location."
ts_context:  "Symbol definition + dependency graph. Shows what this symbol calls
              and what calls it."
ts_overview: "Project structure: module tree, language breakdown, public API surface,
              and entry points."
```

---

### Pillar 7: Production Hardening

**Testing tiers:**

| Tier | What | Target count |
|------|------|-------------|
| Unit | Every public function, edge cases, error paths | ~120 |
| Integration | MCP tool calls through server handler | ~10 |
| Relevance | Search quality assertions on known corpus | ~15 |
| Real-world parsing | Parse non-trivial code per language | ~13 |
| Concurrency | Watcher + query contention, transaction isolation | ~5 |
| Benchmark | Index speed, search latency, re-index speed | 5 |

Tests are continuous, not a separate phase. Every pillar includes its own tests.

**MCP integration tests:**

Test through the `ServerHandler` trait, calling `list_tools` and tool handlers with serialized params:

```rust
#[tokio::test]
async fn test_search_roundtrip() {
    let server = setup_server_with_data();
    // Verify tool exists in list
    let tools = server.list_tools().await;
    assert!(tools.iter().any(|t| t.name == "ts_search"));

    // Call tool handler directly with deserialized params
    let params = Parameters(TsSearchParams { query: "authenticate".into(), ..Default::default() });
    let result = server.ts_search(params).await.unwrap();
    let text = extract_text(&result);
    assert!(text.contains("authenticateUser"));
}
```

**Search relevance tests:**

Index a known corpus of 50+ blocks across 10+ files:

```rust
#[test]
fn test_search_relevance_ranking() {
    let store = build_relevance_corpus(); // 50+ blocks, multiple files
    let engine = SearchEngine::new(&store);

    // Exact name match should rank highest
    let results = engine.search(&SearchQuery { text: "authenticateUser".into(), limit: 5, ..Default::default() }).unwrap();
    assert_eq!(results[0].block.name, "authenticateUser");

    // Keyword match should find relevant symbols
    let results = engine.search(&SearchQuery { text: "authentication".into(), limit: 5, ..Default::default() }).unwrap();
    let names: Vec<&str> = results.iter().map(|r| r.block.name.as_str()).collect();
    assert!(names.contains(&"authenticateUser"), "authenticateUser should appear in top 5 for 'authentication'");
}
```

**Real-world parsing tests:**

One test per language, 30+ line snippet with nested structures:

```rust
#[test]
fn test_parse_rust_real_world() {
    let src = r#"
use std::collections::HashMap;

pub struct Config {
    pub port: u16,
    pub host: String,
}

impl Config {
    pub fn new(port: u16, host: String) -> Self {
        Self { port, host }
    }

    pub fn default_port() -> u16 {
        8080
    }
}

pub trait Service {
    fn start(&self) -> Result<(), Box<dyn std::error::Error>>;
}

pub enum Status {
    Running,
    Stopped,
    Error(String),
}
"#;
    let extractor = make_extractor();
    let result = extractor.extract(src, Language::Rust).unwrap();
    assert!(result.symbols.len() >= 4); // Config, impl methods, Service, Status
    assert!(!result.references.is_empty()); // use std::collections::HashMap
}
```

**Concurrency tests:**

```rust
#[test]
fn test_concurrent_read_write() {
    let store = Store::open_in_memory().unwrap();
    // ... spawn threads: one writing blocks, one reading via search
    // Assert: no panics, no data corruption, reads never block indefinitely
}
```

**Benchmarks** (`benches/`):

Using `criterion` crate:
- `bench_index_1000_files` -- wall time to index 1000 generated files
- `bench_search_latency` -- p50, p95, p99 on 100 queries against 1000-block corpus
- `bench_reindex_10_files` -- time to re-index 10 changed files out of 1000
- `bench_overview_generation` -- cold (no cache) vs warm (cached) overview
- `bench_cosine_similarity` -- time to rank 10k embeddings

---

## Crate Change Summary

| Crate | Changes |
|-------|---------|
| `tokenstunt-parser` | Split extract.rs into per-language modules. Add `ParseResult` with references. Add 9 tree-sitter grammars. Add `LanguageExtractor` trait. Feature gates for P2 languages. |
| `tokenstunt-store` | Fix `transaction` concurrency. Read/write connection split. Add `embeddings` table. Add overview cache methods. Fix `insert_dependency` to accept `Option<i64>`. Add dependency CRUD methods. Fix `delete_stale_files` for large repos. Bump schema to v2. |
| `tokenstunt-index` | Populate dependencies during indexing. Generate overview cache. File watcher (`notify`). Startup reconciliation. `index_file` extracted as reusable unit. |
| `tokenstunt-search` | Push filters into SQL. Hybrid BM25+cosine ranking. `SearchSource` variants for Bm25/Semantic/Hybrid. |
| `tokenstunt-embeddings` | Full implementation: provider trait, Ollama client, OpenAI-compat client. Health check, batch retry. |
| `tokenstunt-server` | Richer `ts_overview` with module tree and public API. Accurate tool descriptions. New `TsOverviewParams` with scope and depth. |
| `tokenstunt` (CLI) | Config file loading (`.tokenstunt/config.toml`). Config struct with serde. Pass config to server/indexer. |

## New Dependencies

| Crate | Dependency | Purpose |
|-------|-----------|---------|
| `tokenstunt-parser` | `tree-sitter-rust` | Rust grammar |
| `tokenstunt-parser` | `tree-sitter-go` | Go grammar |
| `tokenstunt-parser` | `tree-sitter-java` | Java grammar |
| `tokenstunt-parser` | `tree-sitter-c` | C grammar |
| `tokenstunt-parser` | `tree-sitter-cpp` | C++ grammar |
| `tokenstunt-parser` | `tree-sitter-ruby` | Ruby grammar |
| `tokenstunt-parser` | `tree-sitter-swift` | Swift grammar (feature-gated) |
| `tokenstunt-parser` | `tree-sitter-kotlin` | Kotlin grammar (feature-gated) |
| `tokenstunt-parser` | `tree-sitter-dart` | Dart grammar (feature-gated) |
| `tokenstunt-index` | `notify` | File system watcher |
| `tokenstunt-embeddings` | `reqwest` | HTTP client for embedding APIs |
| `tokenstunt-embeddings` | `async-trait` | Async trait support |
| `tokenstunt` | `toml` | Config file parsing |
| workspace (dev) | `criterion` | Benchmarks |

## Phasing

| Phase | Pillars | Delivers |
|-------|---------|----------|
| 0 | Pre-req | Fix Store concurrency, schema migration |
| 1 | 3 + 6 | All 13 languages, honest descriptions |
| 2 | 1 + 2 | Dependency graph, real overview |
| 3 | 5 | File watcher, live reactivity |
| 4 | 4 | Semantic search (embeddings) |
| continuous | 7 | Tests ship with every phase |

Each phase is independently shippable. Phase N does not depend on Phase N+1.

## Non-Goals (Explicitly Out of Scope)

- IDE plugin (VS Code, JetBrains) -- TokenStunt is MCP-only
- Multi-repo support -- one instance per repo
- Cross-language dependency resolution -- deps resolve within same language only
- Custom query language -- natural language and keyword search only
- Cloud/SaaS deployment -- local tool only
- Streaming responses -- MCP tools return complete results
- ANN/HNSW vector index -- brute-force cosine is sufficient for v1 scale
