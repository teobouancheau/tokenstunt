use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use notify::{Event, RecursiveMode, Watcher};
use tracing::{error, info};

use crate::Indexer;

pub struct FileWatcher {
    _watcher: notify::RecommendedWatcher,
}

impl FileWatcher {
    pub fn start(indexer: Arc<Indexer>, root: PathBuf) -> Result<Self> {
        let pending = Arc::new(std::sync::Mutex::new(HashSet::<PathBuf>::new()));
        let pending_for_task = Arc::clone(&pending);

        let indexer_for_task = Arc::clone(&indexer);
        let root_for_task = root.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
            loop {
                interval.tick().await;
                let paths: Vec<PathBuf> = {
                    let mut set = match pending_for_task.lock() {
                        Ok(s) => s,
                        Err(e) => {
                            error!(error = %e, "pending set lock poisoned");
                            continue;
                        }
                    };
                    set.drain().collect()
                };
                if paths.is_empty() {
                    continue;
                }
                info!(count = paths.len(), "reindexing changed files");
                if let Err(e) = indexer_for_task.reindex_files(&root_for_task, &paths) {
                    error!(error = %e, "reindex error");
                }
            }
        });

        let mut watcher =
            notify::recommended_watcher(move |res: std::result::Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    if let Ok(mut set) = pending.lock() {
                        for path in event.paths {
                            set.insert(path);
                        }
                    }
                }
            })?;

        watcher.watch(root.as_ref(), RecursiveMode::Recursive)?;

        Ok(Self { _watcher: watcher })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokenstunt_store::Store;

    /// File watcher integration test.
    /// Marked `#[ignore]` because file system event delivery timing is
    /// platform-dependent and can cause flaky results in CI.
    #[ignore]
    #[tokio::test]
    async fn test_watcher_detects_file_change() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("greet.ts"), "export function greet() {}").unwrap();

        let store = Store::open_in_memory().unwrap();
        let indexer = Arc::new(Indexer::new(store, None).unwrap());
        indexer.index_directory(dir.path()).unwrap();

        let _watcher = FileWatcher::start(Arc::clone(&indexer), dir.path().to_path_buf()).unwrap();

        std::fs::write(src.join("new.ts"), "export function newFn() {}").unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

        let results = indexer.store().lookup_symbol("newFn", None).unwrap();
        assert!(!results.is_empty());
    }
}
