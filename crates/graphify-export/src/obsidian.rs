//! Obsidian vault export — one `.md` file per node with `[[wikilinks]]`.

use std::collections::HashMap;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use graphify_core::graph::KnowledgeGraph;
use tracing::info;

/// Export graph as an Obsidian vault (folder of `.md` files with `[[wikilinks]]`).
///
/// Each node becomes a markdown file with YAML frontmatter and a **Connections**
/// section listing all neighbours as `[[wikilinks]]`.
pub fn export_obsidian(
    graph: &KnowledgeGraph,
    communities: &HashMap<usize, Vec<String>>,
    community_labels: &HashMap<usize, String>,
    output_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let vault_dir = output_dir.join("obsidian");
    fs::create_dir_all(&vault_dir)?;

    // Pre-compute node → community mapping for frontmatter
    let node_community: HashMap<&str, usize> = communities
        .iter()
        .flat_map(|(&cid, members)| members.iter().map(move |nid| (nid.as_str(), cid)))
        .collect();

    // Collect all edges grouped by source/target for fast lookup
    let all_edges = graph.edges();
    let mut edges_for: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
    for edge in &all_edges {
        edges_for
            .entry(edge.source.as_str())
            .or_default()
            .push((edge.target.as_str(), edge.relation.as_str()));
        edges_for
            .entry(edge.target.as_str())
            .or_default()
            .push((edge.source.as_str(), edge.relation.as_str()));
    }

    for node in graph.nodes() {
        let filename = sanitize_filename(&node.label);
        let filepath = vault_dir.join(format!("{}.md", filename));

        let mut content = String::with_capacity(512);

        // --- YAML frontmatter ---
        content.push_str("---\n");
        writeln!(content, "id: {}", node.id).unwrap();
        writeln!(content, "type: {:?}", node.node_type).unwrap();
        if !node.source_file.is_empty() {
            writeln!(content, "source: {}", node.source_file).unwrap();
        }
        if let Some(&cid) = node_community.get(node.id.as_str()) {
            writeln!(content, "community: {}", cid).unwrap();
            if let Some(clabel) = community_labels.get(&cid) {
                writeln!(content, "community_label: {}", clabel).unwrap();
            }
        }
        content.push_str("---\n\n");

        // --- Connections ---
        if let Some(neighbours) = edges_for.get(node.id.as_str())
            && !neighbours.is_empty() {
                content.push_str("## Connections\n\n");
                for &(neighbor_id, relation) in neighbours {
                    let link_label = graph
                        .get_node(neighbor_id)
                        .map(|n| sanitize_filename(&n.label))
                        .unwrap_or_else(|| sanitize_filename(neighbor_id));
                    writeln!(content, "- [[{}]] ({})", link_label, relation).unwrap();
                }
            }

        fs::write(&filepath, &content)?;
    }

    info!(path = %vault_dir.display(), "exported Obsidian vault");
    Ok(vault_dir)
}

/// Sanitize a label for use as both a filename and a `[[wikilink]]` target.
fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphify_core::confidence::Confidence;
    use graphify_core::graph::KnowledgeGraph;
    use graphify_core::model::{GraphEdge, GraphNode, NodeType};

    fn sample_graph() -> KnowledgeGraph {
        let mut kg = KnowledgeGraph::new();
        for i in 0..3 {
            kg.add_node(GraphNode {
                id: format!("n{}", i),
                label: format!("Node{}", i),
                source_file: "test.rs".into(),
                source_location: None,
                node_type: NodeType::Class,
                community: Some(0),
                extra: HashMap::new(),
            })
            .unwrap();
        }
        kg.add_edge(GraphEdge {
            source: "n0".into(),
            target: "n1".into(),
            relation: "calls".into(),
            confidence: Confidence::Extracted,
            confidence_score: 1.0,
            source_file: "test.rs".into(),
            source_location: None,
            weight: 1.0,
            extra: HashMap::new(),
        })
        .unwrap();
        kg.add_edge(GraphEdge {
            source: "n0".into(),
            target: "n2".into(),
            relation: "imports".into(),
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
    fn export_obsidian_creates_files() {
        let dir = tempfile::tempdir().unwrap();
        let kg = sample_graph();
        let communities: HashMap<usize, Vec<String>> =
            [(0, vec!["n0".into(), "n1".into(), "n2".into()])].into();
        let labels: HashMap<usize, String> = [(0, "Core".into())].into();

        let vault = export_obsidian(&kg, &communities, &labels, dir.path()).unwrap();
        assert!(vault.join("Node0.md").exists());
        assert!(vault.join("Node1.md").exists());
        assert!(vault.join("Node2.md").exists());
    }

    #[test]
    fn obsidian_file_contains_wikilinks() {
        let dir = tempfile::tempdir().unwrap();
        let kg = sample_graph();
        let communities: HashMap<usize, Vec<String>> =
            [(0, vec!["n0".into(), "n1".into(), "n2".into()])].into();
        let labels: HashMap<usize, String> = [(0, "Core".into())].into();

        let vault = export_obsidian(&kg, &communities, &labels, dir.path()).unwrap();
        let content = std::fs::read_to_string(vault.join("Node0.md")).unwrap();
        assert!(content.contains("[[Node1]]"), "missing wikilink to Node1");
        assert!(content.contains("[[Node2]]"), "missing wikilink to Node2");
        assert!(content.contains("## Connections"));
    }

    #[test]
    fn obsidian_frontmatter_has_community() {
        let dir = tempfile::tempdir().unwrap();
        let kg = sample_graph();
        let communities: HashMap<usize, Vec<String>> =
            [(0, vec!["n0".into(), "n1".into(), "n2".into()])].into();
        let labels: HashMap<usize, String> = [(0, "Core".into())].into();

        let vault = export_obsidian(&kg, &communities, &labels, dir.path()).unwrap();
        let content = std::fs::read_to_string(vault.join("Node0.md")).unwrap();
        assert!(content.contains("community: 0"));
        assert!(content.contains("community_label: Core"));
    }

    #[test]
    fn sanitize_filename_works() {
        assert_eq!(sanitize_filename("my.class"), "my_class");
        assert_eq!(sanitize_filename("hello world"), "hello world");
        assert_eq!(sanitize_filename("a/b\\c"), "a_b_c");
    }
}
