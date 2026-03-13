use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use tokio::task::JoinHandle;
use tracing::{info, warn};
use xxhash_rust::xxh3::xxh3_64;

use tokenstunt_embeddings::EmbeddingProvider;
use tokenstunt_parser::{Language, LanguageRegistry, ParsedSymbol, SymbolExtractor};
use tokenstunt_store::{CodeBlockKind, Connection, Store};

use crate::progress::{EmbeddingProgress, IndexProgress};
use crate::walker;

pub struct Indexer {
    store: Store,
    extractor: SymbolExtractor,
    embedder: Option<Arc<dyn EmbeddingProvider>>,
    embedding_handles: Mutex<Vec<JoinHandle<()>>>,
    embedding_progress: Option<Arc<dyn EmbeddingProgress>>,
}

impl Indexer {
    pub fn new(store: Store, embedder: Option<Arc<dyn EmbeddingProvider>>) -> Result<Self> {
        let registry = LanguageRegistry::new()?;
        let extractor = SymbolExtractor::new(registry);
        Ok(Self {
            store,
            extractor,
            embedder,
            embedding_handles: Mutex::new(Vec::new()),
            embedding_progress: None,
        })
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn embedder(&self) -> Option<&Arc<dyn EmbeddingProvider>> {
        self.embedder.as_ref()
    }

    pub fn set_embedding_progress(&mut self, progress: Arc<dyn EmbeddingProgress>) {
        self.embedding_progress = Some(progress);
    }

    fn spawn_embeddings_if_needed(&self, embedding_work: Vec<(i64, String)>) {
        if !embedding_work.is_empty()
            && let Some(embedder) = &self.embedder
        {
            if let Some(ref p) = self.embedding_progress {
                p.on_start(embedding_work.len() as u64);
            }
            let model_name = embedder.model_name().to_string();
            let handle = spawn_embedding_task(
                self.store.db_path().to_path_buf(),
                Arc::clone(embedder),
                embedding_work,
                model_name,
                self.embedding_progress.clone(),
            );
            if let Ok(mut handles) = self.embedding_handles.lock() {
                handles.push(handle);
            }
        }
    }

    pub fn backfill_embeddings(&self) -> Result<u64> {
        let Some(embedder) = &self.embedder else {
            return Ok(0);
        };

        let model = embedder.model_name();
        let work = self.store.get_blocks_without_embeddings(Some(model))?;
        let count = work.len() as u64;

        if count > 0 {
            self.spawn_embeddings_if_needed(work);
        }

        Ok(count)
    }

    pub async fn await_embeddings(&self) {
        let handles: Vec<_> = {
            let mut lock = self
                .embedding_handles
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *lock)
        };
        for handle in handles {
            let _ = handle.await;
        }
    }

