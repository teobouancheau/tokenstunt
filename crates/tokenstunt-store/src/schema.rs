use anyhow::Result;
use rusqlite::Connection;

pub const SCHEMA_VERSION: i32 = 2;

pub fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA journal_mode = WAL;")?;
    conn.execute_batch("PRAGMA synchronous = NORMAL;")?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    conn.execute_batch("PRAGMA busy_timeout = 5000;")?;

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS repos (
            id INTEGER PRIMARY KEY,
            path TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS files (
            id INTEGER PRIMARY KEY,
            repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
            path TEXT NOT NULL,
            content_hash INTEGER NOT NULL,
            language TEXT NOT NULL DEFAULT '',
            mtime_ns INTEGER NOT NULL DEFAULT 0,
            UNIQUE(repo_id, path)
        );

        CREATE TABLE IF NOT EXISTS code_blocks (
            id INTEGER PRIMARY KEY,
            file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            kind TEXT NOT NULL,
            start_line INTEGER NOT NULL,
            end_line INTEGER NOT NULL,
            content TEXT NOT NULL,
            signature TEXT NOT NULL DEFAULT '',
            parent_id INTEGER REFERENCES code_blocks(id) ON DELETE SET NULL,
            embedding_id INTEGER
        );

        CREATE INDEX IF NOT EXISTS idx_code_blocks_file ON code_blocks(file_id);
        CREATE INDEX IF NOT EXISTS idx_code_blocks_name ON code_blocks(name);
        CREATE INDEX IF NOT EXISTS idx_code_blocks_kind ON code_blocks(kind);

        CREATE VIRTUAL TABLE IF NOT EXISTS code_blocks_fts USING fts5(
            name,
            content,
            signature,
            content='code_blocks',
            content_rowid='id',
            tokenize='porter unicode61'
        );

        CREATE TABLE IF NOT EXISTS dependencies (
            source_block_id INTEGER NOT NULL REFERENCES code_blocks(id) ON DELETE CASCADE,
            target_block_id INTEGER REFERENCES code_blocks(id) ON DELETE SET NULL,
            target_name TEXT NOT NULL,
            kind TEXT NOT NULL,
            resolved INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (source_block_id, target_name, kind)
        );

        CREATE INDEX IF NOT EXISTS idx_deps_target ON dependencies(target_block_id);
        CREATE INDEX IF NOT EXISTS idx_deps_target_name ON dependencies(target_name);

        CREATE TABLE IF NOT EXISTS embeddings (
            id INTEGER PRIMARY KEY,
            block_id INTEGER NOT NULL UNIQUE REFERENCES code_blocks(id) ON DELETE CASCADE,
            vector BLOB NOT NULL,
            model TEXT NOT NULL,
            computed_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_embeddings_block ON embeddings(block_id);

        CREATE TABLE IF NOT EXISTS overview_cache (
            scope TEXT NOT NULL,
            depth INTEGER NOT NULL,
            content TEXT NOT NULL,
            computed_at INTEGER NOT NULL,
            PRIMARY KEY (scope, depth)
        );

        -- FTS sync triggers
        CREATE TRIGGER IF NOT EXISTS code_blocks_ai AFTER INSERT ON code_blocks BEGIN
            INSERT INTO code_blocks_fts(rowid, name, content, signature)
            VALUES (new.id, new.name, new.content, new.signature);
        END;

        CREATE TRIGGER IF NOT EXISTS code_blocks_ad AFTER DELETE ON code_blocks BEGIN
            INSERT INTO code_blocks_fts(code_blocks_fts, rowid, name, content, signature)
            VALUES ('delete', old.id, old.name, old.content, old.signature);
        END;

        CREATE TRIGGER IF NOT EXISTS code_blocks_au AFTER UPDATE ON code_blocks BEGIN
            INSERT INTO code_blocks_fts(code_blocks_fts, rowid, name, content, signature)
            VALUES ('delete', old.id, old.name, old.content, old.signature);
            INSERT INTO code_blocks_fts(rowid, name, content, signature)
            VALUES (new.id, new.name, new.content, new.signature);
        END;
        ",
    )?;

    let version: Option<i32> = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
            row.get(0)
        })
        .ok();

    match version {
        None => {
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                [SCHEMA_VERSION],
            )?;
        }
        Some(v) if v != SCHEMA_VERSION => {
            anyhow::bail!(
                "schema version mismatch: database has v{v}, expected v{SCHEMA_VERSION}. \
                 Delete the database and re-index."
            );
        }
        Some(_) => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_v2_has_embeddings_table() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='embeddings'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_schema_version_mismatch() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();

        conn.execute("UPDATE schema_version SET version = 999", [])
            .unwrap();

        let err = initialize(&conn).unwrap_err();
        assert!(
            err.to_string().contains("schema version mismatch"),
            "expected 'schema version mismatch', got: {err}"
        );
    }
}
