use anyhow::Result;
use rusqlite::Connection;

pub const SCHEMA_VERSION: i32 = 3;

type MigrationFn = fn(&Connection) -> Result<()>;

const MIGRATIONS: &[(i32, MigrationFn)] = &[(2, migrate_v1_to_v2), (3, migrate_v2_to_v3)];

fn migrate_v1_to_v2(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
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
        ",
    )?;
    Ok(())
}

fn migrate_v2_to_v3(conn: &Connection) -> Result<()> {
    // Check if docstring column already exists (fresh v3 databases have it)
    let has_docstring: bool = conn
        .prepare("SELECT docstring FROM code_blocks LIMIT 0")
        .is_ok();

    if !has_docstring {
        conn.execute_batch(
            "ALTER TABLE code_blocks ADD COLUMN docstring TEXT NOT NULL DEFAULT '';",
        )?;
    }

    conn.execute_batch(
        "
        -- Rebuild FTS to include docstring column
        DROP TRIGGER IF EXISTS code_blocks_ai;
        DROP TRIGGER IF EXISTS code_blocks_ad;
        DROP TRIGGER IF EXISTS code_blocks_au;
        DROP TABLE IF EXISTS code_blocks_fts;

        CREATE VIRTUAL TABLE code_blocks_fts USING fts5(
            name,
            content,
            signature,
            docstring,
            content='code_blocks',
            content_rowid='id',
            tokenize='porter unicode61'
        );

        -- Re-populate FTS from existing data
        INSERT INTO code_blocks_fts(rowid, name, content, signature, docstring)
        SELECT id, name, content, signature, docstring FROM code_blocks;

        CREATE TRIGGER code_blocks_ai AFTER INSERT ON code_blocks BEGIN
            INSERT INTO code_blocks_fts(rowid, name, content, signature, docstring)
            VALUES (new.id, new.name, new.content, new.signature, new.docstring);
        END;

        CREATE TRIGGER code_blocks_ad AFTER DELETE ON code_blocks BEGIN
            INSERT INTO code_blocks_fts(code_blocks_fts, rowid, name, content, signature, docstring)
            VALUES ('delete', old.id, old.name, old.content, old.signature, old.docstring);
        END;

        CREATE TRIGGER code_blocks_au AFTER UPDATE ON code_blocks BEGIN
            INSERT INTO code_blocks_fts(code_blocks_fts, rowid, name, content, signature, docstring)
            VALUES ('delete', old.id, old.name, old.content, old.signature, old.docstring);
            INSERT INTO code_blocks_fts(rowid, name, content, signature, docstring)
            VALUES (new.id, new.name, new.content, new.signature, new.docstring);
        END;
        ",
    )?;
    Ok(())
}

