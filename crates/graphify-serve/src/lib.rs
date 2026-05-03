//! MCP server for graph queries.
//!
//! Provides graph traversal and scoring functions used by the query
//! engine and MCP protocol server. Port of Python query tools.

pub mod http;
pub mod mcp;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::Mutex;

use graphify_core::graph::KnowledgeGraph;
use graphify_embed::{SemanticEngine, SemanticMatch, default_index_path_for_graph};
use serde_json::Value;
use thiserror::Error;
use tracing::warn;

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

/// Optional semantic query state loaded next to `graph.json`.
///
/// Model2Vec tokenizers are CPU-bound and internally immutable for encode, but
/// the concrete upstream type is wrapped in a mutex here so HTTP handlers can
/// share one loaded model safely without making public Sync assumptions.
pub struct SemanticState {
    engine: Mutex<SemanticEngine>,
}

impl SemanticState {
    pub fn load_for_graph_path(
        graph_path: &Path,
        graph: &KnowledgeGraph,
    ) -> std::result::Result<Option<Self>, ServeError> {
        let index_path = default_index_path_for_graph(graph_path);
        if !index_path.exists() {
            return Ok(None);
        }
        let engine = SemanticEngine::load_for_graph(&index_path, graph)
            .map_err(|err| ServeError::GraphLoad(format!("load semantic index: {err:#}")))?;
        Ok(Some(Self {
            engine: Mutex::new(engine),
        }))
    }

    pub fn query(
        &self,
        graph: &KnowledgeGraph,
        question: &str,
        top_n: usize,
    ) -> std::result::Result<Vec<SemanticMatch>, ServeError> {
        let engine = self
            .engine
            .lock()
            .map_err(|_| ServeError::GraphLoad("semantic engine lock poisoned".into()))?;
        engine
            .query(graph, question, top_n)
            .map_err(|err| ServeError::GraphLoad(format!("semantic query: {err:#}")))
    }

    pub fn description(&self) -> String {
        match self.engine.lock() {
            Ok(engine) => format!("{} nodes via {}", engine.node_count(), engine.model()),
            Err(_) => "semantic engine unavailable".to_string(),
        }
    }
}

pub fn load_semantic_state(graph_path: &Path, graph: &KnowledgeGraph) -> Option<SemanticState> {
    match SemanticState::load_for_graph_path(graph_path, graph) {
        Ok(state) => state,
        Err(err) => {
            warn!("{err}");
            None
        }
    }
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

// ---------------------------------------------------------------------------
// Smart graph summarization
// ---------------------------------------------------------------------------

/// Abstraction level for graph summaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryLevel {
    /// Full detail: all nodes and edges within budget.
    Detailed,
    /// Community representatives + cross-community edges.
    Community,
    /// Directory-level super-nodes with aggregated dependencies.
    Architecture,
}

/// Generate a multi-level graph summary within a token budget.
///
/// - `Detailed`: Equivalent to `subgraph_to_text` on the full graph.
/// - `Community`: One representative node per community (highest degree) + cross-community edges.
/// - `Architecture`: Groups nodes by directory into super-nodes, merges edges.
pub fn smart_summary(
    graph: &KnowledgeGraph,
    communities: &HashMap<usize, Vec<String>>,
    level: SummaryLevel,
    token_budget: usize,
) -> String {
    match level {
        SummaryLevel::Detailed => {
            let all_nodes = graph.node_ids();
            let all_edges: Vec<(String, String)> = graph
                .edges_with_endpoints()
                .iter()
                .map(|(s, t, _)| (s.to_string(), t.to_string()))
                .collect();
            subgraph_to_text(graph, &all_nodes, &all_edges, token_budget)
        }
        SummaryLevel::Community => community_level_summary(graph, communities, token_budget),
        SummaryLevel::Architecture => architecture_level_summary(graph, token_budget),
    }
}

