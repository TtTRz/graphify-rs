//! Interactive vis.js HTML export.

use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::{Path, PathBuf};

use graphify_core::confidence::Confidence;
use graphify_core::graph::KnowledgeGraph;
use tracing::{info, warn};

const COMMUNITY_COLORS: &[&str] = &[
    "#4E79A7", "#F28E2B", "#E15759", "#76B7B2", "#59A14F", "#EDC948", "#B07AA1", "#FF9DA7",
    "#9C755F", "#BAB0AC",
];

const MAX_NODES_FOR_VIZ: usize = 5000;

/// Export an interactive HTML visualization of the knowledge graph.
///
/// `communities`: mapping from community id → list of node ids.
/// `community_labels`: mapping from community id → human-readable label.
pub fn export_html(
    graph: &KnowledgeGraph,
    communities: &HashMap<usize, Vec<String>>,
    community_labels: &HashMap<usize, String>,
    output_dir: &Path,
) -> anyhow::Result<PathBuf> {
    if graph.node_count() > MAX_NODES_FOR_VIZ {
        warn!(
            node_count = graph.node_count(),
            max = MAX_NODES_FOR_VIZ,
            "graph too large for interactive viz; output may be slow"
        );
    }

    // Build reverse lookup: node_id → community_id
    let mut node_community: HashMap<&str, usize> = HashMap::new();
    for (&cid, members) in communities {
        for nid in members {
            node_community.insert(nid.as_str(), cid);
        }
    }

    // Build vis.js nodes JSON array
    let mut vis_nodes = String::from("[");
    let mut first = true;
    for node in graph.nodes() {
        if !first {
            vis_nodes.push(',');
        }
        first = false;
        let cid = node
            .community
            .or_else(|| node_community.get(node.id.as_str()).copied());
        let color = cid
            .map(|c| COMMUNITY_COLORS[c % COMMUNITY_COLORS.len()])
            .unwrap_or("#888888");
        let label_escaped = escape_js(&node.label);
        let title_escaped = escape_js(&format!(
            "{} ({})\nFile: {}\nType: {:?}",
            node.label, node.id, node.source_file, node.node_type
        ));
        write!(
            vis_nodes,
            r#"{{id:"{}",label:"{}",title:"{}",color:"{}",community:{}}}"#,
            escape_js(&node.id),
            label_escaped,
            title_escaped,
            color,
            cid.unwrap_or(0),
        )
        .unwrap();
    }
    vis_nodes.push(']');

    // Build vis.js edges JSON array
    let mut vis_edges = String::from("[");
    first = true;
    for edge in graph.edges() {
        if !first {
            vis_edges.push(',');
        }
        first = false;
        let dashes = match edge.confidence {
            Confidence::Extracted => "false",
            Confidence::Inferred | Confidence::Ambiguous => "true",
        };
        let width = 1.0 + edge.confidence_score * 2.0;
        let title_escaped = escape_js(&format!(
            "{}: {} → {}\nConfidence: {:?} ({:.2})\nFile: {}",
            edge.relation,
            edge.source,
            edge.target,
            edge.confidence,
            edge.confidence_score,
            edge.source_file
        ));
        write!(
            vis_edges,
            r#"{{from:"{}",to:"{}",label:"{}",title:"{}",dashes:{},width:{:.1}}}"#,
            escape_js(&edge.source),
            escape_js(&edge.target),
            escape_js(&edge.relation),
            title_escaped,
            dashes,
            width,
        )
        .unwrap();
    }
    vis_edges.push(']');

    // Build legend HTML
    let mut legend_html = String::new();
    for (&cid, label) in community_labels {
        let color = COMMUNITY_COLORS[cid % COMMUNITY_COLORS.len()];
        write!(
            legend_html,
            r#"<div class="legend-item"><span class="legend-dot" style="background:{}"></span>{}</div>"#,
            color,
            escape_html(label),
        )
        .unwrap();
    }

    // Build hyperedge info
    let mut hyperedge_html = String::new();
    for he in &graph.hyperedges {
        write!(
            hyperedge_html,
            "<li><b>{}</b>: {} ({})</li>",
            escape_html(&he.relation),
            escape_html(&he.label),
            he.nodes.join(", "),
        )
        .unwrap();
    }

    let html = build_html_template(&vis_nodes, &vis_edges, &legend_html, &hyperedge_html);

    fs::create_dir_all(output_dir)?;
    let path = output_dir.join("graph.html");
    fs::write(&path, &html)?;
    info!(path = %path.display(), "exported interactive HTML visualization");
    Ok(path)
}

