use anyhow::Result;
use tokenstunt_store::{CodeBlock, CodeBlockKind, Store};

pub struct SearchEngine<'a> {
    store: &'a Store,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub block: CodeBlock,
    pub score: f64,
    pub source: SearchSource,
}

#[derive(Debug, Clone, Copy)]
pub enum SearchSource {
    Bm25,
}

#[derive(Debug, Default)]
pub struct SearchQuery {
    pub text: String,
    pub scope: Option<String>,
    pub language: Option<String>,
    pub symbol_kind: Option<CodeBlockKind>,
    pub limit: usize,
}

impl<'a> SearchEngine<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    pub fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        if query.text.trim().is_empty() {
            return Ok(Vec::new());
        }

        let limit = if query.limit == 0 { 10 } else { query.limit };
        let fts_query = build_fts_query(&query.text);
        let blocks = self.store.search_fts(&fts_query, limit)?;

        let results = blocks
            .into_iter()
            .enumerate()
            .filter(|(_, block)| {
                if let Some(ref lang) = query.language {
                    if let Some(ref block_lang) = block.language {
                        if block_lang != lang {
                            return false;
                        }
                    }
                }
                if let Some(kind) = query.symbol_kind {
                    if block.kind != kind {
                        return false;
                    }
                }
                if let Some(ref scope) = query.scope {
                    if let Some(ref path) = block.file_path {
                        if !path.starts_with(scope.as_str()) {
                            return false;
                        }
                    }
                }
                true
            })
            .map(|(rank, block)| SearchResult {
                score: 1.0 / (rank as f64 + 1.0),
                block,
                source: SearchSource::Bm25,
            })
            .collect();

        Ok(results)
    }

    pub fn lookup_symbol(
        &self,
        name: &str,
        kind: Option<CodeBlockKind>,
    ) -> Result<Vec<CodeBlock>> {
        self.store.lookup_symbol(name, kind)
    }
}

fn build_fts_query(input: &str) -> String {
    let terms: Vec<String> = input
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| format!("{t}*"))
        .collect();

    if terms.is_empty() {
        return String::new();
    }

    terms.join(" OR ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokenstunt_store::Store;

    fn setup_store() -> Store {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        let file_id = store
            .upsert_file(repo_id, "src/auth.ts", 111, "typescript", 0)
            .unwrap();

        store
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

        store
    }

    #[test]
    fn test_build_fts_query() {
        assert_eq!(build_fts_query("authenticate user"), "authenticate* OR user*");
        assert_eq!(build_fts_query("single"), "single*");
        assert_eq!(build_fts_query(""), "");
    }

    #[test]
    fn test_search_basic() {
        let store = setup_store();
        let engine = SearchEngine::new(&store);

        let query = SearchQuery {
            text: "authenticate".to_string(),
            limit: 10,
            ..Default::default()
        };

        let results = engine.search(&query).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].block.name, "authenticateUser");
        assert!(results[0].score > 0.0);
        assert!(matches!(results[0].source, SearchSource::Bm25));
    }

    #[test]
    fn test_search_empty_query() {
        let store = setup_store();
        let engine = SearchEngine::new(&store);

        let query = SearchQuery {
            text: "".to_string(),
            ..Default::default()
        };
        assert!(engine.search(&query).unwrap().is_empty());

        let query = SearchQuery {
            text: "   ".to_string(),
            ..Default::default()
        };
        assert!(engine.search(&query).unwrap().is_empty());
    }

    #[test]
    fn test_search_filters() {
        let store = setup_store();
        let engine = SearchEngine::new(&store);

        let query = SearchQuery {
            text: "User".to_string(),
            symbol_kind: Some(CodeBlockKind::Class),
            limit: 10,
            ..Default::default()
        };
        let results = engine.search(&query).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].block.name, "UserProfile");

        let query = SearchQuery {
            text: "User".to_string(),
            language: Some("python".to_string()),
            limit: 10,
            ..Default::default()
        };
        let results = engine.search(&query).unwrap();
        assert!(results.is_empty());

        let query = SearchQuery {
            text: "User".to_string(),
            scope: Some("src/".to_string()),
            limit: 10,
            ..Default::default()
        };
        let results = engine.search(&query).unwrap();
        assert!(!results.is_empty());

        let query = SearchQuery {
            text: "User".to_string(),
            scope: Some("other/".to_string()),
            limit: 10,
            ..Default::default()
        };
        let results = engine.search(&query).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_lookup_symbol() {
        let store = setup_store();
        let engine = SearchEngine::new(&store);

        let results = engine.lookup_symbol("authenticateUser", None).unwrap();
        assert_eq!(results.len(), 1);

        let results = engine
            .lookup_symbol("authenticateUser", Some(CodeBlockKind::Function))
            .unwrap();
        assert_eq!(results.len(), 1);

        let results = engine
            .lookup_symbol("authenticateUser", Some(CodeBlockKind::Class))
            .unwrap();
        assert!(results.is_empty());

        let results = engine.lookup_symbol("nonexistent", None).unwrap();
        assert!(results.is_empty());
    }
}
