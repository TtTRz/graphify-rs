//! GRAPH_REPORT.md generation.

use std::collections::HashMap;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use graphify_core::confidence::Confidence;
use graphify_core::graph::KnowledgeGraph;
use tracing::info;

/// Generate a comprehensive markdown analysis report.
#[allow(clippy::too_many_arguments)]
pub fn generate_report(
    graph: &KnowledgeGraph,
    communities: &HashMap<usize, Vec<String>>,
    cohesion_scores: &HashMap<usize, f64>,
    community_labels: &HashMap<usize, String>,
    god_nodes: &[serde_json::Value],
    surprises: &[serde_json::Value],
    detection_result: &serde_json::Value,
    token_cost: &HashMap<String, usize>,
    root: &str,
    suggested_questions: Option<&[serde_json::Value]>,
) -> anyhow::Result<String> {
    let mut report = String::with_capacity(8192);

    // Header
    writeln!(report, "# 📊 Graph Analysis Report")?;
    writeln!(report)?;
    writeln!(report, "**Root:** `{}`", root)?;
    writeln!(report)?;

    // Summary
    writeln!(report, "## Summary")?;
    writeln!(report)?;

    let node_count = graph.node_count();
    let edge_count = graph.edge_count();
    let community_count = communities.len();

    writeln!(report, "| Metric | Value |")?;
    writeln!(report, "|--------|-------|")?;
    writeln!(report, "| Nodes | {} |", node_count)?;
    writeln!(report, "| Edges | {} |", edge_count)?;
    writeln!(report, "| Communities | {} |", community_count)?;
    writeln!(report, "| Hyperedges | {} |", graph.hyperedges.len())?;
    writeln!(report)?;

    // Confidence breakdown
    let mut extracted = 0usize;
    let mut inferred = 0usize;
    let mut ambiguous = 0usize;
    for edge in graph.edges() {
        match edge.confidence {
            Confidence::Extracted => extracted += 1,
            Confidence::Inferred => inferred += 1,
            Confidence::Ambiguous => ambiguous += 1,
        }
    }
    writeln!(report, "### Confidence Breakdown")?;
    writeln!(report)?;
    writeln!(report, "| Level | Count | Percentage |")?;
    writeln!(report, "|-------|-------|------------|")?;
    let total = (extracted + inferred + ambiguous).max(1);
    writeln!(
        report,
        "| EXTRACTED | {} | {:.1}% |",
        extracted,
        extracted as f64 / total as f64 * 100.0
    )
    ?;
    writeln!(
        report,
        "| INFERRED | {} | {:.1}% |",
        inferred,
        inferred as f64 / total as f64 * 100.0
    )
    ?;
    writeln!(
        report,
        "| AMBIGUOUS | {} | {:.1}% |",
        ambiguous,
        ambiguous as f64 / total as f64 * 100.0
    )
    ?;
    writeln!(report)?;

    // God Nodes
    writeln!(report, "## 🌟 God Nodes (Most Connected)")?;
    writeln!(report)?;
    if god_nodes.is_empty() {
        writeln!(report, "_No god nodes detected._")?;
    } else {
        writeln!(report, "| Node | Degree | Community |")?;
        writeln!(report, "|------|--------|-----------|")?;
        for gn in god_nodes {
            let label = gn.get("label").and_then(|v| v.as_str()).unwrap_or("?");
            let degree = gn.get("degree").and_then(|v| v.as_u64()).unwrap_or(0);
            let comm = gn
                .get("community")
                .and_then(|v| v.as_u64())
                .map(|c| c.to_string())
                .unwrap_or_else(|| "–".into());
            writeln!(report, "| {} | {} | {} |", label, degree, comm)?;
        }
    }
    writeln!(report)?;

    // Surprising Connections
    writeln!(report, "## 🔮 Surprising Connections")?;
    writeln!(report)?;
    if surprises.is_empty() {
        writeln!(report, "_No surprising connections found._")?;
    } else {
        for s in surprises {
            let src = s.get("source").and_then(|v| v.as_str()).unwrap_or("?");
            let tgt = s.get("target").and_then(|v| v.as_str()).unwrap_or("?");
            let rel = s.get("relation").and_then(|v| v.as_str()).unwrap_or("?");
            writeln!(report, "- **{}** → **{}** ({})", src, tgt, rel)?;
        }
    }
    writeln!(report)?;

    // Hyperedges
    if !graph.hyperedges.is_empty() {
        writeln!(report, "## 🔗 Hyperedges")?;
        writeln!(report)?;
        for he in &graph.hyperedges {
            writeln!(
                report,
                "- **{}**: {} (nodes: {})",
                he.relation,
                he.label,
                he.nodes.join(", ")
            )
            ?;
        }
        writeln!(report)?;
    }

    // Communities
    writeln!(report, "## 🏘️ Communities")?;
    writeln!(report)?;
    let mut sorted_communities: Vec<_> = communities.iter().collect();
    sorted_communities.sort_by_key(|(cid, _)| **cid);
    for (cid, members) in &sorted_communities {
        let label = community_labels
            .get(cid)
            .map(|s| s.as_str())
            .unwrap_or("Unnamed");
        let cohesion = cohesion_scores.get(cid).copied().unwrap_or(0.0);
        writeln!(
            report,
            "### Community {} — {} ({} nodes, cohesion: {:.2})",
            cid,
            label,
            members.len(),
            cohesion
        )
        ?;
        writeln!(report)?;
        for nid in members.iter().take(20) {
            let node_label = graph
                .get_node(nid)
                .map(|n| n.label.as_str())
                .unwrap_or(nid.as_str());
            writeln!(report, "- {}", node_label)?;
        }
        if members.len() > 20 {
            writeln!(report, "- _…and {} more_", members.len() - 20)?;
        }
        writeln!(report)?;
    }

    // Ambiguous Edges
    if ambiguous > 0 {
        writeln!(report, "## ⚠️ Ambiguous Edges")?;
        writeln!(report)?;
        let mut count = 0;
        for edge in graph.edges() {
            if edge.confidence == Confidence::Ambiguous {
                writeln!(
                    report,
                    "- {} → {} ({}, score: {:.2})",
                    edge.source, edge.target, edge.relation, edge.confidence_score
                )
                ?;
                count += 1;
                if count >= 30 {
                    writeln!(report, "- _…and more_")?;
                    break;
                }
            }
        }
        writeln!(report)?;
    }

    // Knowledge Gaps
    writeln!(report, "## 🕳️ Knowledge Gaps")?;
    writeln!(report)?;

    // Isolated nodes (degree 0)
    let isolated: Vec<_> = graph
        .nodes()
        .iter()
        .filter(|n| graph.degree(&n.id) == 0)
        .map(|n| n.label.as_str())
        .collect();
    if isolated.is_empty() {
        writeln!(report, "No isolated nodes.")?;
    } else {
        writeln!(report, "**Isolated nodes** ({}):", isolated.len())?;
        for label in isolated.iter().take(20) {
            writeln!(report, "- {}", label)?;
        }
        if isolated.len() > 20 {
            writeln!(report, "- _…and {} more_", isolated.len() - 20)?;
        }
    }
    writeln!(report)?;

    // Thin communities (< 3 nodes)
    let thin: Vec<_> = communities
        .iter()
        .filter(|(_, members)| members.len() < 3)
        .collect();
    if !thin.is_empty() {
        writeln!(
            report,
            "**Thin communities** (< 3 nodes): {} communities",
            thin.len()
        )
        ?;
        writeln!(report)?;
    }

    // Detection result info
    if let Some(method) = detection_result.get("method").and_then(|v| v.as_str()) {
        writeln!(report, "**Community detection method:** {}", method)?;
        writeln!(report)?;
    }

    // Token cost
    if !token_cost.is_empty() {
        writeln!(report, "## 💰 Token Cost")?;
        writeln!(report)?;
        writeln!(report, "| File | Tokens |")?;
        writeln!(report, "|------|--------|")?;
        let mut total_tokens = 0usize;
        for (file, &tokens) in token_cost {
            writeln!(report, "| {} | {} |", file, tokens)?;
            total_tokens += tokens;
        }
        writeln!(report, "| **Total** | **{}** |", total_tokens)?;
        writeln!(report)?;
    }

    // Suggested Questions
    if let Some(questions) = suggested_questions
        && !questions.is_empty()
    {
        writeln!(report, "## ❓ Suggested Questions")?;
        writeln!(report)?;
        for q in questions {
            if let Some(text) = q.as_str() {
                writeln!(report, "1. {}", text)?;
            } else if let Some(text) = q.get("question").and_then(|v| v.as_str()) {
                writeln!(report, "1. {}", text)?;
            }
        }
        writeln!(report)?;
    }

    writeln!(report, "---")?;
    writeln!(report, "_Generated by graphify-rs_")?;
    Ok(report)
}

