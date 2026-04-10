//! Graph analysis algorithms for graphify.
//!
//! Identifies god nodes, surprising cross-community connections, and generates
//! suggested questions for exploration.

use std::collections::{HashMap, HashSet};

use tracing::debug;

use graphify_core::graph::KnowledgeGraph;
use graphify_core::model::{GodNode, Surprise};

// ---------------------------------------------------------------------------
// God nodes
// ---------------------------------------------------------------------------

/// Find the most-connected nodes, excluding file-level hubs and method stubs.
///
/// Returns up to `top_n` nodes sorted by degree descending.
pub fn god_nodes(graph: &KnowledgeGraph, top_n: usize) -> Vec<GodNode> {
    let mut candidates: Vec<GodNode> = graph
        .node_ids()
        .into_iter()
        .filter(|id| !is_file_node(graph, id) && !is_method_stub(graph, id))
        .map(|id| {
            let node = graph.get_node(&id).unwrap();
            GodNode {
                id: id.clone(),
                label: node.label.clone(),
                degree: graph.degree(&id),
                community: node.community,
            }
        })
        .collect();

    candidates.sort_by(|a, b| b.degree.cmp(&a.degree));
    candidates.truncate(top_n);
    debug!("found {} god node candidates", candidates.len());
    candidates
}

// ---------------------------------------------------------------------------
// Surprising connections
// ---------------------------------------------------------------------------

/// Find surprising connections that span different communities or source files.
///
/// A connection is "surprising" if:
/// - the two endpoints belong to different communities, or
/// - the two endpoints come from different source files, or
/// - the edge confidence is `AMBIGUOUS` or `INFERRED`.
///
/// Results are scored and the top `top_n` are returned.
pub fn surprising_connections(
    graph: &KnowledgeGraph,
    communities: &HashMap<usize, Vec<String>>,
    top_n: usize,
) -> Vec<Surprise> {
    // Build reverse map: node_id → community_id
    let node_to_community: HashMap<&str, usize> = communities
        .iter()
        .flat_map(|(&cid, nodes)| nodes.iter().map(move |n| (n.as_str(), cid)))
        .collect();

    let mut surprises: Vec<(f64, Surprise)> = Vec::new();

    for (src, tgt, edge) in graph.edges_with_endpoints() {
        // Skip file/stub nodes
        if is_file_node(graph, src) || is_file_node(graph, tgt) {
            continue;
        }
        if is_method_stub(graph, src) || is_method_stub(graph, tgt) {
            continue;
        }

        let src_comm = node_to_community.get(src).copied().unwrap_or(usize::MAX);
        let tgt_comm = node_to_community.get(tgt).copied().unwrap_or(usize::MAX);

        let mut score = 0.0;

        // Cross-community bonus
        if src_comm != tgt_comm {
            score += 2.0;
        }

        // Cross-file bonus
        let src_node = graph.get_node(src);
        let tgt_node = graph.get_node(tgt);
        if let (Some(sn), Some(tn)) = (src_node, tgt_node) {
            if !sn.source_file.is_empty()
                && !tn.source_file.is_empty()
                && sn.source_file != tn.source_file
            {
                score += 1.0;
            }
        }

        // Confidence bonus: AMBIGUOUS > INFERRED > EXTRACTED
        use graphify_core::confidence::Confidence;
        match edge.confidence {
            Confidence::Ambiguous => score += 3.0,
            Confidence::Inferred => score += 1.5,
            Confidence::Extracted => {}
        }

        if score > 0.0 {
            surprises.push((
                score,
                Surprise {
                    source: src.to_string(),
                    target: tgt.to_string(),
                    source_community: src_comm,
                    target_community: tgt_comm,
                    relation: edge.relation.clone(),
                },
            ));
        }
    }

    surprises.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    surprises.truncate(top_n);
    debug!("found {} surprising connections", surprises.len());
    surprises.into_iter().map(|(_, s)| s).collect()
}

// ---------------------------------------------------------------------------
// Suggest questions
// ---------------------------------------------------------------------------

