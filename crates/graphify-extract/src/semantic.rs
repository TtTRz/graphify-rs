//! Semantic extraction via Claude API (Pass 2).
//!
//! Extracts higher-level concepts and relationships from documents, papers, and
//! images using the Anthropic Messages API. This is the second pass of the
//! extraction pipeline — it complements the deterministic AST extraction from
//! Pass 1 by discovering semantic relationships that cannot be inferred from
//! syntax alone.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use graphify_core::confidence::Confidence;
use graphify_core::id::make_id;
use graphify_core::model::{ExtractionResult, GraphEdge, GraphNode, NodeType};
use serde::{Deserialize, Serialize};
use tracing::debug;

// ---------------------------------------------------------------------------
// Claude API request/response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct MessageRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<Message>,
    system: String,
}

#[derive(Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct MessageResponse {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    text: Option<String>,
}

/// Entities and relationships extracted by the LLM.
#[derive(Deserialize, Debug)]
struct SemanticOutput {
    #[serde(default)]
    entities: Vec<SemanticEntity>,
    #[serde(default)]
    relationships: Vec<SemanticRelation>,
}

#[derive(Deserialize, Debug)]
struct SemanticEntity {
    name: String,
    #[serde(default = "default_entity_type")]
    entity_type: String,
}

fn default_entity_type() -> String {
    "concept".to_string()
}

#[derive(Deserialize, Debug)]
struct SemanticRelation {
    source: String,
    target: String,
    #[serde(default = "default_relation")]
    relation: String,
}

fn default_relation() -> String {
    "related_to".to_string()
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Extract semantic concepts from a document, paper, or image using the Claude API.
///
/// # Arguments
/// * `path` — the file path (used for source_file metadata)
/// * `content` — the text content to analyse
/// * `file_type` — one of `"document"`, `"paper"`, or `"image"`
/// * `api_key` — Anthropic API key
///
/// # Errors
/// Returns an error if the HTTP request fails or the response cannot be parsed.
pub async fn extract_semantic(
    path: &Path,
    content: &str,
    file_type: &str,
    api_key: &str,
) -> Result<ExtractionResult> {
    let file_str = path.to_string_lossy();
    let system_prompt = build_system_prompt(file_type);
    let user_prompt = build_user_prompt(content, file_type);

    debug!("sending semantic extraction request for {}", file_str);

    let request_body = MessageRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 4096,
        messages: vec![Message {
            role: "user".to_string(),
            content: user_prompt,
        }],
        system: system_prompt,
    };

    let client = reqwest::Client::new();
    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&request_body)
        .send()
        .await
        .context("failed to send request to Claude API")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Claude API returned {status}: {body}");
    }

    let msg: MessageResponse = response
        .json()
        .await
        .context("failed to parse Claude API response")?;

    let text = msg
        .content
        .first()
        .and_then(|b| b.text.as_deref())
        .unwrap_or("{}");

    parse_semantic_response(text, &file_str)
}

/// Extract semantic concepts by running a user-provided local LLM CLI command.
///
/// The command is executed through the platform shell with the extraction prompt
/// written to stdin. It should write the same JSON shape as the Anthropic path:
/// `{ "entities": [...], "relationships": [...] }`.
pub fn extract_semantic_with_cli(
    path: &Path,
    content: &str,
    file_type: &str,
    command: &str,
    existing: Option<&ExtractionResult>,
) -> Result<ExtractionResult> {
    let file_str = path.to_string_lossy();
    let prompt = build_cli_prompt(content, file_type, existing);

    debug!("running semantic extraction command for {}", file_str);

    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .env("GRAPHIFY_FILE", file_str.as_ref())
        .env("GRAPHIFY_FILE_TYPE", file_type)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to start LLM command `{command}`"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(prompt.as_bytes())
            .context("failed to write prompt to LLM command stdin")?;
    }

    let output = child
        .wait_with_output()
        .context("failed to wait for LLM command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "LLM command exited with status {}: {}",
            output.status,
            stderr.trim()
        );
    }

    let stdout = String::from_utf8(output.stdout).context("LLM command stdout was not UTF-8")?;
    parse_semantic_response(&stdout, &file_str)
}

