//! Community detection (Leiden algorithm) for graphify.
//!
//! Partitions the knowledge graph into communities using the Leiden algorithm,
//! which improves upon Louvain by adding a refinement phase that guarantees
//! well-connected communities. Falls back to greedy modularity when refinement
//! yields no improvement.

use std::collections::{HashMap, HashSet, VecDeque};

use tracing::debug;

use graphify_core::graph::KnowledgeGraph;
use graphify_core::model::CommunityInfo;

/// Maximum fraction of total nodes a single community may contain before
/// being split further.
const MAX_COMMUNITY_FRACTION: f64 = 0.25;

/// Minimum community size below which we never attempt a split.
const MIN_SPLIT_SIZE: usize = 10;

/// Minimum community size — communities smaller than this get merged into
/// their most-connected neighbor.
const MIN_COMMUNITY_SIZE: usize = 5;

/// Resolution parameter for modularity. Lower = fewer, larger communities.
/// Default Leiden uses 1.0; we use a lower value to avoid over-fragmentation.
const RESOLUTION: f64 = 0.3;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run community detection on the graph. Returns `{community_id: [node_ids]}`.
///
/// Uses the Leiden algorithm: greedy modularity optimization (Louvain phase)
/// followed by a refinement phase that ensures communities are internally
/// well-connected.
pub fn cluster(graph: &KnowledgeGraph) -> HashMap<usize, Vec<String>> {
    let node_count = graph.node_count();
    if node_count == 0 {
        return HashMap::new();
    }

    // If no edges, each node is its own community
    if graph.edge_count() == 0 {
        return graph
            .node_ids()
            .into_iter()
            .enumerate()
            .map(|(i, id)| (i, vec![id]))
            .collect();
    }

    let partition = leiden_partition(graph);

    // Group by community
    let mut communities: HashMap<usize, Vec<String>> = HashMap::new();
    for (node_id, cid) in &partition {
        communities.entry(*cid).or_default().push(node_id.clone());
    }

    // Merge tiny communities into their most-connected neighbor
    let adj = build_adjacency(graph);
    merge_small_communities(&mut communities, &adj);

    // Split oversized communities
    let max_size = std::cmp::max(
        MIN_SPLIT_SIZE,
        (node_count as f64 * MAX_COMMUNITY_FRACTION) as usize,
    );
    let mut final_communities: Vec<Vec<String>> = Vec::new();
    for nodes in communities.values() {
        if nodes.len() > max_size {
            final_communities.extend(split_community(graph, nodes));
        } else {
            final_communities.push(nodes.clone());
        }
    }

    // Re-index by size descending
    final_communities.sort_by_key(|b| std::cmp::Reverse(b.len()));
    final_communities
        .into_iter()
        .enumerate()
        .map(|(i, mut nodes)| {
            nodes.sort();
            (i, nodes)
        })
        .collect()
}

/// Run community detection and mutate graph in-place, storing community info.
pub fn cluster_graph(graph: &mut KnowledgeGraph) -> HashMap<usize, Vec<String>> {
    let communities = cluster(graph);

    // Build CommunityInfo entries
    let scores = score_all(graph, &communities);
    let mut infos: Vec<CommunityInfo> = communities
        .iter()
        .map(|(&cid, nodes)| CommunityInfo {
            id: cid,
            nodes: nodes.clone(),
            cohesion: scores.get(&cid).copied().unwrap_or(0.0),
            label: None,
        })
        .collect();
    infos.sort_by_key(|c| c.id);
    graph.communities = infos;

    communities
}

/// Cohesion score: ratio of actual intra-community edges to maximum possible.
///
/// Returns a value in `[0.0, 1.0]` rounded to two decimal places.
pub fn cohesion_score(graph: &KnowledgeGraph, community_nodes: &[String]) -> f64 {
    let n = community_nodes.len();
    if n <= 1 {
        return 1.0;
    }

    let node_set: HashSet<&str> = community_nodes.iter().map(|s| s.as_str()).collect();
    let mut actual_edges = 0usize;

    // Count edges where both endpoints are in the community
    for node_id in community_nodes {
        let neighbors = graph.get_neighbors(node_id);
        for neighbor in &neighbors {
            if node_set.contains(neighbor.id.as_str()) {
                actual_edges += 1;
            }
        }
    }
    // Each edge is counted twice (once from each endpoint)
    actual_edges /= 2;

    let possible = n * (n - 1) / 2;
    if possible == 0 {
        return 0.0;
    }
    ((actual_edges as f64 / possible as f64) * 100.0).round() / 100.0
}

