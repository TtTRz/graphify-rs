//! Compact LLM context-pack export.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use graphify_core::graph::KnowledgeGraph;
use graphify_core::model::{GraphNode, NodeType};
use graphify_core::quality;
use tracing::info;

const MAX_PROJECT_DOCS: usize = 8;
const MAX_COMMUNITIES: usize = 8;
const MAX_NODES_PER_COMMUNITY: usize = 8;
const MAX_DOC_TEXT_CHARS: usize = 700;

pub fn generate_llm_context(
    graph: &KnowledgeGraph,
    communities: &HashMap<usize, Vec<String>>,
    community_labels: &HashMap<usize, String>,
    root: &str,
) -> String {
    let mut out = String::with_capacity(12 * 1024);
    writeln!(out, "# LLM Context Pack").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "**Root:** `{root}`").unwrap();
    writeln!(
        out,
        "**Graph:** {} nodes, {} edges, {} communities",
        graph.node_count(),
        graph.edge_count(),
        communities.len()
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "> Compact, ranked context for agents. Full graph data stays in `graph.json`; query with `graphifyq` for focused subgraphs.").unwrap();
    writeln!(out).unwrap();

    write_project_context(&mut out, graph);
    write_entrypoints(&mut out, graph);
    write_communities(&mut out, graph, communities, community_labels);
    write_low_signal_summary(&mut out, graph);
    write_query_hints(&mut out);

    out
}

pub fn export_llm_context(context: &str, output_dir: &Path) -> anyhow::Result<PathBuf> {
    fs::create_dir_all(output_dir)?;
    let path = output_dir.join("LLM_CONTEXT.md");
    fs::write(&path, context)?;
    info!(path = %path.display(), "exported LLM context pack");
    Ok(path)
}

fn write_project_context(out: &mut String, graph: &KnowledgeGraph) {
    writeln!(out, "## Project Context").unwrap();
    writeln!(out).unwrap();

    let mut docs: Vec<&GraphNode> = graph
        .nodes()
        .into_iter()
        .filter(|n| n.node_type == NodeType::Concept)
        .filter(|n| {
            quality::node_flags(n)
                .iter()
                .any(|f| f == "project_context")
        })
        .filter(|n| {
            n.extra
                .get("doc_level")
                .and_then(|v| v.as_u64())
                .unwrap_or(9)
                <= 3
        })
        .collect();
    docs.sort_by(|a, b| {
        let a_rank = project_doc_rank(a);
        let b_rank = project_doc_rank(b);
        b_rank
            .cmp(&a_rank)
            .then_with(|| a.source_file.cmp(&b.source_file))
            .then_with(|| a.label.cmp(&b.label))
    });

    if docs.is_empty() {
        writeln!(out, "_No local project-context docs were indexed. Add README/PRODUCT/docs or allow `.planning` if needed._").unwrap();
        writeln!(out).unwrap();
        return;
    }

    for node in docs.into_iter().take(MAX_PROJECT_DOCS) {
        writeln!(out, "### {}", node.label).unwrap();
        writeln!(
            out,
            "- file: `{}`{}",
            node.source_file,
            node.source_location
                .as_ref()
                .map(|l| format!(" {l}"))
                .unwrap_or_default()
        )
        .unwrap();
        if let Some(text) = node.extra.get("doc_text").and_then(|v| v.as_str()) {
            let text = truncate_chars(text, MAX_DOC_TEXT_CHARS);
            for line in text.lines().take(8) {
                writeln!(out, "  {line}").unwrap();
            }
        }
        writeln!(out).unwrap();
    }
}

fn project_doc_rank(node: &GraphNode) -> i32 {
    let label = node.label.to_ascii_lowercase();
    let path = node.source_file.to_ascii_lowercase();
    let mut score = 0;
    if path.ends_with("product.md") || path.contains("project.md") || path.ends_with("readme.md") {
        score += 20;
    }
    if label.contains("purpose")
        || label.contains("goal")
        || label.contains("what this is")
        || label.contains("core value")
    {
        score += 30;
    }
    if label.contains("architecture") || label.contains("how it works") {
        score += 15;
    }
    let level = node
        .extra
        .get("doc_level")
        .and_then(|v| v.as_u64())
        .unwrap_or(9) as i32;
    score - level
}

