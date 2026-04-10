//! Token efficiency benchmarking for graphify.
//!
//! Measures graph quality, compression ratio, and query performance to
//! validate that the graph representation is efficient for LLM consumption.
//! Port of Python `benchmark.py`.

use std::collections::HashSet;
use std::path::Path;

use graphify_core::graph::KnowledgeGraph;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;

/// Errors from the benchmark runner.
#[derive(Debug, Error)]
pub enum BenchmarkError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("graph load error: {0}")]
    GraphLoad(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Benchmark result metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub graph_nodes: usize,
    pub graph_edges: usize,
    pub graph_tokens: usize,
    pub corpus_words: Option<usize>,
    pub corpus_tokens: Option<usize>,
    pub compression_ratio: Option<f64>,
    pub community_count: usize,
    pub sample_queries: Vec<QuerySample>,
}

/// A single sample query benchmark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuerySample {
    pub question: String,
    pub context_tokens: usize,
    pub full_corpus_tokens: usize,
    pub reduction: f64,
}

/// Sample questions used for benchmarking query efficiency.
const SAMPLE_QUESTIONS: &[&str] = &[
    "What are the main components?",
    "How does authentication work?",
    "What are the key abstractions?",
    "How do components communicate?",
    "What are the entry points?",
];

/// Estimate the number of tokens in a string.
///
/// Uses the approximation: 1 token ≈ 4 characters.
fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

/// Estimate tokens from word count.
fn tokens_from_words(words: usize) -> usize {
    ((words as f64) * 1.3).ceil() as usize
}

/// Simulate a query against the graph and estimate context tokens needed.
///
/// For each query, we find matching nodes and gather their neighborhood,
/// then measure how many tokens the resulting context would consume.
fn simulate_query(graph: &KnowledgeGraph, question: &str) -> usize {
    let terms: Vec<String> = question
        .to_lowercase()
        .split_whitespace()
        .filter(|w| w.len() > 3) // skip short words
        .map(String::from)
        .collect();

    // Score nodes by term overlap
    let mut matched_nodes: Vec<(f64, String)> = Vec::new();
    for node_id in graph.node_ids() {
        if let Some(node) = graph.get_node(&node_id) {
            let label_lower = node.label.to_lowercase();
            let score: f64 = terms
                .iter()
                .map(|t| {
                    if label_lower.contains(t.as_str()) {
                        1.0
                    } else {
                        0.0
                    }
                })
                .sum();
            if score > 0.0 {
                matched_nodes.push((score, node_id));
            }
        }
    }

    matched_nodes.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // Take top-5 matches and their 1-hop neighbors
    let top_nodes: Vec<String> = matched_nodes
        .into_iter()
        .take(5)
        .map(|(_, id)| id)
        .collect();

    let mut context_parts: Vec<String> = Vec::new();
    let mut seen = HashSet::new();

    for node_id in &top_nodes {
        if seen.insert(node_id.clone()) {
            if let Some(node) = graph.get_node(node_id) {
                context_parts.push(format!(
                    "{} [{}] (type: {:?}, file: {})",
                    node.label, node.id, node.node_type, node.source_file
                ));
            }
        }

        // Add 1-hop neighbors
        for neighbor in graph.neighbor_ids(node_id) {
            if seen.insert(neighbor.clone()) {
                if let Some(node) = graph.get_node(&neighbor) {
                    context_parts.push(format!(
                        "  -> {} [{}] (type: {:?})",
                        node.label, node.id, node.node_type
                    ));
                }
            }
        }
    }

    // If no matches, estimate a minimum context
    if context_parts.is_empty() {
        let json = graph.to_node_link_json();
        let total = estimate_tokens(&json.to_string());
        return total / 5; // ~20% of graph
    }

    let context_text = context_parts.join("\n");
    estimate_tokens(&context_text)
}

/// Run the benchmark suite on the graph at `graph_path`.
///
/// # Arguments
/// * `graph_path` - Path to the graph JSON file.
/// * `corpus_words` - Optional word count of the original corpus for compression ratio.
pub fn run_benchmark(
    graph_path: &Path,
    corpus_words: Option<usize>,
) -> Result<BenchmarkResult, BenchmarkError> {
    // Load graph JSON
    let content = std::fs::read_to_string(graph_path)?;
    let value: serde_json::Value = serde_json::from_str(&content)?;
    let graph = KnowledgeGraph::from_node_link_json(&value)
        .map_err(|e| BenchmarkError::GraphLoad(e.to_string()))?;

    let graph_tokens = estimate_tokens(&content);
    let corpus_tokens = corpus_words.map(tokens_from_words);

    let compression_ratio = corpus_tokens.map(|ct| {
        if graph_tokens > 0 {
            ct as f64 / graph_tokens as f64
        } else {
            0.0
        }
    });

    // Run sample queries
    let full_corpus_tokens = corpus_tokens.unwrap_or(graph_tokens);
    let sample_queries: Vec<QuerySample> = SAMPLE_QUESTIONS
        .iter()
        .map(|q| {
            let context_tokens = simulate_query(&graph, q);
            let reduction = if context_tokens > 0 {
                full_corpus_tokens as f64 / context_tokens as f64
            } else {
                0.0
            };
            QuerySample {
                question: q.to_string(),
                context_tokens,
                full_corpus_tokens,
                reduction,
            }
        })
        .collect();

    let result = BenchmarkResult {
        graph_nodes: graph.node_count(),
        graph_edges: graph.edge_count(),
        graph_tokens,
        corpus_words,
        corpus_tokens,
        compression_ratio,
        community_count: graph.communities.len(),
        sample_queries,
    };

    info!(
        "Benchmark complete: {} nodes, {} edges, {} tokens",
        result.graph_nodes, result.graph_edges, result.graph_tokens
    );

    Ok(result)
}

