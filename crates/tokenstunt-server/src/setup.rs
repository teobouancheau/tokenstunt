use std::path::Path;

use anyhow::Result;
use tokenstunt_store::Store;

pub fn build_setup_report(store: &Store, root: &Path, has_embeddings: bool) -> Result<String> {
    let file_count = store.file_count()?;
    let block_count = store.block_count()?;
    let (dep_total, dep_resolved) = store.dependency_count()?;

    let mut out = String::from("## TokenStunt Setup\n\n");

    out.push_str("### Index Health\n\n");
    out.push_str(&format!("- **Root**: {}\n", root.display()));
    out.push_str(&format!("- **Database**: {}\n", store.db_path().display()));
    out.push_str(&format!("- **Files indexed**: {file_count}\n"));
    out.push_str(&format!("- **Code blocks**: {block_count}\n"));
    out.push_str(&format!(
        "- **Dependencies**: {dep_total} ({dep_resolved} resolved)\n"
    ));

    if file_count == 0 {
        out.push_str("\n> **Warning**: No files indexed. Run `tokenstunt index` or restart the server.\n");
    }

    let lang_stats = store.get_language_stats()?;
    if !lang_stats.is_empty() {
        out.push_str("\n### Languages Detected\n\n");
        for (lang, count) in &lang_stats {
            out.push_str(&format!("- {lang}: {count} files\n"));
        }
    }

    out.push_str("\n### Embeddings\n\n");
    if has_embeddings {
        let emb_count = store.embedding_count()?;
        let coverage = if block_count > 0 {
            (emb_count as f64 / block_count as f64 * 100.0) as u32
        } else {
            0
        };
        out.push_str(&format!(
            "- **Status**: Configured\n- **Vectors**: {emb_count}/{block_count} ({coverage}% coverage)\n"
        ));
    } else {
        out.push_str("- **Status**: Not configured\n\n");
        out.push_str("To enable semantic search, add to `.tokenstunt/config.toml`:\n\n");
        out.push_str("```toml\n[embeddings]\nenabled = true\nprovider = \"ollama\"\nmodel = \"nomic-embed-text\"\nendpoint = \"http://localhost:11434\"\ndimensions = 768\n```\n");
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
                None,
            )
            .unwrap();
        (store, PathBuf::from("/test"))
    }

    #[test]
    fn test_no_embeddings() {
        let (store, root) = setup_store();
        let report = build_setup_report(&store, &root, false).unwrap();
        assert!(report.contains("Not configured"));
        assert!(report.contains("config.toml"));
        assert!(report.contains("Files indexed"));
    }

    #[test]
    fn test_with_embeddings() {
        let (store, root) = setup_store();
        let block_id = store.lookup_symbol("main", None).unwrap()[0].id;
        store
            .insert_embedding(block_id, &[0.1, 0.2, 0.3], "test")
            .unwrap();
        let report = build_setup_report(&store, &root, true).unwrap();
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
                None,
            )
            .unwrap();

        let block_id = store.lookup_symbol("main", None).unwrap()[0].id;
        store
            .insert_embedding(block_id, &[0.1, 0.2, 0.3], "test")
            .unwrap();

        let report = build_setup_report(&store, &root, true).unwrap();
        assert!(report.contains("1/2"));
        assert!(report.contains("50%"));
    }
}