fn write_entrypoints(out: &mut String, graph: &KnowledgeGraph) {
    writeln!(out, "## Primary Code Entrypoints").unwrap();
    writeln!(out).unwrap();

    let mut nodes: Vec<&GraphNode> = graph
        .nodes()
        .into_iter()
        .filter(|n| quality::is_summary_candidate(n))
        .filter(|n| {
            matches!(
                n.node_type,
                NodeType::Function
                    | NodeType::Method
                    | NodeType::Struct
                    | NodeType::Class
                    | NodeType::Interface
                    | NodeType::Trait
            )
        })
        .collect();
    nodes.sort_by(|a, b| {
        let a_score = (graph.degree(&a.id) as f32 * quality::node_priority(a) * 100.0) as i64;
        let b_score = (graph.degree(&b.id) as f32 * quality::node_priority(b) * 100.0) as i64;
        b_score.cmp(&a_score).then_with(|| a.label.cmp(&b.label))
    });

    for node in nodes.into_iter().take(12) {
        writeln!(
            out,
            "- `{}` — {} in `{}`{}",
            node.label,
            graph.degree(&node.id),
            node.source_file,
            node.source_location
                .as_ref()
                .map(|l| format!(" {l}"))
                .unwrap_or_default()
        )
        .unwrap();
    }
    writeln!(out).unwrap();
}

fn write_communities(
    out: &mut String,
    graph: &KnowledgeGraph,
    communities: &HashMap<usize, Vec<String>>,
    community_labels: &HashMap<usize, String>,
) {
    writeln!(out, "## Major Communities").unwrap();
    writeln!(out).unwrap();

    let mut ranked: Vec<(usize, f32, usize)> = communities
        .iter()
        .map(|(cid, members)| {
            let score = members
                .iter()
                .filter_map(|id| graph.get_node(id))
                .map(quality::node_priority)
                .sum::<f32>();
            (*cid, score, members.len())
        })
        .collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut emitted = 0usize;
    for (cid, _, size) in ranked {
        if emitted >= MAX_COMMUNITIES {
            break;
        }
        let mut nodes: Vec<&GraphNode> = communities
            .get(&cid)
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(|id| graph.get_node(id))
            .filter(|n| quality::is_summary_candidate(n))
            .collect();
        nodes.sort_by(|a, b| {
            let a_score = (graph.degree(&a.id) as f32 * quality::node_priority(a) * 100.0) as i64;
            let b_score = (graph.degree(&b.id) as f32 * quality::node_priority(b) * 100.0) as i64;
            b_score.cmp(&a_score).then_with(|| a.label.cmp(&b.label))
        });
        if nodes.is_empty() {
            continue;
        }
        let label = community_labels
            .get(&cid)
            .map(String::as_str)
            .unwrap_or("Unnamed");
        writeln!(out, "### Community {cid}: {label} ({size} nodes)").unwrap();
        emitted += 1;
        for node in nodes.into_iter().take(MAX_NODES_PER_COMMUNITY) {
            writeln!(
                out,
                "- `{}` ({:?}) — `{}`",
                node.label, node.node_type, node.source_file
            )
            .unwrap();
        }
        writeln!(out).unwrap();
    }
}

fn write_low_signal_summary(out: &mut String, graph: &KnowledgeGraph) {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for node in graph.nodes() {
        let kind = quality::node_source_kind(node);
        if kind != "source" && kind != "project_context" && kind != "schema" {
            *counts.entry(kind).or_default() += 1;
        }
    }

    writeln!(out, "## Downranked Low-Signal Sources").unwrap();
    writeln!(out).unwrap();
    if counts.is_empty() {
        writeln!(
            out,
            "_No generated/minified/build/test/dependency-heavy nodes detected._"
        )
        .unwrap();
    } else {
        for (kind, count) in counts {
            writeln!(out, "- {kind}: {count} nodes").unwrap();
        }
    }
    writeln!(out).unwrap();
}

fn write_query_hints(out: &mut String) {
    writeln!(out, "## Agent Workflow").unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "1. Read this file first for project intent and architecture map."
    )
    .unwrap();
    writeln!(
        out,
        "2. Use `graphifyq query \"specific question\"` to fetch a focused subgraph."
    )
    .unwrap();
    writeln!(out, "3. Then read exact source files with FFF/grepai/shell; do not paste full `graph.json` into LLM context.").unwrap();
    writeln!(out).unwrap();
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push('…');
    out
}
