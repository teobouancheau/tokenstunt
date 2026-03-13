use std::path::PathBuf;
use std::sync::Arc;

use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::{ErrorData as McpError, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use tokenstunt_index::Indexer;
use tokenstunt_search::{SearchEngine, SearchQuery};
use tokenstunt_store::CodeBlockKind;

use crate::format;
use crate::render;

#[derive(Clone)]
#[allow(dead_code)]
pub struct TokenStuntServer {
    indexer: Arc<Indexer>,
    root: PathBuf,
    has_embeddings: bool,
    hybrid_alpha: f64,
    default_limit: usize,
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct TsSearchParams {
    /// Search query (natural language or keyword)
    query: String,
    /// Scope search to a directory path
    scope: Option<String>,
    /// Filter by language (e.g. "typescript", "python")
    language: Option<String>,
    /// Filter by symbol kind (function, class, interface, etc.)
    symbol_kind: Option<String>,
    /// Max results to return (default: 10)
    limit: Option<usize>,
    /// Offset for pagination (default: 0)
    offset: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct TsSymbolParams {
    /// Exact symbol name to look up
    name: String,
    /// Filter by symbol kind
    kind: Option<String>,
    /// Filter by file path
    file: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct TsContextParams {
    /// Symbol name to get context for
    symbol: String,
    /// Direction: "dependencies", "dependents", or "both"
    direction: Option<String>,
    /// Filter by file path to disambiguate
    file: Option<String>,
    /// Filter by symbol kind to disambiguate
    kind: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct TsOverviewParams {
    /// Scope to a directory path (e.g. "src/")
    scope: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct TsSetupParams {}

#[derive(Deserialize, schemars::JsonSchema)]
struct TsFileParams {
    /// File path to inspect
    path: String,
    /// Filter by symbol kind (function, class, interface, etc.)
    kind: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct TsUsagesParams {
    /// Symbol name to find usages of
    symbol: String,
    /// Filter by symbol kind (function, class, interface, etc.)
    kind: Option<String>,
    /// Max results to return (default: 20)
    limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct TsImpactParams {
    /// Symbol name to analyze blast radius for
    symbol: String,
    /// Max traversal depth (default: 3, max: 5)
    max_depth: Option<u32>,
}

#[tool_router]
impl TokenStuntServer {
    pub fn new(indexer: Arc<Indexer>, root: PathBuf, has_embeddings: bool) -> Self {
        Self::with_config(indexer, root, has_embeddings, 0.4, 20)
    }

    pub fn with_config(
        indexer: Arc<Indexer>,
        root: PathBuf,
        has_embeddings: bool,
        hybrid_alpha: f64,
        default_limit: usize,
    ) -> Self {
        Self {
            indexer,
            root,
            has_embeddings,
            hybrid_alpha,
            default_limit,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "ts_search",
        description = "Semantic code search — returns exact function/class/type bodies ranked by relevance. Use instead of Grep+Read when searching by concept or keyword. Saves 95% tokens vs reading full files."
    )]
    async fn ts_search(
        &self,
        params: Parameters<TsSearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        // Compute embedding async before calling synchronous search
        let query_embedding: Option<Vec<f32>> = if let Some(embedder) = self.indexer.embedder() {
            let texts = vec![p.query.clone()];
            match embedder.embed_batch(&texts).await {
                Ok(mut vecs) => vecs.pop(),
                Err(_) => None,
            }
        } else {
            None
        };

        let engine = SearchEngine::with_alpha(self.indexer.store(), self.hybrid_alpha);

        let query_text = p.query.clone();
        let query = SearchQuery {
            text: p.query,
            scope: p.scope,
            language: p.language,
            symbol_kind: p.symbol_kind.and_then(|s| CodeBlockKind::from_str(&s)),
            limit: p.limit.unwrap_or(self.default_limit),
            query_embedding,
        };

        let results = engine
            .search(&query)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        if results.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No results found.",
            )]));
        }

        let total = results.len();
        let offset = p.offset.unwrap_or(0);
        let page: Vec<_> = results
            .iter()
            .skip(offset)
            .map(|r| (r.block.clone(), Some(r.score)))
            .collect();

        let mut output = format::format_blocks(&query_text, &page);
        if offset > 0 || total > page.len() + offset {
            output.push_str(&format!(
                "\n\nShowing {} of {} results (offset {})",
                page.len(),
                total,
                offset
            ));
        }
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    #[tool(
        name = "ts_symbol",
        description = "Exact symbol lookup by name — returns the full definition with file path and line numbers. Faster than Grep for known symbol names."
    )]
    async fn ts_symbol(
        &self,
        params: Parameters<TsSymbolParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let engine = SearchEngine::with_alpha(self.indexer.store(), self.hybrid_alpha);

        let kind = p.kind.and_then(|s| CodeBlockKind::from_str(&s));
        let mut results = engine
            .lookup_symbol(&p.name, kind)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        if let Some(ref file_filter) = p.file {
            results.retain(|b| {
                b.file_path
                    .as_deref()
                    .is_some_and(|fp| fp.contains(file_filter.as_str()))
            });
        }

        if results.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(format!(
                "Symbol '{}' not found.",
                p.name
            ))]));
        }

        let mut out = render::header("Symbol", &p.name);
        out.push_str("\n\n");

        let blocks: Vec<_> = results.iter().map(|b| (b.clone(), None)).collect();
        out.push_str(&format::format_symbol_blocks(&blocks));
        Ok(CallToolResult::success(vec![Content::text(out)]))
    }

    #[tool(
        name = "ts_context",
        description = "Symbol definition + dependency graph — shows what this symbol calls and what calls it. Use to understand coupling before modifying code."
    )]
    async fn ts_context(
        &self,
        params: Parameters<TsContextParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let store = self.indexer.store();

        let kind_filter = p.kind.and_then(|s| CodeBlockKind::from_str(&s));
        let mut symbols = store
            .lookup_symbol(&p.symbol, kind_filter)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        if let Some(ref file_filter) = p.file {
            symbols.retain(|b| {
                b.file_path
                    .as_deref()
                    .is_some_and(|fp| fp.contains(file_filter.as_str()))
            });
        }

        if symbols.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(format!(
                "Symbol '{}' not found.",
                p.symbol
            ))]));
        }

        let symbol = &symbols[0];
        let file_path = symbol.file_path.as_deref().unwrap_or("unknown");
        let location = format!("{file_path}:{}-{}", symbol.start_line, symbol.end_line);
        let language = symbol.language.as_deref().unwrap_or("text");

        let mut output = render::header("Context", &format!("{}  {}", p.symbol, location));
        output.push_str("\n\n");
        output.push_str(&render::code_block(language, &symbol.content));

        let direction = p.direction.as_deref().unwrap_or("both");

        if matches!(direction, "dependencies" | "both") {
            let deps = store
                .get_dependencies(symbol.id)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            if !deps.is_empty() {
                let items: Vec<render::TreeItem> = deps
                    .iter()
                    .map(|(block, kind)| {
                        let dep_path = block.file_path.as_deref().unwrap_or("unknown");
                        let dep_loc = format!("{dep_path}:{}-{}", block.start_line, block.end_line);
                        render::TreeItem {
                            label: format!(
                                "{}  {:<24} {:<28} {}",
                                render::kind_label(&block.kind),
                                block.name,
                                dep_loc,
                                render::capitalize(kind),
                            ),
                        }
                    })
                    .collect();

                output.push('\n');
                output.push('\n');
                output.push_str(&render::render_tree_with_trunk("Dependencies", &items));
            }
        }

        if matches!(direction, "dependents" | "both") {
            let deps = store
                .get_dependents(symbol.id)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            if !deps.is_empty() {
                let items: Vec<render::TreeItem> = deps
                    .iter()
                    .map(|(block, kind)| {
                        let dep_path = block.file_path.as_deref().unwrap_or("unknown");
                        let dep_loc = format!("{dep_path}:{}-{}", block.start_line, block.end_line);
                        render::TreeItem {
                            label: format!(
                                "{}  {:<24} {:<28} {}",
                                render::kind_label(&block.kind),
                                block.name,
                                dep_loc,
                                render::capitalize(kind),
                            ),
                        }
                    })
                    .collect();

                output.push('\n');
                output.push('\n');
                output.push_str(&render::render_tree_with_trunk("Dependents", &items));
            }
        }

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    #[tool(
        name = "ts_overview",
        description = "Project structure overview — module tree, language breakdown, public API surface, and entry points. Start here to orient in an unfamiliar codebase."
    )]
    async fn ts_overview(
        &self,
        params: Parameters<TsOverviewParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let store = self.indexer.store();
        let scope = p.scope.as_deref().unwrap_or("");

        if let Some(cached) = store
            .get_overview_cache(scope, 1)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?
        {
            return Ok(CallToolResult::success(vec![Content::text(cached)]));
        }

        let output = build_overview(store, &self.root, scope)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let _ = store.set_overview_cache(scope, 1, &output);

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    #[tool(
        name = "ts_setup",
        description = "Project diagnostics: index health, languages, embeddings status, and configuration guidance."
    )]
    async fn ts_setup(
        &self,
        _params: Parameters<TsSetupParams>,
    ) -> Result<CallToolResult, McpError> {
        let report =
            crate::setup::build_setup_report(self.indexer.store(), &self.root, self.has_embeddings)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(report)]))
    }

    #[tool(
        name = "ts_impact",
        description = "Blast radius analysis: shows all symbols and files affected by changing a given symbol. Use before refactoring."
    )]
    async fn ts_impact(
        &self,
        params: Parameters<TsImpactParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let result = crate::impact::walk_dependents(self.indexer.store(), &p.symbol, p.max_depth)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let output = crate::impact::format_impact(&result);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    #[tool(
        name = "ts_file",
        description = "All symbols in a file with signatures and line numbers. Use instead of Read when you need to understand file structure."
    )]
    async fn ts_file(&self, params: Parameters<TsFileParams>) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let store = self.indexer.store();

        let kind_filter = p.kind.and_then(|s| CodeBlockKind::from_str(&s));

        let mut blocks = store
            .get_blocks_by_file_path(&p.path)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        if let Some(k) = kind_filter {
            blocks.retain(|b| b.kind == k);
        }

        let output = format::format_file_blocks(&p.path, &blocks);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    #[tool(
        name = "ts_usages",
        description = "Find all call sites and usages of a symbol. Shows the actual code at each usage location."
    )]
    async fn ts_usages(
        &self,
        params: Parameters<TsUsagesParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let store = self.indexer.store();
        let limit = p.limit.unwrap_or(20);

        let kind = p.kind.and_then(|s| CodeBlockKind::from_str(&s));
        let symbols = store
            .lookup_symbol(&p.symbol, kind)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        if symbols.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(format!(
                "Symbol '{}' not found.",
                p.symbol
            ))]));
        }

        let mut all_usages: Vec<(tokenstunt_store::CodeBlock, String)> = Vec::new();
        for symbol in &symbols {
            let dependents = store
                .get_dependents(symbol.id)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            all_usages.extend(dependents);
        }

        all_usages.truncate(limit);

        let output = format::format_usages(&p.symbol, &all_usages);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }
}

