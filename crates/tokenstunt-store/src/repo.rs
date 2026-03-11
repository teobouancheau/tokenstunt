use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::models::{CodeBlock, CodeBlockKind};
use crate::schema;

pub struct Store {
    read_conn: Mutex<Connection>,
    write_conn: Mutex<Connection>,
    db_path: PathBuf,
    is_temp: bool,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let db_path = path.to_path_buf();
        let write_conn = Connection::open(path)
            .with_context(|| format!("failed to open database at {}", path.display()))?;
        schema::initialize(&write_conn)?;
        let read_conn = Connection::open(path)
            .with_context(|| format!("failed to open read connection at {}", path.display()))?;
        read_conn.execute_batch("PRAGMA journal_mode = WAL;")?;
        read_conn.execute_batch("PRAGMA synchronous = NORMAL;")?;
        read_conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(Self {
            read_conn: Mutex::new(read_conn),
            write_conn: Mutex::new(write_conn),
            db_path,
            is_temp: false,
        })
    }

    pub fn open_in_memory() -> Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let db_path =
            std::env::temp_dir().join(format!("tokenstunt_mem_{}_{id}.db", std::process::id()));
        let _ = std::fs::remove_file(&db_path);
        let mut store = Self::open(&db_path)?;
        store.is_temp = true;
        Ok(store)
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

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

    pub fn ensure_repo(&self, path: &str, name: &str) -> Result<i64> {
        let conn = self.write_lock()?;
        self.ensure_repo_with_conn(&conn, path, name)
    }

    pub fn ensure_repo_with_conn(
        &self,
        conn: &Connection,
        path: &str,
        name: &str,
    ) -> Result<i64> {
        conn.execute(
            "INSERT OR IGNORE INTO repos (path, name) VALUES (?1, ?2)",
            params![path, name],
        )?;
        let id: i64 = conn.query_row(
            "SELECT id FROM repos WHERE path = ?1",
            params![path],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    pub fn upsert_file(
        &self,
        repo_id: i64,
        path: &str,
        content_hash: u64,
        language: &str,
        mtime_ns: i64,
    ) -> Result<i64> {
        let conn = self.write_lock()?;
        self.upsert_file_with_conn(&conn, repo_id, path, content_hash, language, mtime_ns)
    }

    pub fn upsert_file_with_conn(
        &self,
        conn: &Connection,
        repo_id: i64,
        path: &str,
        content_hash: u64,
        language: &str,
        mtime_ns: i64,
    ) -> Result<i64> {
        conn.execute(
            "INSERT INTO files (repo_id, path, content_hash, language, mtime_ns)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(repo_id, path) DO UPDATE SET
                content_hash = excluded.content_hash,
                language = excluded.language,
                mtime_ns = excluded.mtime_ns",
            params![repo_id, path, content_hash as i64, language, mtime_ns],
        )?;
        let id: i64 = conn.query_row(
            "SELECT id FROM files WHERE repo_id = ?1 AND path = ?2",
            params![repo_id, path],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    pub fn get_file_hash(&self, repo_id: i64, path: &str) -> Result<Option<u64>> {
        let conn = self.read_lock()?;
        self.get_file_hash_with_conn(&conn, repo_id, path)
    }

    pub fn get_file_hash_with_conn(
        &self,
        conn: &Connection,
        repo_id: i64,
        path: &str,
    ) -> Result<Option<u64>> {
        let result = conn.query_row(
            "SELECT content_hash FROM files WHERE repo_id = ?1 AND path = ?2",
            params![repo_id, path],
            |row| {
                let hash: i64 = row.get(0)?;
                Ok(hash as u64)
            },
        );
        match result {
            Ok(hash) => Ok(Some(hash)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn delete_file_blocks(&self, file_id: i64) -> Result<()> {
        let conn = self.write_lock()?;
        self.delete_file_blocks_with_conn(&conn, file_id)
    }

    pub fn delete_file_blocks_with_conn(
        &self,
        conn: &Connection,
        file_id: i64,
    ) -> Result<()> {
        conn.execute(
            "DELETE FROM code_blocks WHERE file_id = ?1",
            params![file_id],
        )?;
        Ok(())
    }

    pub fn insert_code_block(
        &self,
        file_id: i64,
        name: &str,
        kind: CodeBlockKind,
        start_line: u32,
        end_line: u32,
        content: &str,
        signature: &str,
        parent_id: Option<i64>,
    ) -> Result<i64> {
        let conn = self.write_lock()?;
        self.insert_code_block_with_conn(
            &conn, file_id, name, kind, start_line, end_line, content, signature, parent_id,
        )
    }

    pub fn insert_code_block_with_conn(
        &self,
        conn: &Connection,
        file_id: i64,
        name: &str,
        kind: CodeBlockKind,
        start_line: u32,
        end_line: u32,
        content: &str,
        signature: &str,
        parent_id: Option<i64>,
    ) -> Result<i64> {
        conn.execute(
            "INSERT INTO code_blocks (file_id, name, kind, start_line, end_line, content, signature, parent_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                file_id,
                name,
                kind.as_str(),
                start_line,
                end_line,
                content,
                signature,
                parent_id,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn search_fts(&self, query: &str, limit: usize) -> Result<Vec<CodeBlock>> {
        let conn = self.read_lock()?;
        let mut stmt = conn.prepare(
            "SELECT cb.id, cb.file_id, cb.name, cb.kind, cb.start_line, cb.end_line,
                    cb.content, cb.signature, cb.parent_id, f.path, f.language
             FROM code_blocks_fts fts
             JOIN code_blocks cb ON cb.id = fts.rowid
             JOIN files f ON f.id = cb.file_id
             WHERE code_blocks_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;

        let blocks = stmt
            .query_map(params![query, limit as i64], |row| {
                let kind_str: String = row.get(3)?;
                Ok(CodeBlock {
                    id: row.get(0)?,
                    file_id: row.get(1)?,
                    name: row.get(2)?,
                    kind: CodeBlockKind::from_str(&kind_str).unwrap_or(CodeBlockKind::Function),
                    start_line: row.get(4)?,
                    end_line: row.get(5)?,
                    content: row.get(6)?,
                    signature: row.get(7)?,
                    parent_id: row.get(8)?,
                    file_path: row.get(9)?,
                    language: row.get(10)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(blocks)
    }

    pub fn lookup_symbol(
        &self,
        name: &str,
        kind: Option<CodeBlockKind>,
    ) -> Result<Vec<CodeBlock>> {
        let conn = self.read_lock()?;
        self.lookup_symbol_with_conn(&conn, name, kind)
    }

    pub fn lookup_symbol_with_conn(
        &self,
        conn: &Connection,
        name: &str,
        kind: Option<CodeBlockKind>,
    ) -> Result<Vec<CodeBlock>> {
        if let Some(k) = kind {
            let kind_filter = k.as_str();
            let mut stmt = conn.prepare(
                "SELECT cb.id, cb.file_id, cb.name, cb.kind, cb.start_line, cb.end_line,
                        cb.content, cb.signature, cb.parent_id, f.path, f.language
                 FROM code_blocks cb
                 JOIN files f ON f.id = cb.file_id
                 WHERE cb.name = ?1 AND cb.kind = ?2
                 LIMIT 20",
            )?;
            let rows = stmt
                .query_map(params![name, kind_filter], Self::map_code_block)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        } else {
            let mut stmt = conn.prepare(
                "SELECT cb.id, cb.file_id, cb.name, cb.kind, cb.start_line, cb.end_line,
                        cb.content, cb.signature, cb.parent_id, f.path, f.language
                 FROM code_blocks cb
                 JOIN files f ON f.id = cb.file_id
                 WHERE cb.name = ?1
                 LIMIT 20",
            )?;
            let rows = stmt
                .query_map(params![name], Self::map_code_block)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }
    }

    pub fn get_block_by_id(&self, block_id: i64) -> Result<Option<CodeBlock>> {
        let conn = self.read_lock()?;
        let result = conn.query_row(
            "SELECT cb.id, cb.file_id, cb.name, cb.kind, cb.start_line, cb.end_line,
                    cb.content, cb.signature, cb.parent_id, f.path, f.language
             FROM code_blocks cb
             JOIN files f ON f.id = cb.file_id
             WHERE cb.id = ?1",
            params![block_id],
            Self::map_code_block,
        );
        match result {
            Ok(block) => Ok(Some(block)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_dependents(&self, block_id: i64) -> Result<Vec<(CodeBlock, String)>> {
        let conn = self.read_lock()?;
        let mut stmt = conn.prepare(
            "SELECT cb.id, cb.file_id, cb.name, cb.kind, cb.start_line, cb.end_line,
                    cb.content, cb.signature, cb.parent_id, f.path, f.language, d.kind
             FROM dependencies d
             JOIN code_blocks cb ON cb.id = d.source_block_id
             JOIN files f ON f.id = cb.file_id
             WHERE d.target_block_id = ?1",
        )?;
        let rows = stmt
            .query_map(params![block_id], |row| {
                let dep_kind: String = row.get(11)?;
                Ok((Self::map_code_block(row)?, dep_kind))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_dependencies(&self, block_id: i64) -> Result<Vec<(CodeBlock, String)>> {
        let conn = self.read_lock()?;
        let mut stmt = conn.prepare(
            "SELECT cb.id, cb.file_id, cb.name, cb.kind, cb.start_line, cb.end_line,
                    cb.content, cb.signature, cb.parent_id, f.path, f.language, d.kind
             FROM dependencies d
             JOIN code_blocks cb ON cb.id = d.target_block_id
             JOIN files f ON f.id = cb.file_id
             WHERE d.source_block_id = ?1 AND d.target_block_id IS NOT NULL",
        )?;
        let rows = stmt
            .query_map(params![block_id], |row| {
                let dep_kind: String = row.get(11)?;
                Ok((Self::map_code_block(row)?, dep_kind))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn insert_dependency(
        &self,
        source_block_id: i64,
        target_block_id: i64,
        target_name: &str,
        kind: &str,
    ) -> Result<()> {
        let conn = self.write_lock()?;
        self.insert_dependency_with_conn(&conn, source_block_id, target_block_id, target_name, kind)
    }

    pub fn insert_dependency_with_conn(
        &self,
        conn: &Connection,
        source_block_id: i64,
        target_block_id: i64,
        target_name: &str,
        kind: &str,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO dependencies (source_block_id, target_block_id, target_name, kind, resolved)
             VALUES (?1, ?2, ?3, ?4, 1)",
            params![source_block_id, target_block_id, target_name, kind],
        )?;
        Ok(())
    }

    pub fn file_count(&self) -> Result<i64> {
        let conn = self.read_lock()?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))?;
        Ok(count)
    }

    pub fn block_count(&self) -> Result<i64> {
        let conn = self.read_lock()?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM code_blocks", [], |row| row.get(0))?;
        Ok(count)
    }

    pub fn delete_stale_files(&self, repo_id: i64, current_paths: &[String]) -> Result<u64> {
        let conn = self.write_lock()?;
        self.delete_stale_files_with_conn(&conn, repo_id, current_paths)
    }

    pub fn delete_stale_files_with_conn(
        &self,
        conn: &Connection,
        repo_id: i64,
        current_paths: &[String],
    ) -> Result<u64> {
        if current_paths.is_empty() {
            return Ok(0);
        }

        let placeholders: String = current_paths
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 2))
            .collect::<Vec<_>>()
            .join(",");

        let sql = format!(
            "DELETE FROM files WHERE repo_id = ?1 AND path NOT IN ({})",
            placeholders
        );

        let mut stmt = conn.prepare(&sql)?;
        let mut param_idx = 1;
        stmt.raw_bind_parameter(param_idx, repo_id)?;
        for path in current_paths {
            param_idx += 1;
            stmt.raw_bind_parameter(param_idx, path.as_str())?;
        }

        let deleted = stmt.raw_execute()?;
        Ok(deleted as u64)
    }

    fn map_code_block(row: &rusqlite::Row<'_>) -> rusqlite::Result<CodeBlock> {
        let kind_str: String = row.get(3)?;
        Ok(CodeBlock {
            id: row.get(0)?,
            file_id: row.get(1)?,
            name: row.get(2)?,
            kind: CodeBlockKind::from_str(&kind_str).unwrap_or(CodeBlockKind::Function),
            start_line: row.get(4)?,
            end_line: row.get(5)?,
            content: row.get(6)?,
            signature: row.get(7)?,
            parent_id: row.get(8)?,
            file_path: row.get(9)?,
            language: row.get(10)?,
        })
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        if self.is_temp {
            let _ = std::fs::remove_file(&self.db_path);
            let wal = self.db_path.with_extension("db-wal");
            let shm = self.db_path.with_extension("db-shm");
            let _ = std::fs::remove_file(wal);
            let _ = std::fs::remove_file(shm);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestFixture {
        store: Store,
        repo_id: i64,
        file_id: i64,
        block_id: i64,
    }

    fn setup() -> TestFixture {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        let file_id = store
            .upsert_file(repo_id, "src/main.ts", 12345, "typescript", 0)
            .unwrap();
        let block_id = store
            .insert_code_block(
                file_id,
                "greet",
                CodeBlockKind::Function,
                1,
                5,
                "function greet(name: string) { return `Hello ${name}`; }",
                "function greet(name: string): string",
                None,
            )
            .unwrap();
        TestFixture {
            store,
            repo_id,
            file_id,
            block_id,
        }
    }

    #[test]
    fn test_store_roundtrip() {
        let f = setup();
        let results = f.store.lookup_symbol("greet", None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, f.block_id);
        assert_eq!(results[0].name, "greet");
    }

    #[test]
    fn test_fts_search() {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        let file_id = store
            .upsert_file(repo_id, "src/auth.ts", 111, "typescript", 0)
            .unwrap();
        store
            .insert_code_block(
                file_id,
                "authenticateUser",
                CodeBlockKind::Function,
                1,
                10,
                "function authenticateUser(token: string): User { ... }",
                "function authenticateUser(token: string): User",
                None,
            )
            .unwrap();
        let results = store.search_fts("authenticate*", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "authenticateUser");
    }

    #[test]
    fn test_file_hash_roundtrip() {
        let f = setup();

        let hash = f.store.get_file_hash(f.repo_id, "src/main.ts").unwrap();
        assert_eq!(hash, Some(12345));

        let miss = f.store.get_file_hash(f.repo_id, "nonexistent.ts").unwrap();
        assert_eq!(miss, None);
    }

    #[test]
    fn test_delete_file_blocks() {
        let f = setup();
        assert_eq!(f.store.block_count().unwrap(), 1);

        f.store.delete_file_blocks(f.file_id).unwrap();
        assert_eq!(f.store.block_count().unwrap(), 0);
    }

    #[test]
    fn test_block_by_id() {
        let f = setup();

        let block = f.store.get_block_by_id(f.block_id).unwrap();
        assert!(block.is_some());
        assert_eq!(block.unwrap().name, "greet");

        let miss = f.store.get_block_by_id(99999).unwrap();
        assert!(miss.is_none());
    }

    #[test]
    fn test_dependencies() {
        let f = setup();
        let target_id = f
            .store
            .insert_code_block(
                f.file_id,
                "helper",
                CodeBlockKind::Function,
                10,
                15,
                "function helper() {}",
                "function helper()",
                None,
            )
            .unwrap();

        f.store
            .insert_dependency(f.block_id, target_id, "helper", "call")
            .unwrap();

        let deps = f.store.get_dependencies(f.block_id).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].0.name, "helper");
        assert_eq!(deps[0].1, "call");

        let dependents = f.store.get_dependents(target_id).unwrap();
        assert_eq!(dependents.len(), 1);
        assert_eq!(dependents[0].0.name, "greet");
    }

    #[test]
    fn test_counts() {
        let f = setup();
        assert_eq!(f.store.file_count().unwrap(), 1);
        assert_eq!(f.store.block_count().unwrap(), 1);

        f.store
            .upsert_file(f.repo_id, "src/other.ts", 999, "typescript", 0)
            .unwrap();
        assert_eq!(f.store.file_count().unwrap(), 2);
    }

    #[test]
    fn test_transaction() {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();

        let result = store.write_transaction(|conn| {
            store.upsert_file_with_conn(conn, repo_id, "a.ts", 1, "typescript", 0)?;
            store.upsert_file_with_conn(conn, repo_id, "b.ts", 2, "typescript", 0)?;
            Ok(())
        });
        assert!(result.is_ok());
        assert_eq!(store.file_count().unwrap(), 2);

        let result: Result<()> = store.write_transaction(|conn| {
            store.upsert_file_with_conn(conn, repo_id, "c.ts", 3, "typescript", 0)?;
            anyhow::bail!("simulated failure");
        });
        assert!(result.is_err());
        assert_eq!(store.file_count().unwrap(), 2);
    }

    #[test]
    fn test_delete_stale_files() {
        let f = setup();
        f.store
            .upsert_file(f.repo_id, "src/stale.ts", 999, "typescript", 0)
            .unwrap();
        assert_eq!(f.store.file_count().unwrap(), 2);

        let current = vec!["src/main.ts".to_string()];
        let deleted = f.store.delete_stale_files(f.repo_id, &current).unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(f.store.file_count().unwrap(), 1);
    }

    #[test]
    fn test_read_write_separation() {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        store
            .upsert_file(repo_id, "a.ts", 1, "typescript", 0)
            .unwrap();
        store
            .insert_code_block(
                store
                    .upsert_file(repo_id, "b.ts", 2, "typescript", 0)
                    .unwrap(),
                "hello",
                CodeBlockKind::Function,
                1,
                5,
                "function hello() {}",
                "function hello()",
                None,
            )
            .unwrap();

        // Read while "write transaction" is conceptually happening
        // This should not deadlock
        let count = store.file_count().unwrap();
        assert!(count >= 1);
        let results = store.search_fts("hello", 10).unwrap();
        assert_eq!(results.len(), 1);
    }

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

    #[test]
    fn test_with_conn_variants_in_transaction() {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();

        store
            .write_transaction(|conn| {
                let file_id =
                    store.upsert_file_with_conn(conn, repo_id, "a.ts", 1, "typescript", 0)?;
                store.insert_code_block_with_conn(
                    conn,
                    file_id,
                    "greet",
                    CodeBlockKind::Function,
                    1,
                    5,
                    "function greet() {}",
                    "function greet()",
                    None,
                )?;
                store.delete_file_blocks_with_conn(conn, file_id)?;
                Ok(())
            })
            .unwrap();

        assert_eq!(store.block_count().unwrap(), 0);
    }

    #[test]
    fn test_open_file_db() {
        let dir = std::env::temp_dir().join("tokenstunt_test_open_file");
        let _ = std::fs::remove_file(&dir);
        let db_path = dir.with_extension("db");

        let store = Store::open(&db_path).unwrap();
        assert_eq!(store.db_path(), db_path);
        store.ensure_repo("/test", "test").unwrap();

        let _ = std::fs::remove_file(&db_path);
    }
}
