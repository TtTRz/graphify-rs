use super::*;
use graphify_core::confidence::Confidence;
use graphify_core::graph::KnowledgeGraph;

fn is_concept_node(graph: &KnowledgeGraph, node_id: &str) -> bool {
    if let Some(node) = graph.get_node(node_id) {
        if node.source_file.is_empty() {
            return true;
        }
        let parts: Vec<&str> = node.source_file.split('/').collect();
        if let Some(last) = parts.last()
            && !last.contains('.')
        {
            return true;
        }
    }
    false
}

use graphify_core::model::{GraphEdge, GraphNode, NodeType};
use std::collections::HashMap as StdHashMap;

fn make_node(id: &str, label: &str, source_file: &str) -> GraphNode {
    GraphNode {
        id: id.into(),
        label: label.into(),
        source_file: source_file.into(),
        source_location: None,
        node_type: NodeType::Class,
        community: None,
        extra: StdHashMap::new(),
    }
}

fn make_edge(src: &str, tgt: &str, relation: &str, confidence: Confidence) -> GraphEdge {
    GraphEdge {
        source: src.into(),
        target: tgt.into(),
        relation: relation.into(),
        confidence,
        confidence_score: 1.0,
        source_file: "test.rs".into(),
        source_location: None,
        weight: 1.0,
        extra: StdHashMap::new(),
    }
}

fn simple_node(id: &str) -> GraphNode {
    make_node(id, id, "test.rs")
}

fn simple_edge(src: &str, tgt: &str) -> GraphEdge {
    make_edge(src, tgt, "calls", Confidence::Extracted)
}

fn build_graph(nodes: &[GraphNode], edges: &[GraphEdge]) -> KnowledgeGraph {
    let mut g = KnowledgeGraph::new();
    for n in nodes {
        let _ = g.add_node(n.clone());
    }
    for e in edges {
        let _ = g.add_edge(e.clone());
    }
    g
}

#[test]
fn god_nodes_empty_graph() {
    let g = KnowledgeGraph::new();
    assert!(god_nodes(&g, 5).is_empty());
}

#[test]
fn god_nodes_returns_highest_degree() {
    let g = build_graph(
        &[
            simple_node("hub"),
            simple_node("a"),
            simple_node("b"),
            simple_node("c"),
            simple_node("leaf"),
        ],
        &[
            simple_edge("hub", "a"),
            simple_edge("hub", "b"),
            simple_edge("hub", "c"),
            simple_edge("a", "leaf"),
        ],
    );
    let gods = god_nodes(&g, 2);
    assert_eq!(gods.len(), 2);
    assert_eq!(gods[0].id, "hub");
    assert_eq!(gods[0].degree, 3);
}

#[test]
fn god_nodes_skips_file_nodes() {
    let g = build_graph(
        &[
            make_node("file_hub", "main.rs", "src/main.rs"), // file node
            simple_node("a"),
            simple_node("b"),
        ],
        &[simple_edge("file_hub", "a"), simple_edge("file_hub", "b")],
    );
    let gods = god_nodes(&g, 5);
    // file_hub should be excluded
    assert!(gods.iter().all(|g| g.id != "file_hub"));
}

#[test]
fn god_nodes_skips_method_stubs() {
    let g = build_graph(
        &[
            make_node("stub", ".init()", "test.rs"), // method stub
            simple_node("a"),
        ],
        &[simple_edge("stub", "a")],
    );
    let gods = god_nodes(&g, 5);
    assert!(gods.iter().all(|g| g.id != "stub"));
}

#[test]
fn surprising_connections_empty() {
    let g = KnowledgeGraph::new();
    let communities = HashMap::new();
    assert!(surprising_connections(&g, &communities, 5).is_empty());
}

#[test]
fn cross_community_edge_is_surprising() {
    let g = build_graph(
        &[simple_node("a"), simple_node("b")],
        &[simple_edge("a", "b")],
    );
    let mut communities = HashMap::new();
    communities.insert(0, vec!["a".into()]);
    communities.insert(1, vec!["b".into()]);
    let surprises = surprising_connections(&g, &communities, 10);
    assert!(!surprises.is_empty());
    assert_eq!(surprises[0].source_community, 0);
    assert_eq!(surprises[0].target_community, 1);
}

#[test]
fn ambiguous_edge_is_surprising() {
    let g = build_graph(
        &[simple_node("a"), simple_node("b")],
        &[make_edge("a", "b", "relates", Confidence::Ambiguous)],
    );
    let mut communities = HashMap::new();
    communities.insert(0, vec!["a".into(), "b".into()]);
    let surprises = surprising_connections(&g, &communities, 10);
    assert!(!surprises.is_empty());
}