/// Compute cohesion scores for all communities.
pub fn score_all(
    graph: &KnowledgeGraph,
    communities: &HashMap<usize, Vec<String>>,
) -> HashMap<usize, f64> {
    communities
        .iter()
        .map(|(&cid, nodes)| (cid, cohesion_score(graph, nodes)))
        .collect()
}

// ---------------------------------------------------------------------------
// Adjacency helpers
// ---------------------------------------------------------------------------

/// Build an adjacency list from the KnowledgeGraph for efficient lookups.
fn build_adjacency(graph: &KnowledgeGraph) -> HashMap<String, Vec<(String, f64)>> {
    let mut adj: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for id in graph.node_ids() {
        adj.entry(id).or_default();
    }
    for (src, tgt, edge) in graph.edges_with_endpoints() {
        adj.entry(src.to_string())
            .or_default()
            .push((tgt.to_string(), edge.weight));
        adj.entry(tgt.to_string())
            .or_default()
            .push((src.to_string(), edge.weight));
    }
    adj
}

/// Compute total weight of edges in the graph (sum of all edge weights).
fn total_weight(adj: &HashMap<String, Vec<(String, f64)>>) -> f64 {
    let mut m = 0.0;
    for neighbors in adj.values() {
        for (_, w) in neighbors {
            m += w;
        }
    }
    m / 2.0 // each edge counted twice
}

/// Sum of weights of edges incident to a node.
fn node_strength(adj: &HashMap<String, Vec<(String, f64)>>, node: &str) -> f64 {
    adj.get(node)
        .map(|neighbors| neighbors.iter().map(|(_, w)| w).sum())
        .unwrap_or(0.0)
}

/// Sum of weights of edges from `node` to nodes in `community`.
fn edges_to_community(
    adj: &HashMap<String, Vec<(String, f64)>>,
    node: &str,
    community: &HashSet<&str>,
) -> f64 {
    adj.get(node)
        .map(|neighbors| {
            neighbors
                .iter()
                .filter(|(n, _)| community.contains(n.as_str()))
                .map(|(_, w)| w)
                .sum()
        })
        .unwrap_or(0.0)
}

/// Sum of strengths of all nodes in a community.
fn community_strength(adj: &HashMap<String, Vec<(String, f64)>>, members: &HashSet<&str>) -> f64 {
    members.iter().map(|n| node_strength(adj, n)).sum()
}

// ---------------------------------------------------------------------------
// Leiden algorithm
// ---------------------------------------------------------------------------

/// Leiden algorithm: Louvain phase + refinement phase, iterated until stable.
///
/// Reference: Traag, Waltman & van Eck (2019) "From Louvain to Leiden:
/// guaranteeing well-connected communities"
fn leiden_partition(graph: &KnowledgeGraph) -> HashMap<String, usize> {
    let adj = build_adjacency(graph);
    let m = total_weight(&adj);
    if m == 0.0 {
        return graph
            .node_ids()
            .into_iter()
            .enumerate()
            .map(|(i, id)| (id, i))
            .collect();
    }

    let node_ids = graph.node_ids();

    // Initialize: each node in its own community
    let mut community_of: HashMap<String, usize> = node_ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.clone(), i))
        .collect();

    let max_outer_iterations = 10;
    for _outer in 0..max_outer_iterations {
        // ── Phase 1: Louvain (greedy modularity move) ──
        let changed = louvain_phase(&adj, &node_ids, &mut community_of, m);

        // ── Phase 2: Refinement (ensure well-connected communities) ──
        let refined = refinement_phase(&adj, &mut community_of, m);

        if !changed && !refined {
            break;
        }
    }

    // Compact community IDs
    compact_ids(&mut community_of);
    community_of
}