/// Print a human-readable benchmark report.
pub fn print_benchmark(result: &BenchmarkResult) {
    println!("=== graphify Benchmark ===");
    println!();
    println!(
        "Graph: {} nodes, {} edges, {} communities",
        result.graph_nodes, result.graph_edges, result.community_count
    );
    println!("Graph tokens: {}", result.graph_tokens);

    if let Some(words) = result.corpus_words {
        println!("Corpus words: {}", words);
    }
    if let Some(tokens) = result.corpus_tokens {
        println!("Corpus tokens (est.): {}", tokens);
    }
    if let Some(ratio) = result.compression_ratio {
        println!("Compression: {:.1}x", ratio);
    }

    println!();
    println!("Sample queries:");
    for q in &result.sample_queries {
        println!("  Q: {}", q.question);
        println!(
            "    Context: {} tokens (vs {} full) = {:.1}x reduction",
            q.context_tokens, q.full_corpus_tokens, q.reduction
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use graphify_core::confidence::Confidence;
    use graphify_core::model::{GraphEdge, GraphNode, NodeType};
    use std::collections::HashMap;

    fn make_node(id: &str, label: &str) -> GraphNode {
        GraphNode {
            id: id.into(),
            label: label.into(),
            source_file: "test.rs".into(),
            source_location: None,
            node_type: NodeType::Class,
            community: None,
            extra: HashMap::new(),
        }
    }

    fn make_edge(src: &str, tgt: &str) -> GraphEdge {
        GraphEdge {
            source: src.into(),
            target: tgt.into(),
            relation: "calls".into(),
            confidence: Confidence::Extracted,
            confidence_score: 1.0,
            source_file: "test.rs".into(),
            source_location: None,
            weight: 1.0,
            extra: HashMap::new(),
        }
    }

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("hello world"), 3); // (11+3)/4 = 3
        assert!(estimate_tokens(&"a".repeat(100)) >= 25);
    }

    #[test]
    fn test_tokens_from_words() {
        assert_eq!(tokens_from_words(100), 130);
        assert_eq!(tokens_from_words(0), 0);
        assert_eq!(tokens_from_words(1), 2); // ceil(1.3)
    }

    #[test]
    fn test_simulate_query() {
        let mut g = KnowledgeGraph::new();
        g.add_node(make_node("auth", "AuthService")).unwrap();
        g.add_node(make_node("user", "UserManager")).unwrap();
        g.add_node(make_node("db", "Database")).unwrap();
        g.add_edge(make_edge("auth", "user")).unwrap();
        g.add_edge(make_edge("auth", "db")).unwrap();

        let tokens = simulate_query(&g, "How does authentication work?");
        assert!(tokens > 0, "Query should produce some context tokens");
    }

    #[test]
    fn test_simulate_query_no_match() {
        let mut g = KnowledgeGraph::new();
        g.add_node(make_node("auth", "AuthService")).unwrap();

        let tokens = simulate_query(&g, "zzzzz qqqqq");
        assert!(
            tokens > 0,
            "Even with no matches, should return fallback tokens"
        );
    }

    #[test]
    fn test_run_benchmark_from_file() {
        let mut g = KnowledgeGraph::new();
        g.add_node(make_node("auth", "AuthService")).unwrap();
        g.add_node(make_node("user", "UserManager")).unwrap();
        g.add_node(make_node("db", "Database")).unwrap();
        g.add_edge(make_edge("auth", "user")).unwrap();
        g.add_edge(make_edge("user", "db")).unwrap();

        let json = g.to_node_link_json();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), serde_json::to_string_pretty(&json).unwrap()).unwrap();

        let result = run_benchmark(tmp.path(), Some(10000)).unwrap();
        assert_eq!(result.graph_nodes, 3);
        assert_eq!(result.graph_edges, 2);
        assert!(result.graph_tokens > 0);
        assert_eq!(result.corpus_words, Some(10000));
        assert_eq!(result.corpus_tokens, Some(13000));
        assert!(result.compression_ratio.unwrap() > 0.0);
        assert_eq!(result.sample_queries.len(), SAMPLE_QUESTIONS.len());
    }

    #[test]
    fn test_run_benchmark_no_corpus() {
        let mut g = KnowledgeGraph::new();
        g.add_node(make_node("a", "Alpha")).unwrap();
        let json = g.to_node_link_json();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), serde_json::to_string(&json).unwrap()).unwrap();

        let result = run_benchmark(tmp.path(), None).unwrap();
        assert!(result.compression_ratio.is_none());
        assert!(result.corpus_words.is_none());
    }

    #[test]
    fn test_print_benchmark_no_panic() {
        let result = BenchmarkResult {
            graph_nodes: 10,
            graph_edges: 15,
            graph_tokens: 500,
            corpus_words: Some(5000),
            corpus_tokens: Some(6500),
            compression_ratio: Some(13.0),
            community_count: 3,
            sample_queries: vec![QuerySample {
                question: "Test?".to_string(),
                context_tokens: 50,
                full_corpus_tokens: 6500,
                reduction: 130.0,
            }],
        };
        // Should not panic
        print_benchmark(&result);
    }
}
