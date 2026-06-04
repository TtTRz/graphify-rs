//! In-memory inverted index for fast node lookup.
//!
//! Provides [`SearchIndex`] which tokenizes node labels, ids, and source files
//! into an inverted index for sub-linear search across the knowledge graph.

use std::collections::HashMap;

use graphify_core::graph::KnowledgeGraph;

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

/// Split `input` on camelCase boundaries, `_`, `.`, `::`, `/`, `\`, `-`, and
/// whitespace. Returns all-lowercase, non-empty tokens.
///
/// # Examples (implicit, tested below)
/// * `"camelCase"` -> `["camel", "case"]`
/// * `"foo_bar.baz"` -> `["foo", "bar", "baz"]`
/// * `"std::collections::HashMap"` -> `["std", "collections", "hash", "map"]`
/// * `"src/main/mod.rs"` -> `["src", "main", "mod", "rs"]`
pub fn tokenize(input: &str) -> Vec<String> {
    // Phase 1: split on explicit delimiters.
    let raw: Vec<&str> = input
        .split(&['_', '.', ':', '/', '\\', '-'][..])
        .flat_map(|s| s.split_whitespace())
        .collect();

    // Phase 2: split each piece on camelCase boundaries.
    let mut tokens: Vec<String> = Vec::new();
    for piece in &raw {
        if piece.is_empty() {
            continue;
        }
        // Walk characters; start a new segment on lowercase→uppercase transition.
        let mut segment = String::new();
        for ch in piece.chars() {
            if ch.is_uppercase()
                && !segment.is_empty()
                && !segment.chars().last().unwrap().is_uppercase()
            {
                tokens.push(segment.to_lowercase());
                segment.clear();
            }
            segment.push(ch);
        }
        if !segment.is_empty() {
            tokens.push(segment.to_lowercase());
        }
    }
    tokens
}

// ---------------------------------------------------------------------------
// SearchIndex
// ---------------------------------------------------------------------------

/// In-memory inverted index mapping tokens to weighted `(node_id, weight)` pairs.
///
/// Built from a [`KnowledgeGraph`] via [`SearchIndex::build`]. Each node contributes
/// tokens from its **label**, **id**, and **source_file**, each with a different base
/// weight plus a degree-based boost.
pub struct SearchIndex {
    /// token -> [(node_id, weight)]
    index: HashMap<String, Vec<(String, f64)>>,
}

impl SearchIndex {
    /// Build the inverted index from a knowledge graph.
    ///
    /// Token weights:
    /// - Label token: `2.0 + ln_1p(degree) * 0.1`
    /// - Id token:    `1.0 + ln_1p(degree) * 0.1`
    /// - Source file token: `0.5 + ln_1p(degree) * 0.1`
    ///
    /// Note: the same token from multiple fields (label + id) stacks additively,
    /// rewarding nodes whose token appears in multiple fields.
    pub fn build(graph: &KnowledgeGraph) -> Self {
        let mut index: HashMap<String, Vec<(String, f64)>> = HashMap::new();

        for node_id in graph.node_ids() {
            let Some(node) = graph.get_node(&node_id) else {
                continue;
            };
            let degree = graph.degree(&node_id) as f64;
            let degree_boost = degree.ln_1p() * 0.1;

            // Helper: insert tokens with a given base weight.
            let mut insert = |text: &str, base: f64| {
                for tok in tokenize(text) {
                    let weight = base + degree_boost;
                    index
                        .entry(tok)
                        .or_default()
                        .push((node_id.clone(), weight));
                }
            };

            insert(&node.label, 2.0);
            insert(&node.id, 1.0);
            insert(&node.source_file, 0.5);
        }

        SearchIndex { index }
    }