const ENTRY_POINT_PREFIXES: &[&str] = &["main.", "index.", "app.", "mod.", "lib."];

fn build_overview(
    store: &tokenstunt_store::Store,
    root: &std::path::Path,
    scope: &str,
) -> anyhow::Result<String> {
    let file_count = store.file_count()?;
    let block_count = store.block_count()?;

    let mut out = render::header("Overview", &root.display().to_string());
    out.push_str("\n\n");
    out.push_str(&render::kv_line("Files", &file_count.to_string()));
    out.push('\n');
    out.push_str(&render::kv_line("Code Blocks", &block_count.to_string()));
    out.push('\n');

    let lang_stats = store.get_language_stats()?;
    if !lang_stats.is_empty() {
        let max_count = lang_stats.iter().map(|(_, c)| *c).max().unwrap_or(1);
        let items: Vec<render::TreeItem> = lang_stats
            .iter()
            .map(|(lang, count)| {
                let ratio = *count as f64 / max_count as f64;
                let b = render::bar(ratio, 20);
                render::TreeItem {
                    label: format!("{lang:<16} {count:>3} files  {b}"),
                }
            })
            .collect();
        out.push('\n');
        out.push_str(&render::render_tree_with_trunk("Languages", &items));
    }

    let scope_arg = if scope.is_empty() { None } else { Some(scope) };

    let dir_stats = store.get_directory_stats(scope_arg)?;
    if !dir_stats.is_empty() {
        let items: Vec<render::TreeItem> = dir_stats
            .iter()
            .map(|(dir, fc, bc)| render::TreeItem {
                label: format!("{dir:<16} {fc:>3} files   {bc:>4} blocks"),
            })
            .collect();
        out.push('\n');
        out.push_str(&render::render_tree_with_trunk("Modules", &items));
    }

    let symbols = store.get_exported_symbols(scope_arg)?;
    if !symbols.is_empty() {
        let display_count = 20.min(symbols.len());
        let mut items: Vec<render::TreeItem> = symbols
            .iter()
            .take(display_count)
            .map(|s| {
                let path = s.file_path.as_deref().unwrap_or("unknown");
                render::TreeItem {
                    label: format!("{}  {:<24} {}", render::kind_label(&s.kind), s.name, path,),
                }
            })
            .collect();
        if symbols.len() > 20 {
            items.push(render::TreeItem {
                label: format!("... {} more", symbols.len() - 20),
            });
        }
        out.push('\n');
        out.push_str(&render::render_tree_with_trunk("Public API", &items));
    }

    let mut entry_paths: Vec<String> = symbols
        .iter()
        .filter_map(|s| {
            let path = s.file_path.as_deref()?;
            let filename = path.rsplit('/').next().unwrap_or(path);
            ENTRY_POINT_PREFIXES
                .iter()
                .any(|prefix| filename.starts_with(prefix))
                .then(|| path.to_string())
        })
        .collect();
    entry_paths.sort();
    entry_paths.dedup();

    if !entry_paths.is_empty() {
        let items: Vec<render::TreeItem> = entry_paths
            .iter()
            .map(|p| render::TreeItem { label: p.clone() })
            .collect();
        out.push('\n');
        out.push_str(&render::render_tree_with_trunk("Entry Points", &items));
    }

    Ok(out)
}

