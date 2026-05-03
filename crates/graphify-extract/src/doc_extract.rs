//! Local document extraction for Markdown/RST/plain text.
//!
//! This is deliberately LLM-free. It gives graphify a project-goal layer even
//! when `--no-llm` is used: headings become concept nodes, nearby prose is kept
//! as indexed text, and the document file is connected to its concepts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use graphify_core::confidence::Confidence;
use graphify_core::id::make_id;
use graphify_core::model::{ExtractionResult, GraphEdge, GraphNode, NodeType};
use graphify_core::quality;
use regex::Regex;
use tracing::{debug, warn};

const MAX_DOC_BYTES: usize = 512 * 1024;
const MAX_CONCEPT_TEXT_CHARS: usize = 1600;
const MAX_PLAIN_CHUNKS: usize = 12;

pub fn extract_documents(paths: &[PathBuf]) -> ExtractionResult {
    let mut combined = ExtractionResult::default();
    for path in paths {
        match extract_document(path) {
            Some(result) => {
                combined.nodes.extend(result.nodes);
                combined.edges.extend(result.edges);
                combined.hyperedges.extend(result.hyperedges);
            }
            None => debug!(path = %path.display(), "document extraction skipped"),
        }
    }
    combined
}

pub fn extract_document(path: &Path) -> Option<ExtractionResult> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(ext.as_str(), "md" | "mdx" | "rst" | "txt") {
        return None;
    }

    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() as usize > MAX_DOC_BYTES {
        warn!(path = %path.display(), bytes = metadata.len(), "document too large for local extraction");
        return None;
    }

    let content = std::fs::read_to_string(path).ok()?;
    if content.trim().is_empty() {
        return None;
    }

    if ext == "md" || ext == "mdx" {
        Some(extract_markdown(path, &content))
    } else {
        Some(extract_plain(path, &content))
    }
}

fn extract_markdown(path: &Path, content: &str) -> ExtractionResult {
    let mut result = ExtractionResult::default();
    let file_node = make_doc_file_node(path, "markdown");
    let file_id = file_node.id.clone();
    result.nodes.push(file_node);

    let heading_re = Regex::new(r"^(#{1,6})\s+(.+?)\s*$").unwrap();
    let mut current: Option<HeadingChunk> = None;
    let mut chunks = Vec::new();

    let mut in_fence = false;
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some(cap) = heading_re.captures(line) {
            if let Some(chunk) = current.take() {
                chunks.push(chunk);
            }
            let title = cleanup_heading(&cap[2]);
            if looks_like_shell_comment_heading(&title) {
                continue;
            }
            current = Some(HeadingChunk {
                level: cap[1].len(),
                title,
                line: idx + 1,
                body: Vec::new(),
            });
        } else if let Some(chunk) = current.as_mut() {
            chunk.body.push(line.to_string());
        }
    }
    if let Some(chunk) = current.take() {
        chunks.push(chunk);
    }

    if chunks.is_empty() {
        return extract_plain(path, content);
    }

    let mut previous_id: Option<String> = None;
    for chunk in chunks.into_iter().filter(|c| !c.title.trim().is_empty()) {
        let text = compact_text(&chunk.body.join("\n"), MAX_CONCEPT_TEXT_CHARS);
        let node_id = make_id(&[
            &path.to_string_lossy(),
            "heading",
            &chunk.line.to_string(),
            &chunk.title,
        ]);
        let mut extra = HashMap::new();
        let q =
            quality::classify_source(&path.to_string_lossy(), &chunk.title, &node_id, Some(&text));
        extra.insert("doc_level".into(), serde_json::json!(chunk.level));
        extra.insert("doc_text".into(), serde_json::json!(text));
        extra.insert("extractor".into(), serde_json::json!("local_markdown"));
        extra.insert(quality::EXTRA_SOURCE_KIND.into(), serde_json::json!(q.kind));
        extra.insert(
            quality::EXTRA_SOURCE_PRIORITY.into(),
            serde_json::json!(q.priority),
        );
        extra.insert(
            quality::EXTRA_SOURCE_FLAGS.into(),
            serde_json::json!(q.flags),
        );
        result.nodes.push(GraphNode {
            id: node_id.clone(),
            label: chunk.title,
            source_file: path.to_string_lossy().into_owned(),
            source_location: Some(format!("L{}", chunk.line)),
            node_type: NodeType::Concept,
            community: None,
            extra,
        });
        result.edges.push(make_edge(
            &file_id,
            &node_id,
            "contains",
            path,
            Confidence::Extracted,
        ));
        if let Some(prev) = previous_id {
            result.edges.push(make_edge(
                &prev,
                &node_id,
                "next_section",
                path,
                Confidence::Extracted,
            ));
        }
        previous_id = Some(node_id);
    }

    result
}

