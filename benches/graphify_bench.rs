//! Criterion benchmarks for graphify-rs core operations.
//!
//! Run: `cargo bench`

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use graphify_core::confidence::Confidence;
use graphify_core::graph::KnowledgeGraph;
use graphify_core::model::{GraphEdge, GraphNode, NodeType};
use std::collections::HashMap;

fn make_node(id: &str) -> GraphNode {
    GraphNode {
        id: id.into(),
        label: id.into(),
        source_file: "bench.rs".into(),
        source_location: None,
        node_type: NodeType::Function,
        community: None,
        extra: HashMap::new(),
    }
}

fn make_edge(src: &str, tgt: &str) -> GraphEdge {
    GraphEdge {
        source: src.into(),
        target: tgt.into(),
        relation: "calls".into(),
        confidence: Confidence::Extracted,
        confidence_score: 1.0,
        source_file: "bench.rs".into(),
        source_location: None,
        weight: 1.0,
        extra: HashMap::new(),
    }
}

fn build_test_graph(n: usize) -> KnowledgeGraph {
    let mut g = KnowledgeGraph::new();
    for i in 0..n {
        let _ = g.add_node(make_node(&format!("n{i}")));
    }
    // Create a connected graph: chain + some cross-links
    for i in 0..n.saturating_sub(1) {
        let _ = g.add_edge(make_edge(&format!("n{i}"), &format!("n{}", i + 1)));
    }
    // Add some cross-links for community structure
    for i in (0..n).step_by(5) {
        let j = (i + n / 3) % n;
        let _ = g.add_edge(make_edge(&format!("n{i}"), &format!("n{j}")));
    }
    g
}

fn bench_cluster(c: &mut Criterion) {
    let g = build_test_graph(500);
    c.bench_function("leiden_500_nodes", |b| {
        b.iter(|| graphify_cluster::cluster(black_box(&g)))
    });
}

fn bench_pagerank(c: &mut Criterion) {
    let g = build_test_graph(1000);
    c.bench_function("pagerank_1000_nodes", |b| {
        b.iter(|| graphify_analyze::pagerank(black_box(&g), 10, 0.85, 20))
    });
}

fn bench_detect_cycles(c: &mut Criterion) {
    let g = build_test_graph(1000);
    c.bench_function("detect_cycles_1000_nodes", |b| {
        b.iter(|| graphify_analyze::detect_cycles(black_box(&g), 10))
    });
}

fn bench_god_nodes(c: &mut Criterion) {
    let g = build_test_graph(1000);
    c.bench_function("god_nodes_1000", |b| {
        b.iter(|| graphify_analyze::god_nodes(black_box(&g), 10))
    });
}

fn bench_json_export(c: &mut Criterion) {
    let g = build_test_graph(500);
    c.bench_function("json_streaming_500_nodes", |b| {
        b.iter(|| {
            let mut buf = Vec::with_capacity(64 * 1024);
            g.write_node_link_json(&mut buf).unwrap();
            black_box(buf);
        })
    });
}

fn bench_extraction(c: &mut Criterion) {
    let source = r#"
import os
from pathlib import Path

class Config:
    def __init__(self, name):
        self.name = name
    def validate(self):
        return len(self.name) > 0

def main():
    c = Config("test")
    c.validate()
"#;
    c.bench_function("extract_python_file", |b| {
        b.iter(|| {
            graphify_extract::ast_extract::extract_file(
                black_box(std::path::Path::new("bench.py")),
                black_box(source),
                "python",
            )
        })
    });
}

criterion_group!(
    benches,
    bench_cluster,
    bench_pagerank,
    bench_detect_cycles,
    bench_god_nodes,
    bench_json_export,
    bench_extraction,
);
criterion_main!(benches);