// ---------------------------------------------------------------------------
// Prompt construction
// ---------------------------------------------------------------------------

fn build_system_prompt(file_type: &str) -> String {
    format!(
        "You are an expert knowledge-graph extraction engine. \
         Given a {file_type}, extract entities and their relationships. \
         Respond ONLY with a JSON object having two arrays: \
         \"entities\" (each with \"name\" and \"entity_type\") and \
         \"relationships\" (each with \"source\", \"target\", and \"relation\"). \
         Entity types should be one of: concept, class, function, module, paper, image. \
         Keep entity names concise and unique."
    )
}

fn build_user_prompt(content: &str, file_type: &str) -> String {
    // Truncate very long content
    let max_chars = 100_000;
    let truncated = if content.len() > max_chars {
        let mut end = max_chars;
        while end > 0 && !content.is_char_boundary(end) {
            end -= 1;
        }
        &content[..end]
    } else {
        content
    };

    format!("Extract all entities and relationships from this {file_type}:\n\n{truncated}")
}

fn build_cli_prompt(content: &str, file_type: &str, existing: Option<&ExtractionResult>) -> String {
    let system = build_system_prompt(file_type);
    let user = build_user_prompt(content, file_type);
    let existing_json = existing
        .and_then(|result| serde_json::to_string_pretty(result).ok())
        .unwrap_or_else(|| "null".to_string());

    format!(
        "{system}\n\n\
         You are running as an external local CLI for graphify-rs. \
         Return ONLY the JSON object, with no markdown and no commentary. \
         If existing_extraction is not null, treat it as the previous graphify \
         extraction for this source file: update it for the current content, \
         preserve stable concise entity names where still valid, remove stale \
         relationships, and add newly discovered entities/relationships.\n\n\
         existing_extraction:\n{existing_json}\n\n\
         {user}"
    )
}

// ---------------------------------------------------------------------------
// Response parsing
// ---------------------------------------------------------------------------

pub fn parse_semantic_response(text: &str, file_str: &str) -> Result<ExtractionResult> {
    // Try to find JSON in the response (might be wrapped in markdown fences)
    let json_str = extract_json_block(text);

    let output: SemanticOutput =
        serde_json::from_str(json_str).context("failed to parse semantic extraction JSON")?;

    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // Convert entities to nodes
    let mut name_to_id: HashMap<String, String> = HashMap::new();
    for entity in &output.entities {
        let id = make_id(&[file_str, &entity.name]);
        let node_type = match entity.entity_type.as_str() {
            "class" => NodeType::Class,
            "function" => NodeType::Function,
            "module" => NodeType::Module,
            "paper" => NodeType::Paper,
            "image" => NodeType::Image,
            _ => NodeType::Concept,
        };
        name_to_id.insert(entity.name.clone(), id.clone());
        nodes.push(GraphNode {
            id,
            label: entity.name.clone(),
            source_file: file_str.to_string(),
            source_location: None,
            node_type,
            community: None,
            extra: HashMap::new(),
        });
    }

    // Convert relationships to edges
    for rel in &output.relationships {
        let source_id = name_to_id
            .get(&rel.source)
            .cloned()
            .unwrap_or_else(|| make_id(&[file_str, &rel.source]));
        let target_id = name_to_id
            .get(&rel.target)
            .cloned()
            .unwrap_or_else(|| make_id(&[file_str, &rel.target]));

        edges.push(GraphEdge {
            source: source_id,
            target: target_id,
            relation: rel.relation.clone(),
            confidence: Confidence::Inferred,
            confidence_score: Confidence::Inferred.default_score(),
            source_file: file_str.to_string(),
            source_location: None,
            weight: 1.0,
            extra: HashMap::new(),
        });
    }

    Ok(ExtractionResult {
        nodes,
        edges,
        hyperedges: Vec::new(),
    })
}

