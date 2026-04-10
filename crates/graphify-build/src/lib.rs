//! Graph assembly and deduplication for graphify.
//!
//! Takes [`ExtractionResult`]s from multiple files and assembles them into a
//! single [`KnowledgeGraph`], skipping dangling edges.

use std::collections::HashSet;

use tracing::debug;

use graphify_core::error::Result;
use graphify_core::graph::KnowledgeGraph;
use graphify_core::model::ExtractionResult;

/// Build a [`KnowledgeGraph`] from a single extraction result.
///
/// All nodes are added first; edges that reference unknown source/target
/// nodes are silently skipped (dangling-edge protection).
pub fn build_from_extraction(extraction: &ExtractionResult) -> Result<KnowledgeGraph> {
    let mut graph = KnowledgeGraph::new();

    // Add all nodes
    for node in &extraction.nodes {
        let _ = graph.add_node(node.clone());
    }

    // Collect known node IDs for dangling-edge check
    let node_ids: HashSet<&str> = extraction.nodes.iter().map(|n| n.id.as_str()).collect();

    // Add edges, skipping those that reference unknown nodes
    let mut skipped = 0usize;
    for edge in &extraction.edges {
        if node_ids.contains(edge.source.as_str()) && node_ids.contains(edge.target.as_str()) {
            let _ = graph.add_edge(edge.clone());
        } else {
            skipped += 1;
        }
    }
    if skipped > 0 {
        debug!("skipped {skipped} dangling edge(s)");
    }

    // Store hyperedges
    graph.set_hyperedges(extraction.hyperedges.clone());

    Ok(graph)
}

/// Merge multiple extraction results into one graph.
///
/// Later extractions override earlier ones for same node IDs (first-write-wins
/// via `add_node` which rejects duplicates, so the first occurrence is kept).
pub fn build(extractions: &[ExtractionResult]) -> Result<KnowledgeGraph> {
    let mut combined = ExtractionResult::default();
    for ext in extractions {
        combined.nodes.extend(ext.nodes.clone());
        combined.edges.extend(ext.edges.clone());
        combined.hyperedges.extend(ext.hyperedges.clone());
    }
    build_from_extraction(&combined)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use graphify_core::confidence::Confidence;
    use graphify_core::model::{GraphEdge, GraphNode, Hyperedge, NodeType};
    use std::collections::HashMap;

    fn make_node(id: &str) -> GraphNode {
        GraphNode {
            id: id.into(),
            label: id.into(),
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
    fn build_from_empty() {
        let ext = ExtractionResult::default();
        let graph = build_from_extraction(&ext).unwrap();
        assert_eq!(graph.node_count(), 0);
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn build_with_nodes_and_edges() {
        let ext = ExtractionResult {
            nodes: vec![make_node("a"), make_node("b"), make_node("c")],
            edges: vec![make_edge("a", "b"), make_edge("b", "c")],
            hyperedges: vec![],
        };
        let graph = build_from_extraction(&ext).unwrap();
        assert_eq!(graph.node_count(), 3);
        assert_eq!(graph.edge_count(), 2);
        assert!(graph.get_node("a").is_some());
        assert!(graph.get_node("b").is_some());
        assert!(graph.get_node("c").is_some());
    }

    #[test]
    fn dangling_edges_skipped() {
        let ext = ExtractionResult {
            nodes: vec![make_node("a"), make_node("b")],
            edges: vec![
                make_edge("a", "b"),       // valid
                make_edge("a", "missing"), // dangling
                make_edge("gone", "b"),    // dangling
            ],
            hyperedges: vec![],
        };
        let graph = build_from_extraction(&ext).unwrap();
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1); // only a->b
    }

    #[test]
    fn build_merges_multiple_extractions() {
        let ext1 = ExtractionResult {
            nodes: vec![make_node("a"), make_node("b")],
            edges: vec![make_edge("a", "b")],
            hyperedges: vec![],
        };
        let ext2 = ExtractionResult {
            nodes: vec![make_node("c")],
            edges: vec![make_edge("b", "c")],
            hyperedges: vec![],
        };
        let graph = build(&[ext1, ext2]).unwrap();
        assert_eq!(graph.node_count(), 3);
        assert_eq!(graph.edge_count(), 2);
    }

    #[test]
    fn duplicate_nodes_first_wins() {
        let ext = ExtractionResult {
            nodes: vec![make_node("a"), make_node("a")],
            edges: vec![],
            hyperedges: vec![],
        };
        let graph = build_from_extraction(&ext).unwrap();
        // second "a" silently skipped
        assert_eq!(graph.node_count(), 1);
    }

    #[test]
    fn hyperedges_stored() {
        let ext = ExtractionResult {
            nodes: vec![make_node("a"), make_node("b")],
            edges: vec![],
            hyperedges: vec![Hyperedge {
                nodes: vec!["a".into(), "b".into()],
                relation: "coexist".into(),
                label: "together".into(),
            }],
        };
        let graph = build_from_extraction(&ext).unwrap();
        assert_eq!(graph.hyperedges.len(), 1);
        assert_eq!(graph.hyperedges[0].relation, "coexist");
    }
}