fn run_migrations(conn: &Connection, from_version: i32) -> Result<()> {
    for &(target_version, migration_fn) in MIGRATIONS {
        if from_version < target_version {
            migration_fn(conn)?;
            conn.execute("UPDATE schema_version SET version = ?1", [target_version])?;
        }
    }
    Ok(())
}

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
            docstring TEXT NOT NULL DEFAULT '',
            parent_id INTEGER REFERENCES code_blocks(id) ON DELETE SET NULL,
            embedding_id INTEGER
        );

        CREATE INDEX IF NOT EXISTS idx_code_blocks_file ON code_blocks(file_id);
        CREATE INDEX IF NOT EXISTS idx_code_blocks_name ON code_blocks(name);
        CREATE INDEX IF NOT EXISTS idx_code_blocks_kind ON code_blocks(kind);
        CREATE INDEX IF NOT EXISTS idx_files_path ON files(path);

        CREATE VIRTUAL TABLE IF NOT EXISTS code_blocks_fts USING fts5(
            name,
            content,
            signature,
            docstring,
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
            INSERT INTO code_blocks_fts(rowid, name, content, signature, docstring)
            VALUES (new.id, new.name, new.content, new.signature, new.docstring);
        END;

        CREATE TRIGGER IF NOT EXISTS code_blocks_ad AFTER DELETE ON code_blocks BEGIN
            INSERT INTO code_blocks_fts(code_blocks_fts, rowid, name, content, signature, docstring)
            VALUES ('delete', old.id, old.name, old.content, old.signature, old.docstring);
        END;

        CREATE TRIGGER IF NOT EXISTS code_blocks_au AFTER UPDATE ON code_blocks BEGIN
            INSERT INTO code_blocks_fts(code_blocks_fts, rowid, name, content, signature, docstring)
            VALUES ('delete', old.id, old.name, old.content, old.signature, old.docstring);
            INSERT INTO code_blocks_fts(rowid, name, content, signature, docstring)
            VALUES (new.id, new.name, new.content, new.signature, new.docstring);
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
        Some(v) if v > SCHEMA_VERSION => {
            anyhow::bail!(
                "schema version mismatch: database has v{v}, expected v{SCHEMA_VERSION}. \
                 The database was created by a newer version of Token Stunt."
            );
        }
        Some(v) if v < SCHEMA_VERSION => {
            run_migrations(conn, v)?;
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
    fn test_schema_fresh_initialization_inserts_version() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();

        let version: i32 = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn test_schema_idempotent_initialization() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        initialize(&conn).unwrap();

        let version: i32 = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn test_schema_insert_version_fails_when_table_is_unwritable() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();

        conn.execute("DELETE FROM schema_version", []).unwrap();

        conn.execute_batch(
            "CREATE TRIGGER block_version_insert BEFORE INSERT ON schema_version
             BEGIN SELECT RAISE(ABORT, 'blocked by trigger'); END;",
        )
        .unwrap();

        let err = initialize(&conn);
        assert!(
            err.is_err(),
            "INSERT into schema_version should fail due to trigger"
        );
    }

    #[test]
    fn test_schema_future_version_bails() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();

        conn.execute(
            "UPDATE schema_version SET version = ?1",
            [SCHEMA_VERSION + 1],
        )
        .unwrap();

        let err = initialize(&conn).unwrap_err();
        assert!(
            err.to_string().contains("schema version mismatch"),
            "expected 'schema version mismatch', got: {err}"
        );
    }

    #[test]
    fn test_schema_migration_from_v1_to_v2() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();

        // Simulate a v1 database by dropping v2 tables and setting version to 1
        conn.execute_batch("DROP TABLE IF EXISTS overview_cache;")
            .unwrap();
        conn.execute_batch("DROP TABLE IF EXISTS embeddings;")
            .unwrap();
        conn.execute("UPDATE schema_version SET version = 1", [])
            .unwrap();

        // Re-initialize should run the migration
        initialize(&conn).unwrap();

        let version: i32 = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);

        // Verify the migrated tables exist
        let emb_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='embeddings'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(emb_count, 1);

        let cache_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='overview_cache'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cache_count, 1);
    }

    #[test]
    fn test_schema_execute_batch_fails_with_conflicting_table() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE code_blocks (id INTEGER PRIMARY KEY, name TEXT);")
            .unwrap();
        let err = initialize(&conn);
        assert!(
            err.is_err(),
            "execute_batch should fail with conflicting schema"
        );
    }

    #[test]
    fn test_run_migrations_noop_when_current() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();

        // Running migrations from current version should be a no-op
        run_migrations(&conn, SCHEMA_VERSION).unwrap();

        let version: i32 = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn test_schema_v3_has_docstring_column() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();

        conn.execute("INSERT INTO repos (path, name) VALUES ('/', 'test')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO files (repo_id, path, content_hash, language) VALUES (1, 'a.rs', 0, 'rust')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO code_blocks (file_id, name, kind, start_line, end_line, content, signature, docstring)
             VALUES (1, 'test', 'function', 1, 5, 'fn test() {}', 'fn test()', 'A test function')",
            [],
        )
        .unwrap();

        let docstring: String = conn
            .query_row(
                "SELECT docstring FROM code_blocks WHERE name = 'test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(docstring, "A test function");
    }

    #[test]
    fn test_schema_migration_from_v2_to_v3() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();

        // Simulate a v2 database by reverting v3 changes
        conn.execute_batch("DROP TRIGGER IF EXISTS code_blocks_ai;")
            .unwrap();
        conn.execute_batch("DROP TRIGGER IF EXISTS code_blocks_ad;")
            .unwrap();
        conn.execute_batch("DROP TRIGGER IF EXISTS code_blocks_au;")
            .unwrap();
        conn.execute_batch("DROP TABLE IF EXISTS code_blocks_fts;")
            .unwrap();
        // Recreate v2-style FTS (without docstring)
        conn.execute_batch(
            "CREATE VIRTUAL TABLE code_blocks_fts USING fts5(
                name, content, signature,
                content='code_blocks', content_rowid='id',
                tokenize='porter unicode61'
            );
            CREATE TRIGGER code_blocks_ai AFTER INSERT ON code_blocks BEGIN
                INSERT INTO code_blocks_fts(rowid, name, content, signature)
                VALUES (new.id, new.name, new.content, new.signature);
            END;
            CREATE TRIGGER code_blocks_ad AFTER DELETE ON code_blocks BEGIN
                INSERT INTO code_blocks_fts(code_blocks_fts, rowid, name, content, signature)
                VALUES ('delete', old.id, old.name, old.content, old.signature);
            END;
            CREATE TRIGGER code_blocks_au AFTER UPDATE ON code_blocks BEGIN
                INSERT INTO code_blocks_fts(code_blocks_fts, rowid, name, content, signature)
                VALUES ('delete', old.id, old.name, old.content, old.signature);
                INSERT INTO code_blocks_fts(rowid, name, content, signature)
                VALUES (new.id, new.name, new.content, new.signature);
            END;",
        )
        .unwrap();
        conn.execute("UPDATE schema_version SET version = 2", [])
            .unwrap();

        // Re-initialize should run the v2->v3 migration
        initialize(&conn).unwrap();

        let version: i32 = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);

        // Verify docstring column is queryable
        let default_val: String = conn
            .query_row("SELECT docstring FROM code_blocks LIMIT 0", [], |row| {
                row.get(0)
            })
            .unwrap_or_default();
        assert_eq!(default_val, "");
    }
}
