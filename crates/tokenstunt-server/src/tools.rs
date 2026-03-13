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
}

#[derive(Deserialize, schemars::JsonSchema)]
struct TsSymbolParams {
    /// Exact symbol name to look up
    name: String,
    /// Filter by symbol kind
    kind: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct TsContextParams {
    /// Symbol name to get context for
    symbol: String,
    /// Direction: "dependencies", "dependents", or "both"
    direction: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct TsOverviewParams {
    /// Scope to a directory path (e.g. "src/")
    scope: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct TsSetupParams {}

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
        Self {
            indexer,
            root,
            has_embeddings,
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
        let engine = match self.indexer.embedder() {
            Some(embedder) => SearchEngine::with_embedder(self.indexer.store(), embedder.as_ref()),
            None => SearchEngine::new(self.indexer.store()),
        };

        let query_text = p.query.clone();
        let query = SearchQuery {
            text: p.query,
            scope: p.scope,
            language: p.language,
            symbol_kind: p.symbol_kind.and_then(|s| CodeBlockKind::from_str(&s)),
            limit: p.limit.unwrap_or(10),
        };

        let results = engine
            .search(&query)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        if results.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No results found.",
            )]));
        }

        let blocks: Vec<_> = results
            .iter()
            .map(|r| (r.block.clone(), Some(r.score)))
            .collect();

        let output = format::format_blocks(&query_text, &blocks);
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
        let engine = SearchEngine::new(self.indexer.store());

        let kind = p.kind.and_then(|s| CodeBlockKind::from_str(&s));
        let results = engine
            .lookup_symbol(&p.name, kind)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

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

        let symbols = store
            .lookup_symbol(&p.symbol, None)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

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
                "Token Stunt provides AST-level semantic code search. Use ts_search instead of Grep+Read when looking for code by concept — it returns exact symbol bodies, saving 95% of tokens. Use ts_symbol for exact name lookups. Use ts_context to understand what a symbol calls and what calls it. Use ts_impact before refactoring to understand blast radius. Use ts_overview to orient in the project. Use ts_setup to check index health. Only use Read for files you need to modify. Recommended workflow: ts_overview → ts_search → ts_symbol → ts_context/ts_impact → Read."
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::handler::server::ServerHandler;
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
                None,
            )
            .unwrap();

        // authenticateUser --calls--> validateToken
        store
            .insert_dependency(auth_id, Some(validate_id), "validateToken", "call")
            .unwrap();

        let indexer = Arc::new(tokenstunt_index::Indexer::new(store, None).unwrap());
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
        let indexer = Arc::new(tokenstunt_index::Indexer::new(store, None).unwrap());
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
}
