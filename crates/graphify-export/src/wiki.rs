//! Wikipedia-style markdown export.

use std::collections::HashMap;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use graphify_core::graph::KnowledgeGraph;
use tracing::info;

/// Export wiki-style markdown documentation.
///
/// Generates a `wiki/` directory with:
/// - `index.md` — table of contents
/// - One `.md` file per community
/// - One `.md` file per god node (nodes with degree > average * 2)
pub fn export_wiki(
    graph: &KnowledgeGraph,
    communities: &HashMap<usize, Vec<String>>,
    community_labels: &HashMap<usize, String>,
    output_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let wiki_dir = output_dir.join("wiki");
    fs::create_dir_all(&wiki_dir)?;

    // --- index.md ---
    let mut index = String::with_capacity(2048);
    writeln!(index, "# Knowledge Graph Wiki").unwrap();
    writeln!(index).unwrap();
    writeln!(index, "## Communities").unwrap();
    writeln!(index).unwrap();

    let mut sorted_cids: Vec<usize> = communities.keys().copied().collect();
    sorted_cids.sort();

    for &cid in &sorted_cids {
        let members = &communities[&cid];
        let label = community_labels
            .get(&cid)
            .map(|s| s.as_str())
            .unwrap_or("Unnamed");
        let filename = community_filename(cid);
        writeln!(
            index,
            "- [[{}|{}]] ({} nodes)",
            filename.trim_end_matches(".md"),
            label,
            members.len()
        )
        .unwrap();
    }
    writeln!(index).unwrap();

    // Find god nodes: degree > average * 2
    let degrees: Vec<(String, usize)> = graph
        .nodes()
        .iter()
        .map(|n| (n.id.clone(), graph.degree(&n.id)))
        .collect();
    let avg_degree = if degrees.is_empty() {
        0.0
    } else {
        degrees.iter().map(|(_, d)| *d).sum::<usize>() as f64 / degrees.len() as f64
    };
    let threshold = (avg_degree * 2.0).max(3.0) as usize;
    let god_nodes: Vec<&(String, usize)> =
        degrees.iter().filter(|(_, d)| *d >= threshold).collect();

    if !god_nodes.is_empty() {
        writeln!(index, "## Key Entities").unwrap();
        writeln!(index).unwrap();
        for (nid, degree) in &god_nodes {
            let node = graph.get_node(nid);
            let label = node.map(|n| n.label.as_str()).unwrap_or(nid.as_str());
            let filename = node_filename(nid);
            writeln!(
                index,
                "- [[{}|{}]] (degree: {})",
                filename.trim_end_matches(".md"),
                label,
                degree
            )
            .unwrap();
        }
        writeln!(index).unwrap();
    }

    fs::write(wiki_dir.join("index.md"), &index)?;

    // --- Community pages ---
    for &cid in &sorted_cids {
        let members = &communities[&cid];
        let label = community_labels
            .get(&cid)
            .map(|s| s.as_str())
            .unwrap_or("Unnamed");
        let mut page = String::with_capacity(1024);
        writeln!(page, "# Community {}: {}", cid, label).unwrap();
        writeln!(page).unwrap();
        writeln!(page, "**Members:** {}", members.len()).unwrap();
        writeln!(page).unwrap();

        writeln!(page, "## Nodes").unwrap();
        writeln!(page).unwrap();
        for nid in members {
            let node = graph.get_node(nid);
            let node_label = node.map(|n| n.label.as_str()).unwrap_or(nid.as_str());
            let node_type = node
                .map(|n| format!("{:?}", n.node_type))
                .unwrap_or_default();
            let degree = graph.degree(nid);
            writeln!(
                page,
                "- **{}** (`{}`, {}, degree: {})",
                node_label, nid, node_type, degree
            )
            .unwrap();
        }
        writeln!(page).unwrap();

        // Internal edges
        let member_set: std::collections::HashSet<&str> =
            members.iter().map(|s| s.as_str()).collect();
        let all_edges = graph.edges();
        let internal_edges: Vec<_> = all_edges
            .iter()
            .filter(|e| {
                member_set.contains(e.source.as_str()) && member_set.contains(e.target.as_str())
            })
            .collect();

        if !internal_edges.is_empty() {
            writeln!(page, "## Relationships").unwrap();
            writeln!(page).unwrap();
            for edge in &internal_edges {
                writeln!(
                    page,
                    "- {} → {} ({})",
                    edge.source, edge.target, edge.relation
                )
                .unwrap();
            }
            writeln!(page).unwrap();
        }

        fs::write(wiki_dir.join(community_filename(cid)), &page)?;
    }

    // --- God node pages ---
    for (nid, _) in &god_nodes {
        let node = match graph.get_node(nid) {
            Some(n) => n,
            None => continue,
        };
        let mut page = String::with_capacity(512);
        writeln!(page, "# {}", node.label).unwrap();
        writeln!(page).unwrap();
        writeln!(page, "- **ID:** `{}`", node.id).unwrap();
        writeln!(page, "- **Type:** {:?}", node.node_type).unwrap();
        writeln!(page, "- **File:** `{}`", node.source_file).unwrap();
        if let Some(loc) = &node.source_location {
            writeln!(page, "- **Location:** {}", loc).unwrap();
        }
        if let Some(c) = node.community {
            let clabel = community_labels.get(&c).map(|s| s.as_str()).unwrap_or("?");
            writeln!(page, "- **Community:** {} ({})", c, clabel).unwrap();
        }
        writeln!(page).unwrap();

        // Related edges
        let all_edges = graph.edges();
        let related: Vec<_> = all_edges
            .iter()
            .filter(|e| e.source.as_str() == nid.as_str() || e.target.as_str() == nid.as_str())
            .collect();
        if !related.is_empty() {
            writeln!(page, "## Relationships").unwrap();
            writeln!(page).unwrap();
            for edge in &related {
                writeln!(
                    page,
                    "- {} → {} ({}, {:?})",
                    edge.source, edge.target, edge.relation, edge.confidence
                )
                .unwrap();
            }
            writeln!(page).unwrap();
        }

        fs::write(wiki_dir.join(node_filename(nid)), &page)?;
    }

    info!(path = %wiki_dir.display(), "exported wiki documentation");
    Ok(wiki_dir)
}