/// Community-level summary: one representative per community + cross-community edges.
fn community_level_summary(
    graph: &KnowledgeGraph,
    communities: &HashMap<usize, Vec<String>>,
    token_budget: usize,
) -> String {
    let char_budget = token_budget * 4;
    let mut output = String::with_capacity(char_budget.min(64 * 1024));

    // Pick representative (highest degree) for each community
    let mut representatives: HashMap<usize, (&str, usize)> = HashMap::new();
    for (&cid, members) in communities {
        let best = members
            .iter()
            .map(|id| (id.as_str(), graph.degree(id)))
            .max_by_key(|(_, d)| *d)
            .unwrap_or(("", 0));
        representatives.insert(cid, best);
    }

    output.push_str(&format!(
        "=== Architecture Summary ({} communities, {} total nodes) ===\n\n",
        communities.len(),
        graph.node_count()
    ));

    // Node→community reverse map
    let mut node_cid: HashMap<&str, usize> = HashMap::new();
    for (&cid, members) in communities {
        for m in members {
            node_cid.insert(m.as_str(), cid);
        }
    }

    // Communities
    output.push_str("## Communities\n\n");
    let mut sorted_cids: Vec<usize> = communities.keys().copied().collect();
    sorted_cids.sort();
    for cid in &sorted_cids {
        if output.len() >= char_budget {
            output.push_str("\n... (truncated)\n");
            break;
        }
        let members = &communities[cid];
        let (rep_id, rep_deg) = representatives[cid];
        let rep_label = graph
            .get_node(rep_id)
            .map(|n| n.label.as_str())
            .unwrap_or(rep_id);
        output.push_str(&format!(
            "- Community {} ({} nodes): representative **{}** (degree {})\n",
            cid,
            members.len(),
            rep_label,
            rep_deg
        ));
    }

    // Cross-community edges
    output.push_str("\n## Cross-Community Dependencies\n\n");
    let mut cross_edges: HashMap<(usize, usize), usize> = HashMap::new();
    for (src, tgt, _) in graph.edges_with_endpoints() {
        let sc = node_cid.get(src).copied().unwrap_or(usize::MAX);
        let tc = node_cid.get(tgt).copied().unwrap_or(usize::MAX);
        if sc != tc && sc != usize::MAX && tc != usize::MAX {
            let key = if sc < tc { (sc, tc) } else { (tc, sc) };
            *cross_edges.entry(key).or_default() += 1;
        }
    }
    let mut sorted_cross: Vec<_> = cross_edges.into_iter().collect();
    sorted_cross.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    for ((c1, c2), count) in sorted_cross.iter().take(20) {
        if output.len() >= char_budget {
            break;
        }
        output.push_str(&format!(
            "- Community {} ↔ Community {}: {} edges\n",
            c1, c2, count
        ));
    }

    output
}

