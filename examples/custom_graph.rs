//! Example: Build a custom knowledge graph programmatically.
//!
//! Shows how to create nodes and edges directly without file extraction.
//!
//! ```bash
//! cargo run --example custom_graph
//! ```

use graphify_core::confidence::Confidence;
use graphify_core::graph::KnowledgeGraph;
use graphify_core::model::{GraphEdge, GraphNode, NodeType};
use std::collections::HashMap;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let mut graph = KnowledgeGraph::new();

    // Create nodes representing a microservice architecture
    let services = [
        ("api-gateway", NodeType::Module),
        ("user-service", NodeType::Module),
        ("order-service", NodeType::Module),
        ("payment-service", NodeType::Module),
        ("notification-service", NodeType::Module),
        ("auth-middleware", NodeType::Function),
        ("database", NodeType::Struct),
    ];

    for (name, node_type) in &services {
        graph.add_node(GraphNode {
            id: name.to_string(),
            label: name.to_string(),
            source_file: "architecture.md".into(),
            source_location: None,
            node_type: node_type.clone(),
            community: None,
            extra: HashMap::new(),
        })?;
    }

    // Define relationships
    let edges = [
        (
            "api-gateway",
            "user-service",
            "routes_to",
            Confidence::Extracted,
        ),
        (
            "api-gateway",
            "order-service",
            "routes_to",
            Confidence::Extracted,
        ),
        (
            "api-gateway",
            "auth-middleware",
            "uses",
            Confidence::Extracted,
        ),
        (
            "order-service",
            "payment-service",
            "calls",
            Confidence::Extracted,
        ),
        (
            "order-service",
            "notification-service",
            "calls",
            Confidence::Inferred,
        ),
        (
            "user-service",
            "database",
            "reads_from",
            Confidence::Extracted,
        ),
        (
            "order-service",
            "database",
            "reads_from",
            Confidence::Extracted,
        ),
        (
            "payment-service",
            "notification-service",
            "calls",
            Confidence::Inferred,
        ),
    ];

    for (src, tgt, rel, conf) in &edges {
        graph.add_edge(GraphEdge {
            source: src.to_string(),
            target: tgt.to_string(),
            relation: rel.to_string(),
            confidence: conf.clone(),
            confidence_score: conf.default_score(),
            source_file: "architecture.md".into(),
            source_location: None,
            weight: 1.0,
            extra: HashMap::new(),
        })?;
    }

    println!(
        "Graph: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );

    // Run analysis
    let god_nodes = graphify_analyze::god_nodes(&graph, 3);
    println!("\nMost connected services:");
    for gn in &god_nodes {
        println!("  - {} (degree: {})", gn.label, gn.degree);
    }

    let pagerank = graphify_analyze::pagerank(&graph, 3, 0.85, 20);
    println!("\nMost important services (PageRank):");
    for pr in &pagerank {
        println!("  - {} (score: {:.4})", pr.label, pr.score);
    }

    let cycles = graphify_analyze::detect_cycles(&graph, 5);
    println!("\nDependency cycles: {}", cycles.len());

    // Export
    let output = PathBuf::from(".graphify");
    graphify_export::export_json(&graph, &output)?;
    println!("\nExported to .graphify/graph.json");

    Ok(())
}