fn community_filename(cid: usize) -> String {
    format!("community_{}.md", cid)
}

fn node_filename(id: &str) -> String {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let truncated = graphify_core::truncate_to_bytes(&safe, graphify_core::MAX_FILENAME_BYTES);
    format!("{}.md", truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphify_core::confidence::Confidence;
    use graphify_core::graph::KnowledgeGraph;
    use graphify_core::model::{GraphEdge, GraphNode, NodeType};

    fn sample_graph() -> KnowledgeGraph {
        let mut kg = KnowledgeGraph::new();
        for i in 0..5 {
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
        // Make n0 a hub
        for i in 1..5 {
            kg.add_edge(GraphEdge {
                source: "n0".into(),
                target: format!("n{}", i),
                relation: "calls".into(),
                confidence: Confidence::Extracted,
                confidence_score: 1.0,
                source_file: "test.rs".into(),
                source_location: None,
                weight: 1.0,
                extra: HashMap::new(),
            })
            .unwrap();
        }
        kg
    }

    #[test]
    fn export_wiki_creates_index() {
        let dir = tempfile::tempdir().unwrap();
        let kg = sample_graph();
        let communities: HashMap<usize, Vec<String>> = [(
            0,
            vec![
                "n0".into(),
                "n1".into(),
                "n2".into(),
                "n3".into(),
                "n4".into(),
            ],
        )]
        .into();
        let labels: HashMap<usize, String> = [(0, "Core".into())].into();

        let wiki_dir = export_wiki(&kg, &communities, &labels, dir.path()).unwrap();
        assert!(wiki_dir.join("index.md").exists());
        assert!(wiki_dir.join("community_0.md").exists());
    }

    #[test]
    fn node_filename_sanitizes() {
        assert_eq!(node_filename("my.class"), "my_class.md");
        assert_eq!(node_filename("ok_name"), "ok_name.md");
    }
}