/// Write the report string to `GRAPH_REPORT.md`.
pub fn export_report(report: &str, output_dir: &Path) -> anyhow::Result<PathBuf> {
    fs::create_dir_all(output_dir)?;
    let path = output_dir.join("GRAPH_REPORT.md");
    fs::write(&path, report)?;
    info!(path = %path.display(), "exported analysis report");
    Ok(path)
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
            community: Some(0),
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
    fn generate_report_contains_sections() {
        let kg = sample_graph();
        let communities: HashMap<usize, Vec<String>> = [(0, vec!["a".into(), "b".into()])].into();
        let cohesion: HashMap<usize, f64> = [(0, 0.9)].into();
        let labels: HashMap<usize, String> = [(0, "Core".into())].into();

        let report = generate_report(
            &kg,
            &communities,
            &cohesion,
            &labels,
            &[],
            &[],
            &serde_json::json!({}),
            &HashMap::new(),
            "/test",
            None,
        ).unwrap();

        assert!(report.contains("# 📊 Graph Analysis Report"));
        assert!(report.contains("## Summary"));
        assert!(report.contains("| Nodes | 2 |"));
        assert!(report.contains("## 🏘️ Communities"));
        assert!(report.contains("Core"));
    }

    #[test]
    fn export_report_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = export_report("# Test Report\n", dir.path()).unwrap();
        assert!(path.exists());
        assert!(path.ends_with("GRAPH_REPORT.md"));
    }
}
