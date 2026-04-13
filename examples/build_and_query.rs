//! Example: Build a knowledge graph and query it programmatically.
//!
//! ```bash
//! cargo run --example build_and_query
//! ```

use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    // 1. Collect source files from current directory
    let files = graphify_extract::collect_files(&PathBuf::from("."));
    println!("Found {} source files", files.len());

    // 2. Extract AST nodes and edges
    let extraction = graphify_extract::extract(&files);
    println!(
        "Extracted {} nodes, {} edges",
        extraction.nodes.len(),
        extraction.edges.len()
    );

    // 3. Build the knowledge graph
    let graph = graphify_build::build_from_extraction(&extraction)?;
    println!(
        "Graph: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );

    // 4. Run community detection
    let communities = graphify_cluster::cluster(&graph);
    println!("Detected {} communities", communities.len());

    // 5. Find the most important nodes
    let god_nodes = graphify_analyze::god_nodes(&graph, 5);
    println!("\nTop 5 God Nodes (by degree):");
    for gn in &god_nodes {
        println!("  - {} (degree: {})", gn.label, gn.degree);
    }

    // 6. Run PageRank for structural importance
    let pagerank = graphify_analyze::pagerank(&graph, 5, 0.85, 20);
    println!("\nTop 5 PageRank:");
    for pr in &pagerank {
        println!("  - {} (score: {:.4})", pr.label, pr.score);
    }

    // 7. Detect dependency cycles
    let cycles = graphify_analyze::detect_cycles(&graph, 5);
    if cycles.is_empty() {
        println!("\nNo dependency cycles found.");
    } else {
        println!("\nDependency Cycles:");
        for c in &cycles {
            println!(
                "  - {} nodes (severity: {:.2}): {:?}",
                c.nodes.len(),
                c.severity,
                c.nodes
            );
        }
    }

    // 8. Export
    let output_dir = PathBuf::from("graphify-out");
    let json_path = graphify_export::export_json(&graph, &output_dir)?;
    println!("\nExported to: {}", json_path.display());

    Ok(())
}