/// Phase 1: Greedy modularity optimization (Louvain move phase).
///
/// Iterates over nodes and moves each to the neighboring community that
/// yields the greatest modularity gain. Returns true if any move was made.
fn louvain_phase(
    adj: &HashMap<String, Vec<(String, f64)>>,
    node_ids: &[String],
    community_of: &mut HashMap<String, usize>,
    m: f64,
) -> bool {
    let mut community_members: HashMap<usize, HashSet<String>> = HashMap::new();
    for (node, &cid) in community_of.iter() {
        community_members
            .entry(cid)
            .or_default()
            .insert(node.clone());
    }

    // Pre-compute node strengths (ki) — avoids repeated O(deg) recalculation
    let ki_cache: HashMap<&str, f64> = adj
        .keys()
        .map(|n| (n.as_str(), node_strength(adj, n)))
        .collect();

    // Pre-compute community strength sums (sigma_c) — maintained incrementally
    let mut sigma_c: HashMap<usize, f64> = HashMap::new();
    for (&cid, members) in &community_members {
        let sum: f64 = members
            .iter()
            .map(|n| ki_cache.get(n.as_str()).copied().unwrap_or(0.0))
            .sum();
        sigma_c.insert(cid, sum);
    }

    let max_iterations = 50;
    let mut any_changed = false;

    for _iteration in 0..max_iterations {
        let mut improved = false;

        for node in node_ids {
            let current_community = community_of[node];
            let ki = ki_cache.get(node.as_str()).copied().unwrap_or(0.0);

            // Aggregate edges to each neighboring community in one pass
            let mut ki_to: HashMap<usize, f64> = HashMap::new();
            if let Some(neighbors) = adj.get(node.as_str()) {
                for (nbr, w) in neighbors {
                    let nbr_cid = community_of[nbr];
                    *ki_to.entry(nbr_cid).or_default() += w;
                }
            }

            let mut best_community = current_community;
            let mut best_gain = 0.0f64;

            // ki to current community (excluding self-loop edges already handled)
            let ki_in_current = ki_to.get(&current_community).copied().unwrap_or(0.0);
            let sigma_current = sigma_c.get(&current_community).copied().unwrap_or(0.0) - ki;

            for (&target_community, &ki_in_target) in &ki_to {
                if target_community == current_community {
                    continue;
                }

                let sigma_target = sigma_c.get(&target_community).copied().unwrap_or(0.0);

                let gain = (ki_in_target - ki_in_current) / m
                    - RESOLUTION * ki * (sigma_target - sigma_current) / (2.0 * m * m);

                if gain > best_gain {
                    best_gain = gain;
                    best_community = target_community;
                }
            }

            if best_community != current_community {
                // Update community_members
                community_members
                    .get_mut(&current_community)
                    .unwrap()
                    .remove(node);
                community_members
                    .entry(best_community)
                    .or_default()
                    .insert(node.clone());

                // Update sigma_c incrementally
                *sigma_c.entry(current_community).or_default() -= ki;
                *sigma_c.entry(best_community).or_default() += ki;

                community_of.insert(node.clone(), best_community);
                improved = true;
                any_changed = true;
            }
        }

        if !improved {
            break;
        }
    }

    any_changed
}

