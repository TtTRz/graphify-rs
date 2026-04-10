//! NetworkX node_link_data compatible JSON export.

use std::fs;
use std::path::{Path, PathBuf};

use graphify_core::graph::KnowledgeGraph;
use tracing::info;

/// Export graph to `graph.json` in NetworkX `node_link_data` format.
pub fn export_json(graph: &KnowledgeGraph, output_dir: &Path) -> anyhow::Result<PathBuf> {
    let json = graph.to_node_link_json();
    fs::create_dir_all(output_dir)?;
    let path = output_dir.join("graph.json");
    fs::write(&path, serde_json::to_string_pretty(&json)?)?;
    info!(path = %path.display(), "exported graph JSON");
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphify_core::confidence::Confidence;
    use graphify_core::model::{GraphEdge, GraphNode, NodeType};
    use std::collections::HashMap;

    fn sample_graph() -> KnowledgeGraph {
        let mut kg = KnowledgeGraph::new();
        kg.add_node(GraphNode {
            id: "a".into(),
            label: "A".into(),
            source_file: "test.rs".into(),
            source_location: None,
            node_type: NodeType::Class,
            community: None,
            extra: HashMap::new(),
        })
        .unwrap();
        kg.add_node(GraphNode {
            id: "b".into(),
            label: "B".into(),
            source_file: "test.rs".into(),
            source_location: None,
            node_type: NodeType::Function,
            community: None,
            extra: HashMap::new(),
        })
        .unwrap();
        kg.add_edge(GraphEdge {
            source: "a".into(),
            target: "b".into(),
            relation: "calls".into(),
            confidence: Confidence::Extracted,
            confidence_score: 1.0,
            source_file: "test.rs".into(),
            source_location: None,
            weight: 1.0,
            extra: HashMap::new(),
        })
        .unwrap();
        kg
    }

    #[test]
    fn export_json_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let kg = sample_graph();
        let path = export_json(&kg, dir.path()).unwrap();
        assert!(path.exists());

        let content: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(content["nodes"].as_array().unwrap().len(), 2);
        assert_eq!(content["links"].as_array().unwrap().len(), 1);
    }
}