#[test]
fn suggest_questions_empty() {
    let g = KnowledgeGraph::new();
    let qs = suggest_questions(&g, &HashMap::new(), &HashMap::new(), 10);
    assert!(qs.is_empty());
}

#[test]
fn suggest_questions_ambiguous_edge() {
    let g = build_graph(
        &[simple_node("a"), simple_node("b")],
        &[make_edge("a", "b", "relates", Confidence::Ambiguous)],
    );
    let mut communities = HashMap::new();
    communities.insert(0, vec!["a".into(), "b".into()]);
    let qs = suggest_questions(&g, &communities, &HashMap::new(), 10);
    let has_ambiguous = qs.iter().any(|q| {
        q.get("category")
            .map(|c| c == "ambiguous_relationship")
            .unwrap_or(false)
    });
    assert!(has_ambiguous);
}

#[test]
fn suggest_questions_isolated_node() {
    let g = build_graph(&[simple_node("lonely")], &[]);
    let communities = HashMap::new();
    let qs = suggest_questions(&g, &communities, &HashMap::new(), 10);
    let has_isolated = qs.iter().any(|q| {
        q.get("category")
            .map(|c| c == "isolated_node")
            .unwrap_or(false)
    });
    assert!(has_isolated);
}

#[test]
fn graph_diff_identical() {
    let g = build_graph(
        &[simple_node("a"), simple_node("b")],
        &[simple_edge("a", "b")],
    );
    let diff = graph_diff(&g, &g);
    let summary = diff.get("summary").unwrap();
    assert_eq!(summary["nodes_added"], 0);
    assert_eq!(summary["nodes_removed"], 0);
}

#[test]
fn graph_diff_added_node() {
    let old = build_graph(&[simple_node("a")], &[]);
    let new = build_graph(&[simple_node("a"), simple_node("b")], &[]);
    let diff = graph_diff(&old, &new);
    let summary = diff.get("summary").unwrap();
    assert_eq!(summary["nodes_added"], 1);
    assert_eq!(summary["nodes_removed"], 0);
}

#[test]
fn graph_diff_removed_node() {
    let old = build_graph(&[simple_node("a"), simple_node("b")], &[]);
    let new = build_graph(&[simple_node("a")], &[]);
    let diff = graph_diff(&old, &new);
    let summary = diff.get("summary").unwrap();
    assert_eq!(summary["nodes_removed"], 1);
}

#[test]
fn is_file_node_true() {
    let g = build_graph(&[make_node("f", "main.rs", "src/main.rs")], &[]);
    assert!(is_file_node(&g, "f"));
}

#[test]
fn is_file_node_false() {
    let g = build_graph(&[simple_node("a")], &[]);
    assert!(!is_file_node(&g, "a"));
}

#[test]
fn is_method_stub_true() {
    let g = build_graph(&[make_node("m", ".init()", "test.rs")], &[]);
    assert!(is_method_stub(&g, "m"));
}

#[test]
fn is_concept_node_no_source() {
    let g = build_graph(&[make_node("c", "SomeConcept", "")], &[]);
    assert!(is_concept_node(&g, "c"));
}

#[test]
fn god_nodes_disambiguates_lib_labels() {
    let mut n1 = make_node("lib1", "lib", "crates/graphify-export/src/lib.rs");
    n1.node_type = NodeType::Module;
    let mut n2 = make_node("lib2", "lib", "crates/graphify-analyze/src/lib.rs");
    n2.node_type = NodeType::Module;
    let a = simple_node("a");
    let b = simple_node("b");
    let c = simple_node("c");

    let g = build_graph(
        &[n1, n2, a, b, c],
        &[
            simple_edge("lib1", "a"),
            simple_edge("lib1", "b"),
            simple_edge("lib1", "c"),
            simple_edge("lib2", "a"),
            simple_edge("lib2", "b"),
        ],
    );

    let gods = god_nodes(&g, 5);
    let labels: Vec<&str> = gods.iter().map(|g| g.label.as_str()).collect();
    // Both should be disambiguated with crate name
    assert!(
        labels.contains(&"graphify-export::lib"),
        "missing graphify-export::lib in {labels:?}"
    );
    assert!(
        labels.contains(&"graphify-analyze::lib"),
        "missing graphify-analyze::lib in {labels:?}"
    );
}

