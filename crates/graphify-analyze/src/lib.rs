//! Graph analysis algorithms for graphify.
//!
//! Identifies god nodes, surprising cross-community connections, and generates
//! suggested questions for exploration.

use std::collections::{HashMap, HashSet};

use tracing::debug;

use graphify_core::graph::KnowledgeGraph;
use graphify_core::model::{BridgeNode, DependencyCycle, GodNode, PageRankNode, Surprise};

// ---------------------------------------------------------------------------
// God nodes
// ---------------------------------------------------------------------------

/// Find the most-connected nodes, excluding file-level hubs and method stubs.
///
/// Returns up to `top_n` nodes sorted by degree descending.
/// Generic labels like "lib", "mod", "main" are disambiguated with the crate/module
/// name extracted from `source_file`.
pub fn god_nodes(graph: &KnowledgeGraph, top_n: usize) -> Vec<GodNode> {
    let generic_labels = ["lib", "mod", "main", "index", "init"];

    let mut candidates: Vec<GodNode> = graph
        .node_ids()
        .into_iter()
        .filter(|id| !is_file_node(graph, id) && !is_method_stub(graph, id))
        .map(|id| {
            let node = graph.get_node(&id).unwrap();
            let label = if generic_labels.contains(&node.label.as_str()) {
                // Extract crate/module name from source_file path
                // e.g. "crates/graphify-export/src/lib.rs" → "graphify-export::lib"
                disambiguate_label(&node.label, &node.source_file)
            } else {
                node.label.clone()
            };
            GodNode {
                id: id.clone(),
                label,
                degree: graph.degree(&id),
                community: node.community,
            }
        })
        .collect();

    candidates.sort_by_key(|b| std::cmp::Reverse(b.degree));
    candidates.truncate(top_n);
    debug!("found {} god node candidates", candidates.len());
    candidates
}