/// Extract a JSON block from text that might be wrapped in markdown fences.
fn extract_json_block(text: &str) -> &str {
    // Try to find ```json ... ``` block
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim();
        }
    }
    // Try to find ``` ... ``` block
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        if let Some(end) = after.find("```") {
            return after[..end].trim();
        }
    }
    // Try to find { ... } directly
    if let Some(start) = text.find('{')
        && let Some(end) = text.rfind('}')
    {
        return &text[start..=end];
    }
    text.trim()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_semantic_json() {
        let json = r#"{
            "entities": [
                {"name": "Machine Learning", "entity_type": "concept"},
                {"name": "Neural Network", "entity_type": "concept"},
                {"name": "Backpropagation", "entity_type": "concept"}
            ],
            "relationships": [
                {"source": "Neural Network", "target": "Machine Learning", "relation": "is_a"},
                {"source": "Backpropagation", "target": "Neural Network", "relation": "used_by"}
            ]
        }"#;

        let result = parse_semantic_response(json, "paper.pdf").unwrap();
        assert_eq!(result.nodes.len(), 3);
        assert_eq!(result.edges.len(), 2);
        assert!(
            result
                .nodes
                .iter()
                .all(|n| n.node_type == NodeType::Concept)
        );
        assert_eq!(result.edges[0].relation, "is_a");
    }

    #[test]
    fn parse_markdown_wrapped_json() {
        let text = r#"Here is the extraction:
```json
{
    "entities": [{"name": "Foo", "entity_type": "class"}],
    "relationships": []
}
```
"#;
        let result = parse_semantic_response(text, "doc.md").unwrap();
        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].label, "Foo");
        assert_eq!(result.nodes[0].node_type, NodeType::Class);
    }

    #[test]
    fn parse_empty_response() {
        let json = r#"{"entities": [], "relationships": []}"#;
        let result = parse_semantic_response(json, "empty.txt").unwrap();
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn extract_json_block_plain() {
        assert_eq!(extract_json_block(r#"{"a": 1}"#), r#"{"a": 1}"#);
    }

    #[test]
    fn extract_json_block_fenced() {
        let text = "blah\n```json\n{\"a\": 1}\n```\nmore";
        assert_eq!(extract_json_block(text), r#"{"a": 1}"#);
    }

    #[test]
    fn semantic_edges_are_inferred_confidence() {
        let json = r#"{
            "entities": [
                {"name": "A", "entity_type": "concept"},
                {"name": "B", "entity_type": "concept"}
            ],
            "relationships": [
                {"source": "A", "target": "B", "relation": "depends_on"}
            ]
        }"#;
        let result = parse_semantic_response(json, "test.md").unwrap();
        assert_eq!(result.edges[0].confidence, Confidence::Inferred);
    }

    #[test]
    fn build_prompts_contain_file_type() {
        let sys = build_system_prompt("paper");
        assert!(sys.contains("paper"));

        let user = build_user_prompt("hello world", "document");
        assert!(user.contains("document"));
        assert!(user.contains("hello world"));
    }

    #[test]
    fn cli_prompt_includes_existing_extraction() {
        let existing = ExtractionResult {
            nodes: vec![GraphNode {
                id: "old".into(),
                label: "Old".into(),
                source_file: "doc.md".into(),
                source_location: None,
                node_type: NodeType::Concept,
                community: None,
                extra: HashMap::new(),
            }],
            edges: Vec::new(),
            hyperedges: Vec::new(),
        };

        let prompt = build_cli_prompt("new content", "document", Some(&existing));
        assert!(prompt.contains("existing_extraction"));
        assert!(prompt.contains("\"label\": \"Old\""));
        assert!(prompt.contains("Return ONLY the JSON object"));
    }
}
