//! Static SVG export.

use std::collections::HashMap;
use std::f64::consts::PI;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use graphify_core::graph::KnowledgeGraph;
use tracing::info;

const COMMUNITY_COLORS: &[&str] = &[
    "#4E79A7", "#F28E2B", "#E15759", "#76B7B2", "#59A14F", "#EDC948", "#B07AA1", "#FF9DA7",
    "#9C755F", "#BAB0AC",
];

const BG_COLOR: &str = "#0f0f1a";
const EDGE_COLOR: &str = "#3a3a5a";
const LABEL_COLOR: &str = "#ccc";
const FALLBACK_COLOR: &str = "#888888";
const TEXT_COLOR: &str = "#888";

const SVG_WIDTH: f64 = 1200.0;
const SVG_HEIGHT: f64 = 900.0;
const NODE_RADIUS: f64 = 6.0;
const MARGIN: f64 = 60.0;

/// Export a simple static SVG with circular layout.
pub fn export_svg(
    graph: &KnowledgeGraph,
    communities: &HashMap<usize, Vec<String>>,
    output_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let nodes = graph.nodes();
    let edges = graph.edges();

    fs::create_dir_all(output_dir)?;
    let path = output_dir.join("graph.svg");

    if nodes.is_empty() {
        let mut svg = String::new();
        write!(
            svg,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\">",
            SVG_WIDTH, SVG_HEIGHT
        )
        .unwrap();
        write!(
            svg,
            "<rect width=\"100%\" height=\"100%\" fill=\"{}\"/>",
            BG_COLOR
        )
        .unwrap();
        write!(
            svg,
            "<text x=\"50%\" y=\"50%\" fill=\"{}\" text-anchor=\"middle\" font-family=\"sans-serif\">Empty graph</text>",
            TEXT_COLOR
        )
        .unwrap();
        svg.push_str("</svg>");
        fs::write(&path, &svg)?;
        return Ok(path);
    }

    // Reverse map: node_id → community_id
    let mut node_community: HashMap<&str, usize> = HashMap::new();
    for (&cid, members) in communities {
        for nid in members {
            node_community.insert(nid.as_str(), cid);
        }
    }

    // Assign positions in a circle
    let n = nodes.len();
    let cx = SVG_WIDTH / 2.0;
    let cy = SVG_HEIGHT / 2.0;
    let radius = (SVG_WIDTH / 2.0 - MARGIN).min(SVG_HEIGHT / 2.0 - MARGIN);

    let mut positions: HashMap<&str, (f64, f64)> = HashMap::new();
    for (i, node) in nodes.iter().enumerate() {
        let angle = 2.0 * PI * i as f64 / n as f64 - PI / 2.0;
        let x = cx + radius * angle.cos();
        let y = cy + radius * angle.sin();
        positions.insert(node.id.as_str(), (x, y));
    }

    let mut svg = String::with_capacity(4096);
    writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
        SVG_WIDTH, SVG_HEIGHT, SVG_WIDTH, SVG_HEIGHT
    )
    .unwrap();
    writeln!(
        svg,
        "<rect width=\"100%\" height=\"100%\" fill=\"{}\"/>",
        BG_COLOR
    )
    .unwrap();

    // Edges
    for edge in &edges {
        if let (Some(&(x1, y1)), Some(&(x2, y2))) = (
            positions.get(edge.source.as_str()),
            positions.get(edge.target.as_str()),
        ) {
            writeln!(
                svg,
                "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" stroke=\"{}\" stroke-width=\"0.5\" stroke-opacity=\"0.6\"/>",
                x1, y1, x2, y2, EDGE_COLOR
            )
            .unwrap();
        }
    }

    // Nodes
    for node in &nodes {
        if let Some(&(x, y)) = positions.get(node.id.as_str()) {
            let cid = node
                .community
                .or_else(|| node_community.get(node.id.as_str()).copied());
            let color = cid
                .map(|c| COMMUNITY_COLORS[c % COMMUNITY_COLORS.len()])
                .unwrap_or(FALLBACK_COLOR);
            writeln!(
                svg,
                "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"{}\" fill=\"{}\" opacity=\"0.85\"><title>{}</title></circle>",
                x, y, NODE_RADIUS, color, svg_escape(&node.label)
            )
            .unwrap();
        }
    }

    // Labels for small graphs
    if n <= 50 {
        for node in &nodes {
            if let Some(&(x, y)) = positions.get(node.id.as_str()) {
                writeln!(
                    svg,
                    "<text x=\"{:.1}\" y=\"{:.1}\" fill=\"{}\" font-size=\"9\" font-family=\"sans-serif\" text-anchor=\"middle\">{}</text>",
                    x,
                    y - NODE_RADIUS - 3.0,
                    LABEL_COLOR,
                    svg_escape(&node.label)
                )
                .unwrap();
            }
        }
    }

    svg.push_str("</svg>\n");

    fs::write(&path, &svg)?;
    info!(path = %path.display(), "exported SVG");
    Ok(path)
}

fn svg_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphify_core::confidence::Confidence;
    use graphify_core::graph::KnowledgeGraph;
    use graphify_core::model::{GraphEdge, GraphNode, NodeType};

    fn sample_graph() -> KnowledgeGraph {
        let mut kg = KnowledgeGraph::new();
        kg.add_node(GraphNode {
            id: "a".into(),
            label: "A".into(),
            source_file: "test.rs".into(),
            source_location: None,
            node_type: NodeType::Class,
            community: Some(0),
            extra: HashMap::new(),
        })
        .unwrap();
        kg.add_node(GraphNode {
            id: "b".into(),
            label: "B".into(),
            source_file: "test.rs".into(),
            source_location: None,
            node_type: NodeType::Function,
            community: Some(1),
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
    fn export_svg_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let kg = sample_graph();
        let communities: HashMap<usize, Vec<String>> =
            [(0, vec!["a".into()]), (1, vec!["b".into()])].into();

        let path = export_svg(&kg, &communities, dir.path()).unwrap();
        assert!(path.exists());

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("<svg"));
        assert!(content.contains("<circle"));
        assert!(content.contains("<line"));
    }

    #[test]
    fn export_svg_empty_graph() {
        let dir = tempfile::tempdir().unwrap();
        let kg = KnowledgeGraph::new();
        let communities = HashMap::new();

        let path = export_svg(&kg, &communities, dir.path()).unwrap();
        assert!(path.exists());

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Empty graph"));
    }
}
