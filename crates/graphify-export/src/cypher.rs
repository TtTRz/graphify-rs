//! Neo4j Cypher export.

use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use graphify_core::graph::KnowledgeGraph;
use tracing::info;

/// Export the graph as Cypher CREATE statements for Neo4j.
pub fn export_cypher(graph: &KnowledgeGraph, output_dir: &Path) -> anyhow::Result<PathBuf> {
    let mut cypher = String::with_capacity(4096);

    // Nodes
    for node in graph.nodes() {
        let node_type_label = format!("{:?}", node.node_type);
        write!(
            cypher,
            "CREATE (n{}:{} {{id: '{}', label: '{}', source_file: '{}'",
            sanitize_var(&node.id),
            node_type_label,
            cypher_escape(&node.id),
            cypher_escape(&node.label),
            cypher_escape(&node.source_file),
        )
        .unwrap();
        if let Some(loc) = &node.source_location {
            write!(cypher, ", source_location: '{}'", cypher_escape(loc)).unwrap();
        }
        if let Some(c) = node.community {
            write!(cypher, ", community: {}", c).unwrap();
        }
        writeln!(cypher, "}});").unwrap();
    }

    writeln!(cypher).unwrap();

    // Edges
    for edge in graph.edges() {
        let rel_type = edge
            .relation
            .to_uppercase()
            .replace(|c: char| !c.is_alphanumeric() && c != '_', "_");
        writeln!(
            cypher,
            "CREATE (n{})-[:{}  {{relation: '{}', confidence: '{:?}', confidence_score: {:.2}, source_file: '{}', weight: {:.2}}}]->(n{});",
            sanitize_var(&edge.source),
            rel_type,
            cypher_escape(&edge.relation),
            edge.confidence,
            edge.confidence_score,
            cypher_escape(&edge.source_file),
            edge.weight,
            sanitize_var(&edge.target),
        )
        .unwrap();
    }

    fs::create_dir_all(output_dir)?;
    let path = output_dir.join("graph.cypher");
    fs::write(&path, &cypher)?;
    info!(path = %path.display(), "exported Cypher statements");
    Ok(path)
}

/// Make a valid Cypher variable name from a node ID.
fn sanitize_var(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for c in id.chars() {
        if c.is_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    // Ensure it doesn't start with a digit
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

fn cypher_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
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
            id: "my_class".into(),
            label: "MyClass".into(),
            source_file: "src/main.rs".into(),
            source_location: Some("L42".into()),
            node_type: NodeType::Class,
            community: Some(0),
            extra: HashMap::new(),
        })
        .unwrap();
        kg.add_node(GraphNode {
            id: "helper".into(),
            label: "Helper".into(),
            source_file: "src/util.rs".into(),
            source_location: None,
            node_type: NodeType::Function,
            community: None,
            extra: HashMap::new(),
        })
        .unwrap();
        kg.add_edge(GraphEdge {
            source: "my_class".into(),
            target: "helper".into(),
            relation: "calls".into(),
            confidence: Confidence::Extracted,
            confidence_score: 1.0,
            source_file: "src/main.rs".into(),
            source_location: None,
            weight: 1.0,
            extra: HashMap::new(),
        })
        .unwrap();
        kg
    }

    #[test]
    fn export_cypher_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let kg = sample_graph();
        let path = export_cypher(&kg, dir.path()).unwrap();
        assert!(path.exists());

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("CREATE (n"));
        assert!(content.contains("CALLS"));
        assert!(content.contains("MyClass"));
    }

    #[test]
    fn sanitize_var_removes_special_chars() {
        assert_eq!(sanitize_var("my-class.foo"), "my_class_foo");
        assert_eq!(sanitize_var("123abc"), "_123abc");
    }

    #[test]
    fn cypher_escape_quotes() {
        assert_eq!(cypher_escape("it's"), "it\\'s");
    }
}
