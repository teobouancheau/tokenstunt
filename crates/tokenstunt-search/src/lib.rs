use std::collections::HashMap;

use anyhow::Result;
use tokenstunt_store::{CodeBlock, CodeBlockKind, Store};

const DEFAULT_HYBRID_ALPHA: f64 = 0.4;

pub struct SearchEngine<'a> {
    store: &'a Store,
    hybrid_alpha: f64,
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
    pub query_embedding: Option<Vec<f32>>,
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
            hybrid_alpha: DEFAULT_HYBRID_ALPHA,
        }
    }

    pub fn with_alpha(store: &'a Store, hybrid_alpha: f64) -> Self {
        Self {
            store,
            hybrid_alpha,
        }
    }

    pub fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        if query.text.trim().is_empty() {
            return Ok(Vec::new());
        }

        let limit = if query.limit == 0 { 10 } else { query.limit };
        let fts_query = build_fts_query(&query.text);
        let kind_str = query.symbol_kind.map(|k| k.as_str().to_string());

        let bm25_results = self.store.search_fts(
            &fts_query,
            query.language.as_deref(),
            kind_str.as_deref(),
            query.scope.as_deref(),
            limit,
        )?;

        let query_embedding = query.query_embedding.as_deref();

        if query_embedding.is_none() {
            // Use raw BM25 scores with min-max normalization
            return Ok(normalize_bm25_results(bm25_results));
        }

        let query_vec = query_embedding.expect("checked above");

        // Fetch embeddings only for BM25 candidate blocks (O(K) not O(N))
        let candidate_ids: Vec<i64> = bm25_results.iter().map(|(b, _)| b.id).collect();
        let stored_embeddings = self.store.get_embeddings_by_block_ids(&candidate_ids)?;

        if stored_embeddings.is_empty() {
            return Ok(normalize_bm25_results(bm25_results));
        }

        let semantic_scores: HashMap<i64, f64> = stored_embeddings
            .iter()
            .map(|(block_id, embedding)| {
                (*block_id, cosine_similarity(query_vec, embedding) as f64)
            })
            .collect();

        // Min-max normalize BM25 scores (FTS5 rank is negative, lower = better)
        let raw_scores: Vec<f64> = bm25_results.iter().map(|(_, s)| *s).collect();
        let bm25_min = raw_scores.iter().cloned().fold(f64::INFINITY, f64::min);
        let bm25_max = raw_scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let bm25_range = bm25_max - bm25_min;

        let alpha = self.hybrid_alpha;

        let mut results: Vec<SearchResult> = bm25_results
            .into_iter()
            .map(|(block, raw_score)| {
                // Normalize: best score (most negative) -> 1.0, worst -> 0.0
                let bm25_normalized = if bm25_range.abs() < f64::EPSILON {
                    1.0
                } else {
                    1.0 - ((raw_score - bm25_min) / bm25_range)
                };
                let cosine_score = semantic_scores.get(&block.id).copied().unwrap_or(0.0);
                let score = (alpha * bm25_normalized) + ((1.0 - alpha) * cosine_score);
                let source = if cosine_score > 0.0 {
                    SearchSource::Hybrid
                } else {
                    SearchSource::Bm25
                };
                SearchResult {
                    block,
                    score,
                    source,
                }
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

fn normalize_bm25_results(results: Vec<(CodeBlock, f64)>) -> Vec<SearchResult> {
    if results.is_empty() {
        return Vec::new();
    }

    let raw_scores: Vec<f64> = results.iter().map(|(_, s)| *s).collect();
    let min = raw_scores.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = raw_scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;

    results
        .into_iter()
        .map(|(block, raw_score)| {
            let score = if range.abs() < f64::EPSILON {
                1.0
            } else {
                1.0 - ((raw_score - min) / range)
            };
            SearchResult {
                block,
                score,
                source: SearchSource::Bm25,
            }
        })
        .collect()
}

fn sanitize_fts_term(term: &str) -> String {
    term.chars()
        .filter(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

/// Split a compound identifier into its component parts.
/// Handles dot-separated (`auth.service`), hyphen-separated (`my-component`),
/// colon-separated (`std::io`), and camelCase (`getUserById`).
fn split_identifier(input: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();

    // Split on dots, hyphens, colons, slashes
    let segments: Vec<&str> = input
        .split(['.', '-', ':', '/'])
        .filter(|s| !s.is_empty())
        .collect();

    for segment in &segments {
        let sanitized = sanitize_fts_term(segment);
        if sanitized.is_empty() {
            continue;
        }

        // Split camelCase / PascalCase
        let camel_parts = split_camel_case(&sanitized);
        for part in &camel_parts {
            let lower = part.to_lowercase();
            if !lower.is_empty() && !parts.contains(&lower) {
                parts.push(lower);
            }
        }

        // Also keep the full segment if it differs from the parts
        let full_lower = sanitized.to_lowercase();
        if !full_lower.is_empty() && !parts.contains(&full_lower) {
            parts.push(full_lower);
        }
    }

    // Keep the full original term (sanitized) if it has multiple segments
    if segments.len() > 1 {
        let full = sanitize_fts_term(input).to_lowercase();
        if !full.is_empty() && !parts.contains(&full) {
            parts.push(full);
        }
    }

    parts
}

fn split_camel_case(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = input.chars().collect();

    for i in 0..chars.len() {
        let c = chars[i];
        if c.is_uppercase() && !current.is_empty() {
            // Check if this is the start of a new word
            let prev_lower = i > 0 && chars[i - 1].is_lowercase();
            let next_lower = i + 1 < chars.len() && chars[i + 1].is_lowercase();
            if prev_lower || (next_lower && current.len() > 1) {
                parts.push(current.clone());
                current.clear();
            }
        }
        current.push(c);
    }
    if !current.is_empty() {
        parts.push(current);
    }

    // Only return parts if we actually split something
    if parts.len() <= 1 {
        return Vec::new();
    }

    parts
}

fn build_fts_query(input: &str) -> String {
    let mut all_terms: Vec<String> = Vec::new();

    for word in input.split_whitespace() {
        let parts = split_identifier(word);
        if parts.is_empty() {
            let sanitized = sanitize_fts_term(word);
            if !sanitized.is_empty() && !all_terms.contains(&sanitized) {
                all_terms.push(sanitized);
            }
        } else {
            for part in parts {
                if !all_terms.contains(&part) {
                    all_terms.push(part);
                }
            }
        }
    }

    if all_terms.is_empty() {
        return String::new();
    }

    all_terms
        .iter()
        .map(|t| format!("{t}*"))
        .collect::<Vec<_>>()
        .join(" OR ")
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

        store
    }

    #[test]
    fn test_build_fts_query_simple() {
        assert_eq!(
            build_fts_query("authenticate user"),
            "authenticate* OR user*"
        );
        assert_eq!(build_fts_query("single"), "single*");
        assert_eq!(build_fts_query(""), "");
    }

    #[test]
    fn test_build_fts_query_special_chars() {
        assert_eq!(build_fts_query("\"hello\""), "hello*");
        assert_eq!(build_fts_query("user_name"), "user_name*");
        assert_eq!(build_fts_query("()"), "");
    }

    #[test]
    fn test_build_fts_query_dot_separated() {
        let query = build_fts_query("auth.service");
        assert!(query.contains("auth*"));
        assert!(query.contains("service*"));
        assert!(query.contains("authservice*"));
    }

    #[test]
    fn test_build_fts_query_hyphen_separated() {
        let query = build_fts_query("a-b");
        assert!(query.contains("a*"));
        assert!(query.contains("b*"));
    }

    #[test]
    fn test_build_fts_query_camel_case() {
        let query = build_fts_query("getUserById");
        assert!(query.contains("get*"));
        assert!(query.contains("user*"));
        assert!(query.contains("by*"));
        assert!(query.contains("id*"));
        assert!(query.contains("getuserbyid*"));
    }

    #[test]
    fn test_build_fts_query_colon_separated() {
        let query = build_fts_query("std::io");
        assert!(query.contains("std*"));
        assert!(query.contains("io*"));
    }

    #[test]
    fn test_split_camel_case() {
        assert_eq!(
            split_camel_case("getUserById"),
            vec!["get", "User", "By", "Id"]
        );
        assert!(split_camel_case("simple").is_empty());
        assert_eq!(split_camel_case("HTMLParser"), vec!["HTML", "Parser"]);
    }

    #[test]
    fn test_split_identifier_compound() {
        let parts = split_identifier("auth.service");
        assert!(parts.contains(&"auth".to_string()));
        assert!(parts.contains(&"service".to_string()));

        let parts = split_identifier("getUserById");
        assert!(parts.contains(&"get".to_string()));
        assert!(parts.contains(&"user".to_string()));
        assert!(parts.contains(&"getuserbyid".to_string()));
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
    fn test_hybrid_search_with_embeddings() {
        let store = setup_store();

        let blocks = store.lookup_symbol("authenticateUser", None).unwrap();
        assert!(!blocks.is_empty());
        store
            .insert_embedding(blocks[0].id, &vec![0.1; 64], "fake-model")
            .unwrap();

        let blocks2 = store.lookup_symbol("UserProfile", None).unwrap();
        assert!(!blocks2.is_empty());
        store
            .insert_embedding(blocks2[0].id, &vec![0.2; 64], "fake-model")
            .unwrap();

        let engine = SearchEngine::new(&store);

        let query = SearchQuery {
            text: "authenticate".to_string(),
            limit: 10,
            query_embedding: Some(vec![0.1; 64]),
            ..Default::default()
        };
        let results = engine.search(&query).unwrap();
        assert!(!results.is_empty());

        let has_hybrid = results.iter().any(|r| r.source == SearchSource::Hybrid);
        assert!(
            has_hybrid,
            "should have hybrid results when embeddings exist"
        );

        for pair in results.windows(2) {
            assert!(
                pair[0].score >= pair[1].score,
                "results should be sorted by score descending"
            );
        }
    }

    #[test]
    fn test_hybrid_search_no_embeddings_stored() {
        let store = setup_store();
        let engine = SearchEngine::new(&store);

        let query = SearchQuery {
            text: "authenticate".to_string(),
            limit: 10,
            query_embedding: Some(vec![0.1; 64]),
            ..Default::default()
        };
        let results = engine.search(&query).unwrap();
        assert!(!results.is_empty());

        for r in &results {
            assert_eq!(
                r.source,
                SearchSource::Bm25,
                "without stored embeddings, results should be BM25"
            );
        }
    }

    #[test]
    fn test_hybrid_search_no_query_embedding() {
        let store = setup_store();

        let blocks = store.lookup_symbol("authenticateUser", None).unwrap();
        store
            .insert_embedding(blocks[0].id, &vec![0.1; 64], "fake-model")
            .unwrap();

        let engine = SearchEngine::new(&store);

        let query = SearchQuery {
            text: "authenticate".to_string(),
            limit: 10,
            query_embedding: None,
            ..Default::default()
        };
        let results = engine.search(&query).unwrap();
        assert!(!results.is_empty());

        for r in &results {
            assert_eq!(
                r.source,
                SearchSource::Bm25,
                "without query embedding, results should be BM25"
            );
        }
    }

    #[test]
    fn test_search_source_variants() {
        assert_ne!(SearchSource::Bm25, SearchSource::Semantic);
        assert_ne!(SearchSource::Bm25, SearchSource::Hybrid);
        assert_ne!(SearchSource::Semantic, SearchSource::Hybrid);
    }

    #[test]
    fn test_hybrid_search_bm25_block_without_embedding() {
        let store = setup_store();

        let blocks = store.lookup_symbol("UserProfile", None).unwrap();
        assert!(!blocks.is_empty());
        store
            .insert_embedding(blocks[0].id, &vec![0.2; 64], "fake-model")
            .unwrap();

        let engine = SearchEngine::new(&store);

        let query = SearchQuery {
            text: "authenticate".to_string(),
            limit: 10,
            query_embedding: Some(vec![0.1; 64]),
            ..Default::default()
        };
        let results = engine.search(&query).unwrap();
        assert!(!results.is_empty());

        let auth_result = results.iter().find(|r| r.block.name == "authenticateUser");
        assert!(
            auth_result.is_some(),
            "authenticateUser should appear in results"
        );
        assert_eq!(
            auth_result.unwrap().source,
            SearchSource::Bm25,
            "block without embedding should be Bm25 in hybrid results"
        );
    }

    #[test]
    fn test_search_fts_error_propagation() {
        let store = Store::open_in_memory().unwrap();
        let engine = SearchEngine::new(&store);
        let query = SearchQuery {
            text: "test".to_string(),
            limit: 5,
            ..Default::default()
        };
        let results = engine.search(&query).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_fts_error_propagates_to_search() {
        let store = Store::open_in_memory().unwrap();
        store
            .write_transaction(|conn| {
                conn.execute_batch("DROP TABLE IF EXISTS code_blocks_fts")
                    .unwrap();
                Ok(())
            })
            .unwrap();
        let engine = SearchEngine::new(&store);
        let query = SearchQuery {
            text: "anything".to_string(),
            limit: 10,
            ..Default::default()
        };
        let err = engine.search(&query);
        assert!(err.is_err(), "search should propagate FTS error");
    }

    #[test]
    fn test_with_alpha() {
        let store = setup_store();
        let engine = SearchEngine::with_alpha(&store, 0.7);
        assert!((engine.hybrid_alpha - 0.7).abs() < 0.001);
    }

    #[test]
    fn test_get_embeddings_by_block_ids() {
        let store = setup_store();

        let blocks = store.lookup_symbol("authenticateUser", None).unwrap();
        let block_id = blocks[0].id;
        store
            .insert_embedding(block_id, &vec![0.1; 64], "fake-model")
            .unwrap();

        let results = store.get_embeddings_by_block_ids(&[block_id]).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, block_id);

        let empty = store.get_embeddings_by_block_ids(&[]).unwrap();
        assert!(empty.is_empty());

        let miss = store.get_embeddings_by_block_ids(&[99999]).unwrap();
        assert!(miss.is_empty());
    }
}