#[test]
fn god_nodes_preserves_non_generic_labels() {
    let n = make_node("auth", "AuthService", "src/auth.rs");
    let a = simple_node("a");
    let b = simple_node("b");

    let g = build_graph(
        &[n, a, b],
        &[simple_edge("auth", "a"), simple_edge("auth", "b")],
    );

    let gods = god_nodes(&g, 5);
    assert!(gods.iter().any(|g| g.label == "AuthService"));
}

#[test]
fn community_bridges_finds_cross_community_nodes() {
    let mut a = simple_node("a");
    a.community = Some(0);
    let mut b = simple_node("b");
    b.community = Some(0);
    let mut c = simple_node("c");
    c.community = Some(1);
    let mut bridge = simple_node("bridge");
    bridge.community = Some(0);

    let g = build_graph(
        &[a, b, c, bridge.clone()],
        &[
            simple_edge("bridge", "a"),
            simple_edge("bridge", "b"),
            simple_edge("bridge", "c"),
        ],
    );

    let communities: HashMap<usize, Vec<String>> = [
        (0, vec!["a".into(), "b".into(), "bridge".into()]),
        (1, vec!["c".into()]),
    ]
    .into();

    let bridges = community_bridges(&g, &communities, 10);
    assert!(!bridges.is_empty(), "should find at least one bridge");
    // "bridge" and "c" are both bridge nodes; "c" has ratio 1.0, "bridge" has 0.33
    assert!(
        bridges.iter().any(|b| b.id == "bridge"),
        "bridge node should appear in results"
    );
    let bridge_entry = bridges.iter().find(|b| b.id == "bridge").unwrap();
    assert_eq!(bridge_entry.cross_community_edges, 1);
    assert_eq!(bridge_entry.total_edges, 3);
    assert!(bridge_entry.communities_touched.contains(&0));
    assert!(bridge_entry.communities_touched.contains(&1));
}

#[test]
fn community_bridges_empty_when_single_community() {
    let mut a = simple_node("a");
    a.community = Some(0);
    let mut b = simple_node("b");
    b.community = Some(0);

    let g = build_graph(&[a, b], &[simple_edge("a", "b")]);

    let communities: HashMap<usize, Vec<String>> = [(0, vec!["a".into(), "b".into()])].into();

    let bridges = community_bridges(&g, &communities, 10);
    assert!(bridges.is_empty(), "no bridges in single community");
}

#[test]
fn pagerank_empty_graph() {
    let g = KnowledgeGraph::new();
    let result = pagerank(&g, 10, 0.85, 20);
    assert!(result.is_empty());
}

#[test]
fn pagerank_star_topology() {
    let mut nodes = vec![simple_node("center")];
    let mut edges = vec![];
    for i in 0..5 {
        let id = format!("leaf{i}");
        nodes.push(simple_node(&id));
        edges.push(simple_edge("center", &id));
    }
    let g = build_graph(&nodes, &edges);
    let result = pagerank(&g, 10, 0.85, 20);
    assert!(!result.is_empty());
    assert_eq!(result[0].id, "center");
    assert!(result[0].score > result[1].score);
}

#[test]
fn pagerank_returns_top_n() {
    let nodes: Vec<_> = (0..20).map(|i| simple_node(&format!("n{i}"))).collect();
    let edges: Vec<_> = (0..19)
        .map(|i| simple_edge(&format!("n{i}"), &format!("n{}", i + 1)))
        .collect();
    let g = build_graph(&nodes, &edges);
    let result = pagerank(&g, 5, 0.85, 20);
    assert_eq!(result.len(), 5);
}

#[test]
fn detect_cycles_no_cycles() {
    let g = build_graph(
        &[simple_node("a"), simple_node("b"), simple_node("c")],
        &[simple_edge("a", "b"), simple_edge("b", "c")],
    );
    let cycles = detect_cycles(&g, 10);
    assert!(cycles.is_empty(), "tree should have no cycles");
}

#[test]
fn detect_cycles_finds_triangle() {
    let g = build_graph(
        &[simple_node("a"), simple_node("b"), simple_node("c")],
        &[
            simple_edge("a", "b"),
            simple_edge("b", "c"),
            simple_edge("c", "a"),
        ],
    );
    let cycles = detect_cycles(&g, 10);
    assert!(!cycles.is_empty(), "triangle should be detected as a cycle");
    assert!(cycles[0].nodes.len() >= 3);
    assert!((cycles[0].severity - 1.0 / 3.0).abs() < 0.01);
}

#[test]
fn detect_cycles_empty_graph() {
    let g = KnowledgeGraph::new();
    assert!(detect_cycles(&g, 10).is_empty());
}
