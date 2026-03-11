# TokenStunt v1 Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the ultimate MCP code intelligence server -- 13 languages, dependency graphs, real overview, semantic search, live file watching -- all automatic, progressive, real-time.

**Architecture:** 6-crate Cargo workspace. Store gets read/write connection split for concurrency. Parser splits into per-language modules behind a trait. Embeddings are opt-in via configurable provider (Ollama/OpenAI-compat). File watcher keeps index fresh in real-time.

**Tech Stack:** Rust, tree-sitter (13 grammars), SQLite FTS5, rmcp (MCP protocol), notify (fs watcher), reqwest (HTTP for embeddings), tokio (async runtime)

**Spec:** `docs/superpowers/specs/2026-03-11-tokenstunt-v1-design.md`

---

## Chunk 1: Phase 0 -- Fix Store Concurrency + Schema Migration

### Task 1: Split Store into read/write connections

**Files:**
- Modify: `crates/tokenstunt-store/src/repo.rs`

- [ ] **Step 1: Write failing test for concurrent read/write**

In `crates/tokenstunt-store/src/repo.rs`, add to `mod tests`:

```rust
#[test]
fn test_read_write_separation() {
    let store = Store::open_in_memory().unwrap();
    let repo_id = store.ensure_repo("/test", "test").unwrap();
    store.upsert_file(repo_id, "a.ts", 1, "typescript", 0).unwrap();
    store.insert_code_block(
        store.upsert_file(repo_id, "b.ts", 2, "typescript", 0).unwrap(),
        "hello", CodeBlockKind::Function, 1, 5,
        "function hello() {}", "function hello()", None,
    ).unwrap();

    // Read while "write transaction" is conceptually happening
    // This should not deadlock
    let count = store.file_count().unwrap();
    assert!(count >= 1);
    let results = store.search_fts("hello", 10).unwrap();
    assert_eq!(results.len(), 1);
}
```

- [ ] **Step 2: Run test to verify it passes (baseline)**

Run: `cargo test -p tokenstunt-store test_read_write_separation`

- [ ] **Step 3: Refactor Store to use read_conn and write_conn**

In `crates/tokenstunt-store/src/repo.rs`, change:

```rust
pub struct Store {
    read_conn: Mutex<Connection>,
    write_conn: Mutex<Connection>,
    db_path: PathBuf,
}
```

Update `open()` and `open_in_memory()` to create two connections to the same DB. For in-memory DBs, use a shared cache URI: `file::memory:?cache=shared`.

Add helper methods:

```rust
fn read_lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
    self.read_conn
        .lock()
        .map_err(|_| anyhow::anyhow!("read mutex poisoned"))
}

fn write_lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
    self.write_conn
        .lock()
        .map_err(|_| anyhow::anyhow!("write mutex poisoned"))
}
```

Update all read-only methods (`search_fts`, `lookup_symbol`, `get_block_by_id`, `get_dependencies`, `get_dependents`, `file_count`, `block_count`, `get_file_hash`) to use `self.read_lock()`.

Update all write methods (`ensure_repo`, `upsert_file`, `delete_file_blocks`, `insert_code_block`, `insert_dependency`, `delete_stale_files`) to use `self.write_lock()`.

- [ ] **Step 4: Run all tests**

Run: `cargo test -p tokenstunt-store`
Expected: All 12 tests pass.

- [ ] **Step 5: Commit**

```
git add crates/tokenstunt-store/src/repo.rs
git commit -m "refactor: split Store into read/write connections for WAL concurrency"
```

---

### Task 2: Fix Store::transaction to hold lock

**Files:**
- Modify: `crates/tokenstunt-store/src/repo.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn test_transaction_holds_lock() {
    let store = Store::open_in_memory().unwrap();
    let repo_id = store.ensure_repo("/test", "test").unwrap();

    // Transaction should pass &Connection to closure
    let result = store.write_transaction(|conn| {
        conn.execute(
            "INSERT INTO files (repo_id, path, content_hash, language, mtime_ns) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![repo_id, "direct.ts", 999i64, "typescript", 0i64],
        )?;
        Ok(())
    });
    assert!(result.is_ok());
    assert_eq!(store.file_count().unwrap(), 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tokenstunt-store test_transaction_holds_lock`
Expected: FAIL -- `write_transaction` method doesn't exist yet.

- [ ] **Step 3: Implement write_transaction**

Replace the existing `transaction` method with `write_transaction`:

