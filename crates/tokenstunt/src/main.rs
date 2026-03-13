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
    /// Generate embeddings for indexed code blocks
    Embed {
        /// Root directory (defaults to current directory)
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
    /// Generate a config.toml with auto-detected settings
    Init {
        /// Root directory (defaults to current directory)
        #[arg(short, long)]
        root: Option<PathBuf>,
    },
    /// Delete the index database and cache for the current project
    Clear {
        /// Root directory (defaults to current directory)
        #[arg(short, long)]
        root: Option<PathBuf>,
    },
    /// Configure Claude Code to use Token Stunt as an MCP server
    SetupClaude,
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

fn resolve_repo_name(root: &std::path::Path) -> &str {
    root.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
}

struct CommandContext {
    root: PathBuf,
    store: tokenstunt_store::Store,
    embedder: Option<Arc<dyn EmbeddingProvider>>,
    batch_size: Option<usize>,
    hybrid_alpha: f64,
    default_limit: usize,
}

fn create_indexer(
    store: tokenstunt_store::Store,
    embedder: Option<Arc<dyn EmbeddingProvider>>,
    batch_size: Option<usize>,
) -> Result<tokenstunt_index::Indexer> {
    let has_embeddings = embedder.is_some();
    let mut indexer = tokenstunt_index::Indexer::new(store, embedder, batch_size)?;
    if has_embeddings {
        indexer.set_embedding_progress(Arc::new(output::IndicatifEmbeddingProgress::new()));
    }
    Ok(indexer)
}

fn validate_embedding_dimensions(
    store: &tokenstunt_store::Store,
    embedder: &dyn EmbeddingProvider,
) -> Result<()> {
    if let Some(stored_dim) = store.first_embedding_dimension()? {
        let configured_dim = embedder.dimensions();
        if stored_dim != configured_dim {
            anyhow::bail!(
                "embedding dimension mismatch: stored embeddings have {stored_dim} dimensions, \
                 but the configured model produces {configured_dim}. \
                 Delete existing embeddings or reconfigure the model to match."
            );
        }
    }
    Ok(())
}

async fn init_context_with_detect(
    root: Option<PathBuf>,
    db: Option<PathBuf>,
) -> Result<CommandContext> {
    let root = resolve_root(root)?;
    let cfg = config::Config::load(&root)?;
    let batch_size = cfg.embeddings.as_ref().and_then(|e| e.batch_size);
    let hybrid_alpha = cfg.hybrid_alpha();
    let default_limit = cfg.default_limit();
    let embedder = load_or_detect_embedder(&cfg).await?;
    let db_path = resolve_db(db, &root)?;

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let store = tokenstunt_store::Store::open(&db_path)?;

    if let Some(ref emb) = embedder {
        validate_embedding_dimensions(&store, emb.as_ref())?;
    }

    Ok(CommandContext {
        root,
        store,
        embedder,
        batch_size,
        hybrid_alpha,
        default_limit,
    })
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

async fn load_or_detect_embedder(
    cfg: &config::Config,
) -> Result<Option<Arc<dyn EmbeddingProvider>>> {
    if cfg.embeddings.is_some() {
        return load_embedder(cfg);
    }

    let Some(detected) = tokenstunt_embeddings::detect_ollama().await else {
        return Ok(None);
    };

    info!(
        model = %detected.model,
        dimensions = detected.dimensions,
        "auto-detected Ollama embedding model"
    );

    let provider = tokenstunt_embeddings::load_provider(
        "ollama",
        &detected.endpoint,
        &detected.model,
        detected.dimensions,
        None,
    )?;

    Ok(Some(Arc::from(provider)))
}

fn setup_claude_config() -> Result<()> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    let config_path = PathBuf::from(&home).join(".claude.json");

    let mut config: serde_json::Value = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        serde_json::from_str(&content)?
    } else {
        serde_json::json!({})
    };

    let mcp_entry = serde_json::json!({
        "command": "tokenstunt",
        "args": ["serve"],
        "env": {}
    });

    config
        .as_object_mut()
        .context("config is not an object")?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .context("mcpServers is not an object")?
        .insert("tokenstunt".to_string(), mcp_entry);

    let content = serde_json::to_string_pretty(&config)?;
    std::fs::write(&config_path, content)?;

    eprintln!("Wrote MCP server config to {}", config_path.display());
    eprintln!("Token Stunt will start automatically when Claude Code connects.");

    Ok(())
}