fn extract_plain(path: &Path, content: &str) -> ExtractionResult {
    let mut result = ExtractionResult::default();
    let file_node = make_doc_file_node(path, "text");
    let file_id = file_node.id.clone();
    result.nodes.push(file_node);

    for (idx, chunk) in paragraph_chunks(content)
        .into_iter()
        .take(MAX_PLAIN_CHUNKS)
        .enumerate()
    {
        let title = if idx == 0 {
            file_stem(path)
        } else {
            format!("{} part {}", file_stem(path), idx + 1)
        };
        let line = line_for_offset(content, &chunk).unwrap_or(1);
        let node_id = make_id(&[
            &path.to_string_lossy(),
            "text",
            &(idx + 1).to_string(),
            &title,
        ]);
        let text = compact_text(&chunk, MAX_CONCEPT_TEXT_CHARS);
        let mut extra = HashMap::new();
        let q = quality::classify_source(&path.to_string_lossy(), &title, &node_id, Some(&text));
        extra.insert("doc_text".into(), serde_json::json!(text));
        extra.insert("extractor".into(), serde_json::json!("local_text"));
        extra.insert(quality::EXTRA_SOURCE_KIND.into(), serde_json::json!(q.kind));
        extra.insert(
            quality::EXTRA_SOURCE_PRIORITY.into(),
            serde_json::json!(q.priority),
        );
        extra.insert(
            quality::EXTRA_SOURCE_FLAGS.into(),
            serde_json::json!(q.flags),
        );
        result.nodes.push(GraphNode {
            id: node_id.clone(),
            label: title,
            source_file: path.to_string_lossy().into_owned(),
            source_location: Some(format!("L{line}")),
            node_type: NodeType::Concept,
            community: None,
            extra,
        });
        result.edges.push(make_edge(
            &file_id,
            &node_id,
            "contains",
            path,
            Confidence::Extracted,
        ));
    }

    result
}

#[derive(Debug)]
struct HeadingChunk {
    level: usize,
    title: String,
    line: usize,
    body: Vec<String>,
}

fn make_doc_file_node(path: &Path, kind: &str) -> GraphNode {
    let ps = path.to_string_lossy().into_owned();
    let label = file_stem(path);
    let mut extra = HashMap::new();
    let q = quality::classify_source(&ps, &label, &ps, None);
    extra.insert("document_kind".into(), serde_json::json!(kind));
    extra.insert("extractor".into(), serde_json::json!("local_document"));
    extra.insert(quality::EXTRA_SOURCE_KIND.into(), serde_json::json!(q.kind));
    extra.insert(
        quality::EXTRA_SOURCE_PRIORITY.into(),
        serde_json::json!(q.priority),
    );
    extra.insert(
        quality::EXTRA_SOURCE_FLAGS.into(),
        serde_json::json!(q.flags),
    );
    GraphNode {
        id: make_id(&[&ps]),
        label,
        source_file: ps,
        source_location: None,
        node_type: NodeType::File,
        community: None,
        extra,
    }
}

fn make_edge(
    source_id: &str,
    target_id: &str,
    relation: &str,
    path: &Path,
    confidence: Confidence,
) -> GraphEdge {
    GraphEdge {
        source: source_id.to_string(),
        target: target_id.to_string(),
        relation: relation.to_string(),
        confidence: confidence.clone(),
        confidence_score: confidence.default_score(),
        source_file: path.to_string_lossy().into_owned(),
        source_location: None,
        weight: 1.0,
        extra: HashMap::new(),
    }
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("document")
        .to_string()
}

fn looks_like_shell_comment_heading(title: &str) -> bool {
    let lower = title.to_ascii_lowercase();
    lower.starts_with('(')
        || lower.starts_with("install ")
        || lower.starts_with("build a ")
        || lower.starts_with("query the ")
        || lower.starts_with("explore ")
        || lower.starts_with("short-lived ")
        || lower.starts_with("optional embedding")
        || lower.starts_with("model2vec semantic")
}

fn cleanup_heading(raw: &str) -> String {
    raw.trim()
        .trim_matches('#')
        .trim()
        .trim_matches('`')
        .trim()
        .to_string()
}

fn paragraph_chunks(content: &str) -> Vec<String> {
    content
        .split("\n\n")
        .map(|p| p.trim())
        .filter(|p| p.len() > 40)
        .map(str::to_string)
        .collect()
}

fn compact_text(text: &str, max_chars: usize) -> String {
    let mut out = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if out.chars().count() > max_chars {
        out = out.chars().take(max_chars).collect();
        out.push('…');
    }
    out
}

fn line_for_offset(content: &str, needle: &str) -> Option<usize> {
    let offset = content.find(needle)?;
    Some(content[..offset].lines().count() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_headings_become_concepts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("PRODUCT.md");
        std::fs::write(
            &path,
            "# Product\n\nPurpose text.\n\n```bash\n# Not a heading\n```\n\n## Product Purpose\n\nTelegram ledger and reconciliation workflow.",
        )
        .unwrap();

        let result = extract_document(&path).unwrap();
        assert!(result.nodes.iter().any(|n| n.label == "Product Purpose"));
        let purpose = result
            .nodes
            .iter()
            .find(|n| n.label == "Product Purpose")
            .unwrap();
        assert_eq!(purpose.node_type, NodeType::Concept);
        assert_eq!(purpose.extra.get("extractor").unwrap(), "local_markdown");
    }
}