```rust
pub fn write_transaction<F, T>(&self, f: F) -> Result<T>
where
    F: FnOnce(&Connection) -> Result<T>,
{
    let conn = self.write_lock()?;
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

Keep the old `transaction` method temporarily with a deprecation path -- update `indexer.rs` to use `write_transaction` and pass `&Connection` through.

- [ ] **Step 4: Run all tests**

Run: `cargo test -p tokenstunt-store`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```
git add crates/tokenstunt-store/src/repo.rs
git commit -m "fix: Store::transaction now holds mutex for entire transaction"
```

---

### Task 3: Add _with_conn variants for write methods

**Files:**
- Modify: `crates/tokenstunt-store/src/repo.rs`

- [ ] **Step 1: Write test using _with_conn inside write_transaction**

```rust
#[test]
fn test_with_conn_variants_in_transaction() {
    let store = Store::open_in_memory().unwrap();
    let repo_id = store.ensure_repo("/test", "test").unwrap();

    store.write_transaction(|conn| {
        let file_id = store.upsert_file_with_conn(conn, repo_id, "a.ts", 1, "typescript", 0)?;
        store.insert_code_block_with_conn(
            conn, file_id, "greet", CodeBlockKind::Function,
            1, 5, "function greet() {}", "function greet()", None,
        )?;
        store.delete_file_blocks_with_conn(conn, file_id)?;
        Ok(())
    }).unwrap();

    assert_eq!(store.block_count().unwrap(), 0);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tokenstunt-store test_with_conn_variants`
Expected: FAIL -- methods don't exist.

- [ ] **Step 3: Extract _with_conn variants**

For each write method, extract the logic into a `pub(crate) fn X_with_conn(&self, conn: &Connection, ...)` variant. The public method becomes a thin wrapper:

```rust
pub fn upsert_file(&self, repo_id: i64, path: &str, content_hash: u64, language: &str, mtime_ns: i64) -> Result<i64> {
    let conn = self.write_lock()?;
    self.upsert_file_with_conn(&conn, repo_id, path, content_hash, language, mtime_ns)
}

pub(crate) fn upsert_file_with_conn(&self, conn: &Connection, repo_id: i64, path: &str, content_hash: u64, language: &str, mtime_ns: i64) -> Result<i64> {
    // existing implementation moved here
}
```

Do this for: `upsert_file`, `get_file_hash`, `delete_file_blocks`, `insert_code_block`, `insert_dependency`, `delete_stale_files`, `lookup_symbol`, `ensure_repo`.

- [ ] **Step 4: Run all tests**

Run: `cargo test`
Expected: All tests pass across all crates.

- [ ] **Step 5: Commit**

```
git add crates/tokenstunt-store/src/repo.rs
git commit -m "refactor: add _with_conn variants for transaction-safe store operations"
```

---

### Task 4: Update indexer to use write_transaction

**Files:**
- Modify: `crates/tokenstunt-index/src/indexer.rs`

- [ ] **Step 1: Refactor index_directory to use write_transaction**

Change `self.store.transaction(|| { ... })` to `self.store.write_transaction(|conn| { ... })`. All store calls inside the closure switch to `_with_conn` variants.

Extract a reusable `index_file_with_conn` method:

```rust
fn index_file_with_conn(
    &self,
    conn: &Connection,
    repo_id: i64,
    entry: &FileEntry,
    source: &str,
    content_hash: u64,
) -> Result<u64> {
    let rel_path = ...; // computed by caller
    let mtime = ...; // computed by caller
    let file_id = self.store.upsert_file_with_conn(conn, repo_id, &rel_path, content_hash, entry.language.as_str(), mtime)?;
    self.store.delete_file_blocks_with_conn(conn, file_id)?;
    let symbols = self.extractor.extract(source, entry.language)?;
    let mut block_count = 0u64;
    for symbol in &symbols {
        self.insert_symbol_with_conn(conn, file_id, symbol, None)?;
        block_count += count_symbols(symbol);
    }
    Ok(block_count)
}
```

- [ ] **Step 2: Run all tests**

Run: `cargo test`
Expected: All tests pass (indexer tests + integration test).

- [ ] **Step 3: Remove old transaction method**

Delete `Store::transaction` (the old version that drops the lock). All callers now use `write_transaction`.

- [ ] **Step 4: Run all tests again**

Run: `cargo test`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```
git add crates/tokenstunt-index/src/indexer.rs crates/tokenstunt-store/src/repo.rs
git commit -m "refactor: indexer uses write_transaction with connection threading"
```

---

### Task 5: Bump schema to v2, add embeddings table

**Files:**
- Modify: `crates/tokenstunt-store/src/schema.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn test_schema_v2_has_embeddings_table() {
    let conn = Connection::open_in_memory().unwrap();
    initialize(&conn).unwrap();
    // Verify embeddings table exists
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='embeddings'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tokenstunt-store test_schema_v2`
Expected: FAIL -- embeddings table doesn't exist.

- [ ] **Step 3: Add embeddings table to schema, bump version**

In `schema.rs`:
- Change `SCHEMA_VERSION` to `2`
- Add to the `execute_batch` string:

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

- [ ] **Step 4: Update schema mismatch test**

The `test_schema_version_mismatch` test sets version to 999. It should still fail with mismatch. Verify.

- [ ] **Step 5: Run all tests**

Run: `cargo test -p tokenstunt-store`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```
git add crates/tokenstunt-store/src/schema.rs
git commit -m "feat: bump schema to v2, add embeddings table"
```

---

## Chunk 2: Phase 1 -- Parser Refactor + 13 Languages

### Task 6: Create extract module structure and helpers

**Files:**
- Create: `crates/tokenstunt-parser/src/extract/mod.rs`
- Create: `crates/tokenstunt-parser/src/extract/helpers.rs`
- Delete: `crates/tokenstunt-parser/src/extract.rs` (replaced by directory module)

- [ ] **Step 1: Create extract directory and helpers.rs**

Extract `child_text_by_field`, `node_text`, `node_text_str` from the current `SymbolExtractor` into free functions in `helpers.rs`:

```rust
// crates/tokenstunt-parser/src/extract/helpers.rs
use tree_sitter::Node;

pub(crate) fn child_text_by_field(node: Node, field: &str, source: &[u8]) -> Option<String> {
    let child = node.child_by_field_name(field)?;
    Some(node_text(child, source))
}

pub(crate) fn node_text(node: Node, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}
```

- [ ] **Step 2: Create mod.rs with ParseResult, RawReference, LanguageExtractor trait**

```rust
// crates/tokenstunt-parser/src/extract/mod.rs
mod helpers;
mod typescript;
mod python;

pub(crate) use helpers::*;

use crate::languages::{Language, LanguageRegistry};
use anyhow::{Context, Result};
use tree_sitter::{Node, Parser};

#[derive(Debug, Clone)]
pub struct ParsedSymbol { /* same as before */ }

#[derive(Debug, Clone)]
pub struct RawReference {
    pub source_symbol: String,
    pub target_name: String,
    pub kind: &'static str,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct ParseResult {
    pub symbols: Vec<ParsedSymbol>,
    pub references: Vec<RawReference>,
}

pub(crate) trait LanguageExtractor {
    fn extract_symbols(&self, root: Node, source: &[u8]) -> Vec<ParsedSymbol>;
    fn extract_references(&self, root: Node, source: &[u8]) -> Vec<RawReference>;
}

pub struct SymbolExtractor { registry: LanguageRegistry }

impl SymbolExtractor {
    pub fn new(registry: LanguageRegistry) -> Self { Self { registry } }

    pub fn extract(&self, source: &str, language: Language) -> Result<ParseResult> {
        let ts_lang = self.registry.get_ts_language(language)?;
        let mut parser = Parser::new();
        parser.set_language(&ts_lang).context("failed to set parser language")?;
        let tree = parser.parse(source, None).context("failed to parse source")?;
        let root = tree.root_node();
        let bytes = source.as_bytes();

        let (symbols, references) = match language {
            Language::TypeScript | Language::Tsx | Language::JavaScript => {
                let ext = typescript::TypeScriptExtractor;
                (ext.extract_symbols(root, bytes), ext.extract_references(root, bytes))
            }
            Language::Python => {
                let ext = python::PythonExtractor;
                (ext.extract_symbols(root, bytes), ext.extract_references(root, bytes))
            }
            _ => (vec![], vec![]),
        };

        Ok(ParseResult { symbols, references })
    }
}
```

- [ ] **Step 3: Move TypeScript extraction to typescript.rs**

Move all `extract_typescript`, `visit_ts_node`, `extract_ts_function`, etc. methods into a `TypeScriptExtractor` struct implementing `LanguageExtractor`. Replace `self.child_text_by_field(...)` calls with `helpers::child_text_by_field(...)`.

Add `extract_references` returning empty vec for now (imports come in Phase 2).

- [ ] **Step 4: Move Python extraction to python.rs**

Same pattern. Move `extract_python`, `visit_py_node`, etc. into `PythonExtractor`.

Add `extract_references` returning empty vec for now.

- [ ] **Step 5: Update lib.rs exports**

In `crates/tokenstunt-parser/src/lib.rs`, change `mod extract;` to point to the new directory module. Ensure `ParseResult`, `ParsedSymbol`, `RawReference`, `SymbolExtractor` are re-exported.

- [ ] **Step 6: Update indexer to handle ParseResult**

In `crates/tokenstunt-index/src/indexer.rs`, change `self.extractor.extract(...)` call site to destructure `ParseResult`:

```rust
let parse_result = self.extractor.extract(&source, entry.language)?;
for symbol in &parse_result.symbols {
    // existing symbol insertion
}
// references ignored for now -- Phase 2
```

- [ ] **Step 7: Run all tests**

Run: `cargo test`
Expected: All existing tests pass with refactored code.

- [ ] **Step 8: Commit**

```
git add crates/tokenstunt-parser/src/ crates/tokenstunt-index/src/indexer.rs
git commit -m "refactor: split parser into per-language modules with LanguageExtractor trait"
```

---

### Task 7: Add Rust language support

**Files:**
- Create: `crates/tokenstunt-parser/src/extract/rust_lang.rs`
- Modify: `crates/tokenstunt-parser/src/extract/mod.rs`
- Modify: `crates/tokenstunt-parser/src/languages.rs`
- Modify: `crates/tokenstunt-parser/Cargo.toml`

- [ ] **Step 1: Add tree-sitter-rust dependency**

In `crates/tokenstunt-parser/Cargo.toml`:

```toml
[dependencies]
tree-sitter-rust = { version = "0.23", optional = true }

[features]
default = ["lang-rust"]
lang-rust = ["dep:tree-sitter-rust"]
```

- [ ] **Step 2: Add Rust grammar to LanguageRegistry**

In `languages.rs`, add `ts_rust: Option<tree_sitter::Language>` field (behind `#[cfg(feature = "lang-rust")]`). Load in `new()`. Return from `get_ts_language()` for `Language::Rust`. Update `is_supported()`.

- [ ] **Step 3: Write failing test**

```rust
#[test]
fn test_rust_function_and_struct() {
    let src = r#"
use std::collections::HashMap;

pub fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}

pub struct Config {
    pub port: u16,
    pub host: String,
}

impl Config {
    pub fn new(port: u16, host: String) -> Self {
        Self { port, host }
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
    let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"greet"));
    assert!(names.contains(&"Config"));
    assert!(names.contains(&"Service"));
    assert!(names.contains(&"Status"));
}
```

- [ ] **Step 4: Run test to verify it fails**

Run: `cargo test -p tokenstunt-parser test_rust_function_and_struct`
Expected: FAIL -- Rust dispatch returns empty vec.

- [ ] **Step 5: Implement RustExtractor**

Create `crates/tokenstunt-parser/src/extract/rust_lang.rs`:

Handle these tree-sitter node types:
- `function_item` -> Function
- `struct_item` -> Struct
- `impl_item` -> Impl (with method children)
- `trait_item` -> Trait
- `enum_item` -> Enum
- `const_item` / `static_item` -> Constant
- `use_declaration` -> (references only, Phase 2)

- [ ] **Step 6: Wire into dispatcher in mod.rs**

Add `Language::Rust => { let ext = rust_lang::RustExtractor; ... }` to the match in `extract()`.

- [ ] **Step 7: Run tests**

Run: `cargo test -p tokenstunt-parser`
Expected: All tests pass including new Rust test.

- [ ] **Step 8: Commit**

```
git add crates/tokenstunt-parser/
git commit -m "feat: add Rust language support with tree-sitter-rust"
```

---

### Task 8: Add Go language support

**Files:**
- Create: `crates/tokenstunt-parser/src/extract/go.rs`
- Modify: `crates/tokenstunt-parser/src/extract/mod.rs`
- Modify: `crates/tokenstunt-parser/src/languages.rs`
- Modify: `crates/tokenstunt-parser/Cargo.toml`

Same pattern as Task 7. Handle:
- `function_declaration` -> Function
- `method_declaration` -> Method
- `type_declaration` with `type_spec` containing `struct_type` -> Struct
- `type_declaration` with `type_spec` containing `interface_type` -> Interface
- `const_declaration` / `var_declaration` -> Constant

- [ ] **Step 1: Add tree-sitter-go dependency + feature flag**
- [ ] **Step 2: Add Go grammar to LanguageRegistry**
- [ ] **Step 3: Write failing test with Go snippet (func, struct, interface)**
- [ ] **Step 4: Implement GoExtractor**
- [ ] **Step 5: Wire into dispatcher**
- [ ] **Step 6: Run tests, verify pass**
- [ ] **Step 7: Commit**

```
git commit -m "feat: add Go language support with tree-sitter-go"
```

---

### Task 9: Add Java language support

Same pattern. Handle:
- `method_declaration` -> Method
- `class_declaration` -> Class (with method children)
- `interface_declaration` -> Interface
- `enum_declaration` -> Enum
- `field_declaration` with `final` -> Constant

- [ ] **Step 1-7: Same pattern as Task 7/8**
- [ ] **Commit:** `feat: add Java language support with tree-sitter-java`

---

### Task 10: Add C/C++ language support

**Files:**
- Create: `crates/tokenstunt-parser/src/extract/c_lang.rs`

Shared extractor for both C and C++. Handle:
- `function_definition` -> Function
- `struct_specifier` / `class_specifier` -> Struct/Class
- `enum_specifier` -> Enum
- `declaration` with `const` -> Constant

- [ ] **Step 1-7: Same pattern, two tests (one C, one C++)**
- [ ] **Commit:** `feat: add C/C++ language support with tree-sitter-c and tree-sitter-cpp`

---

### Task 11: Add Ruby language support

Handle:
- `method` -> Function/Method
- `class` -> Class (with method children)
- `module` -> Module
- `assignment` with uppercase identifier -> Constant

- [ ] **Step 1-7: Same pattern**
- [ ] **Commit:** `feat: add Ruby language support with tree-sitter-ruby`

---

### Task 12: Add Swift language support (feature-gated)

Handle:
- `function_declaration` -> Function
- `class_declaration` / `struct_declaration` -> Class/Struct
- `protocol_declaration` -> Interface
- `enum_declaration` -> Enum

Feature-gated: `#[cfg(feature = "lang-swift")]` on the module and registry entry.

- [ ] **Step 1-7: Same pattern, tests also feature-gated**
- [ ] **Commit:** `feat: add Swift language support (feature-gated)`

---

### Task 13: Add Kotlin language support (feature-gated)

Handle:
- `function_declaration` -> Function
- `class_declaration` -> Class
- `object_declaration` -> Module
- `interface_declaration` -> Interface

- [ ] **Step 1-7: Same pattern**
- [ ] **Commit:** `feat: add Kotlin language support (feature-gated)`

---

### Task 14: Add Dart language support (feature-gated)

Handle:
- `function_signature` + `function_body` -> Function
- `class_definition` -> Class
- `enum_declaration` -> Enum

- [ ] **Step 1-7: Same pattern**
- [ ] **Commit:** `feat: add Dart language support (feature-gated)`

---

### Task 15: Update tool descriptions to be honest

**Files:**
- Modify: `crates/tokenstunt-server/src/tools.rs`

- [ ] **Step 1: Update all #[tool] descriptions**

```rust
#[tool(
    name = "ts_search",
    description = "Code search across indexed symbols. Returns ranked code blocks (exact function/class/type bodies), not full files."
)]

#[tool(
    name = "ts_context",
    description = "Symbol definition + dependency graph. Shows what this symbol calls and what calls it."
)]

#[tool(
    name = "ts_overview",
    description = "Project structure: module tree, language breakdown, public API surface, and entry points."
)]
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p tokenstunt-server`
Expected: All tests pass.

- [ ] **Step 3: Commit**

```
git commit -m "fix: update MCP tool descriptions to match actual capabilities"
```

---

## Chunk 3: Phase 2 -- Dependency Graph + Real Overview

### Task 16: Fix insert_dependency to accept Option<i64>

**Files:**
- Modify: `crates/tokenstunt-store/src/repo.rs`

- [ ] **Step 1: Update insert_dependency signature**

```rust
pub fn insert_dependency(
    &self,
    source_block_id: i64,
    target_block_id: Option<i64>,
    target_name: &str,
    kind: &str,
) -> Result<()> {
    let conn = self.write_lock()?;
    self.insert_dependency_with_conn(&conn, source_block_id, target_block_id, target_name, kind)
}

pub(crate) fn insert_dependency_with_conn(
    &self,
    conn: &Connection,
    source_block_id: i64,
    target_block_id: Option<i64>,
    target_name: &str,
    kind: &str,
) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO dependencies (source_block_id, target_block_id, target_name, kind, resolved)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![source_block_id, target_block_id, target_name, kind, target_block_id.is_some() as i32],
    )?;
    Ok(())
}
```

- [ ] **Step 2: Update callers**

Update `test_dependencies` in `repo.rs` and `setup_server` in `tools.rs` to pass `Some(target_id)` instead of `target_id`.

- [ ] **Step 3: Add test for unresolved dependency**

```rust
#[test]
fn test_unresolved_dependency() {
    let f = setup();
    f.store.insert_dependency(f.block_id, None, "externalFn", "call").unwrap();
    // Should not appear in get_dependencies (target_block_id IS NOT NULL filter)
    let deps = f.store.get_dependencies(f.block_id).unwrap();
    assert!(deps.is_empty());
}
```

- [ ] **Step 4: Run tests, commit**

```
git commit -m "refactor: insert_dependency accepts Option<i64> for unresolved deps"
```

---

### Task 17: Add dependency CRUD methods to Store

**Files:**
- Modify: `crates/tokenstunt-store/src/repo.rs`

- [ ] **Step 1: Write tests**

```rust
#[test]
fn test_delete_block_dependencies() {
    let f = setup();
    let target_id = f.store.insert_code_block(...).unwrap();
    f.store.insert_dependency(f.block_id, Some(target_id), "helper", "call").unwrap();
    f.store.delete_block_dependencies(f.block_id).unwrap();
    let deps = f.store.get_dependencies(f.block_id).unwrap();
    assert!(deps.is_empty());
}

#[test]
fn test_get_unresolved_and_resolve() {
    let f = setup();
    f.store.insert_dependency(f.block_id, None, "unknown", "import").unwrap();
    let unresolved = f.store.get_unresolved_dependencies().unwrap();
    assert_eq!(unresolved.len(), 1);

    let target_id = f.store.insert_code_block(...).unwrap();
    f.store.resolve_dependency(f.block_id, "unknown", target_id).unwrap();
    let unresolved = f.store.get_unresolved_dependencies().unwrap();
    assert!(unresolved.is_empty());
}
```

- [ ] **Step 2: Implement methods**

```rust
pub fn delete_block_dependencies(&self, block_id: i64) -> Result<()>
pub fn get_unresolved_dependencies(&self) -> Result<Vec<(i64, String, String)>>
pub fn resolve_dependency(&self, source_block_id: i64, target_name: &str, target_block_id: i64) -> Result<()>
```

Plus `_with_conn` variants for each.

- [ ] **Step 3: Run tests, commit**

```
git commit -m "feat: add dependency CRUD methods to Store"
```

---

### Task 18: Add import extraction to TypeScript and Python extractors

**Files:**
- Modify: `crates/tokenstunt-parser/src/extract/typescript.rs`
- Modify: `crates/tokenstunt-parser/src/extract/python.rs`

- [ ] **Step 1: Write failing test for TS imports**

```rust
#[test]
fn test_typescript_import_extraction() {
    let src = r#"
import { UserService } from './services';
import { Config } from '../config';

export function handler(req: Request) {
    const service = new UserService();
    return service.handle(req);
}
"#;
    let extractor = make_extractor();
    let result = extractor.extract(src, Language::TypeScript).unwrap();
    let ref_names: Vec<&str> = result.references.iter().map(|r| r.target_name.as_str()).collect();
    assert!(ref_names.contains(&"UserService"));
    assert!(ref_names.contains(&"Config"));
}
```

- [ ] **Step 2: Implement TypeScript import extraction**

In `typescript.rs`, implement `extract_references`:
- Walk tree for `import_statement` nodes
- Extract imported names from `import_clause` / `named_imports`
- Create `RawReference` with kind = "import"

- [ ] **Step 3: Write failing test for Python imports**

```rust
#[test]
fn test_python_import_extraction() {
    let src = r#"
from services import UserService
import config

def handler(request):
    service = UserService()
    return service.handle(request)
"#;
    let result = make_extractor().extract(src, Language::Python).unwrap();
    let ref_names: Vec<&str> = result.references.iter().map(|r| r.target_name.as_str()).collect();
    assert!(ref_names.contains(&"UserService"));
    assert!(ref_names.contains(&"config"));
}
```

- [ ] **Step 4: Implement Python import extraction**

Handle `import_statement` and `import_from_statement` nodes.

- [ ] **Step 5: Add import extraction to all other language extractors (P0)**

Each extractor gets basic import extraction for its language's import syntax. Return empty vec if the language has no import mechanism (C's #include is technically a preprocessor directive -- still extract it).

- [ ] **Step 6: Run tests, commit**

```
git commit -m "feat: extract import references from TypeScript and Python"
```

---

### Task 19: Populate dependencies during indexing

**Files:**
- Modify: `crates/tokenstunt-index/src/indexer.rs`

- [ ] **Step 1: Write test**

```rust
#[test]
fn test_index_populates_dependencies() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();

    std::fs::write(src.join("service.ts"), "export class UserService { handle() {} }").unwrap();
    std::fs::write(src.join("handler.ts"), "import { UserService } from './service';\nexport function handler() { const s = new UserService(); }").unwrap();

    let store = Store::open_in_memory().unwrap();
    let indexer = Indexer::new(store).unwrap();
    indexer.index_directory(dir.path()).unwrap();

    // handler.ts should have a dependency referencing "UserService"
    let handler_blocks = indexer.store().lookup_symbol("handler", None).unwrap();
    assert!(!handler_blocks.is_empty());
    // Check dependencies exist (even if unresolved for cross-file)
}
```

- [ ] **Step 2: Update index_file_with_conn to process references**

After inserting symbols, iterate `parse_result.references`, call `store.insert_dependency_with_conn`. Try to resolve target via `lookup_symbol_with_conn`.

- [ ] **Step 3: Run tests, commit**

```
git commit -m "feat: populate dependencies table during indexing"
```

---

### Task 20: Add overview Store methods

**Files:**
- Modify: `crates/tokenstunt-store/src/repo.rs`

- [ ] **Step 1: Write tests for language_stats and directory_stats**

```rust
#[test]
fn test_language_stats() {
    let store = Store::open_in_memory().unwrap();
    let repo_id = store.ensure_repo("/test", "test").unwrap();
    store.upsert_file(repo_id, "a.ts", 1, "typescript", 0).unwrap();
    store.upsert_file(repo_id, "b.ts", 2, "typescript", 0).unwrap();
    store.upsert_file(repo_id, "c.py", 3, "python", 0).unwrap();

    let stats = store.get_language_stats().unwrap();
    assert!(stats.iter().any(|(lang, count)| lang == "typescript" && *count == 2));
    assert!(stats.iter().any(|(lang, count)| lang == "python" && *count == 1));
}
```

- [ ] **Step 2: Implement get_language_stats, get_directory_stats, get_exported_symbols**

```rust
pub fn get_language_stats(&self) -> Result<Vec<(String, i64)>> {
    let conn = self.read_lock()?;
    let mut stmt = conn.prepare(
        "SELECT language, COUNT(*) FROM files GROUP BY language ORDER BY COUNT(*) DESC"
    )?;
    // ...
}

pub fn get_directory_stats(&self, scope: Option<&str>) -> Result<Vec<(String, i64, i64)>> {
    // Group files by directory prefix, count files and blocks per directory
}

pub fn get_exported_symbols(&self, scope: Option<&str>) -> Result<Vec<CodeBlock>> {
    // Select code_blocks where parent_id IS NULL (top-level symbols)
    // Optionally filtered by file path prefix (scope)
}
```

- [ ] **Step 3: Implement overview cache methods**

```rust
pub fn get_overview_cache(&self, scope: &str, depth: i32) -> Result<Option<String>>
pub fn set_overview_cache(&self, scope: &str, depth: i32, content: &str) -> Result<()>
pub fn invalidate_overview_cache(&self, scope: &str) -> Result<()>
```

- [ ] **Step 4: Run tests, commit**

```
git commit -m "feat: add overview query and cache methods to Store"
```

---

### Task 21: Implement rich ts_overview

**Files:**
- Modify: `crates/tokenstunt-server/src/tools.rs`

- [ ] **Step 1: Update TsOverviewParams**

```rust
#[derive(Deserialize, schemars::JsonSchema)]
struct TsOverviewParams {
    scope: Option<String>,
    depth: Option<i32>,
}
```

- [ ] **Step 2: Implement overview generation**

Replace the current `ts_overview` handler. Build the markdown output:
1. Language stats
2. Module structure (directory groupings with file/block counts)
3. Public API (top-level symbols, limited to 20)
4. Entry points (files named main.*, index.*, app.*, etc.)

Check cache first, generate and cache if miss.

- [ ] **Step 3: Update tests**

Update `test_ts_overview` to verify the new output format contains language stats and module structure.

- [ ] **Step 4: Run tests, commit**

```
git commit -m "feat: ts_overview returns module tree, public API, entry points"
```

---

## Chunk 4: Phase 3 -- Live Reactivity

### Task 22: Add startup reconciliation

**Files:**
- Modify: `crates/tokenstunt-index/src/indexer.rs`

- [ ] **Step 1: Write test**

```rust
#[test]
fn test_reconcile_detects_changes() {
    let dir = tempfile::tempdir().unwrap();
    write_test_files(dir.path());

    let store = Store::open_in_memory().unwrap();
    let indexer = Indexer::new(store).unwrap();

    // First index
    indexer.index_directory(dir.path()).unwrap();
    let initial_count = indexer.store().block_count().unwrap();

    // Modify a file
    std::fs::write(dir.path().join("src/greet.ts"), "export function greet2() { return 'hi'; }").unwrap();

    // Reconcile should detect the change
    let repo_id = indexer.store().ensure_repo(dir.path().to_str().unwrap(), "test").unwrap();
    let stats = indexer.reconcile(dir.path(), repo_id).unwrap();
    assert!(stats.updated >= 1);
}
```

- [ ] **Step 2: Implement reconcile method**

Extract `index_file_with_conn` as a standalone method. Implement `reconcile()` per the spec: walk, hash, compare, re-index changed, delete stale.

- [ ] **Step 3: Run tests, commit**

```
git commit -m "feat: add startup reconciliation for incremental re-indexing"
```

---

### Task 23: Implement file watcher

**Files:**
- Create: `crates/tokenstunt-index/src/watcher.rs`
- Modify: `crates/tokenstunt-index/src/lib.rs`
- Modify: `crates/tokenstunt-index/Cargo.toml`

- [ ] **Step 1: Add notify dependency**

In `crates/tokenstunt-index/Cargo.toml`:
```toml
notify = "7"
```

- [ ] **Step 2: Implement FileWatcher**

```rust
pub struct FileWatcher {
    _watcher: RecommendedWatcher,
    stop_tx: tokio::sync::oneshot::Sender<()>,
}

impl FileWatcher {
    pub fn start(indexer: Arc<Indexer>, root: PathBuf) -> Result<Self> {
        // Create notify watcher
        // On event: collect changed paths into pending set
        // Spawn tokio task: every 500ms, drain pending set, call indexer.reindex_files()
    }
}
```

- [ ] **Step 3: Add reindex_files to Indexer**

```rust
pub fn reindex_files(&self, root: &Path, paths: &[PathBuf]) -> Result<ReindexStats> {
    // For each path: read, hash, compare, re-index if changed
    // Invalidate overview cache for affected scopes
}
```

- [ ] **Step 4: Write test**

```rust
#[tokio::test]
async fn test_watcher_detects_file_change() {
    let dir = tempfile::tempdir().unwrap();
    write_test_files(dir.path());

    let store = Store::open_in_memory().unwrap();
    let indexer = Arc::new(Indexer::new(store).unwrap());
    indexer.index_directory(dir.path()).unwrap();

    let _watcher = FileWatcher::start(Arc::clone(&indexer), dir.path().to_path_buf()).unwrap();

    // Write a new file
    std::fs::write(dir.path().join("src/new.ts"), "export function newFn() {}").unwrap();

    // Wait for debounce
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    // Should be indexed
    let results = indexer.store().lookup_symbol("newFn", None).unwrap();
    assert!(!results.is_empty());
}
```

- [ ] **Step 5: Run tests, commit**

```
git commit -m "feat: add file watcher for live reactive indexing"
```

---

### Task 24: Integrate watcher into serve command

**Files:**
- Modify: `crates/tokenstunt/src/main.rs`

- [ ] **Step 1: Update serve command**

After indexing, before starting MCP server, start the file watcher:

```rust
// Reconcile first
let repo_id = indexer.store().ensure_repo(root_str, repo_name)?;
let stats = indexer.reconcile(&root, repo_id)?;
info!(updated = stats.updated, unchanged = stats.unchanged, "reconciliation complete");

// Start watcher
let _watcher = tokenstunt_index::FileWatcher::start(Arc::clone(&indexer), root.clone())?;
info!("file watcher started");

// Start MCP server (existing code)
```

- [ ] **Step 2: Run build**

Run: `cargo build`
Expected: Compiles.

- [ ] **Step 3: Commit**

```
git commit -m "feat: integrate file watcher into serve command"
```

---

## Chunk 5: Phase 4 -- Semantic Search

### Task 25: Config loading

**Files:**
- Modify: `crates/tokenstunt/src/main.rs`
- Modify: `crates/tokenstunt/Cargo.toml`

- [ ] **Step 1: Add toml + serde dependency**

- [ ] **Step 2: Define Config struct**

```rust
#[derive(Debug, Deserialize, Default)]
pub struct Config {
    pub embeddings: Option<EmbeddingsConfig>,
}

#[derive(Debug, Deserialize)]
pub struct EmbeddingsConfig {
    pub enabled: bool,
    pub provider: String,
    pub model: String,
    pub endpoint: String,
    pub api_key: Option<String>,
    pub dimensions: usize,
    pub batch_size: Option<usize>,
}

impl Config {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(".tokenstunt/config.toml");
        if !path.exists() { return Ok(Self::default()); }
        let content = std::fs::read_to_string(&path)?;
        Ok(toml::from_str(&content)?)
    }
}
```

- [ ] **Step 3: Load config in serve/index commands**
- [ ] **Step 4: Commit**

```
git commit -m "feat: add config file loading from .tokenstunt/config.toml"
```

---

### Task 26: EmbeddingProvider trait + Ollama client

**Files:**
- Modify: `crates/tokenstunt-embeddings/src/lib.rs`
- Create: `crates/tokenstunt-embeddings/src/ollama.rs`
- Modify: `crates/tokenstunt-embeddings/Cargo.toml`

- [ ] **Step 1: Add dependencies (reqwest, async-trait, serde, serde_json)**

- [ ] **Step 2: Define EmbeddingProvider trait**

```rust
#[async_trait::async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    fn dimensions(&self) -> usize;
    async fn health_check(&self) -> Result<()>;
}
```

- [ ] **Step 3: Implement OllamaProvider**

```rust
pub struct OllamaProvider {
    client: reqwest::Client,
    endpoint: String,
    model: String,
    dimensions: usize,
}
```

Call Ollama's `/api/embed` endpoint. Parse response. Handle batch by sending each text individually (Ollama doesn't batch natively).

- [ ] **Step 4: Write test (requires Ollama running -- mark #[ignore] for CI)**
- [ ] **Step 5: Commit**

```
git commit -m "feat: implement EmbeddingProvider trait and Ollama client"
```

---

### Task 27: OpenAI-compat client

**Files:**
- Create: `crates/tokenstunt-embeddings/src/openai.rs`

- [ ] **Step 1: Implement OpenAiCompatProvider**

Call `/v1/embeddings` endpoint. Supports batching natively.

- [ ] **Step 2: Add load_provider factory**

```rust
pub fn load_provider(config: &EmbeddingsConfig) -> Result<Box<dyn EmbeddingProvider>> {
    match config.provider.as_str() {
        "ollama" => Ok(Box::new(OllamaProvider::new(...))),
        "openai-compat" => Ok(Box::new(OpenAiCompatProvider::new(...))),
        other => bail!("unknown embedding provider: {other}"),
    }
}
```

- [ ] **Step 3: Commit**

```
git commit -m "feat: add OpenAI-compatible embedding client"
```

---

### Task 28: Embedding storage in Store

**Files:**
- Modify: `crates/tokenstunt-store/src/repo.rs`

- [ ] **Step 1: Write tests**

```rust
#[test]
fn test_embedding_storage() {
    let store = Store::open_in_memory().unwrap();
    let repo_id = store.ensure_repo("/test", "test").unwrap();
    let file_id = store.upsert_file(repo_id, "a.ts", 1, "typescript", 0).unwrap();
    let block_id = store.insert_code_block(file_id, "fn", CodeBlockKind::Function, 1, 5, "fn() {}", "fn()", None).unwrap();

    let vec = vec![0.1f32, 0.2, 0.3];
    store.insert_embedding(block_id, &vec, "nomic-embed-text").unwrap();

    let retrieved = store.get_embedding(block_id).unwrap();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().len(), 3);

    let all = store.get_all_embeddings().unwrap();
    assert_eq!(all.len(), 1);
}
```

- [ ] **Step 2: Implement embedding store methods**

Serialize `&[f32]` as little-endian bytes for BLOB storage. Deserialize on read.

- [ ] **Step 3: Run tests, commit**

```
git commit -m "feat: add embedding storage methods to Store"
```

---

### Task 29: Push search filters into SQL

**Files:**
- Modify: `crates/tokenstunt-store/src/repo.rs`
- Modify: `crates/tokenstunt-search/src/lib.rs`

- [ ] **Step 1: Update search_fts to accept filters**

```rust
pub fn search_fts(
    &self,
    query: &str,
    language: Option<&str>,
    kind: Option<&str>,
    scope: Option<&str>,
    limit: usize,
) -> Result<Vec<CodeBlock>>
```

Use the SQL from the spec with `?2 IS NULL OR f.language = ?2` pattern.

- [ ] **Step 2: Update SearchEngine to pass filters through**

Remove post-query filtering in `search/lib.rs`. Pass filters directly to `store.search_fts(...)`.

- [ ] **Step 3: Run tests, verify existing search tests pass**
- [ ] **Step 4: Commit**

```
git commit -m "perf: push search filters into SQL for correct LIMIT behavior"
```

---

### Task 30: Hybrid BM25 + cosine ranking

**Files:**
- Modify: `crates/tokenstunt-search/src/lib.rs`

- [ ] **Step 1: Add cosine_similarity function**
- [ ] **Step 2: Add SearchSource variants back (Semantic, Hybrid)**
- [ ] **Step 3: Update SearchEngine to accept optional embeddings**

```rust
pub struct SearchEngine<'a> {
    store: &'a Store,
    embedder: Option<&'a dyn EmbeddingProvider>,
}
```

When embedder is present and query can be embedded: compute cosine scores, merge with BM25 using alpha=0.4.

When embedder is absent: pure BM25 (alpha=1.0).

- [ ] **Step 4: Write test for hybrid ranking**
- [ ] **Step 5: Commit**

```
git commit -m "feat: hybrid BM25 + cosine ranking with configurable alpha"
```

---

### Task 31: Integrate embeddings into indexer

**Files:**
- Modify: `crates/tokenstunt-index/src/indexer.rs`

- [ ] **Step 1: Update Indexer::new to accept optional embedder**

```rust
pub fn new(store: Store, embedder: Option<Arc<dyn EmbeddingProvider>>) -> Result<Self>
```

- [ ] **Step 2: After indexing symbols, batch embed new blocks**

After `index_file_with_conn`, collect block IDs. After transaction commits, batch embed asynchronously.

- [ ] **Step 3: Update CLI to thread embedder through**
- [ ] **Step 4: Run tests, commit**

```
git commit -m "feat: integrate embedding computation into indexing pipeline"
```

---

### Task 32: Final integration test

**Files:**
- Modify: `crates/tokenstunt/tests/integration.rs`

- [ ] **Step 1: Add end-to-end test**

Create a temp dir with files in multiple languages. Index. Search. Verify symbols found. Verify overview has language stats. Verify dependencies populated.

- [ ] **Step 2: Run full test suite**

Run: `cargo test`
Expected: All tests pass, zero warnings.

- [ ] **Step 3: Run build check**

Run: `cargo build 2>&1 | grep -c "warning\|error"`
Expected: 0

- [ ] **Step 4: Commit**

```
git commit -m "test: add comprehensive end-to-end integration test"
```
