//! Graph embedding via simplified Node2Vec random walks.
//!
//! Learns low-dimensional vector representations of graph nodes by:
//! 1. Performing random walks from each node
//! 2. Training Skip-gram embeddings with SGD
//! 3. Finding structurally similar node pairs via cosine similarity

use std::collections::HashMap;

use graphify_core::graph::KnowledgeGraph;
use graphify_core::model::SimilarPair;

/// Compute node embeddings using random walks + Skip-gram.
///
/// - `dim`: embedding dimension (default 64)
/// - `walks_per_node`: random walks starting from each node (default 10)
/// - `walk_length`: length of each walk (default 40)
///
/// Returns a map of node_id → embedding vector.
pub fn compute_embeddings(
    graph: &KnowledgeGraph,
    dim: usize,
    walks_per_node: usize,
    walk_length: usize,
) -> HashMap<String, Vec<f64>> {
    let ids = graph.node_ids();
    let n = ids.len();
    if n == 0 {
        return HashMap::new();
    }

    let id_to_idx: HashMap<&str, usize> = ids
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i))
        .collect();

    // Build adjacency list with indices
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (src, tgt, _) in graph.edges_with_endpoints() {
        if let (Some(&si), Some(&ti)) = (id_to_idx.get(src), id_to_idx.get(tgt)) {
            adj[si].push(ti);
            adj[ti].push(si);
        }
    }

    // Initialize embeddings randomly (deterministic seed via simple hash)
    let mut embeddings: Vec<Vec<f64>> = (0..n)
        .map(|i| {
            (0..dim)
                .map(|d| {
                    let seed = (i as u64)
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add((d as u64).wrapping_mul(1442695040888963407));
                    ((seed as f64).sin() * 0.1).abs() - 0.05
                })
                .collect()
        })
        .collect();

    let mut context_vecs: Vec<Vec<f64>> = (0..n)
        .map(|i| {
            (0..dim)
                .map(|d| {
                    let seed = ((i + n) as u64)
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add((d as u64).wrapping_mul(1442695040888963407));
                    ((seed as f64).cos() * 0.1).abs() - 0.05
                })
                .collect()
        })
        .collect();

    let window = 5usize;
    let learning_rate = 0.025;

    // Generate walks and train
    for walk_num in 0..walks_per_node {
        for start in 0..n {
            // Random walk
            let walk = random_walk(&adj, start, walk_length, walk_num);

            // Skip-gram: for each position, update with context window
            for (pos, &center) in walk.iter().enumerate() {
                let ctx_start = pos.saturating_sub(window);
                let ctx_end = (pos + window + 1).min(walk.len());
                for ctx_pos in ctx_start..ctx_end {
                    if ctx_pos == pos {
                        continue;
                    }
                    let context = walk[ctx_pos];
                    // Simplified SGD update (no negative sampling for speed)
                    let dot: f64 = embeddings[center]
                        .iter()
                        .zip(context_vecs[context].iter())
                        .map(|(a, b)| a * b)
                        .sum();
                    let sigmoid = 1.0 / (1.0 + (-dot).exp());
                    let err = 1.0 - sigmoid; // target = 1 for positive pair
                    let lr = learning_rate * err;

                    for d in 0..dim {
                        let grad_e = lr * context_vecs[context][d];
                        let grad_c = lr * embeddings[center][d];
                        embeddings[center][d] += grad_e;
                        context_vecs[context][d] += grad_c;
                    }
                }
            }
        }
    }

    // Build result map
    ids.into_iter()
        .enumerate()
        .map(|(i, id)| (id, embeddings[i].clone()))
        .collect()
}