    pub fn index_directory(&self, root: &Path, progress: &dyn IndexProgress) -> Result<IndexStats> {
        let root_str = root.to_str().context("non-UTF-8 path")?;
        let repo_name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let repo_id = self.store.ensure_repo(root_str, repo_name)?;

        let entries = walker::walk_directory(root)?;
        info!(files = entries.len(), "discovered files");
        progress.on_discover(entries.len());

        let registry = LanguageRegistry::new()?;

        let mut embedding_work: Vec<(i64, String)> = Vec::new();

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
                        progress.on_file_error(&rel_path, &e.to_string());
                        stats.errors += 1;
                        continue;
                    }
                };

                let content_hash = xxh3_64(source.as_bytes());

                if let Ok(Some(existing_hash)) =
                    self.store.get_file_hash_with_conn(conn, repo_id, &rel_path)
                    && existing_hash == content_hash
                {
                    progress.on_file_skipped(&rel_path);
                    stats.skipped += 1;
                    continue;
                }

                if !registry.is_supported(entry.language) {
                    progress.on_file_skipped(&rel_path);
                    stats.skipped += 1;
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
                    collect_embedding_work(symbol, block_id, &mut embedding_work);
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

                        let target = self.store.lookup_symbol_with_conn(
                            conn,
                            &reference.target_name,
                            None,
                        )?;
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

                progress.on_file_indexed(&rel_path);
                stats.files += 1;
            }

            let deleted = self
                .store
                .delete_stale_files_with_conn(conn, repo_id, &indexed_paths)?;
            stats.deleted_files = deleted;

            // Resolve unresolved dependencies now that all symbols are indexed
            let unresolved = self.store.get_unresolved_dependencies_with_conn(conn)?;
            for (source_block_id, target_name, _kind) in &unresolved {
                let targets = self
                    .store
                    .lookup_symbol_with_conn(conn, target_name, None)?;
                if let Some(target) = targets.first() {
                    self.store.resolve_dependency_with_conn(
                        conn,
                        *source_block_id,
                        target_name,
                        target.id,
                    )?;
                }
            }

            Ok(stats)
        })?;

        self.spawn_embeddings_if_needed(embedding_work);

        progress.on_complete(stats.files, stats.blocks, stats.skipped, stats.errors);

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

    pub fn reconcile(&self, root: &Path, repo_id: i64) -> Result<ReconcileStats> {
        let entries = walker::walk_directory(root)?;
        let registry = LanguageRegistry::new()?;
        let mut embedding_work: Vec<(i64, String)> = Vec::new();

        let stats = self.store.write_transaction(|conn| {
            let mut stats = ReconcileStats::default();

            let existing_paths: HashSet<String> = self
                .store
                .get_repo_file_paths_with_conn(conn, repo_id)?
                .into_iter()
                .collect();

            let mut seen_paths: HashSet<String> = HashSet::with_capacity(entries.len());

            for entry in &entries {
                let rel_path = entry
                    .path
                    .strip_prefix(root)
                    .unwrap_or(&entry.path)
                    .to_string_lossy()
                    .to_string();

                seen_paths.insert(rel_path.clone());

                let source = match std::fs::read_to_string(&entry.path) {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(path = %entry.path.display(), error = %e, "failed to read file");
                        continue;
                    }
                };

                let content_hash = xxh3_64(source.as_bytes());

                if let Ok(Some(existing_hash)) =
                    self.store.get_file_hash_with_conn(conn, repo_id, &rel_path)
                    && existing_hash == content_hash
                {
                    stats.unchanged += 1;
                    continue;
                }

                if !registry.is_supported(entry.language) {
                    continue;
                }

                self.index_file_with_conn(
                    conn,
                    repo_id,
                    root,
                    &entry.path,
                    &rel_path,
                    entry.language,
                    &source,
                    &mut embedding_work,
                )?;
                stats.updated += 1;
            }

            for stale_path in &existing_paths {
                if !seen_paths.contains(stale_path) {
                    stats.deleted += 1;
                }
            }

            let current_paths: Vec<String> = seen_paths.into_iter().collect();
            self.store
                .delete_stale_files_with_conn(conn, repo_id, &current_paths)?;

            Ok(stats)
        })?;

        self.spawn_embeddings_if_needed(embedding_work);

        info!(
            updated = stats.updated,
            unchanged = stats.unchanged,
            deleted = stats.deleted,
            "reconciliation complete"
        );

        Ok(stats)
    }

    pub fn reindex_files(&self, root: &Path, paths: &[PathBuf]) -> Result<ReindexStats> {
        let root_str = root.to_str().context("non-UTF-8 path")?;
        let repo_name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let registry = LanguageRegistry::new()?;
        let mut embedding_work: Vec<(i64, String)> = Vec::new();

        let stats = self.store.write_transaction(|conn| {
            let repo_id = self
                .store
                .ensure_repo_with_conn(conn, root_str, repo_name)?;
            let mut stats = ReindexStats::default();

            for path in paths {
                let rel_path = path
                    .strip_prefix(root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();

                if !path.exists() {
                    self.store
                        .delete_file_by_path_with_conn(conn, repo_id, &rel_path)?;
                    stats.deleted += 1;

                    self.invalidate_cache_for_path(conn, &rel_path)?;
                    continue;
                }

                let language = match Language::from_path(path) {
                    Some(l) => l,
                    None => continue,
                };

                if !registry.is_supported(language) {
                    continue;
                }

                let source = match std::fs::read_to_string(path) {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(path = %path.display(), error = %e, "failed to read file");
                        stats.errors += 1;
                        continue;
                    }
                };

                let content_hash = xxh3_64(source.as_bytes());

                if let Ok(Some(existing_hash)) =
                    self.store.get_file_hash_with_conn(conn, repo_id, &rel_path)
                    && existing_hash == content_hash
                {
                    stats.unchanged += 1;
                    continue;
                }

                self.index_file_with_conn(
                    conn,
                    repo_id,
                    root,
                    path,
                    &rel_path,
                    language,
                    &source,
                    &mut embedding_work,
                )?;
                stats.reindexed += 1;

                self.invalidate_cache_for_path(conn, &rel_path)?;
            }

            Ok(stats)
        })?;

        self.spawn_embeddings_if_needed(embedding_work);

        info!(
            reindexed = stats.reindexed,
            unchanged = stats.unchanged,
            deleted = stats.deleted,
            errors = stats.errors,
            "reindex complete"
        );

        Ok(stats)
    }

    #[allow(clippy::too_many_arguments)]
    fn index_file_with_conn(
        &self,
        conn: &Connection,
        repo_id: i64,
        _root: &Path,
        abs_path: &Path,
        rel_path: &str,
        language: Language,
        source: &str,
        embedding_work: &mut Vec<(i64, String)>,
    ) -> Result<()> {
        let content_hash = xxh3_64(source.as_bytes());
        let mtime = std::fs::metadata(abs_path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);

        let file_id = self.store.upsert_file_with_conn(
            conn,
            repo_id,
            rel_path,
            content_hash,
            language.as_str(),
            mtime,
        )?;

        self.store.delete_file_blocks_with_conn(conn, file_id)?;

        let parse_result = match self.extractor.extract(source, language) {
            Ok(r) => r,
            Err(e) => {
                warn!(path = %rel_path, error = %e, "failed to parse");
                return Ok(());
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
            collect_embedding_work(symbol, block_id, embedding_work);
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

                let target =
                    self.store
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

        Ok(())
    }

    fn invalidate_cache_for_path(&self, conn: &Connection, rel_path: &str) -> Result<()> {
        // Invalidate overview cache for parent directories of the changed file
        let parts: Vec<&str> = rel_path.split('/').collect();
        for i in 1..parts.len() {
            let scope = format!("{}/", parts[..i].join("/"));
            self.store
                .invalidate_overview_cache_with_conn(conn, &scope)?;
        }
        // Also invalidate root scope
        self.store.invalidate_overview_cache_with_conn(conn, "")?;
        Ok(())
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

fn collect_embedding_work(symbol: &ParsedSymbol, block_id: i64, work: &mut Vec<(i64, String)>) {
    if !symbol.content.is_empty() {
        work.push((block_id, symbol.content.clone()));
    }
    // Children get their own block IDs during insertion, but we don't
    // have them here. The parent content already covers children, so
    // we skip nested symbols to avoid redundant embeddings.
}

fn spawn_embedding_task(
    db_path: PathBuf,
    embedder: Arc<dyn EmbeddingProvider>,
    work: Vec<(i64, String)>,
    model_name: String,
    progress: Option<Arc<dyn EmbeddingProgress>>,
) -> tokio::task::JoinHandle<()> {
    let total = work.len() as u64;
    tokio::spawn(async move {
        let store = match tokenstunt_store::Store::open(&db_path) {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "failed to open store for embedding task");
                return;
            }
        };

        let texts: Vec<String> = work.iter().map(|(_, text)| text.clone()).collect();
        let batch_size = 32;

        for chunk_start in (0..texts.len()).step_by(batch_size) {
            let chunk_end = (chunk_start + batch_size).min(texts.len());
            let batch = &texts[chunk_start..chunk_end];

            match embedder.embed_batch(batch).await {
                Ok(vectors) => {
                    for (i, vector) in vectors.iter().enumerate() {
                        let (block_id, _) = &work[chunk_start + i];
                        if let Err(e) =
                            store.insert_embedding(*block_id, vector, &model_name)
                        {
                            warn!(block_id, error = %e, "failed to store embedding");
                        }
                    }
                    if let Some(ref p) = progress {
                        p.on_batch_complete(vectors.len() as u64);
                    }
                    info!(count = vectors.len(), "embedded batch");
                }
                Err(e) => {
                    warn!(error = %e, batch_size = batch.len(), "embedding batch failed");
                }
            }
        }

        if let Some(ref p) = progress {
            p.on_complete(total);
        }
        info!(total, "background embedding complete");
    })
}

#[derive(Debug, Default)]
pub struct IndexStats {
    pub files: u64,
    pub blocks: u64,
    pub skipped: u64,
    pub errors: u64,
    pub deleted_files: u64,
}

#[derive(Debug, Default)]
pub struct ReconcileStats {
    pub updated: u64,
    pub unchanged: u64,
    pub deleted: u64,
}

#[derive(Debug, Default)]
pub struct ReindexStats {
    pub reindexed: u64,
    pub unchanged: u64,
    pub deleted: u64,
    pub errors: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::NopProgress;

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
        let indexer = Indexer::new(store, None).unwrap();
        let stats = indexer.index_directory(dir.path(), &NopProgress).unwrap();

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
        let indexer = Indexer::new(store, None).unwrap();

        let stats1 = indexer.index_directory(dir.path(), &NopProgress).unwrap();
        assert!(stats1.files >= 2);

        let stats2 = indexer.index_directory(dir.path(), &NopProgress).unwrap();
        assert!(stats2.skipped >= 2);
        assert_eq!(stats2.files, 0);
    }

    #[test]
    fn test_reconcile_detects_changes() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("greet.ts"),
            "export function greet() { return 'hello'; }",
        )
        .unwrap();

        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store, None).unwrap();

        indexer.index_directory(dir.path(), &NopProgress).unwrap();
        let initial_count = indexer.store().block_count().unwrap();
        assert!(initial_count >= 1);

        // Modify the file so the hash differs
        std::fs::write(
            src.join("greet.ts"),
            "export function greet2() { return 'hi'; }",
        )
        .unwrap();

        let repo_id = indexer
            .store()
            .ensure_repo(dir.path().to_str().unwrap(), "test")
            .unwrap();
        let stats = indexer.reconcile(dir.path(), repo_id).unwrap();
        assert!(stats.updated >= 1);
    }

    #[test]
    fn test_reconcile_detects_deletions() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("greet.ts"),
            "export function greet() { return 'hello'; }",
        )
        .unwrap();
        std::fs::write(src.join("math.py"), "def add(a, b):\n    return a + b\n").unwrap();

        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store, None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();
        assert!(indexer.store().file_count().unwrap() >= 2);

        // Delete one file
        std::fs::remove_file(src.join("math.py")).unwrap();

        let repo_id = indexer
            .store()
            .ensure_repo(dir.path().to_str().unwrap(), "test")
            .unwrap();
        let stats = indexer.reconcile(dir.path(), repo_id).unwrap();
        assert!(stats.deleted >= 1);
        assert_eq!(indexer.store().file_count().unwrap(), 1);
    }

    #[test]
    fn test_reconcile_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("greet.ts"),
            "export function greet() { return 'hello'; }",
        )
        .unwrap();

        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store, None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();

        let repo_id = indexer
            .store()
            .ensure_repo(dir.path().to_str().unwrap(), "test")
            .unwrap();
        let stats = indexer.reconcile(dir.path(), repo_id).unwrap();
        assert_eq!(stats.updated, 0);
        assert!(stats.unchanged >= 1);
        assert_eq!(stats.deleted, 0);
    }

    #[test]
    fn test_reindex_files_changed() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("greet.ts"),
            "export function greet() { return 'hello'; }",
        )
        .unwrap();

        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store, None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();

        // Modify the file
        std::fs::write(
            src.join("greet.ts"),
            "export function greet() { return 'world'; }",
        )
        .unwrap();

        let stats = indexer
            .reindex_files(dir.path(), &[src.join("greet.ts")])
            .unwrap();
        assert_eq!(stats.reindexed, 1);
        assert_eq!(stats.unchanged, 0);
    }

    #[test]
    fn test_reindex_files_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("greet.ts"),
            "export function greet() { return 'hello'; }",
        )
        .unwrap();

        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store, None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();

        let stats = indexer
            .reindex_files(dir.path(), &[src.join("greet.ts")])
            .unwrap();
        assert_eq!(stats.reindexed, 0);
        assert_eq!(stats.unchanged, 1);
    }

    #[test]
    fn test_backfill_embeddings_without_embedder() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store, None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();

        let backfilled = indexer.backfill_embeddings().unwrap();
        assert_eq!(backfilled, 0);
    }

    #[test]
    fn test_backfill_finds_blocks_without_embeddings() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store, None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();

        let missing = indexer
            .store()
            .get_blocks_without_embeddings(None)
            .unwrap();
        assert!(
            !missing.is_empty(),
            "indexed blocks should appear as missing embeddings"
        );
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
        let indexer = Indexer::new(store, None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();

        let handler_blocks = indexer.store().lookup_symbol("handler", None).unwrap();
        assert!(!handler_blocks.is_empty());

        // UserService should be resolved after the resolution pass
        let unresolved = indexer.store().get_unresolved_dependencies().unwrap();
        assert!(
            !unresolved.iter().any(|(_, name, _)| name == "UserService"),
            "UserService should be resolved, but found in unresolved: {unresolved:?}"
        );

        let service_blocks = indexer.store().lookup_symbol("UserService", None).unwrap();
        assert!(!service_blocks.is_empty());
        let dependents = indexer
            .store()
            .get_dependents(service_blocks[0].id)
            .unwrap();
        assert!(
            !dependents.is_empty(),
            "UserService should have dependents after resolution"
        );
    }
}
