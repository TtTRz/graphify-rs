use std::collections::HashMap;

use petgraph::stable_graph::{NodeIndex, StableGraph};
use petgraph::Undirected;
use serde_json::{json, Value};
use tracing::warn;

use crate::error::{GraphifyError, Result};
use crate::model::{CommunityInfo, GraphEdge, GraphNode, Hyperedge};

// ---------------------------------------------------------------------------
// KnowledgeGraph
// ---------------------------------------------------------------------------

/// A knowledge graph backed by `petgraph::StableGraph`.
///
/// Provides ID-based node lookup and serialization to/from the
/// NetworkX `node_link_data` JSON format for Python interoperability.
#[derive(Debug)]
pub struct KnowledgeGraph {
    graph: StableGraph<GraphNode, GraphEdge, Undirected>,
    index_map: HashMap<String, NodeIndex>,
    pub communities: Vec<CommunityInfo>,
    pub hyperedges: Vec<Hyperedge>,
}

impl Default for KnowledgeGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl KnowledgeGraph {
    pub fn new() -> Self {
        Self {
            graph: StableGraph::default(),
            index_map: HashMap::new(),
            communities: Vec::new(),
            hyperedges: Vec::new(),
        }
    }

    // -- Mutation --------------------------------------------------------

    /// Add a node. Returns an error if a node with the same `id` already exists.
    pub fn add_node(&mut self, node: GraphNode) -> Result<NodeIndex> {
        if self.index_map.contains_key(&node.id) {
            return Err(GraphifyError::DuplicateNode(node.id.clone()));
        }
        let id = node.id.clone();
        let idx = self.graph.add_node(node);
        self.index_map.insert(id, idx);
        Ok(idx)
    }

    /// Add an edge between two nodes identified by their string IDs.
    pub fn add_edge(&mut self, edge: GraphEdge) -> Result<()> {
        let &src = self
            .index_map
            .get(&edge.source)
            .ok_or_else(|| GraphifyError::NodeNotFound(edge.source.clone()))?;
        let &tgt = self
            .index_map
            .get(&edge.target)
            .ok_or_else(|| GraphifyError::NodeNotFound(edge.target.clone()))?;
        self.graph.add_edge(src, tgt, edge);
        Ok(())
    }

    // -- Query -----------------------------------------------------------

    pub fn get_node(&self, id: &str) -> Option<&GraphNode> {
        self.index_map
            .get(id)
            .and_then(|&idx| self.graph.node_weight(idx))
    }

    pub fn get_neighbors(&self, id: &str) -> Vec<&GraphNode> {
        let Some(&idx) = self.index_map.get(id) else {
            return Vec::new();
        };
        self.graph
            .neighbors(idx)
            .filter_map(|ni| self.graph.node_weight(ni))
            .collect()
    }

    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Replace the hyperedges list.
    pub fn set_hyperedges(&mut self, h: Vec<Hyperedge>) {
        self.hyperedges = h;
    }

    /// Iterate over all node IDs.
    pub fn node_ids(&self) -> Vec<String> {
        self.index_map.keys().cloned().collect()
    }

    /// Get the degree (number of edges) for a node by id.
    pub fn degree(&self, id: &str) -> usize {
        self.index_map
            .get(id)
            .map(|&idx| self.graph.edges(idx).count())
            .unwrap_or(0)
    }

    /// Get neighbor IDs as strings.
    pub fn neighbor_ids(&self, id: &str) -> Vec<String> {
        self.get_neighbors(id)
            .iter()
            .map(|n| n.id.clone())
            .collect()
    }

    /// Collect all nodes as a Vec.
    pub fn nodes(&self) -> Vec<&GraphNode> {
        self.graph
            .node_indices()
            .filter_map(|idx| self.graph.node_weight(idx))
            .collect()
    }

    /// Iterate over all edges as `(source_id, target_id, &GraphEdge)`.
    pub fn edges_with_endpoints(&self) -> Vec<(&str, &str, &GraphEdge)> {
        self.graph
            .edge_indices()
            .filter_map(|idx| {
                let (a, b) = self.graph.edge_endpoints(idx)?;
                let na = self.graph.node_weight(a)?;
                let nb = self.graph.node_weight(b)?;
                let e = self.graph.edge_weight(idx)?;
                Some((na.id.as_str(), nb.id.as_str(), e))
            })
            .collect()
    }

    /// Iterate over all edge weights.
    pub fn edges(&self) -> Vec<&GraphEdge> {
        self.graph
            .edge_indices()
            .filter_map(|idx| self.graph.edge_weight(idx))
            .collect()
    }

