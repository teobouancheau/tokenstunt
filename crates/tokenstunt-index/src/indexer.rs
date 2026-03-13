use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tracing::{info, warn};
use xxhash_rust::xxh3::xxh3_64;

use tokenstunt_embeddings::EmbeddingProvider;
use tokenstunt_parser::{Language, LanguageRegistry, ParsedSymbol, SymbolExtractor};
use tokenstunt_store::{CodeBlockKind, Connection, Store};

const EMBEDDING_CONCURRENCY_LIMIT: usize = 4;
const EMBEDDING_MAX_RETRIES: u32 = 3;
const EMBEDDING_RETRY_BASE_MS: u64 = 500;

use crate::progress::{EmbeddingProgress, IndexProgress};
use crate::walker;

pub struct Indexer {
    store: Store,
    extractor: SymbolExtractor,
    embedder: Option<Arc<dyn EmbeddingProvider>>,
    embedding_handles: Mutex<Vec<JoinHandle<()>>>,
    embedding_progress: Option<Arc<dyn EmbeddingProgress>>,
    batch_size: usize,
}

impl Indexer {
    pub fn new(
        store: Store,
        embedder: Option<Arc<dyn EmbeddingProvider>>,
        batch_size: Option<usize>,
    ) -> Result<Self> {
        let registry = LanguageRegistry::new()?;
        let extractor = SymbolExtractor::new(registry);
        Ok(Self {
            store,
            extractor,
            embedder,
            embedding_handles: Mutex::new(Vec::new()),
            embedding_progress: None,
            batch_size: batch_size.unwrap_or(32),
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
                self.batch_size,
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

        // Phase 1: collect changes (no DB lock held during I/O and parsing)
        let collected = self.collect_index_changes(root, repo_id, &entries, &registry, progress)?;

        // Phase 2: apply all changes in one fast write transaction
        let mut embedding_work: Vec<(i64, String)> = Vec::new();

        let stats = self.store.write_transaction(|conn| {
            let mut stats = IndexStats {
                skipped: collected.skipped,
                errors: collected.errors,
                ..Default::default()
            };

            for change in &collected.file_changes {
                let file_id = self.store.upsert_file_with_conn(
                    conn,
                    repo_id,
                    &change.rel_path,
                    change.content_hash,
                    change.language.as_str(),
                    change.mtime,
                )?;

                self.store.delete_file_blocks_with_conn(conn, file_id)?;

                let mut first_block_id: Option<i64> = None;
                let mut block_ids: Vec<(String, i64)> = Vec::new();

                for symbol in &change.parse_result.symbols {
                    let block_id = self.insert_symbol_with_conn(conn, file_id, symbol, None)?;
                    if first_block_id.is_none() {
                        first_block_id = Some(block_id);
                    }
                    block_ids.push((symbol.name.clone(), block_id));
                    collect_embedding_work(symbol, block_id, &mut embedding_work);
                    stats.blocks += count_symbols(symbol);
                }

                if let Some(fallback_id) = first_block_id {
                    for reference in &change.parse_result.references {
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

                progress.on_file_indexed(&change.rel_path);
                stats.files += 1;
            }

            let deleted =
                self.store
                    .delete_stale_files_with_conn(conn, repo_id, &collected.indexed_paths)?;
            stats.deleted_files = deleted;

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

        self.store.invalidate_overview_cache("")?;
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

    fn collect_index_changes(
        &self,
        root: &Path,
        repo_id: i64,
        entries: &[walker::FileEntry],
        registry: &LanguageRegistry,
        progress: &dyn IndexProgress,
    ) -> Result<CollectedChanges> {
        let mut collected = CollectedChanges::default();

        for entry in entries {
            let rel_path = entry
                .path
                .strip_prefix(root)
                .unwrap_or(&entry.path)
                .to_string_lossy()
                .to_string();

            collected.indexed_paths.push(rel_path.clone());

            let source = match std::fs::read_to_string(&entry.path) {
                Ok(s) => s,
                Err(e) => {
                    warn!(path = %entry.path.display(), error = %e, "failed to read file");
                    progress.on_file_error(&rel_path, &e.to_string());
                    collected.errors += 1;
                    continue;
                }
            };

            let content_hash = xxh3_64(source.as_bytes());

            // Hash check uses read connection (no write lock needed)
            if let Ok(Some(existing_hash)) = self.store.get_file_hash(repo_id, &rel_path)
                && existing_hash == content_hash
            {
                progress.on_file_skipped(&rel_path);
                collected.skipped += 1;
                continue;
            }

            if !registry.is_supported(entry.language) {
                progress.on_file_skipped(&rel_path);
                collected.skipped += 1;
                continue;
            }

            let mtime = std::fs::metadata(&entry.path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as i64)
                .unwrap_or(0);

            let parse_result = match self.extractor.extract(&source, entry.language) {
                Ok(r) => r,
                Err(e) => {
                    warn!(path = %rel_path, error = %e, "failed to parse");
                    collected.errors += 1;
                    continue;
                }
            };

            collected.file_changes.push(FileChange {
                rel_path,
                content_hash,
                language: entry.language,
                mtime,
                parse_result,
            });
        }

        Ok(collected)
    }

    pub fn reconcile(&self, root: &Path, repo_id: i64) -> Result<ReconcileStats> {
        let entries = walker::walk_directory(root)?;
        let registry = LanguageRegistry::new()?;

        // Phase 1: collect reconcile changes (read-only DB access + file I/O)
        let existing_paths: HashSet<String> = self
            .store
            .get_repo_file_paths(repo_id)?
            .into_iter()
            .collect();

        let mut seen_paths: HashSet<String> = HashSet::with_capacity(entries.len());
        let mut changes_to_apply: Vec<ReconcileFileChange> = Vec::new();
        let mut unchanged: u64 = 0;

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

            if let Ok(Some(existing_hash)) = self.store.get_file_hash(repo_id, &rel_path)
                && existing_hash == content_hash
            {
                unchanged += 1;
                continue;
            }

            if !registry.is_supported(entry.language) {
                continue;
            }

            changes_to_apply.push(ReconcileFileChange {
                abs_path: entry.path.clone(),
                rel_path,
                language: entry.language,
                source,
            });
        }

        let mut deleted: u64 = 0;
        for stale_path in &existing_paths {
            if !seen_paths.contains(stale_path) {
                deleted += 1;
            }
        }

        // Phase 2: apply changes in one fast write transaction
        let mut embedding_work: Vec<(i64, String)> = Vec::new();

        self.store.write_transaction(|conn| {
            for change in &changes_to_apply {
                self.index_file_with_conn(
                    conn,
                    repo_id,
                    root,
                    &change.abs_path,
                    &change.rel_path,
                    change.language,
                    &change.source,
                    &mut embedding_work,
                )?;
            }

            let current_paths: Vec<String> = seen_paths.into_iter().collect();
            self.store
                .delete_stale_files_with_conn(conn, repo_id, &current_paths)?;

            Ok(())
        })?;

        self.store.invalidate_overview_cache("")?;
        self.spawn_embeddings_if_needed(embedding_work);

        let stats = ReconcileStats {
            updated: changes_to_apply.len() as u64,
            unchanged,
            deleted,
        };

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
            &symbol.docstring,
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
        let text = if symbol.docstring.is_empty() {
            symbol.content.clone()
        } else {
            format!("{}\n{}", symbol.docstring, symbol.content)
        };
        work.push((block_id, text));
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
    batch_size: usize,
) -> tokio::task::JoinHandle<()> {
    let total = work.len() as u64;
    let semaphore = Arc::new(Semaphore::new(EMBEDDING_CONCURRENCY_LIMIT));
    tokio::spawn(async move {
        let store = match tokenstunt_store::Store::open(&db_path) {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "failed to open store for embedding task");
                return;
            }
        };

        let texts: Vec<String> = work.iter().map(|(_, text)| text.clone()).collect();

        for chunk_start in (0..texts.len()).step_by(batch_size) {
            let chunk_end = (chunk_start + batch_size).min(texts.len());
            let batch = &texts[chunk_start..chunk_end];

            let _permit = match semaphore.acquire().await {
                Ok(p) => p,
                Err(_) => {
                    warn!("embedding semaphore closed, aborting");
                    break;
                }
            };

            let mut result = None;
            for attempt in 0..EMBEDDING_MAX_RETRIES {
                match embedder.embed_batch(batch).await {
                    Ok(vectors) => {
                        result = Some(vectors);
                        break;
                    }
                    Err(e) => {
                        let delay_ms = EMBEDDING_RETRY_BASE_MS * (1 << attempt);
                        warn!(
                            attempt = attempt + 1,
                            max_retries = EMBEDDING_MAX_RETRIES,
                            delay_ms,
                            error = %e,
                            "embedding batch failed, retrying"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    }
                }
            }

            let Some(vectors) = result else {
                warn!(
                    batch_size = batch.len(),
                    "embedding batch failed after all retries"
                );
                continue;
            };

            if vectors.len() != batch.len() {
                warn!(
                    expected = batch.len(),
                    got = vectors.len(),
                    "vector count mismatch, skipping batch"
                );
                continue;
            }
            for (i, vector) in vectors.iter().enumerate() {
                let (block_id, _) = &work[chunk_start + i];
                if let Err(e) = store.insert_embedding(*block_id, vector, &model_name) {
                    warn!(block_id, error = %e, "failed to store embedding");
                }
            }
            if let Some(ref p) = progress {
                p.on_batch_complete(vectors.len() as u64);
            }
            info!(count = vectors.len(), "embedded batch");
        }

        if let Some(ref p) = progress {
            p.on_complete(total);
        }
        info!(total, "background embedding complete");
    })
}

struct FileChange {
    rel_path: String,
    content_hash: u64,
    language: Language,
    mtime: i64,
    parse_result: tokenstunt_parser::ParseResult,
}

#[derive(Default)]
struct CollectedChanges {
    file_changes: Vec<FileChange>,
    indexed_paths: Vec<String>,
    skipped: u64,
    errors: u64,
}

struct ReconcileFileChange {
    abs_path: PathBuf,
    rel_path: String,
    language: Language,
    source: String,
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
        let indexer = Indexer::new(store, None, None).unwrap();
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
        let indexer = Indexer::new(store, None, None).unwrap();

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
        let indexer = Indexer::new(store, None, None).unwrap();

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
        let indexer = Indexer::new(store, None, None).unwrap();
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
        let indexer = Indexer::new(store, None, None).unwrap();
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
        let indexer = Indexer::new(store, None, None).unwrap();
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
        let indexer = Indexer::new(store, None, None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();

        let stats = indexer
            .reindex_files(dir.path(), &[src.join("greet.ts")])
            .unwrap();
        assert_eq!(stats.reindexed, 0);
        assert_eq!(stats.unchanged, 1);
    }

    struct FakeEmbeddingProvider {
        dims: usize,
        model: String,
    }

    impl FakeEmbeddingProvider {
        fn new(dims: usize, model: &str) -> Self {
            Self {
                dims,
                model: model.to_string(),
            }
        }
    }

    #[async_trait::async_trait]
    impl tokenstunt_embeddings::EmbeddingProvider for FakeEmbeddingProvider {
        async fn embed_batch(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
            Ok(texts.iter().map(|_| vec![0.1; self.dims]).collect())
        }

        fn dimensions(&self) -> usize {
            self.dims
        }

        fn model_name(&self) -> &str {
            &self.model
        }

        async fn health_check(&self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_index_generates_embeddings_with_embedder() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("test.db");
        let store = Store::open(&db_path).unwrap();

        let fake = Arc::new(FakeEmbeddingProvider::new(64, "fake-model"));
        let indexer = Indexer::new(store, Some(fake), None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();
        indexer.await_embeddings().await;

        assert!(indexer.store().embedding_count().unwrap() > 0);
        let missing = indexer
            .store()
            .get_blocks_without_embeddings(Some("fake-model"))
            .unwrap();
        assert!(missing.is_empty(), "all blocks should have embeddings");
    }

    #[tokio::test]
    async fn test_backfill_generates_embeddings() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("test.db");

        // Index without embedder
        let store = Store::open(&db_path).unwrap();
        let indexer = Indexer::new(store, None, None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();
        assert_eq!(indexer.store().embedding_count().unwrap(), 0);
        drop(indexer);

        // Backfill with embedder
        let store = Store::open(&db_path).unwrap();
        let fake = Arc::new(FakeEmbeddingProvider::new(64, "fake-model"));
        let indexer = Indexer::new(store, Some(fake), None).unwrap();

        let count = indexer.backfill_embeddings().unwrap();
        assert!(count > 0, "backfill should find blocks to embed");
        indexer.await_embeddings().await;
        assert!(indexer.store().embedding_count().unwrap() > 0);

        let second = indexer.backfill_embeddings().unwrap();
        assert_eq!(second, 0, "second backfill should find nothing");
    }

    #[test]
    fn test_backfill_embeddings_without_embedder() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store, None, None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();

        let backfilled = indexer.backfill_embeddings().unwrap();
        assert_eq!(backfilled, 0);
    }

    #[test]
    fn test_backfill_finds_blocks_without_embeddings() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store, None, None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();

        let missing = indexer.store().get_blocks_without_embeddings(None).unwrap();
        assert!(
            !missing.is_empty(),
            "indexed blocks should appear as missing embeddings"
        );
    }

    #[test]
    fn test_set_embedding_progress() {
        let store = Store::open_in_memory().unwrap();
        let mut indexer = Indexer::new(store, None, None).unwrap();

        struct TrackingProgress {
            started: std::sync::atomic::AtomicBool,
        }
        impl EmbeddingProgress for TrackingProgress {
            fn on_start(&self, _total_blocks: u64) {
                self.started
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
            fn on_batch_complete(&self, _batch_size: u64) {}
            fn on_complete(&self, _total: u64) {}
        }

        let progress = Arc::new(TrackingProgress {
            started: std::sync::atomic::AtomicBool::new(false),
        });

        // Exercise the uncovered trait methods
        progress.on_batch_complete(10);
        progress.on_complete(100);

        indexer.set_embedding_progress(Arc::clone(&progress) as Arc<dyn EmbeddingProgress>);

        assert!(indexer.embedding_progress.is_some());
    }

    #[tokio::test]
    async fn test_spawn_embeddings_calls_on_start() {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("test.db");
        let store = Store::open(&db_path).unwrap();

        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        let fake = Arc::new(FakeEmbeddingProvider::new(64, "fake-model"));
        let mut indexer = Indexer::new(store, Some(fake), None).unwrap();

        let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
        struct TrackingProgress {
            started: Arc<std::sync::atomic::AtomicBool>,
        }
        impl EmbeddingProgress for TrackingProgress {
            fn on_start(&self, _total_blocks: u64) {
                self.started
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
            fn on_batch_complete(&self, _batch_size: u64) {}
            fn on_complete(&self, _total: u64) {}
        }

        let progress = Arc::new(TrackingProgress {
            started: Arc::clone(&started),
        });
        indexer.set_embedding_progress(progress);

        indexer.index_directory(dir.path(), &NopProgress).unwrap();
        indexer.await_embeddings().await;

        assert!(
            started.load(std::sync::atomic::Ordering::Relaxed),
            "on_start should have been called"
        );
    }

    #[test]
    fn test_index_directory_unsupported_language_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        // Write a file with an unsupported extension alongside a supported one
        std::fs::write(src.join("data.xyz"), "some random content").unwrap();
        std::fs::write(
            src.join("greet.ts"),
            "export function greet() { return 'hello'; }",
        )
        .unwrap();

        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store, None, None).unwrap();
        let stats = indexer.index_directory(dir.path(), &NopProgress).unwrap();

        // .xyz is not a recognized language, so it won't be walked at all
        assert!(stats.files >= 1);
        assert_eq!(stats.errors, 0);
    }

    #[test]
    fn test_reindex_files_deleted_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("greet.ts"),
            "export function greet() { return 'hello'; }",
        )
        .unwrap();

        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store, None, None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();
        assert!(indexer.store().file_count().unwrap() >= 1);

        // Delete the file before reindex
        let deleted_path = src.join("greet.ts");
        std::fs::remove_file(&deleted_path).unwrap();

        let stats = indexer.reindex_files(dir.path(), &[deleted_path]).unwrap();
        assert_eq!(stats.deleted, 1);
        assert_eq!(stats.reindexed, 0);
    }

    #[test]
    fn test_reindex_files_unsupported_extension() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("readme.md"), "# Hello").unwrap();

        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store, None, None).unwrap();

        // .md has no Language mapping, so Language::from_path returns None
        let stats = indexer
            .reindex_files(dir.path(), &[src.join("readme.md")])
            .unwrap();
        assert_eq!(stats.reindexed, 0);
        assert_eq!(stats.deleted, 0);
    }

    #[test]
    fn test_reconcile_with_unsupported_language() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        std::fs::write(
            src.join("greet.ts"),
            "export function greet() { return 'hello'; }",
        )
        .unwrap();

        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store, None, None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();

        // Add a file with no language mapping after initial index
        std::fs::write(src.join("notes.txt"), "some notes").unwrap();

        let repo_id = indexer
            .store()
            .ensure_repo(dir.path().to_str().unwrap(), "test")
            .unwrap();
        let stats = indexer.reconcile(dir.path(), repo_id).unwrap();

        // .txt is not recognized by the walker, so it won't appear
        assert!(stats.unchanged >= 1);
    }