/// Find top-N most similar node pairs by cosine similarity of embeddings.
pub fn find_similar(
    graph: &KnowledgeGraph,
    embeddings: &HashMap<String, Vec<f64>>,
    top_n: usize,
) -> Vec<SimilarPair> {
    let ids: Vec<&String> = embeddings.keys().collect();
    let n = ids.len();
    if n < 2 {
        return Vec::new();
    }

    // Pre-compute norms
    let norms: HashMap<&String, f64> = ids
        .iter()
        .map(|&id| {
            let norm = embeddings[id]
                .iter()
                .map(|x| x * x)
                .sum::<f64>()
                .sqrt()
                .max(1e-10);
            (id, norm)
        })
        .collect();

    let mut pairs: Vec<SimilarPair> = Vec::new();

    // Compare all pairs (for large graphs, could use LSH)
    let limit = n.min(500); // Cap to avoid O(n²) explosion on large graphs
    for i in 0..limit {
        for j in (i + 1)..limit {
            let id_a = ids[i];
            let id_b = ids[j];
            let emb_a = &embeddings[id_a];
            let emb_b = &embeddings[id_b];

            let dot: f64 = emb_a.iter().zip(emb_b.iter()).map(|(a, b)| a * b).sum();
            let sim = dot / (norms[id_a] * norms[id_b]);

            if sim > 0.5 {
                let label_a = graph
                    .get_node(id_a)
                    .map(|n| n.label.clone())
                    .unwrap_or_default();
                let label_b = graph
                    .get_node(id_b)
                    .map(|n| n.label.clone())
                    .unwrap_or_default();
                pairs.push(SimilarPair {
                    node_a: id_a.clone(),
                    node_b: id_b.clone(),
                    similarity: sim,
                    label_a,
                    label_b,
                });
            }
        }
    }

    pairs.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    pairs.truncate(top_n);
    pairs
}

/// Deterministic random walk from a start node.
fn random_walk(adj: &[Vec<usize>], start: usize, length: usize, seed: usize) -> Vec<usize> {
    let mut walk = Vec::with_capacity(length);
    let mut current = start;
    let mut rng_state = start.wrapping_mul(2654435761) ^ seed.wrapping_mul(1103515245);

    walk.push(current);
    for _ in 1..length {
        let neighbors = &adj[current];
        if neighbors.is_empty() {
            break;
        }
        // Deterministic pseudo-random neighbor selection
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let idx = rng_state % neighbors.len();
        current = neighbors[idx];
        walk.push(current);
    }
    walk
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphify_core::confidence::Confidence;
    use graphify_core::model::{GraphEdge, GraphNode, NodeType};

    fn make_graph() -> KnowledgeGraph {
        let mut kg = KnowledgeGraph::new();
        for id in &["a", "b", "c", "d"] {
            kg.add_node(GraphNode {
                id: id.to_string(),
                label: id.to_string(),
                source_file: "test.rs".into(),
                source_location: None,
                node_type: NodeType::Function,
                community: None,
                extra: Default::default(),
            })
            .unwrap();
        }
        for (s, t) in &[("a", "b"), ("b", "c"), ("c", "d"), ("a", "d")] {
            kg.add_edge(GraphEdge {
                source: s.to_string(),
                target: t.to_string(),
                relation: "calls".into(),
                confidence: Confidence::Extracted,
                confidence_score: 1.0,
                source_file: "test.rs".into(),
                source_location: None,
                weight: 1.0,
                extra: Default::default(),
            })
            .unwrap();
        }
        kg
    }

    #[test]
    fn compute_embeddings_produces_correct_dims() {
        let kg = make_graph();
        let embs = compute_embeddings(&kg, 16, 5, 10);
        assert_eq!(embs.len(), 4);
        for vec in embs.values() {
            assert_eq!(vec.len(), 16);
        }
    }

    #[test]
    fn find_similar_returns_pairs() {
        let kg = make_graph();
        let embs = compute_embeddings(&kg, 16, 10, 20);
        let pairs = find_similar(&kg, &embs, 5);
        // Connected nodes should have some similarity
        assert!(!pairs.is_empty() || embs.len() < 2);
    }

    #[test]
    fn empty_graph_embeddings() {
        let kg = KnowledgeGraph::new();
        let embs = compute_embeddings(&kg, 16, 5, 10);
        assert!(embs.is_empty());
    }
}