    // -- Serialization ---------------------------------------------------

    /// Serialize to the NetworkX `node_link_data` JSON format.
    pub fn to_node_link_json(&self) -> Value {
        let nodes: Vec<Value> = self
            .graph
            .node_indices()
            .filter_map(|idx| {
                let n = self.graph.node_weight(idx)?;
                Some(serde_json::to_value(n).unwrap_or(Value::Null))
            })
            .collect();

        let links: Vec<Value> = self
            .graph
            .edge_indices()
            .filter_map(|idx| {
                let e = self.graph.edge_weight(idx)?;
                Some(serde_json::to_value(e).unwrap_or(Value::Null))
            })
            .collect();

        json!({
            "directed": false,
            "multigraph": false,
            "graph": {},
            "nodes": nodes,
            "links": links,
        })
    }

    /// Deserialize from the NetworkX `node_link_data` JSON format.
    pub fn from_node_link_json(value: &Value) -> Result<Self> {
        let mut kg = Self::new();

        // Nodes
        if let Some(nodes) = value.get("nodes").and_then(|v| v.as_array()) {
            for nv in nodes {
                let node: GraphNode = serde_json::from_value(nv.clone())
                    .map_err(GraphifyError::SerializationError)?;
                if let Err(e) = kg.add_node(node) {
                    warn!("skipping node during import: {e}");
                }
            }
        }

        // Edges (field name is "links" in node_link_data)
        if let Some(links) = value.get("links").and_then(|v| v.as_array()) {
            for lv in links {
                let edge: GraphEdge = serde_json::from_value(lv.clone())
                    .map_err(GraphifyError::SerializationError)?;
                if let Err(e) = kg.add_edge(edge) {
                    warn!("skipping edge during import: {e}");
                }
            }
        }

        Ok(kg)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::confidence::Confidence;
    use crate::model::NodeType;

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
    fn add_and_get_node() {
        let mut kg = KnowledgeGraph::new();
        kg.add_node(make_node("a")).unwrap();
        assert_eq!(kg.node_count(), 1);
        assert!(kg.get_node("a").is_some());
        assert!(kg.get_node("missing").is_none());
    }

    #[test]
    fn duplicate_node_error() {
        let mut kg = KnowledgeGraph::new();
        kg.add_node(make_node("a")).unwrap();
        let err = kg.add_node(make_node("a")).unwrap_err();
        assert!(matches!(err, GraphifyError::DuplicateNode(_)));
    }

    #[test]
    fn add_edge_and_neighbors() {
        let mut kg = KnowledgeGraph::new();
        kg.add_node(make_node("a")).unwrap();
        kg.add_node(make_node("b")).unwrap();
        kg.add_edge(make_edge("a", "b")).unwrap();

        assert_eq!(kg.edge_count(), 1);
        let neighbors = kg.get_neighbors("a");
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0].id, "b");
    }

    #[test]
    fn edge_missing_node() {
        let mut kg = KnowledgeGraph::new();
        kg.add_node(make_node("a")).unwrap();
        let err = kg.add_edge(make_edge("a", "missing")).unwrap_err();
        assert!(matches!(err, GraphifyError::NodeNotFound(_)));
    }

    #[test]
    fn node_link_roundtrip() {
        let mut kg = KnowledgeGraph::new();
        kg.add_node(make_node("x")).unwrap();
        kg.add_node(make_node("y")).unwrap();
        kg.add_edge(make_edge("x", "y")).unwrap();

        let json = kg.to_node_link_json();
        assert_eq!(json["directed"], false);
        assert_eq!(json["multigraph"], false);
        assert!(json["nodes"].as_array().unwrap().len() == 2);
        assert!(json["links"].as_array().unwrap().len() == 1);

        // Reconstruct
        let kg2 = KnowledgeGraph::from_node_link_json(&json).unwrap();
        assert_eq!(kg2.node_count(), 2);
        assert_eq!(kg2.edge_count(), 1);
        assert!(kg2.get_node("x").is_some());
    }

    #[test]
    fn empty_graph_json() {
        let kg = KnowledgeGraph::new();
        let json = kg.to_node_link_json();
        assert!(json["nodes"].as_array().unwrap().is_empty());
        assert!(json["links"].as_array().unwrap().is_empty());
    }

    #[test]
    fn get_neighbors_missing_node() {
        let kg = KnowledgeGraph::new();
        assert!(kg.get_neighbors("nope").is_empty());
    }

    #[test]
    fn default_impl() {
        let kg = KnowledgeGraph::default();
        assert_eq!(kg.node_count(), 0);
    }
}
