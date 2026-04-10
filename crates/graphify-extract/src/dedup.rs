//! Deduplication of extracted nodes and edges.
//!
//! After per-file extraction, duplicate nodes (same ID) and edges (same
//! source + target + relation triple) are removed to produce a clean graph.

use std::collections::HashSet;

use graphify_core::model::ExtractionResult;

/// Deduplicate nodes within a single file's [`ExtractionResult`].
///
/// Retains the **first** occurrence of each node ID and each
/// `(source, target, relation)` edge triple.
pub fn dedup_file(result: &mut ExtractionResult) {
    // Dedup nodes by ID
    let mut seen_nodes = HashSet::new();
    result.nodes.retain(|n| seen_nodes.insert(n.id.clone()));

    // Dedup edges by (source, target, relation) triple
    let mut seen_edges: HashSet<(String, String, String)> = HashSet::new();
    result
        .edges
        .retain(|e| seen_edges.insert((e.source.clone(), e.target.clone(), e.relation.clone())));
}

/// Merge multiple [`ExtractionResult`]s into one, deduplicating across all of them.
pub fn dedup_results(results: Vec<ExtractionResult>) -> ExtractionResult {
    let mut combined = ExtractionResult::default();
    for r in results {
        combined.nodes.extend(r.nodes);
        combined.edges.extend(r.edges);
        combined.hyperedges.extend(r.hyperedges);
    }
    dedup_file(&mut combined);
    combined
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

    fn node(id: &str) -> GraphNode {
        GraphNode {
            id: id.to_string(),
            label: id.to_string(),
            source_file: "test.rs".to_string(),
            source_location: None,
            node_type: NodeType::Function,
            community: None,
            extra: HashMap::new(),
        }
    }

    fn edge(src: &str, tgt: &str, rel: &str) -> GraphEdge {
        GraphEdge {
            source: src.to_string(),
            target: tgt.to_string(),
            relation: rel.to_string(),
            confidence: Confidence::Extracted,
            confidence_score: 1.0,
            source_file: "test.rs".to_string(),
            source_location: None,
            weight: 1.0,
            extra: HashMap::new(),
        }
    }

    #[test]
    fn dedup_removes_duplicate_nodes() {
        let mut result = ExtractionResult {
            nodes: vec![node("a"), node("b"), node("a"), node("c"), node("b")],
            edges: Vec::new(),
            hyperedges: Vec::new(),
        };
        dedup_file(&mut result);
        assert_eq!(result.nodes.len(), 3);
        let ids: Vec<&str> = result.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn dedup_removes_duplicate_edges() {
        let mut result = ExtractionResult {
            nodes: Vec::new(),
            edges: vec![
                edge("a", "b", "calls"),
                edge("a", "b", "calls"),   // duplicate
                edge("a", "b", "imports"), // different relation — keep
                edge("c", "d", "calls"),
            ],
            hyperedges: Vec::new(),
        };
        dedup_file(&mut result);
        assert_eq!(result.edges.len(), 3);
    }

    #[test]
    fn dedup_preserves_first_occurrence() {
        let mut n1 = node("x");
        n1.label = "first".to_string();
        let mut n2 = node("x");
        n2.label = "second".to_string();

        let mut result = ExtractionResult {
            nodes: vec![n1, n2],
            edges: Vec::new(),
            hyperedges: Vec::new(),
        };
        dedup_file(&mut result);
        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].label, "first");
    }

    #[test]
    fn dedup_no_duplicates_is_noop() {
        let mut result = ExtractionResult {
            nodes: vec![node("a"), node("b")],
            edges: vec![edge("a", "b", "calls")],
            hyperedges: Vec::new(),
        };
        dedup_file(&mut result);
        assert_eq!(result.nodes.len(), 2);
        assert_eq!(result.edges.len(), 1);
    }

    #[test]
    fn dedup_empty_is_noop() {
        let mut result = ExtractionResult::default();
        dedup_file(&mut result);
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn dedup_results_merges_and_deduplicates() {
        let r1 = ExtractionResult {
            nodes: vec![node("a"), node("b")],
            edges: vec![edge("a", "b", "calls")],
            hyperedges: Vec::new(),
        };
        let r2 = ExtractionResult {
            nodes: vec![node("b"), node("c")],
            edges: vec![edge("a", "b", "calls"), edge("b", "c", "imports")],
            hyperedges: Vec::new(),
        };
        let merged = dedup_results(vec![r1, r2]);
        assert_eq!(merged.nodes.len(), 3);
        assert_eq!(merged.edges.len(), 2);
    }
}
