use std::path::Path;

use anyhow::{Context, Result};
use tracing::{info, warn};
use xxhash_rust::xxh3::xxh3_64;

use tokenstunt_parser::{LanguageRegistry, ParsedSymbol, SymbolExtractor};
use tokenstunt_store::{CodeBlockKind, Connection, Store};

use crate::walker;

pub struct Indexer {
    store: Store,
    extractor: SymbolExtractor,
}

impl Indexer {
    pub fn new(store: Store) -> Result<Self> {
        let registry = LanguageRegistry::new()?;
        let extractor = SymbolExtractor::new(registry);
        Ok(Self { store, extractor })
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn index_directory(&self, root: &Path) -> Result<IndexStats> {
        let root_str = root
            .to_str()
            .context("non-UTF-8 path")?;
        let repo_name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let repo_id = self.store.ensure_repo(root_str, repo_name)?;

        let entries = walker::walk_directory(root)?;
        info!(files = entries.len(), "discovered files");

        let registry = LanguageRegistry::new()?;

        let stats = self.store.write_transaction(|conn| {
            let mut stats = IndexStats::default();
            let mut indexed_paths: Vec<String> = Vec::with_capacity(entries.len());

            for entry in &entries {
                let rel_path = entry
                    .path
                    .strip_prefix(root)
                    .unwrap_or(&entry.path)
                    .to_string_lossy()
                    .to_string();

                indexed_paths.push(rel_path.clone());

                let source = match std::fs::read_to_string(&entry.path) {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(path = %entry.path.display(), error = %e, "failed to read file");
                        stats.errors += 1;
                        continue;
                    }
                };

                let content_hash = xxh3_64(source.as_bytes());

                if let Ok(Some(existing_hash)) = self.store.get_file_hash_with_conn(conn, repo_id, &rel_path) {
                    if existing_hash == content_hash {
                        stats.skipped += 1;
                        continue;
                    }
                }

                if !registry.is_supported(entry.language) {
                    continue;
                }

                let mtime = std::fs::metadata(&entry.path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos() as i64)
                    .unwrap_or(0);

                let file_id = self.store.upsert_file_with_conn(
                    conn,
                    repo_id,
                    &rel_path,
                    content_hash,
                    entry.language.as_str(),
                    mtime,
                )?;

                self.store.delete_file_blocks_with_conn(conn, file_id)?;

                let parse_result = match self.extractor.extract(&source, entry.language) {
                    Ok(r) => r,
                    Err(e) => {
                        warn!(path = %rel_path, error = %e, "failed to parse");
                        stats.errors += 1;
                        continue;
                    }
                };

                let mut first_block_id: Option<i64> = None;
                let mut block_ids: Vec<(String, i64)> = Vec::new();

                for symbol in &parse_result.symbols {
                    let block_id = self.insert_symbol_with_conn(conn, file_id, symbol, None)?;
                    if first_block_id.is_none() {
                        first_block_id = Some(block_id);
                    }
                    block_ids.push((symbol.name.clone(), block_id));
                    stats.blocks += count_symbols(symbol);
                }

                if let Some(fallback_id) = first_block_id {
                    for reference in &parse_result.references {
                        let source_block_id = if reference.source_symbol.is_empty() {
                            fallback_id
                        } else {
                            block_ids
                                .iter()
                                .find(|(name, _)| name == &reference.source_symbol)
                                .map(|(_, id)| *id)
                                .unwrap_or(fallback_id)
                        };

                        let target = self
                            .store
                            .lookup_symbol_with_conn(conn, &reference.target_name, None)?;
                        let target_block_id = target.first().map(|b| b.id);

                        self.store.insert_dependency_with_conn(
                            conn,
                            source_block_id,
                            target_block_id,
                            &reference.target_name,
                            reference.kind,
                        )?;
                    }
                }

                stats.files += 1;
            }

            let deleted = self.store.delete_stale_files_with_conn(conn, repo_id, &indexed_paths)?;
            stats.deleted_files = deleted;

            Ok(stats)
        })?;

        info!(
            files = stats.files,
            blocks = stats.blocks,
            skipped = stats.skipped,
            errors = stats.errors,
            deleted = stats.deleted_files,
            "indexing complete"
        );

        Ok(stats)
    }

    fn insert_symbol_with_conn(
        &self,
        conn: &Connection,
        file_id: i64,
        symbol: &ParsedSymbol,
        parent_id: Option<i64>,
    ) -> Result<i64> {
        let kind = CodeBlockKind::from_str(symbol.kind).unwrap_or(CodeBlockKind::Function);

        let block_id = self.store.insert_code_block_with_conn(
            conn,
            file_id,
            &symbol.name,
            kind,
            symbol.start_line,
            symbol.end_line,
            &symbol.content,
            &symbol.signature,
            parent_id,
        )?;

        for child in &symbol.children {
            self.insert_symbol_with_conn(conn, file_id, child, Some(block_id))?;
        }

        Ok(block_id)
    }
}

fn count_symbols(symbol: &ParsedSymbol) -> u64 {
    1 + symbol.children.iter().map(count_symbols).sum::<u64>()
}

#[derive(Debug, Default)]
pub struct IndexStats {
    pub files: u64,
    pub blocks: u64,
    pub skipped: u64,
    pub errors: u64,
    pub deleted_files: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_test_files(dir: &Path) {
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();

        std::fs::write(
            src.join("greet.ts"),
            "export function greet(name: string): string {\n  return `Hello ${name}`;\n}\n",
        )
        .unwrap();

        std::fs::write(
            src.join("math.py"),
            "def add(a: int, b: int) -> int:\n    return a + b\n",
        )
        .unwrap();
    }

    #[test]
    fn test_index_directory() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store).unwrap();
        let stats = indexer.index_directory(dir.path()).unwrap();

        assert!(stats.files >= 2);
        assert!(stats.blocks >= 2);
        assert_eq!(stats.errors, 0);
        assert!(indexer.store().file_count().unwrap() >= 2);
        assert!(indexer.store().block_count().unwrap() >= 2);
    }

    #[test]
    fn test_incremental_skip() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store).unwrap();

        let stats1 = indexer.index_directory(dir.path()).unwrap();
        assert!(stats1.files >= 2);

        let stats2 = indexer.index_directory(dir.path()).unwrap();
        assert!(stats2.skipped >= 2);
        assert_eq!(stats2.files, 0);
    }

    #[test]
    fn test_index_populates_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        std::fs::write(
            src.join("service.ts"),
            "export class UserService { handle() {} }",
        )
        .unwrap();
        std::fs::write(
            src.join("handler.ts"),
            "import { UserService } from './service';\nexport function handler() { const s = new UserService(); }",
        )
        .unwrap();

        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store).unwrap();
        indexer.index_directory(dir.path()).unwrap();

        let handler_blocks = indexer.store().lookup_symbol("handler", None).unwrap();
        assert!(!handler_blocks.is_empty());

        let unresolved = indexer.store().get_unresolved_dependencies().unwrap();
        assert!(
            unresolved.iter().any(|(_, name, _)| name == "UserService"),
            "expected unresolved dependency for UserService, got: {unresolved:?}"
        );
    }
}