/// Disambiguate a generic label using the source file path.
///
/// Extracts the parent directory or crate name to create a unique label.
/// Examples:
/// - ("lib", "crates/graphify-export/src/lib.rs") → "graphify-export::lib"
/// - ("mod", "src/config.rs") → "src::mod"
/// - ("lib", "src/lib.rs") → "lib"
fn disambiguate_label(label: &str, source_file: &str) -> String {
    let parts: Vec<&str> = source_file.split('/').collect();
    // Try to find crate name: look for the segment before "src/"
    for (i, &segment) in parts.iter().enumerate() {
        if segment == "src" && i > 0 {
            return format!("{}::{}", parts[i - 1], label);
        }
    }
    // Fallback: use parent directory
    if parts.len() >= 2 {
        return format!("{}::{}", parts[parts.len() - 2], label);
    }
    label.to_string()
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
        if let (Some(sn), Some(tn)) = (src_node, tgt_node)
            && !sn.source_file.is_empty()
            && !tn.source_file.is_empty()
            && sn.source_file != tn.source_file
        {
            score += 1.0;
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
            if graph.degree(&id) == 0
                && !is_file_node(graph, &id)
                && let Some(node) = graph.get_node(&id)
            {
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
                        "Why is '{label}' ({n} nodes) loosely connected (cohesion {cohesion:.2})? Should it be split?"
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
// Community bridges
// ---------------------------------------------------------------------------

/// Find nodes that bridge multiple communities.
///
/// A bridge node has a high ratio of cross-community edges to total edges.
/// Returns up to `top_n` nodes sorted by bridge ratio descending.
pub fn community_bridges(
    graph: &KnowledgeGraph,
    communities: &HashMap<usize, Vec<String>>,
    top_n: usize,
) -> Vec<BridgeNode> {
    // Build node → community mapping
    let node_to_community: HashMap<&str, usize> = communities
        .iter()
        .flat_map(|(&cid, nodes)| nodes.iter().map(move |n| (n.as_str(), cid)))
        .collect();

    let mut bridges: Vec<BridgeNode> = graph
        .node_ids()
        .into_iter()
        .filter(|id| !is_file_node(graph, id))
        .filter_map(|id| {
            let node = graph.get_node(&id)?;
            let my_comm = node_to_community.get(id.as_str()).copied()?;
            let neighbors = graph.neighbor_ids(&id);
            let total = neighbors.len();
            if total == 0 {
                return None;
            }

            let mut touched: HashSet<usize> = HashSet::new();
            touched.insert(my_comm);
            let mut cross = 0usize;
            for nid in &neighbors {
                let neighbor_comm = node_to_community
                    .get(nid.as_str())
                    .copied()
                    .unwrap_or(my_comm);
                if neighbor_comm != my_comm {
                    cross += 1;
                    touched.insert(neighbor_comm);
                }
            }

            if cross == 0 {
                return None;
            }

            let ratio = cross as f64 / total as f64;
            let mut communities_touched: Vec<usize> = touched.into_iter().collect();
            communities_touched.sort_unstable();

            Some(BridgeNode {
                id: id.clone(),
                label: node.label.clone(),
                total_edges: total,
                cross_community_edges: cross,
                bridge_ratio: ratio,
                communities_touched,
            })
        })
        .collect();

    bridges.sort_by(|a, b| {
        b.bridge_ratio
            .partial_cmp(&a.bridge_ratio)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    bridges.truncate(top_n);
    bridges
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Is this a file-level hub node?
fn is_file_node(graph: &KnowledgeGraph, node_id: &str) -> bool {
    if let Some(node) = graph.get_node(node_id) {
        // label matches source filename
        if !node.source_file.is_empty()
            && let Some(fname) = std::path::Path::new(&node.source_file).file_name()
            && node.label == fname.to_string_lossy()
        {
            return true;
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

/// Compute cohesion for a set of nodes (inline helper).
fn compute_cohesion(graph: &KnowledgeGraph, community_nodes: &[String]) -> f64 {
    let n = community_nodes.len();
    if n <= 1 {
        return 1.0;
    }
    let node_set: HashSet<&str> = community_nodes.iter().map(std::string::String::as_str).collect();
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
// PageRank
// ---------------------------------------------------------------------------

/// Compute PageRank importance scores for all nodes.
///
/// Returns the top `top_n` nodes sorted by PageRank score descending.
/// Uses the power iteration method with configurable damping factor (default 0.85).
pub fn pagerank(
    graph: &KnowledgeGraph,
    top_n: usize,
    damping: f64,
    max_iterations: usize,
) -> Vec<PageRankNode> {
    let n = graph.node_count();
    if n == 0 {
        return Vec::new();
    }

    let ids = graph.node_ids();
    let id_to_idx: HashMap<&str, usize> = ids
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i))
        .collect();

    // Build adjacency + out-degree (undirected graph: treat all edges as bidirectional)
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (src, tgt, _) in graph.edges_with_endpoints() {
        if let (Some(&si), Some(&ti)) = (id_to_idx.get(src), id_to_idx.get(tgt)) {
            adj[si].push(ti);
            adj[ti].push(si);
        }
    }

    let out_degree: Vec<usize> = adj.iter().map(std::vec::Vec::len).collect();
    let init = 1.0 / n as f64;
    let mut rank = vec![init; n];
    let mut next_rank = vec![0.0f64; n];

    for _ in 0..max_iterations {
        let teleport = (1.0 - damping) / n as f64;

        // Dangling node mass (nodes with no outgoing edges)
        let dangling_sum: f64 = rank
            .iter()
            .enumerate()
            .filter(|(i, _)| out_degree[*i] == 0)
            .map(|(_, r)| r)
            .sum();

        for v in 0..n {
            let mut sum = 0.0;
            for &u in &adj[v] {
                if out_degree[u] > 0 {
                    sum += rank[u] / out_degree[u] as f64;
                }
            }
            next_rank[v] = teleport + damping * (sum + dangling_sum / n as f64);
        }

        // Check convergence
        let delta: f64 = rank
            .iter()
            .zip(next_rank.iter())
            .map(|(a, b)| (a - b).abs())
            .sum();
        std::mem::swap(&mut rank, &mut next_rank);
        if delta < 1e-6 {
            break;
        }
    }

    // Build result
    let mut results: Vec<PageRankNode> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let node = graph.get_node(id);
            PageRankNode {
                id: id.clone(),
                label: node.map(|n| n.label.clone()).unwrap_or_default(),
                score: rank[i],
                degree: out_degree[i],
            }
        })
        .collect();

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(top_n);
    results
}

// ---------------------------------------------------------------------------
// Dependency cycle detection
// ---------------------------------------------------------------------------

/// Detect dependency cycles using Tarjan's algorithm for strongly connected components.
///
/// Only considers directional edges (imports, uses, calls) to find true dependency cycles.
/// Returns cycles sorted by severity (shorter cycles = more severe).
pub fn detect_cycles(graph: &KnowledgeGraph, max_cycles: usize) -> Vec<DependencyCycle> {
    let directional = ["imports", "uses", "calls", "includes"];

    let ids = graph.node_ids();
    let id_to_idx: HashMap<&str, usize> = ids
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i))
        .collect();
    let n = ids.len();

    // Build directed adjacency list
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (src, tgt, edge) in graph.edges_with_endpoints() {
        if directional.contains(&edge.relation.as_str())
            && let (Some(&si), Some(&ti)) = (id_to_idx.get(src), id_to_idx.get(tgt))
        {
            adj[si].push(ti);
        }
    }

    // Tarjan's SCC
    let sccs = tarjan_scc(&adj, n);

    // For each SCC with size > 1, extract cycle
    let mut cycles: Vec<DependencyCycle> = Vec::new();
    for scc in &sccs {
        if scc.len() <= 1 || cycles.len() >= max_cycles {
            continue;
        }

        // Find a simple cycle within this SCC using DFS
        let scc_set: HashSet<usize> = scc.iter().copied().collect();
        if let Some(cycle_indices) = find_cycle_in_scc(&adj, scc, &scc_set) {
            let nodes: Vec<String> = cycle_indices.iter().map(|&i| ids[i].clone()).collect();
            let edges: Vec<(String, String)> = cycle_indices
                .windows(2)
                .map(|w| (ids[w[0]].clone(), ids[w[1]].clone()))
                .chain(std::iter::once((
                    ids[*cycle_indices.last().unwrap()].clone(),
                    ids[cycle_indices[0]].clone(),
                )))
                .collect();
            let severity = 1.0 / nodes.len() as f64;
            cycles.push(DependencyCycle {
                nodes,
                edges,
                severity,
            });
        }
    }

    cycles.sort_by(|a, b| {
        b.severity
            .partial_cmp(&a.severity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    cycles.truncate(max_cycles);
    cycles
}

/// Tarjan's algorithm for finding strongly connected components.
fn tarjan_scc(adj: &[Vec<usize>], n: usize) -> Vec<Vec<usize>> {
    let mut index_counter = 0usize;
    let mut stack: Vec<usize> = Vec::new();
    let mut on_stack = vec![false; n];
    let mut index = vec![usize::MAX; n];
    let mut lowlink = vec![0usize; n];
    let mut result: Vec<Vec<usize>> = Vec::new();

    #[allow(clippy::too_many_arguments)]
    fn strongconnect(
        v: usize,
        adj: &[Vec<usize>],
        index_counter: &mut usize,
        stack: &mut Vec<usize>,
        on_stack: &mut [bool],
        index: &mut [usize],
        lowlink: &mut [usize],
        result: &mut Vec<Vec<usize>>,
    ) {
        index[v] = *index_counter;
        lowlink[v] = *index_counter;
        *index_counter += 1;
        stack.push(v);
        on_stack[v] = true;

        for &w in &adj[v] {
            if index[w] == usize::MAX {
                strongconnect(
                    w,
                    adj,
                    index_counter,
                    stack,
                    on_stack,
                    index,
                    lowlink,
                    result,
                );
                lowlink[v] = lowlink[v].min(lowlink[w]);
            } else if on_stack[w] {
                lowlink[v] = lowlink[v].min(index[w]);
            }
        }

        if lowlink[v] == index[v] {
            let mut component = Vec::new();
            while let Some(w) = stack.pop() {
                on_stack[w] = false;
                component.push(w);
                if w == v {
                    break;
                }
            }
            result.push(component);
        }
    }

    for v in 0..n {
        if index[v] == usize::MAX {
            strongconnect(
                v,
                adj,
                &mut index_counter,
                &mut stack,
                &mut on_stack,
                &mut index,
                &mut lowlink,
                &mut result,
            );
        }
    }

    result
}

/// Find a simple cycle within a strongly connected component.
fn find_cycle_in_scc(
    adj: &[Vec<usize>],
    scc: &[usize],
    scc_set: &HashSet<usize>,
) -> Option<Vec<usize>> {
    if scc.is_empty() {
        return None;
    }
    let start = scc[0];
    let mut visited = HashSet::new();
    let mut path = Vec::new();

    fn dfs_cycle(
        node: usize,
        start: usize,
        adj: &[Vec<usize>],
        scc_set: &HashSet<usize>,
        visited: &mut HashSet<usize>,
        path: &mut Vec<usize>,
    ) -> bool {
        path.push(node);
        visited.insert(node);

        for &next in &adj[node] {
            if !scc_set.contains(&next) {
                continue;
            }
            if next == start && path.len() > 1 {
                return true; // Found cycle back to start
            }
            if !visited.contains(&next) && dfs_cycle(next, start, adj, scc_set, visited, path) {
                return true;
            }
        }

        path.pop();
        false
    }

    if dfs_cycle(start, start, adj, scc_set, &mut visited, &mut path) {
        Some(path)
    } else {
        None
    }
}

pub mod embedding;
pub mod temporal;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;