fn escape_js(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn build_html_template(
    vis_nodes: &str,
    vis_edges: &str,
    legend_html: &str,
    hyperedge_html: &str,
) -> String {
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Knowledge Graph Visualization</title>
<script src="https://unpkg.com/vis-network/standalone/umd/vis-network.min.js"></script>
<style>
* {{ margin: 0; padding: 0; box-sizing: border-box; }}
body {{ background: #0f0f1a; color: #e0e0e0; font-family: 'Segoe UI', system-ui, sans-serif; display: flex; height: 100vh; overflow: hidden; }}
#sidebar {{ width: 320px; min-width: 320px; background: #1a1a2e; padding: 16px; overflow-y: auto; display: flex; flex-direction: column; gap: 16px; border-right: 1px solid #2a2a4a; }}
#sidebar h2 {{ font-size: 18px; color: #76B7B2; margin-bottom: 4px; }}
#sidebar h3 {{ font-size: 14px; color: #9ca3af; margin-bottom: 4px; }}
#search {{ width: 100%; padding: 8px 12px; border-radius: 6px; border: 1px solid #3a3a5a; background: #0f0f1a; color: #e0e0e0; font-size: 14px; }}
#search:focus {{ outline: none; border-color: #4E79A7; }}
#info-panel {{ background: #0f0f1a; border-radius: 8px; padding: 12px; font-size: 13px; line-height: 1.6; min-height: 120px; }}
#info-panel .prop {{ color: #9ca3af; }}
#info-panel .val {{ color: #e0e0e0; }}
.legend-item {{ display: flex; align-items: center; gap: 8px; font-size: 13px; padding: 2px 0; }}
.legend-dot {{ width: 12px; height: 12px; border-radius: 50%; flex-shrink: 0; }}
#graph-container {{ flex: 1; position: relative; }}
#hyperedges {{ font-size: 13px; }}
#hyperedges ul {{ padding-left: 18px; }}
#hyperedges li {{ margin-bottom: 4px; }}
</style>
</head>
<body>
<div id="sidebar">
    <div>
        <h2>🧠 Knowledge Graph</h2>
        <p style="font-size:12px;color:#666;">Click a node to inspect · Scroll to zoom</p>
    </div>
    <input id="search" type="text" placeholder="Search nodes…" />
    <div>
        <h3>Node Info</h3>
        <div id="info-panel"><i style="color:#666">Click a node to see details</i></div>
    </div>
    <div>
        <h3>Communities</h3>
        <div id="legend">{legend}</div>
    </div>
    <div id="hyperedges">
        <h3>Hyperedges</h3>
        <ul>{hyperedges}</ul>
    </div>
</div>
<div id="graph-container"></div>
<script>
(function() {{
    var nodesData = {nodes};
    var edgesData = {edges};

    var container = document.getElementById('graph-container');
    var nodes = new vis.DataSet(nodesData);
    var edges = new vis.DataSet(edgesData);

    var options = {{
        physics: {{
            solver: 'forceAtlas2Based',
            forceAtlas2Based: {{
                gravitationalConstant: -50,
                centralGravity: 0.01,
                springLength: 120,
                springConstant: 0.08,
                damping: 0.4,
                avoidOverlap: 0.5
            }},
            stabilization: {{ iterations: 200 }}
        }},
        nodes: {{
            shape: 'dot',
            size: 16,
            font: {{ color: '#e0e0e0', size: 12 }},
            borderWidth: 2
        }},
        edges: {{
            color: {{ color: '#4a4a6a', highlight: '#76B7B2', hover: '#76B7B2' }},
            font: {{ color: '#888', size: 10, strokeWidth: 0 }},
            arrows: {{ to: {{ enabled: false }} }},
            smooth: {{ type: 'continuous' }}
        }},
        interaction: {{
            hover: true,
            tooltipDelay: 200,
            zoomView: true,
            dragView: true
        }}
    }};

    var network = new vis.Network(container, {{ nodes: nodes, edges: edges }}, options);

    // Click to inspect
    network.on('click', function(params) {{
        var panel = document.getElementById('info-panel');
        if (params.nodes.length > 0) {{
            var nodeId = params.nodes[0];
            var node = nodes.get(nodeId);
            if (node) {{
                panel.innerHTML =
                    '<div><span class="prop">Label:</span> <span class="val">' + escapeHtml(node.label) + '</span></div>' +
                    '<div><span class="prop">ID:</span> <span class="val">' + escapeHtml(node.id) + '</span></div>' +
                    '<div><span class="prop">Community:</span> <span class="val">' + node.community + '</span></div>';
                network.focus(nodeId, {{ scale: 1.2, animation: true }});
            }}
        }} else {{
            panel.innerHTML = '<i style="color:#666">Click a node to see details</i>';
        }}
    }});

    // Search
    var searchInput = document.getElementById('search');
    searchInput.addEventListener('input', function() {{
        var term = this.value.toLowerCase();
        if (!term) {{
            nodes.forEach(function(n) {{ nodes.update({{ id: n.id, hidden: false }}); }});
            return;
        }}
        nodes.forEach(function(n) {{
            var match = n.label.toLowerCase().includes(term) || n.id.toLowerCase().includes(term);
            nodes.update({{ id: n.id, hidden: !match }});
        }});
    }});

    function escapeHtml(s) {{
        var d = document.createElement('div');
        d.textContent = s;
        return d.innerHTML;
    }}
}})();
</script>
</body>
</html>"##,
        nodes = vis_nodes,
        edges = vis_edges,
        legend = legend_html,
        hyperedges = hyperedge_html,
    )
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
            label: "NodeA".into(),
            source_file: "test.rs".into(),
            source_location: None,
            node_type: NodeType::Class,
            community: Some(0),
            extra: HashMap::new(),
        })
        .unwrap();
        kg.add_node(GraphNode {
            id: "b".into(),
            label: "NodeB".into(),
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
            confidence: Confidence::Inferred,
            confidence_score: 0.7,
            source_file: "test.rs".into(),
            source_location: None,
            weight: 1.0,
            extra: HashMap::new(),
        })
        .unwrap();
        kg
    }

    #[test]
    fn export_html_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let kg = sample_graph();
        let communities: HashMap<usize, Vec<String>> =
            [(0, vec!["a".into()]), (1, vec!["b".into()])].into();
        let labels: HashMap<usize, String> =
            [(0, "Cluster A".into()), (1, "Cluster B".into())].into();

        let path = export_html(&kg, &communities, &labels, dir.path()).unwrap();
        assert!(path.exists());

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("vis-network"));
        assert!(content.contains("NodeA"));
        assert!(content.contains("forceAtlas2Based"));
    }

    #[test]
    fn escape_js_special_chars() {
        assert_eq!(escape_js("a\"b"), r#"a\"b"#);
        assert_eq!(escape_js("a\nb"), r"a\nb");
    }

    #[test]
    fn escape_html_special_chars() {
        assert_eq!(escape_html("<b>hi</b>"), "&lt;b&gt;hi&lt;/b&gt;");
    }
}