/// Phase 2: Leiden refinement.
///
/// For each community, find its connected components. If a community is
/// internally disconnected, split it — move each disconnected sub-component
/// to whichever neighboring community maximizes modularity gain (or keep it
/// as a new community). This guarantees all resulting communities are
/// internally connected.
fn refinement_phase(
    adj: &HashMap<String, Vec<(String, f64)>>,
    community_of: &mut HashMap<String, usize>,
    m: f64,
) -> bool {
    // Group nodes by community
    let mut community_members: HashMap<usize, Vec<String>> = HashMap::new();
    for (node, &cid) in community_of.iter() {
        community_members.entry(cid).or_default().push(node.clone());
    }

    let mut any_refined = false;
    let mut next_cid = community_members.keys().copied().max().unwrap_or(0) + 1;

    let community_ids: Vec<usize> = community_members.keys().copied().collect();
    for cid in community_ids {
        let members = match community_members.get(&cid) {
            Some(m) if m.len() > 1 => m.clone(),
            _ => continue,
        };

        // Find connected components within this community
        let components = connected_components_within(adj, &members);
        if components.len() <= 1 {
            continue; // Already well-connected
        }

        debug!(
            "Leiden refinement: community {} has {} disconnected components, splitting",
            cid,
            components.len()
        );

        // Keep the largest component in the original community,
        // assign each smaller component to the best neighboring community
        // or a new community.
        let mut sorted_components = components;
        sorted_components.sort_by_key(|c| std::cmp::Reverse(c.len()));

        // Largest component stays
        for component in sorted_components.iter().skip(1) {
            // For this sub-component, find the best neighboring community
            let mut neighbor_cid_edges: HashMap<usize, f64> = HashMap::new();
            for node in component {
                if let Some(neighbors) = adj.get(node.as_str()) {
                    for (nbr, w) in neighbors {
                        let nbr_cid = community_of[nbr];
                        if nbr_cid != cid {
                            *neighbor_cid_edges.entry(nbr_cid).or_default() += w;
                        }
                    }
                }
            }

            // Pick the neighbor community with the strongest connection,
            // or create a new community if no neighbor exists
            let target_cid = if let Some((&best_cid, _)) = neighbor_cid_edges
                .iter()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            {
                // Only merge if modularity gain is positive
                let _component_set: HashSet<&str> = component.iter().map(|s| s.as_str()).collect();
                let target_members: HashSet<&str> = community_members
                    .get(&best_cid)
                    .map(|s| s.iter().map(|x| x.as_str()).collect())
                    .unwrap_or_default();

                let ki_sum: f64 = component.iter().map(|n| node_strength(adj, n)).sum();
                let ki_in = component
                    .iter()
                    .map(|n| edges_to_community(adj, n, &target_members))
                    .sum::<f64>();
                let sigma_t = community_strength(adj, &target_members);

                let gain = ki_in / m - ki_sum * sigma_t / (2.0 * m * m);
                if gain > 0.0 {
                    best_cid
                } else {
                    let new_cid = next_cid;
                    next_cid += 1;
                    new_cid
                }
            } else {
                let new_cid = next_cid;
                next_cid += 1;
                new_cid
            };

            // Move all nodes in this component
            for node in component {
                community_of.insert(node.clone(), target_cid);
                community_members
                    .entry(target_cid)
                    .or_default()
                    .push(node.clone());
            }
            any_refined = true;
        }

        // Update original community to only keep largest component
        if any_refined {
            community_members.insert(cid, sorted_components.into_iter().next().unwrap());
        }
    }

    any_refined
}

/// Find connected components within a subset of nodes using BFS.
fn connected_components_within(
    adj: &HashMap<String, Vec<(String, f64)>>,
    members: &[String],
) -> Vec<Vec<String>> {
    let member_set: HashSet<&str> = members.iter().map(|s| s.as_str()).collect();
    let mut visited: HashSet<&str> = HashSet::new();
    let mut components: Vec<Vec<String>> = Vec::new();

    for node in members {
        if visited.contains(node.as_str()) {
            continue;
        }

        let mut component = Vec::new();
        let mut queue = VecDeque::new();
        queue.push_back(node.as_str());
        visited.insert(node.as_str());

        while let Some(current) = queue.pop_front() {
            component.push(current.to_string());
            if let Some(neighbors) = adj.get(current) {
                for (nbr, _) in neighbors {
                    if member_set.contains(nbr.as_str()) && !visited.contains(nbr.as_str()) {
                        visited.insert(nbr.as_str());
                        queue.push_back(nbr.as_str());
                    }
                }
            }
        }

        components.push(component);
    }

    components
}

/// Compact community IDs to be contiguous starting from 0.
fn compact_ids(community_of: &mut HashMap<String, usize>) {
    let mut used: Vec<usize> = community_of
        .values()
        .copied()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    used.sort();
    let remap: HashMap<usize, usize> = used
        .iter()
        .enumerate()
        .map(|(new_id, &old_id)| (old_id, new_id))
        .collect();
    for cid in community_of.values_mut() {
        *cid = remap[cid];
    }
}

/// Merge communities smaller than `MIN_COMMUNITY_SIZE` into their
/// most-connected neighboring community.
fn merge_small_communities(
    communities: &mut HashMap<usize, Vec<String>>,
    adj: &HashMap<String, Vec<(String, f64)>>,
) {
    // Build node → community reverse map once, maintain incrementally
    let mut node_to_cid: HashMap<String, usize> = communities
        .iter()
        .flat_map(|(&cid, nodes)| nodes.iter().map(move |n| (n.clone(), cid)))
        .collect();

    loop {
        // Find one small community to merge
        let merge = communities
            .iter()
            .filter(|(_, nodes)| nodes.len() < MIN_COMMUNITY_SIZE)
            .find_map(|(&small_cid, nodes)| {
                // Count edges to each neighboring community
                let mut neighbor_edges: HashMap<usize, f64> = HashMap::new();
                for node in nodes {
                    if let Some(neighbors) = adj.get(node.as_str()) {
                        for (neighbor, weight) in neighbors {
                            if let Some(&ncid) = node_to_cid.get(neighbor.as_str())
                                && ncid != small_cid
                            {
                                *neighbor_edges.entry(ncid).or_default() += weight;
                            }
                        }
                    }
                }
                // Best target
                neighbor_edges
                    .iter()
                    .max_by(|a, b| a.1.total_cmp(b.1))
                    .map(|(&best_cid, _)| (small_cid, best_cid))
            });

        match merge {
            Some((small_cid, best_cid)) => {
                let nodes = communities.remove(&small_cid).unwrap_or_default();
                // Update node_to_cid incrementally
                for node in &nodes {
                    node_to_cid.insert(node.clone(), best_cid);
                }
                communities.entry(best_cid).or_default().extend(nodes);
            }
            None => break, // No more small communities to merge
        }
    }
}