/// Generate graph-aware questions based on structural patterns.
///
/// Categories:
/// 1. AMBIGUOUS edges → unresolved relationship questions
/// 2. Bridge nodes (high cross-community degree) → cross-cutting concern questions
/// 3. God nodes with INFERRED edges → verification questions
/// 4. Isolated nodes → exploration questions
/// 5. Low-cohesion communities → structural questions
pub fn suggest_questions(
    graph: &KnowledgeGraph,
    communities: &HashMap<usize, Vec<String>>,
    community_labels: &HashMap<usize, String>,
    top_n: usize,
) -> Vec<HashMap<String, String>> {
    let mut questions: Vec<HashMap<String, String>> = Vec::new();

    // 1. AMBIGUOUS edges
    {
        use graphify_core::confidence::Confidence;
        for (src, tgt, edge) in graph.edges_with_endpoints() {
            if edge.confidence == Confidence::Ambiguous {
                let mut q = HashMap::new();
                q.insert("category".into(), "ambiguous_relationship".into());
                q.insert(
                    "question".into(),
                    format!(
                        "What is the exact relationship between '{}' and '{}'? (marked as {})",
                        src, tgt, edge.relation
                    ),
                );
                q.insert("source".into(), src.to_string());
                q.insert("target".into(), tgt.to_string());
                questions.push(q);
            }
        }
    }

    // 2. Bridge nodes (nodes with neighbours in multiple communities)
    {
        let node_to_comm: HashMap<&str, usize> = communities
            .iter()
            .flat_map(|(&cid, nodes)| nodes.iter().map(move |n| (n.as_str(), cid)))
            .collect();

        for id in graph.node_ids() {
            if is_file_node(graph, &id) {
                continue;
            }
            let nbrs = graph.get_neighbors(&id);
            let nbr_comms: HashSet<usize> = nbrs
                .iter()
                .filter_map(|n| node_to_comm.get(n.id.as_str()).copied())
                .collect();
            if nbr_comms.len() >= 3 {
                let comm_names: Vec<String> = nbr_comms
                    .iter()
                    .filter_map(|c| community_labels.get(c).cloned())
                    .collect();
                let mut q = HashMap::new();
                q.insert("category".into(), "bridge_node".into());
                q.insert(
                    "question".into(),
                    format!(
                        "How does '{}' relate to {} different communities{}?",
                        id,
                        nbr_comms.len(),
                        if comm_names.is_empty() {
                            String::new()
                        } else {
                            format!(" ({})", comm_names.join(", "))
                        }
                    ),
                );
                q.insert("node".into(), id.clone());
                questions.push(q);
            }
        }
    }

    // 3. God nodes with INFERRED edges
    {
        use graphify_core::confidence::Confidence;
        let gods = god_nodes(graph, 5);
        for g in &gods {
            let has_inferred = graph.edges_with_endpoints().iter().any(|(s, t, e)| {
                (*s == g.id || *t == g.id) && e.confidence == Confidence::Inferred
            });
            if has_inferred {
                let mut q = HashMap::new();
                q.insert("category".into(), "verification".into());
                q.insert(
                    "question".into(),
                    format!(
                        "Can you verify the inferred relationships of '{}' (degree {})?",
                        g.label, g.degree
                    ),
                );
                q.insert("node".into(), g.id.clone());
                questions.push(q);
            }
        }
    }

    // 4. Isolated nodes (degree 0)
    {
        for id in graph.node_ids() {
            if graph.degree(&id) == 0 && !is_file_node(graph, &id) {
                if let Some(node) = graph.get_node(&id) {
                    let mut q = HashMap::new();
                    q.insert("category".into(), "isolated_node".into());
                    q.insert(
                        "question".into(),
                        format!(
                            "What role does '{}' play? It has no connections in the graph.",
                            node.label
                        ),
                    );
                    q.insert("node".into(), id.clone());
                    questions.push(q);
                }
            }
        }
    }

    // 5. Low-cohesion communities (< 0.3)
    {
        for (&cid, nodes) in communities {
            let n = nodes.len();
            if n <= 1 {
                continue;
            }
            let cohesion = compute_cohesion(graph, nodes);
            if cohesion < 0.3 {
                let label = community_labels
                    .get(&cid)
                    .cloned()
                    .unwrap_or_else(|| format!("community-{cid}"));
                let mut q = HashMap::new();
                q.insert("category".into(), "low_cohesion".into());
                q.insert(
                    "question".into(),
                    format!(
                        "Why is '{}' ({} nodes) loosely connected (cohesion {:.2})? Should it be split?",
                        label, n, cohesion
                    ),
                );
                q.insert("community".into(), cid.to_string());
                questions.push(q);
            }
        }
    }

    questions.truncate(top_n);
    debug!("generated {} questions", questions.len());
    questions
}

