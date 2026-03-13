mod config;
mod output;
mod paths;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rmcp::ServiceExt;
use tokenstunt_embeddings::EmbeddingProvider;
use tracing::info;

#[derive(Parser)]
#[command(
    name = "tokenstunt",
    version,
    about = "AST-level semantic code search MCP server"
)]
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

fn resolve_db(db: Option<PathBuf>, root: &std::path::Path) -> Result<PathBuf> {
    match db {
        Some(path) => Ok(path),
        None => paths::cache_db_path(root),
    }
}

fn init_logging(default_level: &str) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

fn load_embedder(cfg: &config::Config) -> Result<Option<Arc<dyn EmbeddingProvider>>> {
    let Some(emb_cfg) = &cfg.embeddings else {
        return Ok(None);
    };
    if !emb_cfg.enabled {
        return Ok(None);
    }

    let provider = tokenstunt_embeddings::load_provider(
        &emb_cfg.provider,
        &emb_cfg.endpoint,
        &emb_cfg.model,
        emb_cfg.dimensions,
        emb_cfg.api_key.as_deref(),
    )?;

    info!(
        provider = %emb_cfg.provider,
        model = %emb_cfg.model,
        dimensions = emb_cfg.dimensions,
        "embeddings enabled"
    );

    Ok(Some(Arc::from(provider)))
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Serve { root, db } => {
            init_logging("tokenstunt=warn");

            let root = resolve_root(root)?;
            let cfg = config::Config::load(&root)?;
            let embedder = load_embedder(&cfg)?;
            let db_path = resolve_db(db, &root)?;

            if let Some(parent) = db_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            let store = tokenstunt_store::Store::open(&db_path)?;
            let indexer = Arc::new(tokenstunt_index::Indexer::new(store, embedder)?);

            let progress = output::IndicatifProgress::new();
            let stats = indexer.index_directory(&root, &progress)?;

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

            let _watcher =
                tokenstunt_index::FileWatcher::start(Arc::clone(&indexer), root.clone())?;

            let has_embeddings = indexer.embedder().is_some();

            output::print_serve_banner(&root, stats.files, stats.blocks, true);

            let server = tokenstunt_server::TokenStuntServer::new(
                Arc::clone(&indexer),
                root,
                has_embeddings,
            );

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
            init_logging("tokenstunt=info");

            let root = resolve_root(root)?;
            let cfg = config::Config::load(&root)?;
            let embedder = load_embedder(&cfg)?;
            let has_embeddings = embedder.is_some();
            let db_path = resolve_db(db, &root)?;

            if let Some(parent) = db_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            let store = tokenstunt_store::Store::open(&db_path)?;
            let indexer = tokenstunt_index::Indexer::new(store, embedder)?;

            let progress = output::IndicatifProgress::new();
            let stats = indexer.index_directory(&root, &progress)?;
            indexer.await_embeddings().await;

            output::print_index_summary(
                stats.files,
                stats.blocks,
                stats.skipped,
                stats.deleted_files,
                stats.errors,
            );

            if has_embeddings {
                let emb_count = indexer.store().embedding_count()?;
                let block_count = indexer.store().block_count()?;
                output::print_embed_summary(emb_count, block_count);
            }

            Ok(())
        }

        Command::Status { db } => {
            let root = resolve_root(None)?;
            let db_path = resolve_db(db, &root)?;

            if !db_path.exists() {
                eprintln!("No index found at {}", db_path.display());
                eprintln!("Run `tokenstunt index` to create one.");
                return Ok(());
            }

            let store = tokenstunt_store::Store::open(&db_path)?;
            let files = store.file_count()?;
            let blocks = store.block_count()?;

            output::print_status(&db_path, files, blocks);

            Ok(())
        }
    }
}
