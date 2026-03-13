use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

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
        read_conn.execute_batch("PRAGMA busy_timeout = 5000;")?;
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

    pub fn ensure_repo_with_conn(&self, conn: &Connection, path: &str, name: &str) -> Result<i64> {
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

    pub fn delete_file_blocks_with_conn(&self, conn: &Connection, file_id: i64) -> Result<()> {
        conn.execute(
            "DELETE FROM code_blocks WHERE file_id = ?1",
            params![file_id],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_code_block(
        &self,
        file_id: i64,
        name: &str,
        kind: CodeBlockKind,
        start_line: u32,
        end_line: u32,
        content: &str,
        signature: &str,
        docstring: &str,
        parent_id: Option<i64>,
    ) -> Result<i64> {
        let conn = self.write_lock()?;
        self.insert_code_block_with_conn(
            &conn, file_id, name, kind, start_line, end_line, content, signature, docstring,
            parent_id,
        )
    }

    #[allow(clippy::too_many_arguments)]
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
        docstring: &str,
        parent_id: Option<i64>,
    ) -> Result<i64> {
        conn.execute(
            "INSERT INTO code_blocks (file_id, name, kind, start_line, end_line, content, signature, docstring, parent_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                file_id,
                name,
                kind.as_str(),
                start_line,
                end_line,
                content,
                signature,
                docstring,
                parent_id,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn search_fts(
        &self,
        query: &str,
        language: Option<&str>,
        kind: Option<&str>,
        scope: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(CodeBlock, f64)>> {
        let conn = self.read_lock()?;
        let mut stmt = conn.prepare(
            "SELECT cb.id, cb.file_id, cb.name, cb.kind, cb.start_line, cb.end_line,
                    cb.content, cb.signature, cb.docstring, cb.parent_id, f.path, f.language, rank
             FROM code_blocks_fts fts
             JOIN code_blocks cb ON cb.id = fts.rowid
             JOIN files f ON f.id = cb.file_id
             WHERE code_blocks_fts MATCH ?1
               AND (?2 IS NULL OR f.language = ?2)
               AND (?3 IS NULL OR cb.kind = ?3)
               AND (?4 IS NULL OR f.path LIKE ?4 || '%')
             ORDER BY rank
             LIMIT ?5",
        )?;

        let rows = stmt
            .query_map(params![query, language, kind, scope, limit as i64], |row| {
                let block = Self::map_code_block(row)?;
                let rank: f64 = row.get(12)?;
                Ok((block, rank))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    pub fn lookup_symbol(&self, name: &str, kind: Option<CodeBlockKind>) -> Result<Vec<CodeBlock>> {
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
                        cb.content, cb.signature, cb.docstring, cb.parent_id, f.path, f.language
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
                        cb.content, cb.signature, cb.docstring, cb.parent_id, f.path, f.language
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
                    cb.content, cb.signature, cb.docstring, cb.parent_id, f.path, f.language
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
                    cb.content, cb.signature, cb.docstring, cb.parent_id, f.path, f.language, d.kind
             FROM dependencies d
             JOIN code_blocks cb ON cb.id = d.source_block_id
             JOIN files f ON f.id = cb.file_id
             WHERE d.target_block_id = ?1",
        )?;
        let rows = stmt
            .query_map(params![block_id], |row| {
                let dep_kind: String = row.get(12)?;
                Ok((Self::map_code_block(row)?, dep_kind))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_dependencies(&self, block_id: i64) -> Result<Vec<(CodeBlock, String)>> {
        let conn = self.read_lock()?;
        let mut stmt = conn.prepare(
            "SELECT cb.id, cb.file_id, cb.name, cb.kind, cb.start_line, cb.end_line,
                    cb.content, cb.signature, cb.docstring, cb.parent_id, f.path, f.language, d.kind
             FROM dependencies d
             JOIN code_blocks cb ON cb.id = d.target_block_id
             JOIN files f ON f.id = cb.file_id
             WHERE d.source_block_id = ?1 AND d.target_block_id IS NOT NULL",
        )?;
        let rows = stmt
            .query_map(params![block_id], |row| {
                let dep_kind: String = row.get(12)?;
                Ok((Self::map_code_block(row)?, dep_kind))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

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

    pub fn insert_dependency_with_conn(
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

    pub fn delete_block_dependencies(&self, block_id: i64) -> Result<()> {
        let conn = self.write_lock()?;
        self.delete_block_dependencies_with_conn(&conn, block_id)
    }

    pub fn delete_block_dependencies_with_conn(
        &self,
        conn: &Connection,
        block_id: i64,
    ) -> Result<()> {
        conn.execute(
            "DELETE FROM dependencies WHERE source_block_id = ?1",
            params![block_id],
        )?;
        Ok(())
    }

    pub fn get_unresolved_dependencies(&self) -> Result<Vec<(i64, String, String)>> {
        let conn = self.read_lock()?;
        self.get_unresolved_dependencies_with_conn(&conn)
    }

    pub fn get_unresolved_dependencies_with_conn(
        &self,
        conn: &Connection,
    ) -> Result<Vec<(i64, String, String)>> {
        let mut stmt = conn.prepare(
            "SELECT source_block_id, target_name, kind
             FROM dependencies
             WHERE resolved = 0",
        )?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn resolve_dependency(
        &self,
        source_block_id: i64,
        target_name: &str,
        target_block_id: i64,
    ) -> Result<()> {
        let conn = self.write_lock()?;
        self.resolve_dependency_with_conn(&conn, source_block_id, target_name, target_block_id)
    }

    pub fn resolve_dependency_with_conn(
        &self,
        conn: &Connection,
        source_block_id: i64,
        target_name: &str,
        target_block_id: i64,
    ) -> Result<()> {
        conn.execute(
            "UPDATE dependencies
             SET target_block_id = ?1, resolved = 1
             WHERE source_block_id = ?2 AND target_name = ?3",
            params![target_block_id, source_block_id, target_name],
        )?;
        Ok(())
    }

    pub fn file_count(&self) -> Result<i64> {
        let conn = self.read_lock()?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))?;
        Ok(count)
    }

    pub fn block_count(&self) -> Result<i64> {
        let conn = self.read_lock()?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM code_blocks", [], |row| row.get(0))?;
        Ok(count)
    }

    pub fn delete_file_by_path_with_conn(
        &self,
        conn: &Connection,
        repo_id: i64,
        path: &str,
    ) -> Result<bool> {
        let deleted = conn.execute(
            "DELETE FROM files WHERE repo_id = ?1 AND path = ?2",
            params![repo_id, path],
        )?;
        Ok(deleted > 0)
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

    pub fn get_language_stats(&self) -> Result<Vec<(String, i64)>> {
        let conn = self.read_lock()?;
        let mut stmt = conn.prepare(
            "SELECT language, COUNT(*) as cnt FROM files GROUP BY language ORDER BY cnt DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn get_directory_stats(&self, scope: Option<&str>) -> Result<Vec<(String, i64, i64)>> {
        let conn = self.read_lock()?;
        let query = match scope {
            Some(prefix) => {
                let mut stmt = conn.prepare(
                    "SELECT
                        CASE
                            WHEN INSTR(SUBSTR(f.path, LENGTH(?1) + 1), '/') > 0
                            THEN SUBSTR(f.path, 1, LENGTH(?1) + INSTR(SUBSTR(f.path, LENGTH(?1) + 1), '/') - 1)
                            ELSE f.path
                        END as dir,
                        COUNT(DISTINCT f.id) as file_count,
                        COUNT(cb.id) as block_count
                     FROM files f
                     LEFT JOIN code_blocks cb ON cb.file_id = f.id
                     WHERE f.path LIKE ?2
                     GROUP BY dir
                     ORDER BY file_count DESC",
                )?;
                let prefix_pattern = format!("{prefix}%");
                let rows = stmt
                    .query_map(params![prefix, prefix_pattern], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                return Ok(rows);
            }
            None => {
                "SELECT
                    CASE
                        WHEN INSTR(path, '/') > 0
                        THEN SUBSTR(path, 1, INSTR(path, '/') - 1)
                        ELSE path
                    END as dir,
                    COUNT(DISTINCT f.id) as file_count,
                    COUNT(cb.id) as block_count
                 FROM files f
                 LEFT JOIN code_blocks cb ON cb.file_id = f.id
                 GROUP BY dir
                 ORDER BY file_count DESC"
            }
        };
        let mut stmt = conn.prepare(query)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_exported_symbols(&self, scope: Option<&str>) -> Result<Vec<CodeBlock>> {
        let conn = self.read_lock()?;
        match scope {
            Some(prefix) => {
                let prefix_pattern = format!("{prefix}%");
                let mut stmt = conn.prepare(
                    "SELECT cb.id, cb.file_id, cb.name, cb.kind, cb.start_line, cb.end_line,
                            cb.content, cb.signature, cb.docstring, cb.parent_id, f.path, f.language
                     FROM code_blocks cb
                     JOIN files f ON f.id = cb.file_id
                     WHERE cb.parent_id IS NULL AND f.path LIKE ?1
                     ORDER BY f.path, cb.start_line
                     LIMIT 50",
                )?;
                let rows = stmt
                    .query_map(params![prefix_pattern], Self::map_code_block)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(rows)
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT cb.id, cb.file_id, cb.name, cb.kind, cb.start_line, cb.end_line,
                            cb.content, cb.signature, cb.docstring, cb.parent_id, f.path, f.language
                     FROM code_blocks cb
                     JOIN files f ON f.id = cb.file_id
                     WHERE cb.parent_id IS NULL
                     ORDER BY f.path, cb.start_line
                     LIMIT 50",
                )?;
                let rows = stmt
                    .query_map([], Self::map_code_block)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(rows)
            }
        }
    }

    pub fn get_overview_cache(&self, scope: &str, depth: i32) -> Result<Option<String>> {
        let conn = self.read_lock()?;
        let result = conn.query_row(
            "SELECT content FROM overview_cache WHERE scope = ?1 AND depth = ?2",
            params![scope, depth],
            |row| row.get::<_, String>(0),
        );
        match result {
            Ok(content) => Ok(Some(content)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_overview_cache(&self, scope: &str, depth: i32, content: &str) -> Result<()> {
        let conn = self.write_lock()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        conn.execute(
            "INSERT OR REPLACE INTO overview_cache (scope, depth, content, computed_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![scope, depth, content, now],
        )?;
        Ok(())
    }

    pub fn invalidate_overview_cache(&self, scope: &str) -> Result<()> {
        let conn = self.write_lock()?;
        self.invalidate_overview_cache_with_conn(&conn, scope)
    }

    pub fn invalidate_overview_cache_with_conn(
        &self,
        conn: &Connection,
        scope: &str,
    ) -> Result<()> {
        conn.execute(
            "DELETE FROM overview_cache WHERE scope = ?1",
            params![scope],
        )?;
        Ok(())
    }

    pub fn get_repo_file_paths(&self, repo_id: i64) -> Result<Vec<String>> {
        let conn = self.read_lock()?;
        self.get_repo_file_paths_with_conn(&conn, repo_id)
    }

    pub fn get_repo_file_paths_with_conn(
        &self,
        conn: &Connection,
        repo_id: i64,
    ) -> Result<Vec<String>> {
        let mut stmt = conn.prepare("SELECT path FROM files WHERE repo_id = ?1")?;
        let rows = stmt
            .query_map(params![repo_id], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn insert_embedding(&self, block_id: i64, vector: &[f32], model: &str) -> Result<()> {
        let conn = self.write_lock()?;
        let blob = vector_to_blob(vector);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;
        conn.execute(
            "INSERT OR REPLACE INTO embeddings (block_id, vector, model, computed_at) VALUES (?1, ?2, ?3, ?4)",
            params![block_id, blob, model, now],
        )?;
        Ok(())
    }

    pub fn get_embedding(&self, block_id: i64) -> Result<Option<Vec<f32>>> {
        let conn = self.read_lock()?;
        let mut stmt = conn.prepare("SELECT vector FROM embeddings WHERE block_id = ?1")?;
        let result = stmt.query_row(params![block_id], |row| {
            let blob: Vec<u8> = row.get(0)?;
            Ok(blob_to_vector(&blob))
        });
        match result {
            Ok(vec) => Ok(Some(vec)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn embedding_count(&self) -> Result<i64> {
        let conn = self.read_lock()?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM embeddings", [], |row| row.get(0))?;
        Ok(count)
    }

    pub fn first_embedding_dimension(&self) -> Result<Option<usize>> {
        let conn = self.read_lock()?;
        let result = conn.query_row("SELECT vector FROM embeddings LIMIT 1", [], |row| {
            let blob: Vec<u8> = row.get(0)?;
            Ok(blob.len() / std::mem::size_of::<f32>())
        });
        match result {
            Ok(dim) => Ok(Some(dim)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn dependency_count(&self) -> Result<(i64, i64)> {
        let conn = self.read_lock()?;
        let total: i64 =
            conn.query_row("SELECT COUNT(*) FROM dependencies", [], |row| row.get(0))?;
        let resolved: i64 = conn.query_row(
            "SELECT COUNT(*) FROM dependencies WHERE resolved = 1",
            [],
            |row| row.get(0),
        )?;
        Ok((total, resolved))
    }

    pub fn get_blocks_without_embeddings(&self, model: Option<&str>) -> Result<Vec<(i64, String)>> {
        let conn = self.read_lock()?;
        let mut stmt = conn.prepare(
            "SELECT cb.id, cb.content
             FROM code_blocks cb
             WHERE cb.parent_id IS NULL
               AND cb.content != ''
               AND NOT EXISTS (
                   SELECT 1 FROM embeddings e
                   WHERE e.block_id = cb.id
                     AND (?1 IS NULL OR e.model = ?1)
               )",
        )?;
        let rows = stmt
            .query_map(params![model], |row| {
                let id: i64 = row.get(0)?;
                let content: String = row.get(1)?;
                Ok((id, content))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_all_embeddings(&self) -> Result<Vec<(i64, Vec<f32>)>> {
        let conn = self.read_lock()?;
        let mut stmt = conn.prepare("SELECT block_id, vector FROM embeddings")?;
        let rows = stmt
            .query_map([], |row| {
                let block_id: i64 = row.get(0)?;
                let blob: Vec<u8> = row.get(1)?;
                Ok((block_id, blob_to_vector(&blob)))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_embeddings_by_block_ids(&self, block_ids: &[i64]) -> Result<Vec<(i64, Vec<f32>)>> {
        if block_ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.read_lock()?;
        let placeholders: String = block_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");
        let sql =
            format!("SELECT block_id, vector FROM embeddings WHERE block_id IN ({placeholders})");
        let mut stmt = conn.prepare(&sql)?;
        for (i, id) in block_ids.iter().enumerate() {
            stmt.raw_bind_parameter(i + 1, *id)?;
        }
        let mut rows = stmt.raw_query();
        let mut results = Vec::new();
        while let Some(row) = rows.next()? {
            let block_id: i64 = row.get(0)?;
            let blob: Vec<u8> = row.get(1)?;
            results.push((block_id, blob_to_vector(&blob)));
        }
        Ok(results)
    }

    pub fn get_blocks_by_file_path(&self, path: &str) -> Result<Vec<CodeBlock>> {
        let conn = self.read_lock()?;
        let mut stmt = conn.prepare(
            "SELECT cb.id, cb.file_id, cb.name, cb.kind, cb.start_line, cb.end_line,
                    cb.content, cb.signature, cb.docstring, cb.parent_id, f.path, f.language
             FROM code_blocks cb
             JOIN files f ON f.id = cb.file_id
             WHERE f.path = ?1
             ORDER BY cb.start_line",
        )?;
        let rows = stmt
            .query_map(params![path], Self::map_code_block)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
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
            docstring: row.get(8)?,
            parent_id: row.get(9)?,
            file_path: row.get(10)?,
            language: row.get(11)?,
        })
    }
}

fn vector_to_blob(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn blob_to_vector(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
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
                "",
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
                "",
                None,
            )
            .unwrap();
        let results = store
            .search_fts("authenticate*", None, None, None, 10)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.name, "authenticateUser");
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
                "",
                None,
            )
            .unwrap();

        f.store
            .insert_dependency(f.block_id, Some(target_id), "helper", "call")
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
    fn test_unresolved_dependency() {
        let f = setup();
        f.store
            .insert_dependency(f.block_id, None, "externalFn", "call")
            .unwrap();
        let deps = f.store.get_dependencies(f.block_id).unwrap();
        assert!(deps.is_empty());
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
                "",
                None,
            )
            .unwrap();

        // Read while "write transaction" is conceptually happening
        // This should not deadlock
        let count = store.file_count().unwrap();
        assert!(count >= 1);
        let results = store.search_fts("hello", None, None, None, 10).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_transaction_holds_lock() {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();

        // Transaction should pass &Connection to closure
        let result: Result<()> = store.write_transaction(|conn| {
            let rows = conn.execute(
                "INSERT INTO files (repo_id, path, content_hash, language, mtime_ns) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![repo_id, "direct.ts", 999i64, "typescript", 0i64],
            ).unwrap();
            assert_eq!(rows, 1);
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
                let file_id = store
                    .upsert_file_with_conn(conn, repo_id, "a.ts", 1, "typescript", 0)
                    .unwrap();
                store
                    .insert_code_block_with_conn(
                        conn,
                        file_id,
                        "greet",
                        CodeBlockKind::Function,
                        1,
                        5,
                        "function greet() {}",
                        "function greet()",
                        "",
                        None,
                    )
                    .unwrap();
                store.delete_file_blocks_with_conn(conn, file_id).unwrap();
                Ok(())
            })
            .unwrap();

        assert_eq!(store.block_count().unwrap(), 0);
    }

    #[test]
    fn test_delete_block_dependencies() {
        let f = setup();
        let target_id = f
            .store
            .insert_code_block(
                f.file_id,
                "helper",
                CodeBlockKind::Function,
                10,
                15,
                "fn helper() {}",
                "fn helper()",
                "",
                None,
            )
            .unwrap();
        f.store
            .insert_dependency(f.block_id, Some(target_id), "helper", "call")
            .unwrap();
        f.store.delete_block_dependencies(f.block_id).unwrap();
        let deps = f.store.get_dependencies(f.block_id).unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn test_get_unresolved_and_resolve() {
        let f = setup();
        f.store
            .insert_dependency(f.block_id, None, "unknown", "import")
            .unwrap();
        let unresolved = f.store.get_unresolved_dependencies().unwrap();
        assert_eq!(unresolved.len(), 1);

        let target_id = f
            .store
            .insert_code_block(
                f.file_id,
                "unknown",
                CodeBlockKind::Function,
                10,
                15,
                "fn unknown() {}",
                "fn unknown()",
                "",
                None,
            )
            .unwrap();
        f.store
            .resolve_dependency(f.block_id, "unknown", target_id)
            .unwrap();
        let unresolved = f.store.get_unresolved_dependencies().unwrap();
        assert!(unresolved.is_empty());
    }

    #[test]
    fn test_language_stats() {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        store
            .upsert_file(repo_id, "a.ts", 1, "typescript", 0)
            .unwrap();
        store
            .upsert_file(repo_id, "b.ts", 2, "typescript", 0)
            .unwrap();
        store.upsert_file(repo_id, "c.py", 3, "python", 0).unwrap();

        let stats = store.get_language_stats().unwrap();
        assert!(
            stats
                .iter()
                .any(|(lang, count)| lang == "typescript" && *count == 2)
        );
        assert!(
            stats
                .iter()
                .any(|(lang, count)| lang == "python" && *count == 1)
        );
    }

    #[test]
    fn test_directory_stats() {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        let f1 = store
            .upsert_file(repo_id, "src/main.ts", 1, "typescript", 0)
            .unwrap();
        let f2 = store
            .upsert_file(repo_id, "src/lib.ts", 2, "typescript", 0)
            .unwrap();
        store
            .upsert_file(repo_id, "tests/test.ts", 3, "typescript", 0)
            .unwrap();
        store
            .insert_code_block(
                f1,
                "main",
                CodeBlockKind::Function,
                1,
                5,
                "fn main() {}",
                "fn main()",
                "",
                None,
            )
            .unwrap();
        store
            .insert_code_block(
                f2,
                "lib",
                CodeBlockKind::Function,
                1,
                5,
                "fn lib() {}",
                "fn lib()",
                "",
                None,
            )
            .unwrap();

        let stats = store.get_directory_stats(None).unwrap();
        assert!(stats.iter().any(|(dir, fc, _)| dir == "src" && *fc == 2));
        assert!(stats.iter().any(|(dir, fc, _)| dir == "tests" && *fc == 1));

        let scoped = store.get_directory_stats(Some("src/")).unwrap();
        assert!(!scoped.is_empty());
    }

    #[test]
    fn test_exported_symbols() {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        let file_id = store
            .upsert_file(repo_id, "src/main.ts", 1, "typescript", 0)
            .unwrap();
        let parent_id = store
            .insert_code_block(
                file_id,
                "MyClass",
                CodeBlockKind::Class,
                1,
                20,
                "class MyClass {}",
                "class MyClass",
                "",
                None,
            )
            .unwrap();
        store
            .insert_code_block(
                file_id,
                "myMethod",
                CodeBlockKind::Method,
                5,
                10,
                "myMethod() {}",
                "myMethod()",
                "",
                Some(parent_id),
            )
            .unwrap();

        let symbols = store.get_exported_symbols(None).unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "MyClass");

        let scoped = store.get_exported_symbols(Some("src/")).unwrap();
        assert_eq!(scoped.len(), 1);

        let empty = store.get_exported_symbols(Some("other/")).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn test_overview_cache() {
        let store = Store::open_in_memory().unwrap();

        assert!(store.get_overview_cache("/test", 1).unwrap().is_none());

        store
            .set_overview_cache("/test", 1, "cached overview content")
            .unwrap();
        let cached = store.get_overview_cache("/test", 1).unwrap();
        assert_eq!(cached.as_deref(), Some("cached overview content"));

        store
            .set_overview_cache("/test", 1, "updated content")
            .unwrap();
        let updated = store.get_overview_cache("/test", 1).unwrap();
        assert_eq!(updated.as_deref(), Some("updated content"));

        store
            .set_overview_cache("/test", 2, "depth 2 content")
            .unwrap();
        store.invalidate_overview_cache("/test").unwrap();
        assert!(store.get_overview_cache("/test", 1).unwrap().is_none());
        assert!(store.get_overview_cache("/test", 2).unwrap().is_none());
    }

    #[test]
    fn test_get_blocks_without_embeddings() {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        let file_id = store
            .upsert_file(repo_id, "src/main.ts", 1, "typescript", 0)
            .unwrap();

        // Top-level block with content
        let block_a = store
            .insert_code_block(
                file_id,
                "greet",
                CodeBlockKind::Function,
                1,
                5,
                "function greet() { return 'hello'; }",
                "function greet()",
                "",
                None,
            )
            .unwrap();

        // Another top-level block
        let block_b = store
            .insert_code_block(
                file_id,
                "farewell",
                CodeBlockKind::Function,
                6,
                10,
                "function farewell() { return 'bye'; }",
                "function farewell()",
                "",
                None,
            )
            .unwrap();

        // Child block (should be excluded)
        store
            .insert_code_block(
                file_id,
                "inner",
                CodeBlockKind::Function,
                2,
                4,
                "function inner() {}",
                "function inner()",
                "",
                Some(block_a),
            )
            .unwrap();

        // All top-level blocks without embeddings
        let missing = store.get_blocks_without_embeddings(None).unwrap();
        assert_eq!(missing.len(), 2);

        // Add embedding for block_a
        store
            .insert_embedding(block_a, &[0.1, 0.2], "model-v1")
            .unwrap();

        // Now only block_b is missing
        let missing = store.get_blocks_without_embeddings(None).unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].0, block_b);

        // With model filter: block_a has "model-v1", asking for "model-v2" should return both
        let stale = store
            .get_blocks_without_embeddings(Some("model-v2"))
            .unwrap();
        assert_eq!(stale.len(), 2);

        // With matching model: only block_b missing
        let matching = store
            .get_blocks_without_embeddings(Some("model-v1"))
            .unwrap();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].0, block_b);

        // Regression: block_a now has model-v1 embedding, add model-v2 embedding too
        // (simulates INSERT OR REPLACE, but embeddings table is keyed on block_id
        // so this replaces the v1 embedding with v2)
        store
            .insert_embedding(block_a, &[0.3, 0.4], "model-v2")
            .unwrap();

        // Asking for model-v2: block_a has it, only block_b missing
        let after_switch = store
            .get_blocks_without_embeddings(Some("model-v2"))
            .unwrap();
        assert_eq!(after_switch.len(), 1);
        assert_eq!(after_switch[0].0, block_b);

        // Asking for model-v1: block_a was replaced by v2, so both are missing for v1
        let old_model = store
            .get_blocks_without_embeddings(Some("model-v1"))
            .unwrap();
        assert_eq!(old_model.len(), 2);
    }

    #[test]
    fn test_get_blocks_without_embeddings_excludes_empty_content() {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        let file_id = store
            .upsert_file(repo_id, "src/main.ts", 1, "typescript", 0)
            .unwrap();

        // Block with empty content
        store
            .insert_code_block(
                file_id,
                "empty",
                CodeBlockKind::Function,
                1,
                2,
                "",
                "function empty()",
                "",
                None,
            )
            .unwrap();

        // Block with actual content
        let real_id = store
            .insert_code_block(
                file_id,
                "real",
                CodeBlockKind::Function,
                3,
                6,
                "function real() { return 42; }",
                "function real()",
                "",
                None,
            )
            .unwrap();

        let missing = store.get_blocks_without_embeddings(None).unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].0, real_id);
    }

    #[test]
    fn test_embedding_storage() {
        let f = setup();

        let vec = vec![0.1f32, 0.2, 0.3];
        f.store
            .insert_embedding(f.block_id, &vec, "nomic-embed-text")
            .unwrap();

        let retrieved = f.store.get_embedding(f.block_id).unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.len(), 3);
        assert!((retrieved[0] - 0.1).abs() < 0.001);
        assert!((retrieved[1] - 0.2).abs() < 0.001);
        assert!((retrieved[2] - 0.3).abs() < 0.001);

        let all = f.store.get_all_embeddings().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, f.block_id);
    }

    #[test]
    fn test_embedding_missing() {
        let f = setup();
        let result = f.store.get_embedding(f.block_id).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_embedding_replace() {
        let f = setup();

        f.store
            .insert_embedding(f.block_id, &[1.0, 2.0], "model-a")
            .unwrap();
        f.store
            .insert_embedding(f.block_id, &[3.0, 4.0], "model-b")
            .unwrap();

        let retrieved = f.store.get_embedding(f.block_id).unwrap().unwrap();
        assert_eq!(retrieved.len(), 2);
        assert!((retrieved[0] - 3.0).abs() < 0.001);
    }

    #[test]
    fn test_embedding_count() {
        let f = setup();
        assert_eq!(f.store.embedding_count().unwrap(), 0);

        f.store
            .insert_embedding(f.block_id, &[0.1, 0.2, 0.3], "test-model")
            .unwrap();
        assert_eq!(f.store.embedding_count().unwrap(), 1);
    }

    #[test]
    fn test_first_embedding_dimension_empty() {
        let f = setup();
        assert_eq!(f.store.first_embedding_dimension().unwrap(), None);
    }

    #[test]
    fn test_first_embedding_dimension_returns_correct_size() {
        let f = setup();
        let vector = vec![0.1_f32, 0.2, 0.3, 0.4];
        f.store
            .insert_embedding(f.block_id, &vector, "test-model")
            .unwrap();
        assert_eq!(f.store.first_embedding_dimension().unwrap(), Some(4));
    }

    #[test]
    fn test_dependency_count() {
        let f = setup();
        assert_eq!(f.store.dependency_count().unwrap(), (0, 0));

        let target_id = f
            .store
            .insert_code_block(
                f.file_id,
                "helper",
                CodeBlockKind::Function,
                10,
                15,
                "fn helper() {}",
                "fn helper()",
                "",
                None,
            )
            .unwrap();

        f.store
            .insert_dependency(f.block_id, Some(target_id), "helper", "call")
            .unwrap();
        assert_eq!(f.store.dependency_count().unwrap(), (1, 1));

        f.store
            .insert_dependency(f.block_id, None, "external", "import")
            .unwrap();
        assert_eq!(f.store.dependency_count().unwrap(), (2, 1));
    }

    #[test]
    fn test_delete_file_by_path_with_conn() {
        let f = setup();
        assert_eq!(f.store.file_count().unwrap(), 1);

        let deleted = f
            .store
            .write_transaction(|conn| {
                f.store
                    .delete_file_by_path_with_conn(conn, f.repo_id, "src/main.ts")
            })
            .unwrap();
        assert!(deleted);
        assert_eq!(f.store.file_count().unwrap(), 0);

        // Deleting again should return false
        let deleted_again = f
            .store
            .write_transaction(|conn| {
                f.store
                    .delete_file_by_path_with_conn(conn, f.repo_id, "src/main.ts")
            })
            .unwrap();
        assert!(!deleted_again);
    }

    #[test]
    fn test_get_repo_file_paths_with_conn_empty() {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/empty", "empty").unwrap();

        let paths = store
            .write_transaction(|conn| store.get_repo_file_paths_with_conn(conn, repo_id))
            .unwrap();
        assert!(paths.is_empty());
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

    // --- Error path tests ---
    // These cover the `?` error propagation and `Err(e) => Err(e.into())` branches
    // by dropping tables to cause SQL failures.

    fn drop_table(store: &Store, table: &str) {
        store
            .write_transaction(|conn| {
                conn.execute_batch(&format!("DROP TABLE IF EXISTS {table}"))
                    .unwrap();
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn test_ensure_repo_with_conn_error() {
        let store = Store::open_in_memory().unwrap();
        drop_table(&store, "repos");
        let err = store.ensure_repo("/test", "test");
        assert!(err.is_err());
    }

    #[test]
    fn test_upsert_file_with_conn_error() {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        drop_table(&store, "files");
        let err = store.upsert_file(repo_id, "a.ts", 1, "typescript", 0);
        assert!(err.is_err());
    }

    #[test]
    fn test_get_file_hash_error() {
        let store = Store::open_in_memory().unwrap();
        drop_table(&store, "files");
        let err = store.get_file_hash(1, "a.ts");
        assert!(err.is_err());
    }

    #[test]
    fn test_delete_file_blocks_error() {
        let store = Store::open_in_memory().unwrap();
        drop_table(&store, "code_blocks");
        let err = store.delete_file_blocks(1);
        assert!(err.is_err());
    }

    #[test]
    fn test_insert_code_block_error() {
        let store = Store::open_in_memory().unwrap();
        drop_table(&store, "code_blocks");
        let err = store.insert_code_block(
            1,
            "fn",
            CodeBlockKind::Function,
            1,
            5,
            "content",
            "sig",
            "",
            None,
        );
        assert!(err.is_err());
    }

    #[test]
    fn test_search_fts_error() {
        let store = Store::open_in_memory().unwrap();
        // Drop the FTS table to cause an error
        store
            .write_transaction(|conn| {
                conn.execute_batch("DROP TABLE IF EXISTS code_blocks_fts")
                    .unwrap();
                Ok(())
            })
            .unwrap();
        let err = store.search_fts("test", None, None, None, 10);
        assert!(err.is_err());
    }

    #[test]
    fn test_lookup_symbol_with_kind_error() {
        let store = Store::open_in_memory().unwrap();
        drop_table(&store, "code_blocks");
        let err = store.lookup_symbol("test", Some(CodeBlockKind::Function));
        assert!(err.is_err());
    }

    #[test]
    fn test_lookup_symbol_without_kind_error() {
        let store = Store::open_in_memory().unwrap();
        drop_table(&store, "code_blocks");
        let err = store.lookup_symbol("test", None);
        assert!(err.is_err());
    }

    #[test]
    fn test_get_block_by_id_error() {
        let store = Store::open_in_memory().unwrap();
        drop_table(&store, "code_blocks");
        let err = store.get_block_by_id(1);
        assert!(err.is_err());
    }

    #[test]
    fn test_insert_dependency_error() {
        let store = Store::open_in_memory().unwrap();
        drop_table(&store, "dependencies");
        let err = store.insert_dependency(1, Some(2), "target", "call");
        assert!(err.is_err());
    }

    #[test]
    fn test_delete_block_dependencies_error() {
        let store = Store::open_in_memory().unwrap();
        drop_table(&store, "dependencies");
        let err = store.delete_block_dependencies(1);
        assert!(err.is_err());
    }

    #[test]
    fn test_get_unresolved_dependencies_error() {
        let store = Store::open_in_memory().unwrap();
        drop_table(&store, "dependencies");
        let err = store.get_unresolved_dependencies();
        assert!(err.is_err());
    }

    #[test]
    fn test_resolve_dependency_error() {
        let store = Store::open_in_memory().unwrap();
        drop_table(&store, "dependencies");
        let err = store.resolve_dependency(1, "target", 2);
        assert!(err.is_err());
    }

    #[test]
    fn test_delete_file_by_path_error() {
        let store = Store::open_in_memory().unwrap();
        drop_table(&store, "files");
        let err =
            store.write_transaction(|conn| store.delete_file_by_path_with_conn(conn, 1, "a.ts"));
        assert!(err.is_err());
    }

    #[test]
    fn test_delete_stale_files_error() {
        let store = Store::open_in_memory().unwrap();
        drop_table(&store, "files");
        let paths = vec!["a.ts".to_string()];
        let err = store.delete_stale_files(1, &paths);
        assert!(err.is_err());
    }

    #[test]
    fn test_get_overview_cache_error() {
        let store = Store::open_in_memory().unwrap();
        drop_table(&store, "overview_cache");
        let err = store.get_overview_cache("scope", 1);
        assert!(err.is_err());
    }

    #[test]
    fn test_invalidate_overview_cache_error() {
        let store = Store::open_in_memory().unwrap();
        drop_table(&store, "overview_cache");
        let err = store.invalidate_overview_cache("scope");
        assert!(err.is_err());
    }

    #[test]
    fn test_get_embedding_error() {
        let store = Store::open_in_memory().unwrap();
        drop_table(&store, "embeddings");
        let err = store.get_embedding(1);
        assert!(err.is_err());
    }

    #[test]
    fn test_dependency_count_error() {
        let store = Store::open_in_memory().unwrap();
        drop_table(&store, "dependencies");
        let err = store.dependency_count();
        assert!(err.is_err());
    }

    #[test]
    fn test_delete_stale_files_empty_paths() {
        let f = setup();
        let deleted = f
            .store
            .write_transaction(|conn| f.store.delete_stale_files_with_conn(conn, f.repo_id, &[]))
            .unwrap();
        assert_eq!(deleted, 0);
    }

    #[test]
    fn test_ensure_repo_query_row_error() {
        let store = Store::open_in_memory().unwrap();
        // Add a trigger that deletes the row after insert, so the SELECT finds nothing
        store
            .write_transaction(|conn| {
                conn.execute_batch(
                    "CREATE TRIGGER delete_after_insert AFTER INSERT ON repos
                     BEGIN DELETE FROM repos WHERE id = NEW.id; END;",
                )
                .unwrap();
                Ok(())
            })
            .unwrap();
        let err = store.ensure_repo("/new-path", "new-name");
        assert!(err.is_err());
    }

    #[test]
    fn test_upsert_file_query_row_error() {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        // Add a trigger that deletes the row after insert, so the SELECT finds nothing
        store
            .write_transaction(|conn| {
                conn.execute_batch(
                    "CREATE TRIGGER delete_file_after_insert AFTER INSERT ON files
                     BEGIN DELETE FROM files WHERE id = NEW.id; END;",
                )
                .unwrap();
                Ok(())
            })
            .unwrap();
        let err = store.upsert_file(repo_id, "test.ts", 1, "typescript", 0);
        assert!(err.is_err());
    }

    #[test]
    fn test_search_fts_query_map_error() {
        let store = Store::open_in_memory().unwrap();
        // Pass an invalid FTS5 MATCH expression that prepare accepts but execution rejects
        let err = store.search_fts("", None, None, None, 10);
        assert!(err.is_err());
    }

    #[test]
    fn test_lookup_symbol_with_kind_prepare_error() {
        let store = Store::open_in_memory().unwrap();
        // Drop both tables to guarantee prepare fails on the JOIN
        drop_table(&store, "files");
        drop_table(&store, "code_blocks");
        let err = store.lookup_symbol("test", Some(CodeBlockKind::Function));
        assert!(err.is_err());
    }

    #[test]
    fn test_lookup_symbol_without_kind_prepare_error() {
        let store = Store::open_in_memory().unwrap();
        drop_table(&store, "files");
        drop_table(&store, "code_blocks");
        let err = store.lookup_symbol("test", None);
        assert!(err.is_err());
    }

    #[test]
    fn test_get_unresolved_dependencies_prepare_error() {
        let store = Store::open_in_memory().unwrap();
        // Drop via write_transaction to ensure it's committed
        store
            .write_transaction(|conn| {
                conn.execute_batch("DROP TABLE IF EXISTS dependencies")
                    .unwrap();
                Ok(())
            })
            .unwrap();
        let err = store.get_unresolved_dependencies();
        assert!(err.is_err());
    }

    #[test]
    fn test_dependency_count_second_query_error() {
        let store = Store::open_in_memory().unwrap();
        // Create a view that only supports the first COUNT query pattern
        // by replacing dependencies with a view that errors on the WHERE clause
        store
            .write_transaction(|conn| {
                conn.execute_batch("DROP TABLE IF EXISTS dependencies")
                    .unwrap();
                // Create a view where COUNT(*) works but COUNT(*) WHERE resolved=1
                // will also work, so instead we drop and create a table
                // with a missing 'resolved' column
                conn.execute_batch(
                    "CREATE TABLE dependencies (
                        source_block_id INTEGER,
                        target_block_id INTEGER,
                        target_name TEXT,
                        kind TEXT
                    )",
                )
                .unwrap();
                Ok(())
            })
            .unwrap();
        // COUNT(*) succeeds, but COUNT(*) WHERE resolved = 1 fails (no resolved column)
        let err = store.dependency_count();
        assert!(err.is_err());
    }

    #[test]
    fn test_get_blocks_by_file_path() {
        let f = setup();
        let blocks = f.store.get_blocks_by_file_path("src/main.ts").unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].name, "greet");
        assert_eq!(blocks[0].file_path.as_deref(), Some("src/main.ts"));
    }

    #[test]
    fn test_get_blocks_by_file_path_not_found() {
        let f = setup();
        let blocks = f.store.get_blocks_by_file_path("nonexistent.ts").unwrap();
        assert!(blocks.is_empty());
    }

    #[test]
    fn test_get_blocks_by_file_path_ordered_by_line() {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        let file_id = store
            .upsert_file(repo_id, "src/ordered.ts", 111, "typescript", 0)
            .unwrap();
        store
            .insert_code_block(
                file_id,
                "second",
                CodeBlockKind::Function,
                10,
                20,
                "function second() {}",
                "function second()",
                "",
                None,
            )
            .unwrap();
        store
            .insert_code_block(
                file_id,
                "first",
                CodeBlockKind::Function,
                1,
                5,
                "function first() {}",
                "function first()",
                "",
                None,
            )
            .unwrap();

        let blocks = store.get_blocks_by_file_path("src/ordered.ts").unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].name, "first");
        assert_eq!(blocks[1].name, "second");
    }
}