#[tool_handler]
impl rmcp::handler::server::ServerHandler for TokenStuntServer {
    fn get_info(&self) -> InitializeResult {
        let capabilities = ServerCapabilities::builder().enable_tools().build();

        InitializeResult::new(capabilities)
            .with_server_info(
                Implementation::new("tokenstunt", env!("CARGO_PKG_VERSION"))
                    .with_title("Token Stunt")
                    .with_description("Smart code search for Claude Code. Finds the exact code you need — saves 95% of tokens.")
            )
            .with_instructions(
                "Token Stunt provides AST-level semantic code search. Use ts_search instead of Grep+Read when looking for code by concept — it returns exact symbol bodies, saving 95% of tokens. Use ts_symbol for exact name lookups. Use ts_file to understand a file's structure without reading the whole file. Use ts_usages to find all call sites of a symbol. Use ts_context to understand what a symbol calls and what calls it. Use ts_impact before refactoring to understand blast radius. Use ts_overview to orient in the project. Use ts_setup to check index health. Only use Read for files you need to modify. Recommended workflow: ts_overview → ts_search → ts_symbol → ts_file/ts_usages → ts_context/ts_impact → Read."
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::handler::server::ServerHandler;
    use tokenstunt_embeddings::EmbeddingProvider;
    use tokenstunt_store::{CodeBlockKind, Store};

    fn setup_server() -> TokenStuntServer {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        let file_id = store
            .upsert_file(repo_id, "src/auth.ts", 111, "typescript", 0)
            .unwrap();
        let auth_id = store
            .insert_code_block(
                file_id,
                "authenticateUser",
                CodeBlockKind::Function,
                1,
                10,
                "function authenticateUser(token: string): User { ... }",
                "function authenticateUser(token: string): User",
                "",
                None,
            )
            .unwrap();
        store
            .insert_code_block(
                file_id,
                "UserProfile",
                CodeBlockKind::Class,
                12,
                30,
                "class UserProfile { name: string; }",
                "class UserProfile",
                "",
                None,
            )
            .unwrap();
        let validate_id = store
            .insert_code_block(
                file_id,
                "validateToken",
                CodeBlockKind::Function,
                32,
                40,
                "function validateToken(t: string): boolean { ... }",
                "function validateToken(t: string): boolean",
                "",
                None,
            )
            .unwrap();

        // authenticateUser --calls--> validateToken
        store
            .insert_dependency(auth_id, Some(validate_id), "validateToken", "call")
            .unwrap();

        let indexer = Arc::new(tokenstunt_index::Indexer::new(store, None, None).unwrap());
        TokenStuntServer::new(indexer, PathBuf::from("/test"), false)
    }

    fn text_content(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|c| match &c.raw {
                RawContent::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn test_server_info() {
        let store = Store::open_in_memory().unwrap();
        let indexer = Arc::new(tokenstunt_index::Indexer::new(store, None, None).unwrap());
        let server = TokenStuntServer::new(indexer, PathBuf::from("/test"), false);
        let info = server.get_info();

        assert_eq!(info.server_info.name, "tokenstunt");
        assert!(info.capabilities.tools.is_some());
    }

    #[tokio::test]
    async fn test_ts_search_returns_results() {
        let server = setup_server();
        let params = Parameters(TsSearchParams {
            query: "authenticate".to_string(),
            scope: None,
            language: None,
            symbol_kind: None,
            limit: None,
            offset: None,
        });
        let result = server.ts_search(params).await.unwrap();
        let text = text_content(&result);
        assert!(
            text.contains("authenticateUser"),
            "expected block name in results"
        );
    }

    #[tokio::test]
    async fn test_ts_search_no_results() {
        let server = setup_server();
        let params = Parameters(TsSearchParams {
            query: "zzzznonexistent".to_string(),
            scope: None,
            language: None,
            symbol_kind: None,
            limit: None,
            offset: None,
        });
        let result = server.ts_search(params).await.unwrap();
        let text = text_content(&result);
        assert_eq!(text, "No results found.");
    }

    #[tokio::test]
    async fn test_ts_symbol_found() {
        let server = setup_server();
        let params = Parameters(TsSymbolParams {
            name: "authenticateUser".to_string(),
            kind: None,
            file: None,
        });
        let result = server.ts_symbol(params).await.unwrap();
        let text = text_content(&result);
        assert!(text.contains("authenticateUser"));
        assert!(text.contains("src/auth.ts"));
    }

    #[tokio::test]
    async fn test_ts_symbol_not_found() {
        let server = setup_server();
        let params = Parameters(TsSymbolParams {
            name: "nonexistentSymbol".to_string(),
            kind: None,
            file: None,
        });
        let result = server.ts_symbol(params).await.unwrap();
        let text = text_content(&result);
        assert!(text.contains("not found"));
    }

    #[tokio::test]
    async fn test_ts_context_both() {
        let server = setup_server();
        let params = Parameters(TsContextParams {
            symbol: "authenticateUser".to_string(),
            direction: Some("both".to_string()),
            file: None,
            kind: None,
        });
        let result = server.ts_context(params).await.unwrap();
        let text = text_content(&result);
        assert!(text.contains("authenticateUser"));
        assert!(
            text.contains("Dependencies"),
            "should show dependencies section"
        );
        assert!(
            text.contains("validateToken"),
            "should list validateToken as dependency"
        );
    }

    #[tokio::test]
    async fn test_ts_context_dependencies_only() {
        let server = setup_server();
        let params = Parameters(TsContextParams {
            symbol: "authenticateUser".to_string(),
            direction: Some("dependencies".to_string()),
            file: None,
            kind: None,
        });
        let result = server.ts_context(params).await.unwrap();
        let text = text_content(&result);
        assert!(text.contains("Dependencies"));
        assert!(text.contains("validateToken"));
        assert!(
            !text.contains("Dependents"),
            "should not show dependents section"
        );
    }

    #[tokio::test]
    async fn test_ts_context_dependents_only() {
        let server = setup_server();
        let params = Parameters(TsContextParams {
            symbol: "validateToken".to_string(),
            direction: Some("dependents".to_string()),
            file: None,
            kind: None,
        });
        let result = server.ts_context(params).await.unwrap();
        let text = text_content(&result);
        assert!(
            text.contains("Dependents"),
            "should show dependents section"
        );
        assert!(
            text.contains("authenticateUser"),
            "should list authenticateUser as dependent"
        );
        assert!(
            !text.contains("Dependencies"),
            "should not show dependencies section"
        );
    }

    #[tokio::test]
    async fn test_ts_context_not_found() {
        let server = setup_server();
        let params = Parameters(TsContextParams {
            symbol: "nonexistentSymbol".to_string(),
            direction: None,
            file: None,
            kind: None,
        });
        let result = server.ts_context(params).await.unwrap();
        let text = text_content(&result);
        assert!(text.contains("not found"));
    }

    #[tokio::test]
    async fn test_ts_overview() {
        let server = setup_server();
        let params = Parameters(TsOverviewParams { scope: None });
        let result = server.ts_overview(params).await.unwrap();
        let text = text_content(&result);
        assert!(text.contains("\u{25C6} Overview"), "should contain header");
        assert!(text.contains("/test"), "should contain root path");
        assert!(text.contains("Languages"), "should contain language stats");
        assert!(
            text.contains("typescript"),
            "should list typescript language"
        );
        assert!(text.contains("Modules"), "should contain module structure");
        assert!(
            text.contains("Public API"),
            "should list public API symbols"
        );
        assert!(
            text.contains("authenticateUser"),
            "should list authenticateUser"
        );
    }

    #[tokio::test]
    async fn test_ts_overview_uses_cache() {
        let server = setup_server();

        let params = Parameters(TsOverviewParams { scope: None });
        let first = text_content(&server.ts_overview(params).await.unwrap());

        let params = Parameters(TsOverviewParams { scope: None });
        let second = text_content(&server.ts_overview(params).await.unwrap());
        assert_eq!(first, second);
    }

    struct FakeEmbeddingProvider {
        dims: usize,
        model: String,
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
    async fn test_ts_setup() {
        let server = setup_server();
        let params = Parameters(TsSetupParams {});
        let result = server.ts_setup(params).await.unwrap();
        let text = text_content(&result);
        assert!(text.contains("Setup"), "should contain setup header");
        assert!(text.contains("Files"), "should contain files info");
        assert!(text.contains("Code Blocks"), "should contain block count");
        assert!(
            text.contains("Dependencies"),
            "should contain dependencies info"
        );
    }

    #[tokio::test]
    async fn test_ts_impact_found() {
        let server = setup_server();
        let params = Parameters(TsImpactParams {
            symbol: "validateToken".to_string(),
            max_depth: None,
        });
        let result = server.ts_impact(params).await.unwrap();
        let text = text_content(&result);
        assert!(
            text.contains("authenticateUser"),
            "authenticateUser depends on validateToken"
        );
    }

    #[tokio::test]
    async fn test_ts_impact_not_found() {
        let server = setup_server();
        let params = Parameters(TsImpactParams {
            symbol: "nonexistentSymbol".to_string(),
            max_depth: None,
        });
        let result = server.ts_impact(params).await.unwrap();
        let text = text_content(&result);
        assert!(
            text.contains("No dependents found"),
            "unknown symbol should have no dependents"
        );
    }

    #[tokio::test]
    async fn test_ts_overview_many_symbols() {
        // Build a store with >20 exported symbols to test the "... N more" branch
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        let file_id = store
            .upsert_file(repo_id, "src/many.ts", 111, "typescript", 0)
            .unwrap();

        for i in 0..25 {
            store
                .insert_code_block(
                    file_id,
                    &format!("symbol{i}"),
                    CodeBlockKind::Function,
                    i * 10 + 1,
                    i * 10 + 5,
                    &format!("function symbol{i}() {{}}"),
                    &format!("function symbol{i}()"),
                    "",
                    None,
                )
                .unwrap();
        }

        let indexer = Arc::new(tokenstunt_index::Indexer::new(store, None, None).unwrap());
        let server = TokenStuntServer::new(indexer, PathBuf::from("/test"), false);

        let params = Parameters(TsOverviewParams { scope: None });
        let result = server.ts_overview(params).await.unwrap();
        let text = text_content(&result);
        assert!(
            text.contains("... 5 more"),
            "should show '... 5 more' when >20 symbols"
        );
    }

    #[tokio::test]
    async fn test_ts_overview_with_entry_points() {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        let file_id = store
            .upsert_file(repo_id, "src/index.ts", 111, "typescript", 0)
            .unwrap();

        store
            .insert_code_block(
                file_id,
                "startApp",
                CodeBlockKind::Function,
                1,
                10,
                "function startApp() {}",
                "function startApp()",
                "",
                None,
            )
            .unwrap();

        let indexer = Arc::new(tokenstunt_index::Indexer::new(store, None, None).unwrap());
        let server = TokenStuntServer::new(indexer, PathBuf::from("/test"), false);

        let params = Parameters(TsOverviewParams { scope: None });
        let result = server.ts_overview(params).await.unwrap();
        let text = text_content(&result);
        assert!(
            text.contains("Entry Points"),
            "should show entry points for index.ts"
        );
    }

    #[test]
    fn test_fake_embedding_provider_trait_methods() {
        let provider = FakeEmbeddingProvider {
            dims: 128,
            model: "test-model".to_string(),
        };
        assert_eq!(provider.dimensions(), 128);
        assert_eq!(provider.model_name(), "test-model");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_fake_embedding_provider_health_check() {
        let provider = FakeEmbeddingProvider {
            dims: 64,
            model: "test-model".to_string(),
        };
        provider.health_check().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_text_content_with_non_text_content() {
        // Test that text_content filters out non-text content (covers the _ => None branch)
        let result = CallToolResult::success(vec![Content::image(
            "iVBORw0KGgo=".to_string(),
            "image/png",
        )]);
        let text = text_content(&result);
        assert!(text.is_empty(), "non-text content should be filtered out");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_ts_search_with_invalid_symbol_kind() {
        let server = setup_server();
        let params = Parameters(TsSearchParams {
            query: "authenticate".to_string(),
            scope: None,
            language: None,
            symbol_kind: Some("invalid_kind_xyz".to_string()),
            limit: None,
            offset: None,
        });
        let result = server.ts_search(params).await.unwrap();
        let text = text_content(&result);
        // Invalid kind should be treated as None (no filter), so results still returned
        assert!(
            text.contains("authenticateUser"),
            "invalid symbol_kind should be ignored"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_ts_overview_with_scope() {
        let server = setup_server();
        let params = Parameters(TsOverviewParams {
            scope: Some("src/".to_string()),
        });
        let result = server.ts_overview(params).await.unwrap();
        let text = text_content(&result);
        assert!(text.contains("Overview"), "should contain header");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_ts_overview_empty_scope_no_data() {
        // Server with no data to test empty branches in build_overview
        let store = Store::open_in_memory().unwrap();
        let indexer = Arc::new(tokenstunt_index::Indexer::new(store, None, None).unwrap());
        let server = TokenStuntServer::new(indexer, PathBuf::from("/empty"), false);

        let params = Parameters(TsOverviewParams { scope: None });
        let result = server.ts_overview(params).await.unwrap();
        let text = text_content(&result);
        assert!(text.contains("Overview"), "should contain header");
        // Empty store should not contain language/module/API sections
        assert!(
            !text.contains("Languages"),
            "empty store should not show languages"
        );
        assert!(
            !text.contains("Modules"),
            "empty store should not show modules"
        );
        assert!(
            !text.contains("Public API"),
            "empty store should not show public API"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_ts_search_with_embedder() {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        let file_id = store
            .upsert_file(repo_id, "src/auth.ts", 111, "typescript", 0)
            .unwrap();
        let block_id = store
            .insert_code_block(
                file_id,
                "authenticateUser",
                CodeBlockKind::Function,
                1,
                10,
                "function authenticateUser(token: string): User { ... }",
                "function authenticateUser(token: string): User",
                "",
                None,
            )
            .unwrap();

        // Insert embedding vector for the block
        let vector: Vec<f32> = vec![0.1; 64];
        store
            .insert_embedding(block_id, &vector, "fake-model")
            .unwrap();

        let fake = Arc::new(FakeEmbeddingProvider {
            dims: 64,
            model: "fake-model".to_string(),
        });
        let indexer = Arc::new(
            tokenstunt_index::Indexer::new(
                store,
                Some(fake as Arc<dyn tokenstunt_embeddings::EmbeddingProvider>),
                None,
            )
            .unwrap(),
        );
        let server = TokenStuntServer::new(indexer, PathBuf::from("/test"), true);

        let params = Parameters(TsSearchParams {
            query: "authenticate".to_string(),
            scope: None,
            language: None,
            symbol_kind: None,
            limit: None,
            offset: None,
        });
        let result = server.ts_search(params).await.unwrap();
        let text = text_content(&result);
        assert!(
            text.contains("authenticateUser"),
            "hybrid search should return results"
        );
    }

    #[tokio::test]
    async fn test_ts_file_returns_symbols() {
        let server = setup_server();
        let params = Parameters(TsFileParams {
            path: "src/auth.ts".to_string(),
            kind: None,
        });
        let result = server.ts_file(params).await.unwrap();
        let text = text_content(&result);
        assert!(text.contains("File"), "should contain File header");
        assert!(
            text.contains("authenticateUser"),
            "should list authenticateUser"
        );
        assert!(text.contains("UserProfile"), "should list UserProfile");
        assert!(text.contains("validateToken"), "should list validateToken");
        assert!(text.contains("3 symbols"), "should show symbol count");
    }

    #[tokio::test]
    async fn test_ts_file_filter_by_kind() {
        let server = setup_server();
        let params = Parameters(TsFileParams {
            path: "src/auth.ts".to_string(),
            kind: Some("class".to_string()),
        });
        let result = server.ts_file(params).await.unwrap();
        let text = text_content(&result);
        assert!(text.contains("UserProfile"), "should include class");
        assert!(
            !text.contains("authenticateUser"),
            "should exclude functions"
        );
    }

    #[tokio::test]
    async fn test_ts_file_not_found() {
        let server = setup_server();
        let params = Parameters(TsFileParams {
            path: "nonexistent.ts".to_string(),
            kind: None,
        });
        let result = server.ts_file(params).await.unwrap();
        let text = text_content(&result);
        assert!(text.contains("No symbols found"));
    }

    #[tokio::test]
    async fn test_ts_usages_found() {
        let server = setup_server();
        let params = Parameters(TsUsagesParams {
            symbol: "validateToken".to_string(),
            kind: None,
            limit: None,
        });
        let result = server.ts_usages(params).await.unwrap();
        let text = text_content(&result);
        assert!(text.contains("Usages"), "should contain Usages header");
        assert!(
            text.contains("authenticateUser"),
            "authenticateUser calls validateToken"
        );
    }

    #[tokio::test]
    async fn test_ts_usages_no_usages() {
        let server = setup_server();
        let params = Parameters(TsUsagesParams {
            symbol: "authenticateUser".to_string(),
            kind: None,
            limit: None,
        });
        let result = server.ts_usages(params).await.unwrap();
        let text = text_content(&result);
        assert!(
            text.contains("No usages found"),
            "authenticateUser has no dependents"
        );
    }

    #[tokio::test]
    async fn test_ts_usages_symbol_not_found() {
        let server = setup_server();
        let params = Parameters(TsUsagesParams {
            symbol: "nonexistentSymbol".to_string(),
            kind: None,
            limit: None,
        });
        let result = server.ts_usages(params).await.unwrap();
        let text = text_content(&result);
        assert!(text.contains("not found"));
    }

    #[tokio::test]
    async fn test_ts_usages_with_limit() {
        let server = setup_server();
        let params = Parameters(TsUsagesParams {
            symbol: "validateToken".to_string(),
            kind: None,
            limit: Some(1),
        });
        let result = server.ts_usages(params).await.unwrap();
        let text = text_content(&result);
        assert!(text.contains("Usages"));
    }

    #[tokio::test]
    async fn test_ts_usages_with_kind_filter() {
        let server = setup_server();
        let params = Parameters(TsUsagesParams {
            symbol: "validateToken".to_string(),
            kind: Some("function".to_string()),
            limit: None,
        });
        let result = server.ts_usages(params).await.unwrap();
        let text = text_content(&result);
        assert!(
            text.contains("Usages"),
            "should find usages with kind filter"
        );
    }

    #[tokio::test]
    async fn test_ts_context_no_dependencies() {
        let server = setup_server();
        // UserProfile has no dependencies, so the dependencies block should be skipped
        let params = Parameters(TsContextParams {
            symbol: "UserProfile".to_string(),
            direction: Some("dependencies".to_string()),
            file: None,
            kind: None,
        });
        let result = server.ts_context(params).await.unwrap();
        let text = text_content(&result);
        assert!(text.contains("UserProfile"));
        assert!(
            !text.contains("Dependencies"),
            "symbol with no deps should not show Dependencies section"
        );
    }
}