async fn run_mcp_server<T, A>(
    server: tokenstunt_server::TokenStuntServer,
    transport: T,
) -> Result<()>
where
    T: rmcp::transport::IntoTransport<rmcp::service::RoleServer, std::io::Error, A>
        + Send
        + 'static,
{
    let service = server
        .serve(transport)
        .await
        .context("failed to start MCP server")?;

    service.waiting().await?;
    info!("server stopped");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Serve { root, db } => {
            init_logging("tokenstunt=warn");

            let ctx = init_context_with_detect(root, db).await?;
            let indexer = Arc::new(create_indexer(ctx.store, ctx.embedder, ctx.batch_size)?);

            let progress = output::IndicatifProgress::new();
            let stats = indexer.index_directory(&ctx.root, &progress)?;
            indexer.await_embeddings().await;

            let backfilled = indexer.backfill_embeddings()?;
            if backfilled > 0 {
                indexer.await_embeddings().await;
            }

            let root_str = ctx.root.to_str().context("non-UTF-8 path")?;
            let repo_name = resolve_repo_name(&ctx.root);
            let repo_id = indexer.store().ensure_repo(root_str, repo_name)?;
            let reconcile_stats = indexer.reconcile(&ctx.root, repo_id)?;
            info!(
                updated = reconcile_stats.updated,
                unchanged = reconcile_stats.unchanged,
                deleted = reconcile_stats.deleted,
                "reconciliation complete"
            );

            let _watcher =
                tokenstunt_index::FileWatcher::start(Arc::clone(&indexer), ctx.root.clone())?;

            let has_embeddings = indexer.embedder().is_some();

            output::print_serve_banner(&ctx.root, stats.files, stats.blocks, true);

            let server = tokenstunt_server::TokenStuntServer::with_config(
                Arc::clone(&indexer),
                ctx.root,
                has_embeddings,
                ctx.hybrid_alpha,
                ctx.default_limit,
            );

            let transport = rmcp::transport::io::stdio();
            run_mcp_server(server, transport).await
        }

        Command::Index { root, db } => {
            init_logging("tokenstunt=info");

            let ctx = init_context_with_detect(root, db).await?;
            let has_embeddings = ctx.embedder.is_some();
            let indexer = create_indexer(ctx.store, ctx.embedder, ctx.batch_size)?;

            let progress = output::IndicatifProgress::new();
            let stats = indexer.index_directory(&ctx.root, &progress)?;
            indexer.await_embeddings().await;

            let backfilled = indexer.backfill_embeddings()?;
            if backfilled > 0 {
                indexer.await_embeddings().await;
            }

            output::print_index_summary(
                stats.files,
                stats.blocks,
                stats.skipped,
                stats.deleted_files,
                stats.errors,
            );

            if has_embeddings {
                let emb_count = indexer.store().embedding_count()?.max(0) as u64;
                let block_count = indexer.store().block_count()?.max(0) as u64;
                output::print_embed_summary(emb_count, block_count);
            }

            Ok(())
        }

        Command::Embed { root, db } => {
            init_logging("tokenstunt=info");

            let ctx = init_context_with_detect(root, db).await?;

            if ctx.embedder.is_none() {
                eprintln!("No embedding provider configured.");
                eprintln!();
                eprintln!("Add an [embeddings] section to your config.toml:");
                eprintln!("  ~/.cache/tokenstunt/<project>/config.toml");
                eprintln!();
                eprintln!("  [embeddings]");
                eprintln!("  enabled = true");
                eprintln!("  provider = \"ollama\"");
                eprintln!("  endpoint = \"http://localhost:11434\"");
                eprintln!("  model = \"nomic-embed-text\"");
                eprintln!("  dimensions = 768");
                return Ok(());
            }

            let mut indexer =
                tokenstunt_index::Indexer::new(ctx.store, ctx.embedder, ctx.batch_size)?;
            indexer.set_embedding_progress(Arc::new(output::IndicatifEmbeddingProgress::new()));
            let backfilled = indexer.backfill_embeddings()?;

            if backfilled > 0 {
                indexer.await_embeddings().await;
            }

            let emb_count = indexer.store().embedding_count()?.max(0) as u64;
            let block_count = indexer.store().block_count()?.max(0) as u64;
            output::print_embed_summary(emb_count, block_count);

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
            let files = store.file_count()?.max(0) as u64;
            let blocks = store.block_count()?.max(0) as u64;

            output::print_status(&db_path, files, blocks);

            Ok(())
        }

        Command::Init { root } => {
            let root = resolve_root(root)?;
            let config_path = paths::cache_config_path(&root)?;

            if config_path.exists() {
                eprintln!("Config already exists at {}", config_path.display());
                return Ok(());
            }

            let detected = tokenstunt_embeddings::detect_ollama().await;
            let detected_config = detected.map(|d| config::EmbeddingsConfig {
                enabled: true,
                provider: "ollama".to_string(),
                model: d.model,
                endpoint: d.endpoint,
                api_key: None,
                dimensions: d.dimensions,
                batch_size: None,
            });

            let template = config::Config::generate_template(detected_config.as_ref());

            if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&config_path, &template)?;

            eprintln!("Config written to {}", config_path.display());
            if let Some(ref emb) = detected_config {
                eprintln!(
                    "Detected: Ollama with {} ({} dims)",
                    emb.model, emb.dimensions
                );
            }

            Ok(())
        }

        Command::Clear { root } => {
            let root = resolve_root(root)?;
            let cache_dir = paths::project_cache_dir(&root)?;

            if !cache_dir.exists() {
                eprintln!("No cache found for {}", root.display());
                return Ok(());
            }

            std::fs::remove_dir_all(&cache_dir)?;
            eprintln!("Cleared cache at {}", cache_dir.display());

            Ok(())
        }

        Command::SetupClaude => {
            setup_claude_config()?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_root_defaults_to_cwd() {
        let root = resolve_root(None).unwrap();
        assert!(root.is_absolute());
        assert!(root.exists());
    }

    #[test]
    fn resolve_root_with_explicit_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = resolve_root(Some(dir.path().to_path_buf())).unwrap();
        assert!(root.is_absolute());
        assert_eq!(root, std::fs::canonicalize(dir.path()).unwrap());
    }

    #[test]
    fn resolve_root_nonexistent_path_errors() {
        let result = resolve_root(Some(PathBuf::from("/nonexistent/path/xyz")));
        assert!(result.is_err());
    }

    #[test]
    fn resolve_db_explicit_path_returned_as_is() {
        let path = PathBuf::from("/tmp/custom.db");
        let result = resolve_db(Some(path.clone()), std::path::Path::new("/tmp")).unwrap();
        assert_eq!(result, path);
    }

    #[test]
    fn resolve_db_none_derives_from_root() {
        let root = std::path::Path::new("/tmp/test-project");
        let result = resolve_db(None, root).unwrap();
        assert!(result.ends_with("index.db"));
        assert!(result.to_str().unwrap().contains("tokenstunt"));
    }

    #[test]
    fn load_embedder_no_config_returns_none() {
        let cfg = config::Config {
            embeddings: None,
            search: None,
        };
        let result = load_embedder(&cfg).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn load_embedder_disabled_returns_none() {
        let cfg = config::Config {
            search: None,
            embeddings: Some(config::EmbeddingsConfig {
                enabled: false,
                provider: "ollama".to_string(),
                model: "nomic-embed-text".to_string(),
                endpoint: "http://localhost:11434".to_string(),
                api_key: None,
                dimensions: 768,
                batch_size: None,
            }),
        };
        let result = load_embedder(&cfg).unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn init_context_with_valid_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let ctx = init_context_with_detect(Some(dir.path().to_path_buf()), Some(db_path.clone()))
            .await
            .unwrap();
        assert!(ctx.root.is_absolute());
        assert!(ctx.embedder.is_none());
        assert!(db_path.exists());
    }

    #[tokio::test]
    async fn init_context_creates_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("nested").join("deep").join("test.db");
        let ctx = init_context_with_detect(Some(dir.path().to_path_buf()), Some(db_path.clone()))
            .await
            .unwrap();
        assert!(ctx.root.is_absolute());
        assert!(db_path.exists());
    }

    #[test]
    fn load_embedder_enabled_returns_provider() {
        let cfg = config::Config {
            search: None,
            embeddings: Some(config::EmbeddingsConfig {
                enabled: true,
                provider: "ollama".to_string(),
                model: "nomic-embed-text".to_string(),
                endpoint: "http://localhost:11434".to_string(),
                api_key: None,
                dimensions: 768,
                batch_size: None,
            }),
        };
        let result = load_embedder(&cfg).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn load_embedder_enabled_with_api_key() {
        let cfg = config::Config {
            search: None,
            embeddings: Some(config::EmbeddingsConfig {
                enabled: true,
                provider: "openai-compat".to_string(),
                model: "text-embedding-3-small".to_string(),
                endpoint: "http://localhost:8080".to_string(),
                api_key: Some("sk-test-key".to_string()),
                dimensions: 1536,
                batch_size: Some(32),
            }),
        };
        let result = load_embedder(&cfg).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn load_embedder_unknown_provider_errors() {
        let cfg = config::Config {
            search: None,
            embeddings: Some(config::EmbeddingsConfig {
                enabled: true,
                provider: "nonexistent".to_string(),
                model: "model".to_string(),
                endpoint: "http://localhost".to_string(),
                api_key: None,
                dimensions: 768,
                batch_size: None,
            }),
        };
        let result = load_embedder(&cfg);
        assert!(result.is_err());
    }

    #[test]
    fn resolve_repo_name_extracts_directory_name() {
        let path = std::path::Path::new("/home/user/my-project");
        assert_eq!(resolve_repo_name(path), "my-project");
    }

    #[test]
    fn resolve_repo_name_root_path_returns_unknown() {
        let path = std::path::Path::new("/");
        assert_eq!(resolve_repo_name(path), "unknown");
    }

    #[test]
    fn create_indexer_without_embedder() {
        let store = tokenstunt_store::Store::open_in_memory().unwrap();
        let indexer = create_indexer(store, None, None).unwrap();
        assert!(indexer.embedder().is_none());
    }

    #[tokio::test]
    async fn run_mcp_server_exits_on_transport_close() {
        let dir = tempfile::tempdir().unwrap();
        let store = tokenstunt_store::Store::open(&dir.path().join("test.db")).unwrap();
        let indexer = tokenstunt_index::Indexer::new(store, None, None).unwrap();
        let indexer = Arc::new(indexer);
        let root = dir.path().to_path_buf();
        let server = tokenstunt_server::TokenStuntServer::new(indexer, root, false);

        let (client_read, server_write) = tokio::io::duplex(1024);
        let (server_read, mut client_write) = tokio::io::duplex(1024);

        let server_handle =
            tokio::spawn(async move { run_mcp_server(server, (server_read, server_write)).await });

        // Send an MCP initialize request to trigger the server handshake
        use tokio::io::AsyncWriteExt;
        let init_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "test-client",
                    "version": "0.1.0"
                }
            }
        });
        let msg = serde_json::to_string(&init_request).unwrap();
        client_write.write_all(msg.as_bytes()).await.unwrap();
        client_write.write_all(b"\n").await.unwrap();

        // Read the initialize response
        use tokio::io::AsyncBufReadExt;
        let mut reader = tokio::io::BufReader::new(client_read);
        let mut response_line = String::new();
        reader.read_line(&mut response_line).await.unwrap();
        assert!(response_line.contains("\"result\""));

        // Send initialized notification
        let initialized = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        let msg = serde_json::to_string(&initialized).unwrap();
        client_write.write_all(msg.as_bytes()).await.unwrap();
        client_write.write_all(b"\n").await.unwrap();

        // Close the client write side to signal EOF
        drop(client_write);
        drop(reader);

        // Server should exit
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), server_handle)
            .await
            .expect("server did not exit within timeout")
            .expect("server task panicked");

        // The server may return an error or Ok depending on how EOF is handled
        // The important thing is that it exited and the code path was covered
        let _ = result;
    }

    #[test]
    fn create_indexer_with_embedder_sets_progress() {
        let store = tokenstunt_store::Store::open_in_memory().unwrap();
        let provider = tokenstunt_embeddings::load_provider(
            "ollama",
            "http://localhost:11434",
            "nomic-embed-text",
            768,
            None,
        )
        .unwrap();
        let embedder: Option<Arc<dyn EmbeddingProvider>> = Some(Arc::from(provider));
        let indexer = create_indexer(store, embedder, None).unwrap();
        assert!(indexer.embedder().is_some());
    }

    #[tokio::test]
    async fn init_context_fails_when_parent_dir_unwritable() {
        let db_path = PathBuf::from("/dev/null/impossible/nested/test.db");
        let dir = tempfile::tempdir().unwrap();
        let err = init_context_with_detect(Some(dir.path().to_path_buf()), Some(db_path)).await;
        assert!(err.is_err());
    }

    #[test]
    fn validate_dimensions_no_embeddings_passes() {
        let store = tokenstunt_store::Store::open_in_memory().unwrap();
        let provider = tokenstunt_embeddings::load_provider(
            "ollama",
            "http://localhost:11434",
            "nomic-embed-text",
            768,
            None,
        )
        .unwrap();
        let result = validate_embedding_dimensions(&store, provider.as_ref());
        assert!(result.is_ok());
    }

    #[test]
    fn validate_dimensions_matching_passes() {
        let store = tokenstunt_store::Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        let file_id = store
            .upsert_file(repo_id, "test.rs", 123, "rust", 0)
            .unwrap();
        let block_id = store
            .insert_code_block(
                file_id,
                "test_fn",
                tokenstunt_store::CodeBlockKind::Function,
                1,
                5,
                "fn test() {}",
                "fn test()",
                "",
                None,
            )
            .unwrap();
        store
            .insert_embedding(block_id, &vec![0.0_f32; 768], "nomic-embed-text")
            .unwrap();

        let provider = tokenstunt_embeddings::load_provider(
            "ollama",
            "http://localhost:11434",
            "nomic-embed-text",
            768,
            None,
        )
        .unwrap();
        let result = validate_embedding_dimensions(&store, provider.as_ref());
        assert!(result.is_ok());
    }

    #[test]
    fn validate_dimensions_mismatch_errors() {
        let store = tokenstunt_store::Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        let file_id = store
            .upsert_file(repo_id, "test.rs", 123, "rust", 0)
            .unwrap();
        let block_id = store
            .insert_code_block(
                file_id,
                "test_fn",
                tokenstunt_store::CodeBlockKind::Function,
                1,
                5,
                "fn test() {}",
                "fn test()",
                "",
                None,
            )
            .unwrap();
        store
            .insert_embedding(block_id, &vec![0.0_f32; 768], "nomic-embed-text")
            .unwrap();

        let provider = tokenstunt_embeddings::load_provider(
            "ollama",
            "http://localhost:11434",
            "different-model",
            1536,
            None,
        )
        .unwrap();
        let err = validate_embedding_dimensions(&store, provider.as_ref());
        assert!(err.is_err());
        assert!(
            err.unwrap_err()
                .to_string()
                .contains("embedding dimension mismatch")
        );
    }

    #[tokio::test]
    async fn load_or_detect_with_explicit_config_uses_config() {
        let cfg = config::Config {
            search: None,
            embeddings: Some(config::EmbeddingsConfig {
                enabled: true,
                provider: "ollama".to_string(),
                model: "nomic-embed-text".to_string(),
                endpoint: "http://localhost:11434".to_string(),
                api_key: None,
                dimensions: 768,
                batch_size: None,
            }),
        };
        let result = load_or_detect_embedder(&cfg).await.unwrap();
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn load_or_detect_without_config_returns_none_when_no_ollama() {
        let cfg = config::Config {
            search: None,
            embeddings: None,
        };
        // No Ollama running in test env, so detection returns None
        let result = load_or_detect_embedder(&cfg).await.unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn setup_claude_config_writes_mcp_entry() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(".claude.json");

        let original_home = std::env::var("HOME").unwrap();
        unsafe {
            std::env::set_var("HOME", dir.path());
        }

        let result = setup_claude_config();
        unsafe {
            std::env::set_var("HOME", &original_home);
        }

        assert!(result.is_ok());
        assert!(config_path.exists());

        let content = std::fs::read_to_string(&config_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(
            parsed["mcpServers"]["tokenstunt"]["command"]
                .as_str()
                .unwrap()
                .contains("tokenstunt")
        );
    }

    #[test]
    fn setup_claude_config_preserves_existing_entries() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(".claude.json");

        let existing = serde_json::json!({
            "mcpServers": {
                "other-server": { "command": "other" }
            }
        });
        std::fs::write(&config_path, serde_json::to_string(&existing).unwrap()).unwrap();

        let original_home = std::env::var("HOME").unwrap();
        unsafe {
            std::env::set_var("HOME", dir.path());
        }

        let result = setup_claude_config();
        unsafe {
            std::env::set_var("HOME", &original_home);
        }

        assert!(result.is_ok());

        let content = std::fs::read_to_string(&config_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(parsed["mcpServers"]["other-server"].is_object());
        assert!(parsed["mcpServers"]["tokenstunt"].is_object());
    }
}
