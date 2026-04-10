//! MCP server for graph queries.
//!
//! Provides graph traversal and scoring functions used by the query
//! engine and MCP protocol server. Port of Python query tools.

pub mod mcp;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use graphify_core::graph::KnowledgeGraph;
use serde_json::Value;
use thiserror::Error;

/// Errors from the server.
#[derive(Debug, Error)]
pub enum ServeError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("graph load error: {0}")]
    GraphLoad(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Score nodes by relevance to search terms.
///
/// Returns `(score, node_id)` pairs sorted by descending score.
/// Scoring: +2.0 for exact label match, +1.0 for label contains,
/// +0.5 for id contains, plus a small degree-based boost.
pub fn score_nodes(graph: &KnowledgeGraph, terms: &[String]) -> Vec<(f64, String)> {
    let lower_terms: Vec<String> = terms.iter().map(|t| t.to_lowercase()).collect();

    let mut scored = Vec::new();
    for node_id in graph.node_ids() {
        if let Some(node) = graph.get_node(&node_id) {
            let label_lower = node.label.to_lowercase();
            let id_lower = node.id.to_lowercase();

            let mut score: f64 = 0.0;

            for term in &lower_terms {
                // Exact match in label
                if label_lower == *term {
                    score += 2.0;
                } else if label_lower.contains(term.as_str()) {
                    score += 1.0;
                }

                // Match in node ID
                if id_lower.contains(term.as_str()) {
                    score += 0.5;
                }
            }

            if score > 0.0 {
                // Boost by degree (well-connected nodes are more relevant)
                let degree_boost = (graph.degree(&node_id) as f64).ln_1p() * 0.1;
                score += degree_boost;
                scored.push((score, node_id.clone()));
            }
        }
    }

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
}

/// BFS traversal from start nodes up to a maximum depth.
///
/// Returns `(visited_nodes, edges_traversed)` where edges are `(source, target)` pairs.
pub fn bfs(
    graph: &KnowledgeGraph,
    start: &[String],
    depth: usize,
) -> (Vec<String>, Vec<(String, String)>) {
    let mut visited: HashSet<String> = HashSet::new();
    let mut edges: Vec<(String, String)> = Vec::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();

    for s in start {
        if graph.get_node(s).is_some() {
            visited.insert(s.clone());
            queue.push_back((s.clone(), 0));
        }
    }

    while let Some((current, current_depth)) = queue.pop_front() {
        if current_depth >= depth {
            continue;
        }

        for neighbor_id in graph.neighbor_ids(&current) {
            edges.push((current.clone(), neighbor_id.clone()));

            if !visited.contains(&neighbor_id) {
                visited.insert(neighbor_id.clone());
                queue.push_back((neighbor_id, current_depth + 1));
            }
        }
    }

    let visited_vec: Vec<String> = visited.into_iter().collect();
    (visited_vec, edges)
}

/// DFS traversal from start nodes up to a maximum depth.
///
/// Returns `(visited_nodes, edges_traversed)` where edges are `(source, target)` pairs.
pub fn dfs(
    graph: &KnowledgeGraph,
    start: &[String],
    depth: usize,
) -> (Vec<String>, Vec<(String, String)>) {
    let mut visited: HashSet<String> = HashSet::new();
    let mut edges: Vec<(String, String)> = Vec::new();
    let mut stack: Vec<(String, usize)> = Vec::new();

    for s in start {
        if graph.get_node(s).is_some() {
            visited.insert(s.clone());
            stack.push((s.clone(), 0));
        }
    }

    while let Some((current, current_depth)) = stack.pop() {
        if current_depth >= depth {
            continue;
        }

        for neighbor_id in graph.neighbor_ids(&current) {
            edges.push((current.clone(), neighbor_id.clone()));

            if !visited.contains(&neighbor_id) {
                visited.insert(neighbor_id.clone());
                stack.push((neighbor_id, current_depth + 1));
            }
        }
    }

    let visited_vec: Vec<String> = visited.into_iter().collect();
    (visited_vec, edges)
}

/// Convert a subgraph (set of nodes and edges) to a text representation
/// suitable for LLM context windows.
///
/// Respects a `token_budget` (approximate: 1 token ≈ 4 chars).
pub fn subgraph_to_text(
    graph: &KnowledgeGraph,
    nodes: &[String],
    edges: &[(String, String)],
    token_budget: usize,
) -> String {
    let char_budget = token_budget * 4;
    let mut output = String::with_capacity(char_budget.min(64 * 1024));

    // Header
    output.push_str(&format!(
        "=== Knowledge Graph Context ({} nodes, {} edges) ===\n\n",
        nodes.len(),
        edges.len()
    ));

    // Nodes section
    output.push_str("## Nodes\n\n");
    for node_id in nodes {
        if output.len() >= char_budget {
            output.push_str("\n... (truncated due to token budget)\n");
            break;
        }

        if let Some(node) = graph.get_node(node_id) {
            output.push_str(&format!(
                "- **{}** [{}] (type: {:?}",
                node.label, node.id, node.node_type
            ));
            if let Some(community) = node.community {
                output.push_str(&format!(", community: {}", community));
            }
            output.push_str(&format!(", file: {})\n", node.source_file));
        }
    }

    // Edges section
    if output.len() < char_budget {
        output.push_str("\n## Relationships\n\n");

        // Deduplicate edges for display
        let mut seen: HashSet<(&str, &str)> = HashSet::new();
        for (src, tgt) in edges {
            if output.len() >= char_budget {
                output.push_str("\n... (truncated due to token budget)\n");
                break;
            }

            if seen.insert((src.as_str(), tgt.as_str())) {
                let src_label = graph.get_node(src).map(|n| n.label.as_str()).unwrap_or(src);
                let tgt_label = graph.get_node(tgt).map(|n| n.label.as_str()).unwrap_or(tgt);
                output.push_str(&format!("- {} -> {}\n", src_label, tgt_label));
            }
        }
    }

    output
}

/// Load a knowledge graph from a JSON file.
pub fn load_graph(graph_path: &Path) -> Result<KnowledgeGraph, ServeError> {
    let content = std::fs::read_to_string(graph_path)?;
    let value: Value = serde_json::from_str(&content)?;
    KnowledgeGraph::from_node_link_json(&value).map_err(|e| ServeError::GraphLoad(e.to_string()))
}

/// Get basic statistics about the graph.
pub fn graph_stats(graph: &KnowledgeGraph) -> HashMap<String, Value> {
    let mut stats = HashMap::new();
    stats.insert("node_count".to_string(), Value::from(graph.node_count()));
    stats.insert("edge_count".to_string(), Value::from(graph.edge_count()));
    stats.insert(
        "community_count".to_string(),
        Value::from(graph.communities.len()),
    );

    // Degree statistics
    let node_ids = graph.node_ids();
    if !node_ids.is_empty() {
        let degrees: Vec<usize> = node_ids.iter().map(|id| graph.degree(id)).collect();
        let max_degree = degrees.iter().copied().max().unwrap_or(0);
        let avg_degree = degrees.iter().sum::<usize>() as f64 / degrees.len() as f64;
        stats.insert("max_degree".to_string(), Value::from(max_degree));
        stats.insert(
            "avg_degree".to_string(),
            Value::from(format!("{:.2}", avg_degree)),
        );
    }

    stats
}

/// Start the MCP server over stdio (JSON-RPC 2.0).
///
/// Reads requests from stdin, writes responses to stdout.
/// This is the entry point called by the CLI `serve` command.
pub async fn start_server(graph_path: &Path) -> Result<(), ServeError> {
    // Run the synchronous stdio loop; use spawn_blocking so we don't
    // block the tokio runtime (though for stdio this is fine).
    let path = graph_path.to_path_buf();
    tokio::task::spawn_blocking(move || mcp::run_mcp_server(&path))
        .await
        .map_err(|e| ServeError::Io(std::io::Error::other(e)))??;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use graphify_core::confidence::Confidence;
    use graphify_core::model::{GraphEdge, GraphNode, NodeType};

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

    fn make_test_graph() -> KnowledgeGraph {
        let mut g = KnowledgeGraph::new();
        g.add_node(make_node("auth", "AuthService")).unwrap();
        g.add_node(make_node("user", "UserManager")).unwrap();
        g.add_node(make_node("db", "Database")).unwrap();
        g.add_node(make_node("cache", "CacheLayer")).unwrap();
        g.add_edge(make_edge("auth", "user")).unwrap();
        g.add_edge(make_edge("auth", "db")).unwrap();
        g.add_edge(make_edge("user", "db")).unwrap();
        g.add_edge(make_edge("user", "cache")).unwrap();
        g
    }

    #[test]
    fn test_score_nodes_basic() {
        let g = make_test_graph();
        let results = score_nodes(&g, &["auth".to_string()]);
        assert!(!results.is_empty());
        // "auth" node should score highest
        let top_id = &results[0].1;
        assert_eq!(top_id, "auth");
    }

    #[test]
    fn test_score_nodes_no_match() {
        let g = make_test_graph();
        let results = score_nodes(&g, &["nonexistent".to_string()]);
        assert!(results.is_empty());
    }

    #[test]
    fn test_score_nodes_multiple_terms() {
        let g = make_test_graph();
        let results = score_nodes(&g, &["user".to_string(), "manager".to_string()]);
        assert!(!results.is_empty());
        assert!(results.iter().any(|(_, id)| id == "user"));
    }

    #[test]
    fn test_bfs_depth_0() {
        let g = make_test_graph();
        let (nodes, edges) = bfs(&g, &["auth".to_string()], 0);
        assert_eq!(nodes.len(), 1);
        assert!(edges.is_empty());
    }

    #[test]
    fn test_bfs_depth_1() {
        let g = make_test_graph();
        let (nodes, edges) = bfs(&g, &["auth".to_string()], 1);
        // auth -> user, auth -> db
        assert!(nodes.len() >= 3); // auth, user, db
        assert!(!edges.is_empty());
    }

    #[test]
    fn test_bfs_depth_2() {
        let g = make_test_graph();
        let (nodes, _edges) = bfs(&g, &["auth".to_string()], 2);
        // Should reach all 4 nodes
        assert_eq!(nodes.len(), 4);
    }

    #[test]
    fn test_dfs_depth_1() {
        let g = make_test_graph();
        let (nodes, edges) = dfs(&g, &["auth".to_string()], 1);
        assert!(nodes.len() >= 3);
        assert!(!edges.is_empty());
    }

    #[test]
    fn test_bfs_nonexistent_start() {
        let g = make_test_graph();
        let (nodes, edges) = bfs(&g, &["nonexistent".to_string()], 3);
        assert!(nodes.is_empty());
        assert!(edges.is_empty());
    }

    #[test]
    fn test_subgraph_to_text() {
        let g = make_test_graph();
        let nodes = vec!["auth".to_string(), "user".to_string()];
        let edges = vec![("auth".to_string(), "user".to_string())];
        let text = subgraph_to_text(&g, &nodes, &edges, 1000);

        assert!(text.contains("Knowledge Graph Context"));
        assert!(text.contains("AuthService"));
        assert!(text.contains("UserManager"));
        assert!(text.contains("Relationships"));
    }

    #[test]
    fn test_subgraph_to_text_budget() {
        let g = make_test_graph();
        let nodes: Vec<String> = g.node_ids();
        let edges = vec![
            ("auth".to_string(), "user".to_string()),
            ("auth".to_string(), "db".to_string()),
        ];
        // Very small budget
        let text = subgraph_to_text(&g, &nodes, &edges, 10);
        assert!(text.contains("truncated") || text.len() < 200);
    }

    #[test]
    fn test_graph_stats() {
        let g = make_test_graph();
        let stats = graph_stats(&g);
        assert_eq!(stats["node_count"], 4);
        assert_eq!(stats["edge_count"], 4);
    }

    #[test]
    fn test_bfs_multiple_starts() {
        let g = make_test_graph();
        let (nodes, _) = bfs(&g, &["auth".to_string(), "cache".to_string()], 1);
        assert!(nodes.len() >= 4);
    }
}