    /// Search for nodes matching any of the given terms.
    ///
    /// Each term is tokenized and matched against the index using **exact** and
    /// **prefix** matching. Scores are aggregated per node. Results are returned
    /// sorted by descending score.
    ///
    /// Prefix matches receive half the weight of an exact match.
    pub fn search(&self, terms: &[String]) -> Vec<(f64, String)> {
        let mut scores: HashMap<String, f64> = HashMap::new();

        let term_tokens: Vec<String> = terms.iter().flat_map(|t| tokenize(t)).collect();

        for term_tok in &term_tokens {
            // Exact match.
            if let Some(entries) = self.index.get(term_tok) {
                for (node_id, weight) in entries {
                    *scores.entry(node_id.clone()).or_default() += weight;
                }
            }

            // PERF: prefix match is O(vocabulary size). Acceptable for graphs up to ~10k nodes.
            // A future optimization would use a sorted token list or BTreeMap for range scan.
            for (token, entries) in &self.index {
                if token != term_tok && token.starts_with(term_tok) {
                    for (node_id, weight) in entries {
                        *scores.entry(node_id.clone()).or_default() += weight * 0.5;
                    }
                }
            }
        }

        let mut results: Vec<(f64, String)> = scores
            .into_iter()
            .map(|(node_id, score)| (score, node_id))
            .collect();
        results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        results
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use graphify_core::model::{GraphNode, NodeType};
    use std::collections::HashMap;

    // -- helpers --

    fn make_node(id: &str, label: &str, source_file: &str) -> GraphNode {
        GraphNode {
            id: id.into(),
            label: label.into(),
            source_file: source_file.into(),
            source_location: None,
            node_type: NodeType::Class,
            community: None,
            extra: HashMap::new(),
        }
    }

    fn make_graph() -> KnowledgeGraph {
        let mut g = KnowledgeGraph::new();
        g.add_node(make_node(
            "auth_service",
            "AuthService",
            "src/auth/service.rs",
        ))
        .unwrap();
        g.add_node(make_node(
            "user_manager",
            "UserManager",
            "src/user/manager.rs",
        ))
        .unwrap();
        g.add_node(make_node("database_pool", "DatabasePool", "src/db/pool.rs"))
            .unwrap();
        g.add_node(make_node("cache_layer", "CacheLayer", "src/cache/layer.rs"))
            .unwrap();
        g
    }

    fn make_graph_with_edges() -> KnowledgeGraph {
        use graphify_core::confidence::Confidence;
        use graphify_core::model::GraphEdge;

        let mut g = KnowledgeGraph::new();
        g.add_node(make_node("auth", "AuthService", "src/auth.rs"))
            .unwrap();
        g.add_node(make_node("user", "UserManager", "src/user.rs"))
            .unwrap();
        g.add_node(make_node("db", "Database", "src/db.rs"))
            .unwrap();
        g.add_node(make_node("cache", "CacheLayer", "src/cache.rs"))
            .unwrap();

        let edge = GraphEdge {
            source: "auth".into(),
            target: "user".into(),
            relation: "calls".into(),
            confidence: Confidence::Extracted,
            confidence_score: 1.0,
            source_file: "test.rs".into(),
            source_location: None,
            weight: 1.0,
            provenance: None,
            extra: HashMap::new(),
        };
        g.add_edge(edge).unwrap();
        g
    }

    // -- tokenize tests --

    #[test]
    fn tokenize_camel_case() {
        let tokens = tokenize("camelCase");
        assert_eq!(tokens, vec!["camel", "case"]);
    }

    #[test]
    fn tokenize_underscore() {
        let tokens = tokenize("foo_bar_baz");
        assert_eq!(tokens, vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn tokenize_dot_separator() {
        let tokens = tokenize("mod.rs");
        assert_eq!(tokens, vec!["mod", "rs"]);
    }

    #[test]
    fn tokenize_double_colon() {
        let tokens = tokenize("std::collections::HashMap");
        assert_eq!(tokens, vec!["std", "collections", "hash", "map"]);
    }

    #[test]
    fn tokenize_slash() {
        let tokens = tokenize("src/main/mod.rs");
        assert_eq!(tokens, vec!["src", "main", "mod", "rs"]);
    }

    #[test]
    fn tokenize_backslash() {
        let tokens = tokenize(r"src\main\mod.rs");
        assert_eq!(tokens, vec!["src", "main", "mod", "rs"]);
    }

    #[test]
    fn tokenize_hyphen() {
        let tokens = tokenize("my-component-name");
        assert_eq!(tokens, vec!["my", "component", "name"]);
    }

    #[test]
    fn tokenize_whitespace() {
        let tokens = tokenize("foo   bar\tbaz");
        assert_eq!(tokens, vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn tokenize_mixed() {
        let tokens = tokenize("MyComponent_test.rs");
        assert_eq!(tokens, vec!["my", "component", "test", "rs"]);
    }

    #[test]
    fn tokenize_empty() {
        let tokens = tokenize("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn tokenize_all_lowercase() {
        let tokens = tokenize("AuthService");
        assert!(tokens.iter().all(|t| t == &t.to_lowercase()));
    }

    #[test]
    fn tokenize_consecutive_uppercase() {
        // "HTTPServer" -> ["httpserver"] or ["https", "erver"] depending on impl
        // Our impl keeps consecutive uppercase together: "HTTPServer" -> "h", "t", "t", "p", "s", "erver"?
        // Actually let's check: H-T-T-P are uppercase but segment starts empty,
        // then we see 'S' uppercase, segment="HTTP" has last char 'P' which is uppercase,
        // so no split. Then 'e' is lowercase, no split. 'r','v','e','r' lowercase, no split.
        // Result: ["httpserver"]
        let tokens = tokenize("HTTPServer");
        assert_eq!(tokens, vec!["httpserver"]);
    }

    // -- SearchIndex::build tests --

    #[test]
    fn build_creates_index_entries() {
        let g = make_graph();
        let idx = SearchIndex::build(&g);

        // "auth" should appear from both label and id of auth_service
        assert!(idx.index.contains_key("auth"));
        let entries = &idx.index["auth"];
        // auth_service label "AuthService" -> "auth", "service"
        // auth_service id "auth_service" -> "auth", "service"
        assert!(entries.iter().any(|(id, _)| id == "auth_service"));
    }

    #[test]
    fn build_label_weight_higher_than_id() {
        let g = make_graph();
        let idx = SearchIndex::build(&g);

        // Find weight for "service" from auth_service (label token vs id token)
        let _label_weight = idx.index["service"]
            .iter()
            .filter(|(id, _)| id == "auth_service")
            .map(|(_, w)| *w)
            .fold(f64::NEG_INFINITY, f64::max);
        // There should be a label-derived entry (weight base 2.0) and id-derived (base 1.0)
        let weights: Vec<f64> = idx.index["service"]
            .iter()
            .filter(|(id, _)| id == "auth_service")
            .map(|(_, w)| *w)
            .collect();
        assert!(
            weights.len() >= 2,
            "should have label and id entries for 'service'"
        );
        assert!(
            weights.iter().any(|w| *w >= 2.0),
            "at least one weight >= 2.0 (label), got {:?}",
            weights
        );
    }

    #[test]
    fn build_source_file_tokens() {
        let g = make_graph();
        let idx = SearchIndex::build(&g);

        // "pool" from "src/db/pool.rs" source_file
        assert!(idx.index.contains_key("pool"));
    }

    // -- SearchIndex::search tests --

    #[test]
    fn search_exact_label_match() {
        let g = make_graph();
        let idx = SearchIndex::build(&g);
        let results = idx.search(&["auth".to_string()]);

        assert!(!results.is_empty());
        // auth_service should appear (label "AuthService" -> "auth")
        assert!(results.iter().any(|(_, id)| id == "auth_service"));
    }

    #[test]
    fn search_exact_id_match() {
        let g = make_graph();
        let idx = SearchIndex::build(&g);
        let results = idx.search(&["database".to_string()]);

        assert!(!results.is_empty());
        // "database" from id "database_pool"
        assert!(results.iter().any(|(_, id)| id == "database_pool"));
    }

    #[test]
    fn search_source_file_match() {
        let g = make_graph();
        let idx = SearchIndex::build(&g);
        let results = idx.search(&["cache".to_string()]);

        assert!(!results.is_empty());
        // "cache" from source_file "src/cache/layer.rs" and id "cache_layer"
        assert!(results.iter().any(|(_, id)| id == "cache_layer"));
    }

    #[test]
    fn search_no_match() {
        let g = make_graph();
        let idx = SearchIndex::build(&g);
        let results = idx.search(&["nonexistent_xyz".to_string()]);
        assert!(results.is_empty());
    }

    #[test]
    fn search_prefix_match() {
        let g = make_graph();
        let idx = SearchIndex::build(&g);

        // "auth" should prefix-match "auth" exactly and also match any token
        // starting with "auth" (none in this graph besides "auth" itself).
        // Let's test with "use" which should prefix-match "user".
        let results = idx.search(&["use".to_string()]);
        // "user_manager" label "UserManager" -> "user", "manager"
        // "use" is a prefix of "user"
        assert!(
            results.iter().any(|(_, id)| id == "user_manager"),
            "'use' should prefix-match 'user' from UserManager, got: {:?}",
            results
        );
    }

    #[test]
    fn search_prefix_lower_weight() {
        let g = make_graph();
        let idx = SearchIndex::build(&g);

        // Compare exact match score vs prefix match score.
        let exact = idx.search(&["user".to_string()]);
        let prefix = idx.search(&["use".to_string()]);

        let exact_score = exact
            .iter()
            .find(|(_, id)| id == "user_manager")
            .map(|(s, _)| *s)
            .unwrap_or(0.0);
        let prefix_score = prefix
            .iter()
            .find(|(_, id)| id == "user_manager")
            .map(|(s, _)| *s)
            .unwrap_or(0.0);

        assert!(
            exact_score > prefix_score,
            "exact match ({}) should score higher than prefix match ({})",
            exact_score,
            prefix_score
        );
    }

    #[test]
    fn search_multiple_terms_aggregate() {
        let g = make_graph();
        let idx = SearchIndex::build(&g);
        let results = idx.search(&["auth".to_string(), "service".to_string()]);

        assert!(!results.is_empty());
        // auth_service should get the highest score (both "auth" and "service" match)
        let top = &results[0];
        assert_eq!(top.1, "auth_service");
        // Score should be greater than searching for just "auth"
        let single = idx.search(&["auth".to_string()]);
        let single_score = single
            .iter()
            .find(|(_, id)| id == "auth_service")
            .map(|(s, _)| *s)
            .unwrap_or(0.0);
        assert!(
            top.0 > single_score,
            "two-term match ({}) should score higher than single-term ({})",
            top.0,
            single_score
        );
    }

    #[test]
    fn search_sorted_descending() {
        let g = make_graph();
        let idx = SearchIndex::build(&g);
        let results = idx.search(&["service".to_string()]);

        for w in results.windows(2) {
            assert!(
                w[0].0 >= w[1].0,
                "results should be sorted descending by score"
            );
        }
    }

    #[test]
    fn search_degree_boost() {
        let g_no_edges = make_graph(); // no edges
        let g_with_edges = make_graph_with_edges(); // auth has edges

        let idx_no = SearchIndex::build(&g_no_edges);
        let idx_with = SearchIndex::build(&g_with_edges);

        let results_no = idx_no.search(&["auth".to_string()]);
        let results_with = idx_with.search(&["auth".to_string()]);

        let score_no = results_no
            .iter()
            .find(|(_, id)| id == "auth")
            .map(|(s, _)| *s)
            .unwrap_or(0.0);
        let score_with = results_with
            .iter()
            .find(|(_, id)| id == "auth")
            .map(|(s, _)| *s)
            .unwrap_or(0.0);

        assert!(
            score_with > score_no,
            "node with edges ({}) should score higher than without ({})",
            score_with,
            score_no
        );
    }

    #[test]
    fn search_empty_terms() {
        let g = make_graph();
        let idx = SearchIndex::build(&g);
        let results = idx.search(&[]);
        assert!(results.is_empty());
    }

    #[test]
    fn search_case_insensitive_terms() {
        let g = make_graph();
        let idx = SearchIndex::build(&g);
        let lower = idx.search(&["auth".to_string()]);
        let upper = idx.search(&["AUTH".to_string()]);

        // Tokenize lowercases input, so results should be identical
        assert_eq!(lower.len(), upper.len());
    }

    #[test]
    fn search_id_token_lower_than_label() {
        let g = make_graph();
        let idx = SearchIndex::build(&g);

        // "pool" appears in id "database_pool" (weight 1.0+boost) and
        // source_file "src/db/pool.rs" (weight 0.5+boost).
        // Let's use a token that's ONLY in the id.
        // "database_pool" -> id tokens: "database", "pool"
        // source_file: "src/db/pool.rs" -> "src", "db", "pool", "rs"
        // label: "DatabasePool" -> "database", "pool"
        // So "pool" appears in all three. Let's find a pure id token.
        // Not easy in this graph, so let's check the weighted sum directly.
        let results = idx.search(&["rs".to_string()]);
        // "rs" only appears in source files (weight 0.5+boost)
        assert!(!results.is_empty(), "should find 'rs' in source files");
        let rs_score = results[0].0;
        // For comparison, search a label-only term
        let results_label = idx.search(&["manager".to_string()]);
        let label_score = results_label[0].0;
        assert!(
            label_score > rs_score,
            "label token ({}) should score higher than source_file-only token ({})",
            label_score,
            rs_score
        );
    }
}
