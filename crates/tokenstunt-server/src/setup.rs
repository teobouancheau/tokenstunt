use std::path::Path;

use anyhow::Result;
use tokenstunt_store::Store;

use crate::render;

pub struct SetupContext {
    pub has_embeddings: bool,
    pub index_state: u8,
    pub last_error: Option<String>,
}

pub fn build_setup_report(store: &Store, root: &Path, ctx: &SetupContext) -> Result<String> {
    let file_count = store.file_count()?;
    let block_count = store.block_count()?;
    let (dep_total, dep_resolved) = store.dependency_count()?;

    let state_label = match ctx.index_state {
        tokenstunt_index::INDEX_STATE_RUNNING => "In progress",
        tokenstunt_index::INDEX_STATE_READY => "Ready",
        tokenstunt_index::INDEX_STATE_FAILED => "Failed",
        _ => "Idle",
    };

    let mut out = render::header("Setup", "Token Stunt");
    out.push_str("\n\n");

    out.push_str(&render::kv_line("Root", &root.display().to_string()));
    out.push('\n');
    out.push_str(&render::kv_line(
        "Database",
        &store.db_path().display().to_string(),
    ));
    out.push('\n');
    out.push_str(&render::kv_line("Indexing", state_label));
    out.push('\n');
    out.push_str(&render::kv_line("Files", &file_count.to_string()));
    out.push('\n');
    out.push_str(&render::kv_line("Code Blocks", &block_count.to_string()));
    out.push('\n');
    out.push_str(&render::kv_line(
        "Dependencies",
        &format!("{dep_total} ({dep_resolved} resolved)"),
    ));
    out.push('\n');

    if ctx.index_state == tokenstunt_index::INDEX_STATE_FAILED {
        out.push('\n');
        let err = ctx
            .last_error
            .as_deref()
            .unwrap_or("unknown error");
        out.push_str(&render::notice(&format!("Indexing failed: {err}")));
        out.push('\n');
    } else if file_count == 0 && ctx.index_state == tokenstunt_index::INDEX_STATE_RUNNING {
        out.push('\n');
        out.push_str(&render::notice(
            "Indexing is in progress. Tools will return results once complete.",
        ));
        out.push('\n');
    } else if file_count == 0 {
        out.push('\n');
        out.push_str(&render::notice(
            "No files indexed. Run `tokenstunt index` or restart the server.",
        ));
        out.push('\n');
    }

    let lang_stats = store.get_language_stats()?;
    if !lang_stats.is_empty() {
        out.push('\n');
        let items: Vec<render::TreeItem> = lang_stats
            .iter()
            .map(|(lang, count)| render::TreeItem {
                label: format!("{lang:<16} {count}"),
            })
            .collect();
        out.push_str(&render::render_tree_with_trunk("Languages", &items));
    }

    out.push('\n');
    if ctx.has_embeddings {
        let emb_count = store.embedding_count()?;
        let items = vec![render::TreeItem {
            label: format!(
                "Coverage  {}",
                render::bar_with_label(emb_count as u64, block_count as u64, 20)
            ),
        }];
        out.push_str("  \u{25C6} Embeddings  Configured\n  \u{2502}\n");
        out.push_str(&render::render_list(&items));
    } else {
        out.push_str("  \u{25C6} Embeddings  Not configured\n");
        out.push('\n');
        out.push_str("  To enable semantic search, create a config at\n  `~/.cache/tokenstunt/<project>/config.toml`:\n\n");
        out.push_str("  ```toml\n  [embeddings]\n  enabled = true\n  provider = \"ollama\"\n  model = \"nomic-embed-text\"\n  endpoint = \"http://localhost:11434\"\n  dimensions = 768\n  ```\n");
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tokenstunt_store::{CodeBlockKind, Store};

    fn setup_store() -> (Store, PathBuf) {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        let file_id = store
            .upsert_file(repo_id, "src/main.ts", 111, "typescript", 0)
            .unwrap();
        store
            .insert_code_block(
                file_id,
                "main",
                CodeBlockKind::Function,
                1,
                10,
                "function main() {}",
                "function main()",
                "",
                None,
            )
            .unwrap();
        (store, PathBuf::from("/test"))
    }

    fn ready_ctx(has_embeddings: bool) -> SetupContext {
        SetupContext {
            has_embeddings,
            index_state: tokenstunt_index::INDEX_STATE_READY,
            last_error: None,
        }
    }

    #[test]
    fn test_no_files_indexed() {
        let store = Store::open_in_memory().unwrap();
        let root = PathBuf::from("/empty-project");
        let report = build_setup_report(&store, &root, &ready_ctx(false)).unwrap();
        assert!(report.contains("No files indexed"));
        assert!(report.contains("tokenstunt index"));
        assert!(report.contains("Files"));
        assert!(report.contains("0"));
    }

    #[test]
    fn test_no_embeddings() {
        let (store, root) = setup_store();
        let report = build_setup_report(&store, &root, &ready_ctx(false)).unwrap();
        assert!(report.contains("Not configured"));
        assert!(report.contains("config.toml"));
        assert!(report.contains("Files"));
        assert!(report.contains("Ready"));
    }

    #[test]
    fn test_with_embeddings() {
        let (store, root) = setup_store();
        let block_id = store.lookup_symbol("main", None).unwrap()[0].id;
        store
            .insert_embedding(block_id, &[0.1, 0.2, 0.3], "test")
            .unwrap();
        let report = build_setup_report(&store, &root, &ready_ctx(true)).unwrap();
        assert!(report.contains("Configured"));
        assert!(report.contains("1/1"));
        assert!(report.contains("100%"));
    }

    #[test]
    fn test_partial_embeddings() {
        let (store, root) = setup_store();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        let file_id = store
            .upsert_file(repo_id, "src/other.ts", 222, "typescript", 0)
            .unwrap();
        store
            .insert_code_block(
                file_id,
                "other",
                CodeBlockKind::Function,
                1,
                5,
                "function other() {}",
                "function other()",
                "",
                None,
            )
            .unwrap();

        let block_id = store.lookup_symbol("main", None).unwrap()[0].id;
        store
            .insert_embedding(block_id, &[0.1, 0.2, 0.3], "test")
            .unwrap();

        let report = build_setup_report(&store, &root, &ready_ctx(true)).unwrap();
        assert!(report.contains("1/2"));
        assert!(report.contains("50%"));
    }

    #[test]
    fn test_indexing_in_progress() {
        let store = Store::open_in_memory().unwrap();
        let root = PathBuf::from("/test");
        let ctx = SetupContext {
            has_embeddings: false,
            index_state: tokenstunt_index::INDEX_STATE_RUNNING,
            last_error: None,
        };
        let report = build_setup_report(&store, &root, &ctx).unwrap();
        assert!(report.contains("In progress"));
        assert!(report.contains("Indexing is in progress"));
    }

    #[test]
    fn test_indexing_failed() {
        let store = Store::open_in_memory().unwrap();
        let root = PathBuf::from("/test");
        let ctx = SetupContext {
            has_embeddings: false,
            index_state: tokenstunt_index::INDEX_STATE_FAILED,
            last_error: Some("permission denied".to_string()),
        };
        let report = build_setup_report(&store, &root, &ctx).unwrap();
        assert!(report.contains("Failed"));
        assert!(report.contains("permission denied"));
    }
}