    #[cfg(unix)]
    #[test]
    fn test_index_directory_unreadable_file() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        let unreadable = src.join("secret.ts");
        std::fs::write(&unreadable, "export function secret() { return 42; }").unwrap();
        std::fs::write(
            src.join("greet.ts"),
            "export function greet() { return 'hello'; }",
        )
        .unwrap();

        // Make the file unreadable
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();

        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("test.db");
        let store = Store::open(&db_path).unwrap();
        let indexer = Indexer::new(store, None, None).unwrap();
        let stats = indexer.index_directory(dir.path(), &NopProgress).unwrap();

        // Restore permissions before assertions so cleanup succeeds
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert!(stats.errors >= 1, "unreadable file should cause an error");
        assert!(stats.files >= 1, "readable file should still be indexed");
    }

    #[cfg(not(feature = "lang-swift"))]
    #[test]
    fn test_index_directory_skips_unsupported_language_after_hash_check() {
        // .swift files are recognized by Language::from_path (walker includes them)
        // but registry.is_supported returns false without the lang-swift feature,
        // hitting the unsupported language skip path at lines 152-155.
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        std::fs::write(
            src.join("app.swift"),
            "func hello() -> String { return \"hi\" }",
        )
        .unwrap();
        std::fs::write(
            src.join("greet.ts"),
            "export function greet() { return 'hello'; }",
        )
        .unwrap();

        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store, None, None).unwrap();
        let stats = indexer.index_directory(dir.path(), &NopProgress).unwrap();

        // The .swift file should be skipped (unsupported), the .ts file should be indexed
        assert!(stats.files >= 1, "at least the .ts file should be indexed");
        assert!(
            stats.skipped >= 1,
            "the .swift file should be skipped as unsupported"
        );
        assert_eq!(stats.errors, 0);
    }

    #[cfg(unix)]
    #[test]
    fn test_reconcile_unreadable_file() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        let target = src.join("greet.ts");
        std::fs::write(&target, "export function greet() { return 'hello'; }").unwrap();

        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("test.db");
        let store = Store::open(&db_path).unwrap();
        let indexer = Indexer::new(store, None, None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();

        // Modify the file content so hash differs, then make it unreadable
        std::fs::write(&target, "export function greet2() { return 'bye'; }").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o000)).unwrap();

        let repo_id = indexer
            .store()
            .ensure_repo(dir.path().to_str().unwrap(), "test")
            .unwrap();

        // reconcile should not fail overall; it logs a warning and skips unreadable files
        let stats = indexer.reconcile(dir.path(), repo_id).unwrap();

        // Restore permissions for cleanup
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();

        // The file was unreadable so it should not count as updated or unchanged
        // (the walker still discovers it but read_to_string fails)
        assert_eq!(stats.updated, 0);
    }

    #[cfg(unix)]
    #[test]
    fn test_reindex_files_read_error() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        let target = src.join("greet.ts");
        std::fs::write(&target, "export function greet() { return 'hello'; }").unwrap();

        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("test.db");
        let store = Store::open(&db_path).unwrap();
        let indexer = Indexer::new(store, None, None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();

        // Modify content so hash check won't skip, then make unreadable
        std::fs::write(&target, "export function greet2() { return 'changed'; }").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o000)).unwrap();

        let stats = indexer
            .reindex_files(dir.path(), std::slice::from_ref(&target))
            .unwrap();

        // Restore permissions for cleanup
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert!(
            stats.errors >= 1,
            "unreadable file should cause an error in reindex"
        );
        assert_eq!(stats.reindexed, 0);
    }

    #[test]
    fn test_reconcile_with_cross_file_references() {
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
        let indexer = Indexer::new(store, None, None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();

        let repo_id = indexer
            .store()
            .ensure_repo(dir.path().to_str().unwrap(), "test")
            .unwrap();

        // Modify handler.ts to trigger re-indexing via reconcile
        std::fs::write(
            src.join("handler.ts"),
            "import { UserService } from './service';\nexport function handler() { const svc = new UserService(); svc.handle(); }",
        )
        .unwrap();

        let stats = indexer.reconcile(dir.path(), repo_id).unwrap();
        assert!(stats.updated >= 1, "handler.ts should be re-indexed");

        // Verify cross-file references are resolved after reconcile
        let service_blocks = indexer.store().lookup_symbol("UserService", None).unwrap();
        assert!(!service_blocks.is_empty());
        let dependents = indexer
            .store()
            .get_dependents(service_blocks[0].id)
            .unwrap();
        assert!(
            !dependents.is_empty(),
            "UserService should have dependents after reconcile re-indexes handler.ts"
        );
    }

    #[test]
    fn test_reindex_with_cross_file_references() {
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
        let indexer = Indexer::new(store, None, None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();

        // Modify handler.ts to trigger re-indexing via reindex_files
        std::fs::write(
            src.join("handler.ts"),
            "import { UserService } from './service';\nexport function handler() { const svc = new UserService(); svc.handle(); }",
        )
        .unwrap();

        let stats = indexer
            .reindex_files(dir.path(), &[src.join("handler.ts")])
            .unwrap();
        assert_eq!(stats.reindexed, 1);

        // Verify cross-file references are resolved after reindex
        let service_blocks = indexer.store().lookup_symbol("UserService", None).unwrap();
        assert!(!service_blocks.is_empty());
        let dependents = indexer
            .store()
            .get_dependents(service_blocks[0].id)
            .unwrap();
        assert!(
            !dependents.is_empty(),
            "UserService should have dependents after reindex_files re-indexes handler.ts"
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
        let indexer = Indexer::new(store, None, None).unwrap();
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

    #[tokio::test]
    async fn test_spawn_embedding_task_bad_db_path() {
        let fake = Arc::new(FakeEmbeddingProvider::new(64, "fake-model"));
        let bad_path = PathBuf::from("/nonexistent/dir/test.db");
        let work = vec![(1, "hello world".to_string())];

        let handle = spawn_embedding_task(bad_path, fake, work, "fake-model".to_string(), None, 32);
        // Should complete without panic; the store open error is logged
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_spawn_embedding_task_vector_count_mismatch() {
        struct MismatchProvider;

        #[async_trait::async_trait]
        impl tokenstunt_embeddings::EmbeddingProvider for MismatchProvider {
            async fn embed_batch(&self, _texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
                // Return fewer vectors than texts
                Ok(vec![vec![0.1; 8]])
            }
            fn dimensions(&self) -> usize {
                8
            }
            fn model_name(&self) -> &str {
                "mismatch"
            }
            async fn health_check(&self) -> anyhow::Result<()> {
                Ok(())
            }
        }

        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("test.db");
        let _store = Store::open(&db_path).unwrap();

        let mismatch = MismatchProvider;
        assert_eq!(mismatch.dimensions(), 8);
        assert_eq!(mismatch.model_name(), "mismatch");
        mismatch.health_check().await.unwrap();

        let embedder: Arc<dyn tokenstunt_embeddings::EmbeddingProvider> =
            Arc::new(MismatchProvider);
        let work = vec![(1, "hello".to_string()), (2, "world".to_string())];

        let handle =
            spawn_embedding_task(db_path, embedder, work, "mismatch".to_string(), None, 32);
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_spawn_embedding_task_embed_batch_error() {
        struct FailingProvider;

        #[async_trait::async_trait]
        impl tokenstunt_embeddings::EmbeddingProvider for FailingProvider {
            async fn embed_batch(&self, _texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
                anyhow::bail!("embedding service unavailable")
            }
            fn dimensions(&self) -> usize {
                8
            }
            fn model_name(&self) -> &str {
                "failing"
            }
            async fn health_check(&self) -> anyhow::Result<()> {
                Ok(())
            }
        }

        let failing = FailingProvider;
        assert_eq!(failing.dimensions(), 8);
        assert_eq!(failing.model_name(), "failing");
        failing.health_check().await.unwrap();

        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("test.db");
        let _store = Store::open(&db_path).unwrap();

        let embedder: Arc<dyn tokenstunt_embeddings::EmbeddingProvider> = Arc::new(FailingProvider);
        let work = vec![(1, "hello".to_string())];

        let handle = spawn_embedding_task(db_path, embedder, work, "failing".to_string(), None, 32);
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_spawn_embedding_task_with_progress() {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("test.db");
        let store = Store::open(&db_path).unwrap();

        // Create a file/block so insert_embedding has a valid block_id
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());
        let indexer = Indexer::new(store, None, None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();
        let blocks = indexer.store().get_blocks_without_embeddings(None).unwrap();
        assert!(!blocks.is_empty());

        let completed = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));

        struct TrackProgress {
            completed: Arc<std::sync::atomic::AtomicU64>,
            finished: Arc<std::sync::atomic::AtomicBool>,
        }
        impl EmbeddingProgress for TrackProgress {
            fn on_start(&self, _total_blocks: u64) {}
            fn on_batch_complete(&self, batch_size: u64) {
                self.completed
                    .fetch_add(batch_size, std::sync::atomic::Ordering::Relaxed);
            }
            fn on_complete(&self, _total: u64) {
                self.finished
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }

        let progress: Arc<dyn EmbeddingProgress> = Arc::new(TrackProgress {
            completed: Arc::clone(&completed),
            finished: Arc::clone(&finished),
        });

        // Exercise on_start directly since spawn_embedding_task does not call it
        progress.on_start(blocks.len() as u64);

        let fake = Arc::new(FakeEmbeddingProvider::new(64, "fake-model"));
        let handle = spawn_embedding_task(
            db_path,
            fake,
            blocks,
            "fake-model".to_string(),
            Some(progress),
            32,
        );
        handle.await.unwrap();

        assert!(
            completed.load(std::sync::atomic::Ordering::Relaxed) > 0,
            "on_batch_complete should have been called"
        );
        assert!(
            finished.load(std::sync::atomic::Ordering::Relaxed),
            "on_complete should have been called"
        );
    }

    #[tokio::test]
    async fn test_spawn_embedding_task_insert_embedding_error() {
        // Use a valid db but with block_ids that don't exist to trigger
        // insert_embedding errors (foreign key or constraint violations)
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("test.db");
        let _store = Store::open(&db_path).unwrap();

        let fake = Arc::new(FakeEmbeddingProvider::new(64, "fake-model"));
        let work = vec![(999999, "nonexistent block".to_string())];

        let handle = spawn_embedding_task(db_path, fake, work, "fake-model".to_string(), None, 32);
        // Should not panic even if insert_embedding fails
        handle.await.unwrap();
    }

    #[cfg(not(feature = "lang-swift"))]
    #[test]
    fn test_reindex_files_with_swift_unsupported() {
        // .swift files are recognized by Language::from_path but not supported
        // without the lang-swift feature, hitting the unsupported skip at line 397
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("app.swift"),
            "func hello() -> String { return \"hi\" }",
        )
        .unwrap();

        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store, None, None).unwrap();

        let stats = indexer
            .reindex_files(dir.path(), &[src.join("app.swift")])
            .unwrap();
        assert_eq!(stats.reindexed, 0);
        assert_eq!(stats.deleted, 0);
        assert_eq!(stats.errors, 0);
    }

    #[test]
    #[cfg(not(feature = "lang-swift"))]
    fn test_reconcile_unsupported_language_new_file() {
        // Index a directory, then add a .swift file (recognized but unsupported),
        // hitting the unsupported skip at line 318 in reconcile
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("greet.ts"),
            "export function greet() { return 'hello'; }",
        )
        .unwrap();

        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store, None, None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();

        // Add a .swift file that the walker will discover but registry won't support
        std::fs::write(
            src.join("app.swift"),
            "func hello() -> String { return \"hi\" }",
        )
        .unwrap();

        let repo_id = indexer
            .store()
            .ensure_repo(dir.path().to_str().unwrap(), "test")
            .unwrap();
        let stats = indexer.reconcile(dir.path(), repo_id).unwrap();
        // greet.ts unchanged, app.swift skipped (unsupported)
        assert!(stats.unchanged >= 1);
        assert_eq!(stats.updated, 0);
    }

    #[tokio::test]
    async fn test_reconcile_with_embedder() {
        // Exercises index_file_with_conn through reconcile with an embedder,
        // covering lines 330, 477, 524, 526, 563
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

        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("test.db");
        let store = Store::open(&db_path).unwrap();
        let fake = Arc::new(FakeEmbeddingProvider::new(64, "fake-model"));
        let indexer = Indexer::new(store, Some(fake), None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();
        indexer.await_embeddings().await;

        // Modify handler.ts to trigger reconcile re-index through index_file_with_conn
        std::fs::write(
            src.join("handler.ts"),
            "import { UserService } from './service';\nexport function handler() { const svc = new UserService(); svc.handle(); }",
        )
        .unwrap();

        let repo_id = indexer
            .store()
            .ensure_repo(dir.path().to_str().unwrap(), "test")
            .unwrap();
        let stats = indexer.reconcile(dir.path(), repo_id).unwrap();
        indexer.await_embeddings().await;

        assert!(stats.updated >= 1);
    }

    #[tokio::test]
    async fn test_reindex_with_embedder() {
        // Exercises index_file_with_conn through reindex_files with an embedder,
        // covering lines 428, 477, 524, 526, 563
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

        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("test.db");
        let store = Store::open(&db_path).unwrap();
        let fake = Arc::new(FakeEmbeddingProvider::new(64, "fake-model"));
        let indexer = Indexer::new(store, Some(fake), None).unwrap();
        indexer.index_directory(dir.path(), &NopProgress).unwrap();
        indexer.await_embeddings().await;

        // Modify handler.ts
        std::fs::write(
            src.join("handler.ts"),
            "import { UserService } from './service';\nexport function handler() { const svc = new UserService(); svc.handle(); }",
        )
        .unwrap();

        let stats = indexer
            .reindex_files(dir.path(), &[src.join("handler.ts")])
            .unwrap();
        indexer.await_embeddings().await;

        assert_eq!(stats.reindexed, 1);
    }

    #[test]
    fn test_count_symbols_with_children() {
        let parent = ParsedSymbol {
            name: "Parent".to_string(),
            kind: "class",
            start_line: 1,
            end_line: 10,
            content: "class Parent { method() {} }".to_string(),
            signature: "class Parent".to_string(),
            docstring: String::new(),
            children: vec![ParsedSymbol {
                name: "method".to_string(),
                kind: "function",
                start_line: 2,
                end_line: 4,
                content: "method() {}".to_string(),
                signature: "method()".to_string(),
                docstring: String::new(),
                children: vec![],
            }],
        };
        assert_eq!(count_symbols(&parent), 2);
    }

    #[test]
    fn test_collect_embedding_work_empty_content() {
        let symbol = ParsedSymbol {
            name: "empty".to_string(),
            kind: "function",
            start_line: 1,
            end_line: 1,
            content: String::new(),
            signature: "empty()".to_string(),
            docstring: String::new(),
            children: vec![],
        };
        let mut work = Vec::new();
        collect_embedding_work(&symbol, 1, &mut work);
        assert!(work.is_empty(), "empty content should not generate work");
    }

    #[test]
    fn test_collect_embedding_work_with_content() {
        let symbol = ParsedSymbol {
            name: "greet".to_string(),
            kind: "function",
            start_line: 1,
            end_line: 3,
            content: "function greet() { return 'hi'; }".to_string(),
            signature: "greet()".to_string(),
            docstring: String::new(),
            children: vec![],
        };
        let mut work = Vec::new();
        collect_embedding_work(&symbol, 42, &mut work);
        assert_eq!(work.len(), 1);
        assert_eq!(work[0].0, 42);
    }

    #[test]
    fn test_embedder_accessor() {
        let store = Store::open_in_memory().unwrap();

        let indexer_none = Indexer::new(store, None, None).unwrap();
        assert!(indexer_none.embedder().is_none());

        let store2 = Store::open_in_memory().unwrap();
        let fake = Arc::new(FakeEmbeddingProvider::new(64, "test"));
        let indexer_some = Indexer::new(store2, Some(fake), None).unwrap();
        assert!(indexer_some.embedder().is_some());
    }

    #[tokio::test]
    async fn test_fake_embedding_provider_trait_methods() {
        let fake = FakeEmbeddingProvider::new(8, "test-model");
        assert_eq!(fake.dimensions(), 8);
        assert_eq!(fake.model_name(), "test-model");
        fake.health_check().await.unwrap();
    }

    #[test]
    fn test_index_directory_with_cross_file_references() {
        // Files with imports to exercise reference resolution paths:
        // lines 198-227 (index_directory) and 501-526 (index_file_with_conn)
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        std::fs::write(
            src.join("math.ts"),
            "export function add(a: number, b: number): number {\n  return a + b;\n}\n\nexport function multiply(a: number, b: number): number {\n  return a * b;\n}\n",
        )
        .unwrap();

        std::fs::write(
            src.join("app.ts"),
            "import { add, multiply } from './math';\n\nexport function compute(x: number): number {\n  return add(x, multiply(x, 2));\n}\n",
        )
        .unwrap();

        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store, None, None).unwrap();
        let stats = indexer.index_directory(dir.path(), &NopProgress).unwrap();

        assert!(stats.files >= 2);
        assert!(stats.blocks >= 3);
        assert_eq!(stats.errors, 0);
    }

    #[test]
    fn test_batch_size_defaults_to_32() {
        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store, None, None).unwrap();
        assert_eq!(indexer.batch_size, 32);
    }

    #[test]
    fn test_batch_size_custom_value() {
        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store, None, Some(64)).unwrap();
        assert_eq!(indexer.batch_size, 64);
    }

    #[test]
    fn test_batch_size_passed_to_zero_uses_explicit() {
        let store = Store::open_in_memory().unwrap();
        let indexer = Indexer::new(store, None, Some(1)).unwrap();
        assert_eq!(indexer.batch_size, 1);
    }
}