/// Architecture-level summary: group by directory, aggregate edges.
fn architecture_level_summary(graph: &KnowledgeGraph, token_budget: usize) -> String {
    let char_budget = token_budget * 4;
    let mut output = String::with_capacity(char_budget.min(64 * 1024));

    // Group nodes by directory
    let mut dir_nodes: HashMap<String, Vec<&str>> = HashMap::new();
    for node in graph.nodes() {
        let dir = std::path::Path::new(&node.source_file)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or(".")
            .to_string();
        dir_nodes.entry(dir).or_default().push(&node.id);
    }

    // Node→dir mapping
    let mut node_dir: HashMap<&str, &str> = HashMap::new();
    for (dir, nodes) in &dir_nodes {
        for &nid in nodes {
            node_dir.insert(nid, dir.as_str());
        }
    }

    output.push_str(&format!(
        "=== Architecture Overview ({} packages/directories) ===\n\n",
        dir_nodes.len()
    ));

    // Directory summaries
    output.push_str("## Packages\n\n");
    let mut sorted_dirs: Vec<_> = dir_nodes.iter().collect();
    sorted_dirs.sort_by_key(|(_, nodes)| std::cmp::Reverse(nodes.len()));
    for (dir, nodes) in sorted_dirs.iter().take(30) {
        if output.len() >= char_budget {
            output.push_str("\n... (truncated)\n");
            break;
        }
        output.push_str(&format!("- **{}** ({} entities)\n", dir, nodes.len()));
    }

    // Inter-directory dependencies
    output.push_str("\n## Dependencies\n\n");
    let mut dir_edges: HashMap<(&str, &str), usize> = HashMap::new();
    for (src, tgt, _) in graph.edges_with_endpoints() {
        let sd = node_dir.get(src).copied().unwrap_or("?");
        let td = node_dir.get(tgt).copied().unwrap_or("?");
        if sd != td {
            let key = if sd < td { (sd, td) } else { (td, sd) };
            *dir_edges.entry(key).or_default() += 1;
        }
    }
    let mut sorted_deps: Vec<_> = dir_edges.into_iter().collect();
    sorted_deps.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    for ((d1, d2), count) in sorted_deps.iter().take(20) {
        if output.len() >= char_budget {
            break;
        }
        output.push_str(&format!("- {} → {}: {} edges\n", d1, d2, count));
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
    let community_count = if graph.communities.is_empty() {
        graph
            .nodes()
            .iter()
            .filter_map(|node| node.community)
            .collect::<HashSet<_>>()
            .len()
    } else {
        graph.communities.len()
    };
    stats.insert("community_count".to_string(), Value::from(community_count));

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

pub use http::{HttpServerConfig, start_http_server};

// ---------------------------------------------------------------------------
// Advanced graph algorithms
// ---------------------------------------------------------------------------

/// Find all simple paths between `source` and `target` up to `max_length` edges.
///
/// Returns a vec of paths, each path being a vec of node IDs.
/// Limits to at most 50 paths to prevent combinatorial explosion.
pub fn all_simple_paths(
    graph: &KnowledgeGraph,
    source: &str,
    target: &str,
    max_length: usize,
) -> Vec<Vec<String>> {
    const MAX_PATHS: usize = 50;
    let mut result: Vec<Vec<String>> = Vec::new();
    let mut stack: Vec<(String, Vec<String>)> = Vec::new();

    if graph.get_node(source).is_none() || graph.get_node(target).is_none() {
        return result;
    }

    stack.push((source.to_string(), vec![source.to_string()]));

    while let Some((current, path)) = stack.pop() {
        if result.len() >= MAX_PATHS {
            break;
        }
        if current == target && path.len() > 1 {
            result.push(path);
            continue;
        }
        if path.len() > max_length + 1 {
            continue;
        }

        for neighbor_id in graph.neighbor_ids(&current) {
            if !path.contains(&neighbor_id) {
                let mut new_path = path.clone();
                new_path.push(neighbor_id.clone());
                stack.push((neighbor_id, new_path));
            }
        }
    }

    result.sort_by_key(|p| p.len());
    result
}

/// Edge detail in a weighted path: (from_id, to_id, cost, relation).
pub type EdgeDetail = (String, String, f64, String);

/// Dijkstra shortest path using edge weights.
///
/// Cost = 1.0 / edge.weight (higher weight = shorter distance).
/// Optionally filters edges below `min_confidence` score.
/// Returns `(path, total_cost, edge_details)` or None if no path exists.
pub fn dijkstra_path(
    graph: &KnowledgeGraph,
    source: &str,
    target: &str,
    min_confidence: f64,
) -> Option<(Vec<String>, f64, Vec<EdgeDetail>)> {
    use std::cmp::Ordering;
    use std::collections::BinaryHeap;

    if graph.get_node(source).is_none() || graph.get_node(target).is_none() {
        return None;
    }
    if source == target {
        return Some((vec![source.to_string()], 0.0, Vec::new()));
    }

    // Build adjacency with weights from edges
    let mut adj: HashMap<String, Vec<(String, f64, String)>> = HashMap::new();
    for (src, tgt, edge) in graph.edges_with_endpoints() {
        if edge.confidence_score < min_confidence {
            continue;
        }
        let cost = if edge.weight > 0.0 {
            1.0 / edge.weight
        } else {
            f64::MAX
        };
        adj.entry(src.to_string()).or_default().push((
            tgt.to_string(),
            cost,
            edge.relation.clone(),
        ));
        adj.entry(tgt.to_string()).or_default().push((
            src.to_string(),
            cost,
            edge.relation.clone(),
        ));
    }

    #[derive(PartialEq)]
    struct State {
        cost: f64,
        node: String,
    }
    impl Eq for State {}
    impl PartialOrd for State {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for State {
        fn cmp(&self, other: &Self) -> Ordering {
            other
                .cost
                .partial_cmp(&self.cost)
                .unwrap_or(Ordering::Equal)
        }
    }

    let mut dist: HashMap<String, f64> = HashMap::new();
    let mut prev: HashMap<String, (String, f64, String)> = HashMap::new();
    let mut heap = BinaryHeap::new();

    dist.insert(source.to_string(), 0.0);
    heap.push(State {
        cost: 0.0,
        node: source.to_string(),
    });

    while let Some(State { cost, node }) = heap.pop() {
        if node == target {
            break;
        }
        if cost > *dist.get(&node).unwrap_or(&f64::MAX) {
            continue;
        }
        if let Some(neighbors) = adj.get(&node) {
            for (next, edge_cost, relation) in neighbors {
                let new_cost = cost + edge_cost;
                if new_cost < *dist.get(next).unwrap_or(&f64::MAX) {
                    dist.insert(next.clone(), new_cost);
                    prev.insert(next.clone(), (node.clone(), *edge_cost, relation.clone()));
                    heap.push(State {
                        cost: new_cost,
                        node: next.clone(),
                    });
                }
            }
        }
    }

    // Reconstruct path
    if !prev.contains_key(target) {
        return None;
    }

    let mut path = vec![target.to_string()];
    let mut edge_details: Vec<(String, String, f64, String)> = Vec::new();
    let mut current = target.to_string();
    while let Some((from, cost, relation)) = prev.get(&current) {
        edge_details.push((from.clone(), current.clone(), *cost, relation.clone()));
        path.push(from.clone());
        current = from.clone();
    }
    path.reverse();
    edge_details.reverse();

    let total_cost = *dist.get(target).unwrap_or(&f64::MAX);
    Some((path, total_cost, edge_details))
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
    fn test_graph_stats_counts_node_communities_when_top_level_metadata_is_absent() {
        let mut g = KnowledgeGraph::new();
        let mut auth = make_node("auth", "AuthService");
        auth.community = Some(10);
        let mut db = make_node("db", "Database");
        db.community = Some(20);
        let mut cache = make_node("cache", "CacheLayer");
        cache.community = Some(20);

        g.add_node(auth).unwrap();
        g.add_node(db).unwrap();
        g.add_node(cache).unwrap();

        let stats = graph_stats(&g);
        assert_eq!(stats["community_count"], 2);
    }

    #[test]
    fn test_bfs_multiple_starts() {
        let g = make_test_graph();
        let (nodes, _) = bfs(&g, &["auth".to_string(), "cache".to_string()], 1);
        assert!(nodes.len() >= 4);
    }

    // -- all_simple_paths tests --

    #[test]
    fn test_all_simple_paths_direct() {
        let g = make_test_graph();
        let paths = all_simple_paths(&g, "auth", "user", 4);
        assert!(!paths.is_empty());
        // Direct edge exists: auth → user
        assert!(paths.iter().any(|p| p.len() == 2));
    }

    #[test]
    fn test_all_simple_paths_indirect() {
        let g = make_test_graph();
        // auth → db has direct path (len 2) and indirect auth → user → db (len 3)
        let paths = all_simple_paths(&g, "auth", "db", 4);
        assert!(
            paths.len() >= 2,
            "should find multiple paths, got {}",
            paths.len()
        );
    }

    #[test]
    fn test_all_simple_paths_no_path() {
        let mut g = KnowledgeGraph::new();
        g.add_node(make_node("a", "A")).unwrap();
        g.add_node(make_node("b", "B")).unwrap();
        // No edge between a and b
        let paths = all_simple_paths(&g, "a", "b", 4);
        assert!(paths.is_empty());
    }

    #[test]
    fn test_all_simple_paths_nonexistent_node() {
        let g = make_test_graph();
        let paths = all_simple_paths(&g, "auth", "nonexistent", 4);
        assert!(paths.is_empty());
    }

    #[test]
    fn test_all_simple_paths_sorted_by_length() {
        let g = make_test_graph();
        let paths = all_simple_paths(&g, "auth", "cache", 5);
        for w in paths.windows(2) {
            assert!(w[0].len() <= w[1].len(), "paths should be sorted by length");
        }
    }

    // -- dijkstra_path tests --

    #[test]
    fn test_dijkstra_direct_path() {
        let g = make_test_graph();
        let result = dijkstra_path(&g, "auth", "user", 0.0);
        assert!(result.is_some());
        let (path, cost, edges) = result.unwrap();
        assert_eq!(path.first().unwrap(), "auth");
        assert_eq!(path.last().unwrap(), "user");
        assert!(cost > 0.0);
        assert!(!edges.is_empty());
    }

    #[test]
    fn test_dijkstra_same_node() {
        let g = make_test_graph();
        let result = dijkstra_path(&g, "auth", "auth", 0.0);
        assert!(result.is_some());
        let (path, cost, _) = result.unwrap();
        assert_eq!(path.len(), 1);
        assert!((cost - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_dijkstra_no_path() {
        let mut g = KnowledgeGraph::new();
        g.add_node(make_node("a", "A")).unwrap();
        g.add_node(make_node("b", "B")).unwrap();
        let result = dijkstra_path(&g, "a", "b", 0.0);
        assert!(result.is_none());
    }

    #[test]
    fn test_dijkstra_nonexistent_node() {
        let g = make_test_graph();
        assert!(dijkstra_path(&g, "auth", "nonexistent", 0.0).is_none());
    }

    #[test]
    fn test_dijkstra_min_confidence_filter() {
        // Create graph with mixed confidence edges
        let mut g = KnowledgeGraph::new();
        g.add_node(make_node("a", "A")).unwrap();
        g.add_node(make_node("b", "B")).unwrap();
        g.add_node(make_node("c", "C")).unwrap();

        // a→b: low confidence (0.3), a→c→b: high confidence (1.0)
        let mut low_edge = make_edge("a", "b");
        low_edge.confidence_score = 0.3;
        g.add_edge(low_edge).unwrap();

        let mut high1 = make_edge("a", "c");
        high1.confidence_score = 1.0;
        g.add_edge(high1).unwrap();

        let mut high2 = make_edge("c", "b");
        high2.confidence_score = 1.0;
        g.add_edge(high2).unwrap();

        // With min_confidence 0.5, should skip a→b and go a→c→b
        let result = dijkstra_path(&g, "a", "b", 0.5);
        assert!(result.is_some());
        let (path, _, _) = result.unwrap();
        assert_eq!(path.len(), 3, "should go through c, got path: {path:?}");
    }
}
