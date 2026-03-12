mod config;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rmcp::ServiceExt;
use tracing::info;

#[derive(Parser)]
#[command(name = "tokenstunt", version, about = "AST-level semantic code search MCP server")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the MCP server (stdio transport)
    Serve {
        /// Root directory to serve (defaults to current directory)
        #[arg(short, long)]
        root: Option<PathBuf>,

        /// Path to the database file
        #[arg(short, long)]
        db: Option<PathBuf>,
    },
    /// Index a directory
    Index {
        /// Directory to index (defaults to current directory)
        #[arg(short, long)]
        root: Option<PathBuf>,

        /// Path to the database file
        #[arg(short, long)]
        db: Option<PathBuf>,
    },
    /// Show index status
    Status {
        /// Path to the database file
        #[arg(short, long)]
        db: Option<PathBuf>,
    },
}

fn resolve_root(root: Option<PathBuf>) -> Result<PathBuf> {
    let path = root.unwrap_or_else(|| PathBuf::from("."));
    std::fs::canonicalize(&path).with_context(|| format!("cannot resolve path: {}", path.display()))
}

fn resolve_db(db: Option<PathBuf>, root: &std::path::Path) -> PathBuf {
    db.unwrap_or_else(|| root.join(".tokenstunt").join("index.db"))
}

fn env_filter() -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("tokenstunt=info"))
}

fn init_logging_stderr() {
    tracing_subscriber::fmt()
        .with_env_filter(env_filter())
        .with_writer(std::io::stderr)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Serve { root, db } => {
            init_logging_stderr();

            let root = resolve_root(root)?;
            let cfg = config::Config::load(&root)?;
            if cfg.embeddings.as_ref().is_some_and(|e| e.enabled) {
                info!("embeddings enabled");
            }
            let db_path = resolve_db(db, &root);

            if let Some(parent) = db_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            let store = tokenstunt_store::Store::open(&db_path)?;
            let indexer = Arc::new(tokenstunt_index::Indexer::new(store)?);

            info!(root = %root.display(), db = %db_path.display(), "indexing");
            let stats = indexer.index_directory(&root)?;
            info!(
                files = stats.files,
                blocks = stats.blocks,
                skipped = stats.skipped,
                "index ready"
            );

            let root_str = root.to_str().context("non-UTF-8 path")?;
            let repo_name = root
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            let repo_id = indexer.store().ensure_repo(root_str, repo_name)?;
            let reconcile_stats = indexer.reconcile(&root, repo_id)?;
            info!(
                updated = reconcile_stats.updated,
                unchanged = reconcile_stats.unchanged,
                deleted = reconcile_stats.deleted,
                "reconciliation complete"
            );

            let _watcher = tokenstunt_index::FileWatcher::start(Arc::clone(&indexer), root.clone())?;
            info!("file watcher started");

            let server = tokenstunt_server::TokenStuntServer::new(Arc::clone(&indexer), root);

            info!("starting MCP server on stdio");
            let transport = rmcp::transport::io::stdio();
            let service = server
                .serve(transport)
                .await
                .context("failed to start MCP server")?;

            service.waiting().await?;
            info!("server stopped");
            Ok(())
        }

        Command::Index { root, db } => {
            init_logging_stderr();

            let root = resolve_root(root)?;
            let cfg = config::Config::load(&root)?;
            if cfg.embeddings.as_ref().is_some_and(|e| e.enabled) {
                info!("embeddings enabled");
            }
            let db_path = resolve_db(db, &root);

            if let Some(parent) = db_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            let store = tokenstunt_store::Store::open(&db_path)?;
            let indexer = tokenstunt_index::Indexer::new(store)?;

            info!(root = %root.display(), "indexing");
            let stats = indexer.index_directory(&root)?;

            println!(
                "Indexed {} files, {} code blocks ({} skipped, {} errors)",
                stats.files, stats.blocks, stats.skipped, stats.errors
            );

            Ok(())
        }

        Command::Status { db } => {
            let root = resolve_root(None)?;
            let db_path = resolve_db(db, &root);

            if !db_path.exists() {
                println!("No index found at {}", db_path.display());
                println!("Run `tokenstunt index` to create one.");
                return Ok(());
            }

            let store = tokenstunt_store::Store::open(&db_path)?;
            let files = store.file_count()?;
            let blocks = store.block_count()?;

            println!("Index: {}", db_path.display());
            println!("Files: {files}");
            println!("Code blocks: {blocks}");

            Ok(())
        }
    }
}