/// Try to split an oversized community by running partition on its subgraph.
fn split_community(graph: &KnowledgeGraph, nodes: &[String]) -> Vec<Vec<String>> {
    if nodes.len() < MIN_SPLIT_SIZE {
        return vec![nodes.to_vec()];
    }

    let node_set: HashSet<&str> = nodes.iter().map(|s| s.as_str()).collect();

    // Build sub-adjacency list
    let mut sub_adj: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for node in nodes {
        sub_adj.entry(node.clone()).or_default();
    }
    for (src, tgt, edge) in graph.edges_with_endpoints() {
        if node_set.contains(src) && node_set.contains(tgt) {
            sub_adj
                .entry(src.to_string())
                .or_default()
                .push((tgt.to_string(), edge.weight));
            sub_adj
                .entry(tgt.to_string())
                .or_default()
                .push((src.to_string(), edge.weight));
        }
    }

    let m = total_weight(&sub_adj);
    if m == 0.0 {
        return nodes.iter().map(|n| vec![n.clone()]).collect();
    }

    // Run Louvain + refinement on the subgraph
    let mut community_of: HashMap<String, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, id)| (id.clone(), i))
        .collect();

    let node_list: Vec<String> = nodes.to_vec();
    for _ in 0..5 {
        let changed = louvain_phase(&sub_adj, &node_list, &mut community_of, m);
        let refined = refinement_phase(&sub_adj, &mut community_of, m);
        if !changed && !refined {
            break;
        }
    }

    // Group results
    let mut groups: HashMap<usize, Vec<String>> = HashMap::new();
    for (node, cid) in &community_of {
        groups.entry(*cid).or_default().push(node.clone());
    }

    let result: Vec<Vec<String>> = groups.into_values().filter(|s| !s.is_empty()).collect();

    if result.len() <= 1 {
        debug!("could not split community of {} nodes further", nodes.len());
        return vec![nodes.to_vec()];
    }

    result
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
    use std::collections::HashMap as StdMap;

    fn make_node(id: &str) -> GraphNode {
        GraphNode {
            id: id.into(),
            label: id.into(),
            source_file: "test.rs".into(),
            source_location: None,
            node_type: NodeType::Class,
            community: None,
            extra: StdMap::new(),
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
            extra: StdMap::new(),
        }
    }

    fn build_graph(nodes: &[&str], edges: &[(&str, &str)]) -> KnowledgeGraph {
        let mut g = KnowledgeGraph::new();
        for &id in nodes {
            g.add_node(make_node(id)).unwrap();
        }
        for &(s, t) in edges {
            g.add_edge(make_edge(s, t)).unwrap();
        }
        g
    }

    #[test]
    fn cluster_empty_graph() {
        let g = KnowledgeGraph::new();
        let result = cluster(&g);
        assert!(result.is_empty());
    }

    #[test]
    fn cluster_no_edges() {
        let g = build_graph(&["a", "b", "c"], &[]);
        let result = cluster(&g);
        assert_eq!(result.len(), 3);
        for nodes in result.values() {
            assert_eq!(nodes.len(), 1);
        }
    }

    #[test]
    fn cluster_single_clique() {
        let g = build_graph(&["a", "b", "c"], &[("a", "b"), ("b", "c"), ("a", "c")]);
        let result = cluster(&g);
        let total_nodes: usize = result.values().map(|v| v.len()).sum();
        assert_eq!(total_nodes, 3);
        assert!(result.len() <= 3);
    }

    #[test]
    fn cluster_two_cliques() {
        let g = build_graph(
            &["a1", "a2", "a3", "b1", "b2", "b3"],
            &[
                ("a1", "a2"),
                ("a2", "a3"),
                ("a1", "a3"),
                ("b1", "b2"),
                ("b2", "b3"),
                ("b1", "b3"),
                ("a3", "b1"), // bridge
            ],
        );
        let result = cluster(&g);
        let total_nodes: usize = result.values().map(|v| v.len()).sum();
        assert_eq!(total_nodes, 6);
        assert!(!result.is_empty());
    }

    #[test]
    fn cohesion_score_single_node() {
        let g = build_graph(&["a"], &[]);
        let score = cohesion_score(&g, &["a".to_string()]);
        assert!((score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn cohesion_score_complete_graph() {
        let g = build_graph(&["a", "b", "c"], &[("a", "b"), ("b", "c"), ("a", "c")]);
        let score = cohesion_score(&g, &["a".to_string(), "b".to_string(), "c".to_string()]);
        assert!((score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn cohesion_score_no_edges() {
        let g = build_graph(&["a", "b", "c"], &[]);
        let score = cohesion_score(&g, &["a".to_string(), "b".to_string(), "c".to_string()]);
        assert!((score - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn cohesion_score_partial() {
        let g = build_graph(&["a", "b", "c"], &[("a", "b")]);
        let score = cohesion_score(&g, &["a".to_string(), "b".to_string(), "c".to_string()]);
        assert!((score - 0.33).abs() < 0.01);
    }

    #[test]
    fn score_all_works() {
        let g = build_graph(&["a", "b"], &[("a", "b")]);
        let mut communities = HashMap::new();
        communities.insert(0, vec!["a".to_string(), "b".to_string()]);
        let scores = score_all(&g, &communities);
        assert_eq!(scores.len(), 1);
        assert!((scores[&0] - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn cluster_graph_mutates_communities() {
        let mut g = build_graph(&["a", "b", "c"], &[("a", "b"), ("b", "c"), ("a", "c")]);
        let result = cluster_graph(&mut g);
        assert!(!result.is_empty());
        assert!(!g.communities.is_empty());
    }

    // ── Leiden-specific tests ──

    #[test]
    fn leiden_splits_disconnected_community() {
        // Two disconnected cliques — Leiden should guarantee they end up
        // in separate communities (Louvain might not).
        let g = build_graph(
            &["a1", "a2", "a3", "b1", "b2", "b3"],
            &[
                ("a1", "a2"),
                ("a2", "a3"),
                ("a1", "a3"),
                ("b1", "b2"),
                ("b2", "b3"),
                ("b1", "b3"),
                // No bridge between the two cliques
            ],
        );
        let result = cluster(&g);
        // Must have exactly 2 communities
        assert_eq!(
            result.len(),
            2,
            "disconnected cliques should form 2 communities"
        );
        // Each community should have 3 nodes
        for nodes in result.values() {
            assert_eq!(nodes.len(), 3);
        }
    }

    #[test]
    fn leiden_connected_components_within() {
        let mut adj: HashMap<String, Vec<(String, f64)>> = HashMap::new();
        for id in &["a", "b", "c", "d"] {
            adj.insert(id.to_string(), Vec::new());
        }
        // a-b connected, c-d connected, no bridge
        adj.get_mut("a").unwrap().push(("b".into(), 1.0));
        adj.get_mut("b").unwrap().push(("a".into(), 1.0));
        adj.get_mut("c").unwrap().push(("d".into(), 1.0));
        adj.get_mut("d").unwrap().push(("c".into(), 1.0));

        let members: Vec<String> = vec!["a", "b", "c", "d"]
            .into_iter()
            .map(String::from)
            .collect();
        let components = connected_components_within(&adj, &members);
        assert_eq!(components.len(), 2);
    }

    #[test]
    fn leiden_single_component() {
        let mut adj: HashMap<String, Vec<(String, f64)>> = HashMap::new();
        for id in &["a", "b", "c"] {
            adj.insert(id.to_string(), Vec::new());
        }
        adj.get_mut("a").unwrap().push(("b".into(), 1.0));
        adj.get_mut("b").unwrap().push(("a".into(), 1.0));
        adj.get_mut("b").unwrap().push(("c".into(), 1.0));
        adj.get_mut("c").unwrap().push(("b".into(), 1.0));

        let members: Vec<String> = vec!["a", "b", "c"].into_iter().map(String::from).collect();
        let components = connected_components_within(&adj, &members);
        assert_eq!(components.len(), 1);
    }
}
