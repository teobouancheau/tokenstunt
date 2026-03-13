use std::collections::HashMap;

use anyhow::Result;
use tokenstunt_embeddings::EmbeddingProvider;
use tokenstunt_store::{CodeBlock, CodeBlockKind, Store};

const HYBRID_ALPHA: f64 = 0.4;

pub struct SearchEngine<'a> {
    store: &'a Store,
    embedder: Option<&'a dyn EmbeddingProvider>,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub block: CodeBlock,
    pub score: f64,
    pub source: SearchSource,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SearchSource {
    Bm25,
    Semantic,
    Hybrid,
}

#[derive(Debug, Default)]
pub struct SearchQuery {
    pub text: String,
    pub scope: Option<String>,
    pub language: Option<String>,
    pub symbol_kind: Option<CodeBlockKind>,
    pub limit: usize,
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

impl<'a> SearchEngine<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self {
            store,
            embedder: None,
        }
    }

    pub fn with_embedder(store: &'a Store, embedder: &'a dyn EmbeddingProvider) -> Self {
        Self {
            store,
            embedder: Some(embedder),
        }
    }

    pub fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        if query.text.trim().is_empty() {
            return Ok(Vec::new());
        }

        let limit = if query.limit == 0 { 10 } else { query.limit };
        let fts_query = build_fts_query(&query.text);
        let kind_str = query.symbol_kind.map(|k| k.as_str().to_string());

        let bm25_blocks = self.store.search_fts(
            &fts_query,
            query.language.as_deref(),
            kind_str.as_deref(),
            query.scope.as_deref(),
            limit,
        )?;

        if self.embedder.is_none() {
            return Ok(bm25_blocks
                .into_iter()
                .enumerate()
                .map(|(rank, block)| SearchResult {
                    score: 1.0 / (rank as f64 + 1.0),
                    block,
                    source: SearchSource::Bm25,
                })
                .collect());
        }

        let embedder = self.embedder.expect("checked above");
        let query_embedding = match embed_sync(embedder, &query.text) {
            Ok(embedding) => embedding,
            Err(_) => {
                return Ok(bm25_blocks
                    .into_iter()
                    .enumerate()
                    .map(|(rank, block)| SearchResult {
                        score: 1.0 / (rank as f64 + 1.0),
                        block,
                        source: SearchSource::Bm25,
                    })
                    .collect());
            }
        };

        let stored_embeddings = self.store.get_all_embeddings()?;
        if stored_embeddings.is_empty() {
            return Ok(bm25_blocks
                .into_iter()
                .enumerate()
                .map(|(rank, block)| SearchResult {
                    score: 1.0 / (rank as f64 + 1.0),
                    block,
                    source: SearchSource::Bm25,
                })
                .collect());
        }

        let mut semantic_scores: HashMap<i64, f64> = HashMap::new();
        for (block_id, embedding) in &stored_embeddings {
            let sim = cosine_similarity(&query_embedding, embedding);
            semantic_scores.insert(*block_id, sim as f64);
        }

        let bm25_max = bm25_blocks.len() as f64;
        let mut hybrid_scores: HashMap<i64, (f64, CodeBlock, SearchSource)> = HashMap::new();

        for (rank, block) in bm25_blocks.into_iter().enumerate() {
            let bm25_normalized = 1.0 - (rank as f64 / bm25_max);
            let cosine_score = semantic_scores.get(&block.id).copied().unwrap_or(0.0);
            let score = (HYBRID_ALPHA * bm25_normalized) + ((1.0 - HYBRID_ALPHA) * cosine_score);
            let source = if cosine_score > 0.0 {
                SearchSource::Hybrid
            } else {
                SearchSource::Bm25
            };
            hybrid_scores.insert(block.id, (score, block, source));
        }

        for (block_id, cosine_score) in &semantic_scores {
            if hybrid_scores.contains_key(block_id) {
                continue;
            }
            let score = (1.0 - HYBRID_ALPHA) * cosine_score;
            if let Some(block) = self.store.get_block_by_id(*block_id)? {
                hybrid_scores.insert(*block_id, (score, block, SearchSource::Semantic));
            }
        }

        let mut results: Vec<SearchResult> = hybrid_scores
            .into_values()
            .map(|(score, block, source)| SearchResult {
                block,
                score,
                source,
            })
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);

        Ok(results)
    }

    pub fn lookup_symbol(&self, name: &str, kind: Option<CodeBlockKind>) -> Result<Vec<CodeBlock>> {
        self.store.lookup_symbol(name, kind)
    }
}

fn embed_sync(embedder: &dyn EmbeddingProvider, text: &str) -> Result<Vec<f32>> {
    let handle = tokio::runtime::Handle::current();
    let texts = vec![text.to_string()];
    let mut results = handle.block_on(embedder.embed_batch(&texts))?;
    results
        .pop()
        .ok_or_else(|| anyhow::anyhow!("embedding returned no results"))
}

fn sanitize_fts_term(term: &str) -> String {
    term.chars()
        .filter(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

fn build_fts_query(input: &str) -> String {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(sanitize_fts_term)
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
        assert_eq!(
            build_fts_query("authenticate user"),
            "authenticate* OR user*"
        );
        assert_eq!(build_fts_query("single"), "single*");
        assert_eq!(build_fts_query(""), "");
    }

    #[test]
    fn test_build_fts_query_special_chars() {
        assert_eq!(build_fts_query("foo(bar)"), "foobar*");
        assert_eq!(build_fts_query("\"hello\""), "hello*");
        assert_eq!(build_fts_query("a-b"), "ab*");
        assert_eq!(build_fts_query("user_name"), "user_name*");
        assert_eq!(build_fts_query("()"), "");
    }

    #[test]
    fn test_search_special_chars_no_crash() {
        let store = setup_store();
        let engine = SearchEngine::new(&store);
        let query = SearchQuery {
            text: "foo(bar) \"baz\"".to_string(),
            limit: 10,
            ..Default::default()
        };
        let results = engine.search(&query);
        assert!(results.is_ok());
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

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 0.001);

        let c = vec![0.0, 1.0, 0.0];
        assert!((cosine_similarity(&a, &c) - 0.0).abs() < 0.001);

        let d = vec![0.5, 0.5, 0.0];
        let sim = cosine_similarity(&a, &d);
        assert!(sim > 0.0 && sim < 1.0);

        let zero = vec![0.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &zero) - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_search_source_variants() {
        assert_ne!(SearchSource::Bm25, SearchSource::Semantic);
        assert_ne!(SearchSource::Bm25, SearchSource::Hybrid);
        assert_ne!(SearchSource::Semantic, SearchSource::Hybrid);
    }
}
