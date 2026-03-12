use std::path::PathBuf;
use std::sync::Arc;

use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::{schemars, tool, tool_router, ErrorData as McpError};
use serde::Deserialize;
use tokenstunt_index::Indexer;
use tokenstunt_search::{SearchEngine, SearchQuery};
use tokenstunt_store::CodeBlockKind;

use crate::format;

#[derive(Clone)]
#[allow(dead_code)]
pub struct TokenStuntServer {
    indexer: Arc<Indexer>,
    root: PathBuf,
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
struct TsOverviewParams {}

#[tool_router]
impl TokenStuntServer {
    pub fn new(indexer: Arc<Indexer>, root: PathBuf) -> Self {
        Self {
            indexer,
            root,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "ts_search",
        description = "Code search across indexed symbols. Returns ranked code blocks (exact function/class/type bodies), not full files."
    )]
    async fn ts_search(
        &self,
        params: Parameters<TsSearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let engine = SearchEngine::new(self.indexer.store());

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

        let output = format::format_blocks(&blocks);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    #[tool(
        name = "ts_symbol",
        description = "Exact symbol lookup by name. Returns the full definition of a function, class, or type."
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

        let blocks: Vec<_> = results.iter().map(|b| (b.clone(), None)).collect();
        let output = format::format_blocks(&blocks);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    #[tool(
        name = "ts_context",
        description = "Symbol definition + dependency graph. Shows what this symbol calls and what calls it."
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
        let mut output = format::format_block(symbol, None);

        let direction = p.direction.as_deref().unwrap_or("both");

        if matches!(direction, "dependencies" | "both") {
            let deps = store
                .get_dependencies(symbol.id)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            if !deps.is_empty() {
                output.push_str("\n\n### Dependencies\n");
                for (block, kind) in &deps {
                    output.push_str(&format!(
                        "\n- **{}** ({}) [{}]",
                        block.name, block.kind, kind
                    ));
                }
            }
        }

        if matches!(direction, "dependents" | "both") {
            let deps = store
                .get_dependents(symbol.id)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            if !deps.is_empty() {
                output.push_str("\n\n### Dependents\n");
                for (block, kind) in &deps {
                    output.push_str(&format!(
                        "\n- **{}** ({}) [{}]",
                        block.name, block.kind, kind
                    ));
                }
            }
        }

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    #[tool(
        name = "ts_overview",
        description = "Project structure: module tree, language breakdown, public API surface, and entry points."
    )]
    async fn ts_overview(
        &self,
        _params: Parameters<TsOverviewParams>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.indexer.store();

        let file_count = store
            .file_count()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let block_count = store
            .block_count()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let output = format!(
            "## Project Overview\n\n- **Root**: {}\n- **Indexed files**: {}\n- **Code blocks**: {}\n",
            self.root.display(),
            file_count,
            block_count,
        );

        Ok(CallToolResult::success(vec![Content::text(output)]))
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

        let indexer = Arc::new(tokenstunt_index::Indexer::new(store).unwrap());
        TokenStuntServer::new(indexer, PathBuf::from("/test"))
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
        let indexer = Arc::new(tokenstunt_index::Indexer::new(store).unwrap());
        let server = TokenStuntServer::new(indexer, PathBuf::from("/test"));
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
        assert!(text.contains("authenticateUser"), "expected block name in results");
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
        assert!(text.contains("Dependencies"), "should show dependencies section");
        assert!(text.contains("validateToken"), "should list validateToken as dependency");
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
        assert!(!text.contains("Dependents"), "should not show dependents section");
    }

    #[tokio::test]
    async fn test_ts_context_dependents_only() {
        let server = setup_server();
        // validateToken is depended on by authenticateUser
        let params = Parameters(TsContextParams {
            symbol: "validateToken".to_string(),
            direction: Some("dependents".to_string()),
        });
        let result = server.ts_context(params).await.unwrap();
        let text = text_content(&result);
        assert!(text.contains("Dependents"), "should show dependents section");
        assert!(text.contains("authenticateUser"), "should list authenticateUser as dependent");
        assert!(!text.contains("Dependencies"), "should not show dependencies section");
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
        let params = Parameters(TsOverviewParams {});
        let result = server.ts_overview(params).await.unwrap();
        let text = text_content(&result);
        assert!(text.contains("Project Overview"));
        assert!(text.contains("/test"));
        assert!(text.contains("Indexed files"));
        assert!(text.contains("Code blocks"));
    }
}

impl rmcp::handler::server::ServerHandler for TokenStuntServer {
    fn get_info(&self) -> InitializeResult {
        let capabilities = ServerCapabilities::builder()
            .enable_tools()
            .build();

        InitializeResult::new(capabilities)
            .with_server_info(
                Implementation::new("tokenstunt", env!("CARGO_PKG_VERSION"))
                    .with_title("TokenStunt")
                    .with_description("Smart code search for Claude Code. Finds the exact code you need — saves 95% of tokens.")
            )
            .with_instructions(
                "TokenStunt provides AST-level semantic code search. Use ts_search for natural language queries, ts_symbol for exact lookups, ts_context for dependency graphs, ts_overview for project summaries."
            )
    }
}