// ---------------------------------------------------------------------------
// Graph diff
// ---------------------------------------------------------------------------

/// Compare two graph snapshots and return a summary of changes.
pub fn graph_diff(
    old: &KnowledgeGraph,
    new: &KnowledgeGraph,
) -> HashMap<String, serde_json::Value> {
    let old_node_ids: HashSet<String> = old.node_ids().into_iter().collect();
    let new_node_ids: HashSet<String> = new.node_ids().into_iter().collect();

    let added_nodes: Vec<&String> = new_node_ids.difference(&old_node_ids).collect();
    let removed_nodes: Vec<&String> = old_node_ids.difference(&new_node_ids).collect();

    // Edge keys: (source, target, relation)
    let old_edge_keys: HashSet<(String, String, String)> = old
        .edges_with_endpoints()
        .iter()
        .map(|(s, t, e)| (s.to_string(), t.to_string(), e.relation.clone()))
        .collect();
    let new_edge_keys: HashSet<(String, String, String)> = new
        .edges_with_endpoints()
        .iter()
        .map(|(s, t, e)| (s.to_string(), t.to_string(), e.relation.clone()))
        .collect();

    let added_edges: Vec<&(String, String, String)> =
        new_edge_keys.difference(&old_edge_keys).collect();
    let removed_edges: Vec<&(String, String, String)> =
        old_edge_keys.difference(&new_edge_keys).collect();

    let mut result = HashMap::new();
    result.insert("added_nodes".into(), serde_json::json!(added_nodes));
    result.insert("removed_nodes".into(), serde_json::json!(removed_nodes));
    result.insert(
        "added_edges".into(),
        serde_json::json!(
            added_edges
                .iter()
                .map(|(s, t, r)| { serde_json::json!({"source": s, "target": t, "relation": r}) })
                .collect::<Vec<_>>()
        ),
    );
    result.insert(
        "removed_edges".into(),
        serde_json::json!(
            removed_edges
                .iter()
                .map(|(s, t, r)| { serde_json::json!({"source": s, "target": t, "relation": r}) })
                .collect::<Vec<_>>()
        ),
    );
    result.insert(
        "summary".into(),
        serde_json::json!({
            "nodes_added": added_nodes.len(),
            "nodes_removed": removed_nodes.len(),
            "edges_added": added_edges.len(),
            "edges_removed": removed_edges.len(),
            "old_node_count": old.node_count(),
            "new_node_count": new.node_count(),
            "old_edge_count": old.edge_count(),
            "new_edge_count": new.edge_count(),
        }),
    );

    result
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Is this a file-level hub node?
fn is_file_node(graph: &KnowledgeGraph, node_id: &str) -> bool {
    if let Some(node) = graph.get_node(node_id) {
        // label matches source filename
        if !node.source_file.is_empty() {
            if let Some(fname) = std::path::Path::new(&node.source_file).file_name() {
                if node.label == fname.to_string_lossy() {
                    return true;
                }
            }
        }
    }
    false
}

/// Is this a method stub (.method_name() or isolated fn()?
fn is_method_stub(graph: &KnowledgeGraph, node_id: &str) -> bool {
    if let Some(node) = graph.get_node(node_id) {
        // Method stub: ".method_name()"
        if node.label.starts_with('.') && node.label.ends_with("()") {
            return true;
        }
        // Isolated function stub
        if node.label.ends_with("()") && graph.degree(node_id) <= 1 {
            return true;
        }
    }
    false
}

/// Is this a concept node (no file, or no extension)?
#[cfg(test)]
fn is_concept_node(graph: &KnowledgeGraph, node_id: &str) -> bool {
    if let Some(node) = graph.get_node(node_id) {
        if node.source_file.is_empty() {
            return true;
        }
        let parts: Vec<&str> = node.source_file.split('/').collect();
        if let Some(last) = parts.last() {
            if !last.contains('.') {
                return true;
            }
        }
    }
    false
}

/// Compute cohesion for a set of nodes (inline helper).
fn compute_cohesion(graph: &KnowledgeGraph, community_nodes: &[String]) -> f64 {
    let n = community_nodes.len();
    if n <= 1 {
        return 1.0;
    }
    let node_set: HashSet<&str> = community_nodes.iter().map(|s| s.as_str()).collect();
    let mut actual_edges = 0usize;
    for node_id in community_nodes {
        for neighbor in graph.get_neighbors(node_id) {
            if node_set.contains(neighbor.id.as_str()) {
                actual_edges += 1;
            }
        }
    }
    actual_edges /= 2;
    let possible = n * (n - 1) / 2;
    if possible == 0 {
        return 0.0;
    }
    actual_edges as f64 / possible as f64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use graphify_core::confidence::Confidence;
    use graphify_core::graph::KnowledgeGraph;
    use graphify_core::model::{GraphEdge, GraphNode, NodeType};
    use std::collections::HashMap as StdHashMap;

    fn make_node(id: &str, label: &str, source_file: &str) -> GraphNode {
        GraphNode {
            id: id.into(),
            label: label.into(),
            source_file: source_file.into(),
            source_location: None,
            node_type: NodeType::Class,
            community: None,
            extra: StdHashMap::new(),
        }
    }

    fn make_edge(src: &str, tgt: &str, relation: &str, confidence: Confidence) -> GraphEdge {
        GraphEdge {
            source: src.into(),
            target: tgt.into(),
            relation: relation.into(),
            confidence,
            confidence_score: 1.0,
            source_file: "test.rs".into(),
            source_location: None,
            weight: 1.0,
            extra: StdHashMap::new(),
        }
    }

    fn simple_node(id: &str) -> GraphNode {
        make_node(id, id, "test.rs")
    }

    fn simple_edge(src: &str, tgt: &str) -> GraphEdge {
        make_edge(src, tgt, "calls", Confidence::Extracted)
    }

    fn build_graph(nodes: &[GraphNode], edges: &[GraphEdge]) -> KnowledgeGraph {
        let mut g = KnowledgeGraph::new();
        for n in nodes {
            let _ = g.add_node(n.clone());
        }
        for e in edges {
            let _ = g.add_edge(e.clone());
        }
        g
    }

    // -- god_nodes ---------------------------------------------------------

    #[test]
    fn god_nodes_empty_graph() {
        let g = KnowledgeGraph::new();
        assert!(god_nodes(&g, 5).is_empty());
    }

    #[test]
    fn god_nodes_returns_highest_degree() {
        let g = build_graph(
            &[
                simple_node("hub"),
                simple_node("a"),
                simple_node("b"),
                simple_node("c"),
                simple_node("leaf"),
            ],
            &[
                simple_edge("hub", "a"),
                simple_edge("hub", "b"),
                simple_edge("hub", "c"),
                simple_edge("a", "leaf"),
            ],
        );
        let gods = god_nodes(&g, 2);
        assert_eq!(gods.len(), 2);
        assert_eq!(gods[0].id, "hub");
        assert_eq!(gods[0].degree, 3);
    }

    #[test]
    fn god_nodes_skips_file_nodes() {
        let g = build_graph(
            &[
                make_node("file_hub", "main.rs", "src/main.rs"), // file node
                simple_node("a"),
                simple_node("b"),
            ],
            &[simple_edge("file_hub", "a"), simple_edge("file_hub", "b")],
        );
        let gods = god_nodes(&g, 5);
        // file_hub should be excluded
        assert!(gods.iter().all(|g| g.id != "file_hub"));
    }

    #[test]
    fn god_nodes_skips_method_stubs() {
        let g = build_graph(
            &[
                make_node("stub", ".init()", "test.rs"), // method stub
                simple_node("a"),
            ],
            &[simple_edge("stub", "a")],
        );
        let gods = god_nodes(&g, 5);
        assert!(gods.iter().all(|g| g.id != "stub"));
    }

    // -- surprising_connections -------------------------------------------

    #[test]
    fn surprising_connections_empty() {
        let g = KnowledgeGraph::new();
        let communities = HashMap::new();
        assert!(surprising_connections(&g, &communities, 5).is_empty());
    }

    #[test]
    fn cross_community_edge_is_surprising() {
        let g = build_graph(
            &[simple_node("a"), simple_node("b")],
            &[simple_edge("a", "b")],
        );
        let mut communities = HashMap::new();
        communities.insert(0, vec!["a".into()]);
        communities.insert(1, vec!["b".into()]);
        let surprises = surprising_connections(&g, &communities, 10);
        assert!(!surprises.is_empty());
        assert_eq!(surprises[0].source_community, 0);
        assert_eq!(surprises[0].target_community, 1);
    }

    #[test]
    fn ambiguous_edge_is_surprising() {
        let g = build_graph(
            &[simple_node("a"), simple_node("b")],
            &[make_edge("a", "b", "relates", Confidence::Ambiguous)],
        );
        let mut communities = HashMap::new();
        communities.insert(0, vec!["a".into(), "b".into()]);
        let surprises = surprising_connections(&g, &communities, 10);
        assert!(!surprises.is_empty());
    }

    // -- suggest_questions ------------------------------------------------

    #[test]
    fn suggest_questions_empty() {
        let g = KnowledgeGraph::new();
        let qs = suggest_questions(&g, &HashMap::new(), &HashMap::new(), 10);
        assert!(qs.is_empty());
    }

    #[test]
    fn suggest_questions_ambiguous_edge() {
        let g = build_graph(
            &[simple_node("a"), simple_node("b")],
            &[make_edge("a", "b", "relates", Confidence::Ambiguous)],
        );
        let mut communities = HashMap::new();
        communities.insert(0, vec!["a".into(), "b".into()]);
        let qs = suggest_questions(&g, &communities, &HashMap::new(), 10);
        let has_ambiguous = qs.iter().any(|q| {
            q.get("category")
                .map(|c| c == "ambiguous_relationship")
                .unwrap_or(false)
        });
        assert!(has_ambiguous);
    }

    #[test]
    fn suggest_questions_isolated_node() {
        let g = build_graph(&[simple_node("lonely")], &[]);
        let communities = HashMap::new();
        let qs = suggest_questions(&g, &communities, &HashMap::new(), 10);
        let has_isolated = qs.iter().any(|q| {
            q.get("category")
                .map(|c| c == "isolated_node")
                .unwrap_or(false)
        });
        assert!(has_isolated);
    }

    // -- graph_diff -------------------------------------------------------

    #[test]
    fn graph_diff_identical() {
        let g = build_graph(
            &[simple_node("a"), simple_node("b")],
            &[simple_edge("a", "b")],
        );
        let diff = graph_diff(&g, &g);
        let summary = diff.get("summary").unwrap();
        assert_eq!(summary["nodes_added"], 0);
        assert_eq!(summary["nodes_removed"], 0);
    }

    #[test]
    fn graph_diff_added_node() {
        let old = build_graph(&[simple_node("a")], &[]);
        let new = build_graph(&[simple_node("a"), simple_node("b")], &[]);
        let diff = graph_diff(&old, &new);
        let summary = diff.get("summary").unwrap();
        assert_eq!(summary["nodes_added"], 1);
        assert_eq!(summary["nodes_removed"], 0);
    }

    #[test]
    fn graph_diff_removed_node() {
        let old = build_graph(&[simple_node("a"), simple_node("b")], &[]);
        let new = build_graph(&[simple_node("a")], &[]);
        let diff = graph_diff(&old, &new);
        let summary = diff.get("summary").unwrap();
        assert_eq!(summary["nodes_removed"], 1);
    }

    // -- helpers ----------------------------------------------------------

    #[test]
    fn is_file_node_true() {
        let g = build_graph(&[make_node("f", "main.rs", "src/main.rs")], &[]);
        assert!(is_file_node(&g, "f"));
    }

    #[test]
    fn is_file_node_false() {
        let g = build_graph(&[simple_node("a")], &[]);
        assert!(!is_file_node(&g, "a"));
    }

    #[test]
    fn is_method_stub_true() {
        let g = build_graph(&[make_node("m", ".init()", "test.rs")], &[]);
        assert!(is_method_stub(&g, "m"));
    }

    #[test]
    fn is_concept_node_no_source() {
        let g = build_graph(&[make_node("c", "SomeConcept", "")], &[]);
        assert!(is_concept_node(&g, "c"));
    }
}
