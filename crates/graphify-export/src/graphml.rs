//! GraphML XML export.

use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use graphify_core::graph::KnowledgeGraph;
use tracing::info;

/// Export the graph to GraphML format.
pub fn export_graphml(graph: &KnowledgeGraph, output_dir: &Path) -> anyhow::Result<PathBuf> {
    let mut xml = String::with_capacity(4096);

    writeln!(xml, r#"<?xml version="1.0" encoding="UTF-8"?>"#).unwrap();
    writeln!(
        xml,
        r#"<graphml xmlns="http://graphml.graphdrawing.org/xmlns""#
    )
    .unwrap();
    writeln!(
        xml,
        r#"         xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance""#
    )
    .unwrap();
    writeln!(
        xml,
        r#"         xsi:schemaLocation="http://graphml.graphdrawing.org/xmlns http://graphml.graphdrawing.org/xmlns/1.0/graphml.xsd">"#
    )
    .unwrap();

    // Key definitions for node attributes
    writeln!(
        xml,
        r#"  <key id="label" for="node" attr.name="label" attr.type="string"/>"#
    )
    .unwrap();
    writeln!(
        xml,
        r#"  <key id="node_type" for="node" attr.name="node_type" attr.type="string"/>"#
    )
    .unwrap();
    writeln!(
        xml,
        r#"  <key id="source_file" for="node" attr.name="source_file" attr.type="string"/>"#
    )
    .unwrap();
    writeln!(
        xml,
        r#"  <key id="community" for="node" attr.name="community" attr.type="int"/>"#
    )
    .unwrap();

    // Key definitions for edge attributes
    writeln!(
        xml,
        r#"  <key id="relation" for="edge" attr.name="relation" attr.type="string"/>"#
    )
    .unwrap();
    writeln!(
        xml,
        r#"  <key id="confidence" for="edge" attr.name="confidence" attr.type="string"/>"#
    )
    .unwrap();
    writeln!(
        xml,
        r#"  <key id="confidence_score" for="edge" attr.name="confidence_score" attr.type="double"/>"#
    )
    .unwrap();
    writeln!(
        xml,
        r#"  <key id="weight" for="edge" attr.name="weight" attr.type="double"/>"#
    )
    .unwrap();

    writeln!(xml, r#"  <graph id="G" edgedefault="undirected">"#).unwrap();

    // Nodes
    for node in graph.nodes() {
        writeln!(xml, r#"    <node id="{}">"#, xml_escape(&node.id)).unwrap();
        writeln!(
            xml,
            r#"      <data key="label">{}</data>"#,
            xml_escape(&node.label)
        )
        .unwrap();
        writeln!(
            xml,
            r#"      <data key="node_type">{:?}</data>"#,
            node.node_type
        )
        .unwrap();
        writeln!(
            xml,
            r#"      <data key="source_file">{}</data>"#,
            xml_escape(&node.source_file)
        )
        .unwrap();
        if let Some(c) = node.community {
            writeln!(xml, r#"      <data key="community">{}</data>"#, c).unwrap();
        }
        writeln!(xml, "    </node>").unwrap();
    }

    // Edges
    for (i, edge) in graph.edges().iter().enumerate() {
        writeln!(
            xml,
            r#"    <edge id="e{}" source="{}" target="{}">"#,
            i,
            xml_escape(&edge.source),
            xml_escape(&edge.target)
        )
        .unwrap();
        writeln!(
            xml,
            r#"      <data key="relation">{}</data>"#,
            xml_escape(&edge.relation)
        )
        .unwrap();
        writeln!(
            xml,
            r#"      <data key="confidence">{:?}</data>"#,
            edge.confidence
        )
        .unwrap();
        writeln!(
            xml,
            r#"      <data key="confidence_score">{}</data>"#,
            edge.confidence_score
        )
        .unwrap();
        writeln!(xml, r#"      <data key="weight">{}</data>"#, edge.weight).unwrap();
        writeln!(xml, "    </edge>").unwrap();
    }

    writeln!(xml, "  </graph>").unwrap();
    writeln!(xml, "</graphml>").unwrap();

    fs::create_dir_all(output_dir)?;
    let path = output_dir.join("graph.graphml");
    fs::write(&path, &xml)?;
    info!(path = %path.display(), "exported GraphML");
    Ok(path)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphify_core::confidence::Confidence;
    use graphify_core::graph::KnowledgeGraph;
    use graphify_core::model::{GraphEdge, GraphNode, NodeType};
    use std::collections::HashMap;

    fn sample_graph() -> KnowledgeGraph {
        let mut kg = KnowledgeGraph::new();
        kg.add_node(GraphNode {
            id: "a".into(),
            label: "Node A".into(),
            source_file: "test.rs".into(),
            source_location: None,
            node_type: NodeType::Class,
            community: Some(0),
            extra: HashMap::new(),
        })
        .unwrap();
        kg.add_node(GraphNode {
            id: "b".into(),
            label: "Node B".into(),
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
    fn export_graphml_creates_valid_xml() {
        let dir = tempfile::tempdir().unwrap();
        let kg = sample_graph();
        let path = export_graphml(&kg, dir.path()).unwrap();
        assert!(path.exists());

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("<graphml"));
        assert!(content.contains(r#"<node id="a">"#));
        assert!(content.contains(r#"<node id="b">"#));
        assert!(content.contains(r#"source="a""#));
        assert!(content.contains("</graphml>"));
    }

    #[test]
    fn xml_escape_special_chars() {
        assert_eq!(xml_escape("<a&b>"), "&lt;a&amp;b&gt;");
    }
}
