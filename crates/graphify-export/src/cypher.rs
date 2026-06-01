//! Neo4j Cypher export.

use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use std::collections::HashMap;

use graphify_core::graph::KnowledgeGraph;
use tracing::info;

/// Export the graph as Cypher CREATE statements for Neo4j.
pub fn export_cypher(graph: &KnowledgeGraph, output_dir: &Path) -> anyhow::Result<PathBuf> {
    let mut cypher = String::with_capacity(4096);

    let var_names = build_unique_var_names(graph);

    for node in graph.nodes() {
        let var = var_names.get(&node.id).map(|s| s.as_str()).unwrap_or("n");
        let node_type_label = format!("{}", node.node_type);
        write!(
            cypher,
            "CREATE ({}:{} {{id: '{}', label: '{}', source_file: '{}'",
            var,
            node_type_label,
            cypher_escape(&node.id),
            cypher_escape(&node.label),
            cypher_escape(&node.source_file),
        )?;
        if let Some(loc) = &node.source_location {
            write!(cypher, ", source_location: '{}'", cypher_escape(loc))?;
        }
        if let Some(c) = node.community {
            write!(cypher, ", community: {c}")?;
        }
        writeln!(cypher, "}});")?;
    }

    writeln!(cypher)?;

    for edge in graph.edges() {
        let rel_type = edge
            .relation
            .to_uppercase()
            .replace(|c: char| !c.is_ascii_alphanumeric() && c != '_', "_");
        let src_var = var_names
            .get(&edge.source)
            .map(|s| s.as_str())
            .unwrap_or("n");
        let tgt_var = var_names
            .get(&edge.target)
            .map(|s| s.as_str())
            .unwrap_or("n");
        writeln!(
            cypher,
            "CREATE ({src})-[:{rel} {{relation: '{relation}', confidence: '{confidence}', confidence_score: {score:.2}, source_file: '{file}', weight: {weight:.2}}}]->({tgt});",
            src = src_var,
            rel = rel_type,
            relation = cypher_escape(&edge.relation),
            confidence = edge.confidence,
            score = edge.confidence_score,
            file = cypher_escape(&edge.source_file),
            weight = edge.weight,
            tgt = tgt_var,
        )?;
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
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

fn build_unique_var_names(graph: &KnowledgeGraph) -> HashMap<String, String> {
    let mut name_to_ids: HashMap<String, Vec<String>> = HashMap::new();
    for node in graph.nodes() {
        let sanitized = sanitize_var(&node.id);
        name_to_ids
            .entry(sanitized)
            .or_default()
            .push(node.id.clone());
    }

    let mut result = HashMap::new();
    for (sanitized, mut ids) in name_to_ids {
        if ids.len() == 1 {
            if let Some(primary_id) = ids.pop() {
                result.insert(primary_id, sanitized);
            }
        } else {
            for (i, id) in ids.into_iter().enumerate() {
                result.insert(id, format!("{sanitized}_{i}"));
            }
        }
    }
    result
}

fn cypher_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
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
        assert!(content.contains("CREATE ("));
        assert!(content.contains("CALLS"));
        assert!(content.contains("MyClass"));
    }

    #[test]
    fn var_name_collision_gets_suffix() {
        let mut kg = KnowledgeGraph::new();
        kg.add_node(GraphNode {
            id: "foo.bar".into(),
            label: "FooBar".into(),
            source_file: "a.rs".into(),
            source_location: None,
            node_type: NodeType::Class,
            community: None,
            extra: HashMap::new(),
        })
        .unwrap();
        kg.add_node(GraphNode {
            id: "foo_bar".into(),
            label: "FooBar2".into(),
            source_file: "b.rs".into(),
            source_location: None,
            node_type: NodeType::Function,
            community: None,
            extra: HashMap::new(),
        })
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = export_cypher(&kg, dir.path()).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();

        assert!(content.contains("foo_bar_0"));
        assert!(content.contains("foo_bar_1"));
        assert!(!content.contains("foo_bar}"));
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

    #[test]
    fn cypher_escape_newlines() {
        assert_eq!(cypher_escape("line1\nline2"), "line1\\nline2");
        assert_eq!(cypher_escape("line1\r\nline2"), "line1\\r\\nline2");
    }
}
