pub(crate) fn escape_js(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

pub(crate) fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(crate) fn build_html_template(
    vis_nodes: &str,
    vis_edges: &str,
    legend_html: &str,
    hyperedge_html: &str,
    prune_banner: &str,
    is_large: bool,
) -> String {
    let physics_config = if is_large {
        r"
            solver: 'barnesHut',
            barnesHut: {
                gravitationalConstant: -8000,
                centralGravity: 0.3,
                springLength: 95,
                springConstant: 0.04,
                damping: 0.09,
                avoidOverlap: 0.2
            },
            stabilization: { iterations: 150, fit: true },
            adaptiveTimestep: true"
    } else {
        r"
            solver: 'forceAtlas2Based',
            forceAtlas2Based: {
                gravitationalConstant: -50,
                centralGravity: 0.01,
                springLength: 120,
                springConstant: 0.08,
                damping: 0.4,
                avoidOverlap: 0.5
            },
            stabilization: { iterations: 200 }"
    };

    let edge_font_size = if is_large { 0 } else { 10 };
    let node_font_size = if is_large { 10 } else { 12 };

    format!(
        r#"<!DOCTYPE html>
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
#prune-banner {{ background: #2a1a00; border: 1px solid #F28E2B; border-radius: 6px; padding: 8px 12px; font-size: 12px; color: #F28E2B; }}
#loading {{ position: absolute; top: 50%; left: 50%; transform: translate(-50%, -50%); font-size: 16px; color: #76B7B2; z-index: 10; }}
</style>
</head>
<body>
<div id="sidebar">
    <div>
        <h2>🧠 Knowledge Graph</h2>
        <p style="font-size:12px;color:#666;">Click a node to inspect · Scroll to zoom</p>
    </div>
    {prune_banner}
    <input id="search" type="text" placeholder="Search nodes…" />
    <div>
        <h3>Node Info</h3>
        <div id="info-panel"><i style="color:#666">Click a node to see details</i></div>
    </div>
    <div>
        <h3>Communities</h3>
        <div id="legend">{legend_html}</div>
    </div>
    <div id="hyperedges">
        <h3>Hyperedges</h3>
        <ul>{hyperedge_html}</ul>
    </div>
</div>
<div id="graph-container">
    <div id="loading">⏳ Laying out graph…</div>
</div>
<script>
(function() {{
    var nodesData = {vis_nodes};
    var edgesData = {vis_edges};

    var container = document.getElementById('graph-container');
    var loading = document.getElementById('loading');
    var nodes = new vis.DataSet(nodesData);
    var edges = new vis.DataSet(edgesData);

    var options = {{
        physics: {{{physics_config}}},
        nodes: {{
            shape: 'dot',
            font: {{ color: '#e0e0e0', size: {node_font_size} }},
            borderWidth: 2
        }},
        edges: {{
            color: {{ color: '#4a4a6a', highlight: '#76B7B2', hover: '#76B7B2' }},
            font: {{ color: '#888', size: {edge_font_size}, strokeWidth: 0 }},
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

    network.on('stabilizationIterationsDone', function() {{
        loading.style.display = 'none';
        network.setOptions({{ physics: {{ enabled: false }} }});
    }});

    setTimeout(function() {{
        loading.style.display = 'none';
    }}, 10000);

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

    var searchInput = document.getElementById('search');
    var searchTimer = null;
    searchInput.addEventListener('input', function() {{
        clearTimeout(searchTimer);
        searchTimer = setTimeout(function() {{
            var term = searchInput.value.toLowerCase();
            var updates = [];
            nodes.forEach(function(n) {{
                var match = !term || n.label.toLowerCase().includes(term) || n.id.toLowerCase().includes(term);
                if (n.hidden !== !match) {{ updates.push({{ id: n.id, hidden: !match }}); }}
            }});
            if (updates.length > 0) {{ nodes.update(updates); }}
        }}, 200);
    }});

    function escapeHtml(s) {{
        var d = document.createElement('div');
        d.textContent = s;
        return d.innerHTML;
    }}
}})();
</script>
</body>
</html>"#,
    )
}
