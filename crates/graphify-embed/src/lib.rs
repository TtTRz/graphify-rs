//! Semantic embedding index for graphify knowledge graphs.
//!
//! The index is intentionally stored outside `graph.json`: vectors are large,
//! model-specific, and not part of the NetworkX-compatible graph contract.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use graphify_core::graph::KnowledgeGraph;
use graphify_core::model::GraphNode;
use graphify_core::quality;
use model2vec_rs::model::StaticModel;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const DEFAULT_INDEX_FILE: &str = "semantic-index.json";
pub const DEFAULT_MODEL: &str = "minishlab/potion-code-16M";
pub const DEFAULT_PROVIDER: &str = "model2vec";
pub const DEFAULT_OLLAMA_MODEL: &str = "embeddinggemma";
pub const DEFAULT_VOYAGE_MODEL: &str = "voyage-code-3";
const INDEX_VERSION: u32 = 1;
const SNIPPET_CONTEXT_LINES: usize = 2;
const MAX_SNIPPET_CHARS: usize = 900;

/// A persisted vector index for one `graph.json` snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticIndex {
    pub version: u32,
    pub model: String,
    pub graph_fingerprint: String,
    pub dim: usize,
    pub nodes: Vec<IndexedNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedNode {
    pub node_id: String,
    pub text: String,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticMatch {
    pub node_id: String,
    pub label: String,
    pub source_file: String,
    pub score: f32,
    pub semantic_score: f32,
    pub lexical_score: f32,
}

#[derive(Debug, Error)]
pub enum SemanticIndexError {
    #[error(
        "semantic index model '{index_model}' does not match requested model '{requested_model}'"
    )]
    ModelMismatch {
        index_model: String,
        requested_model: String,
    },
    #[error(
        "semantic index was built for graph fingerprint {index_fingerprint}, current graph is {graph_fingerprint}"
    )]
    StaleIndex {
        index_fingerprint: String,
        graph_fingerprint: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedPurpose {
    Document,
    Query,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingProvider {
    Model2Vec,
    Ollama,
    Voyage,
}

impl EmbeddingProvider {
    pub fn parse(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "model2vec" | "m2v" | "minish" => Ok(Self::Model2Vec),
            "ollama" => Ok(Self::Ollama),
            "voyage" | "voyageai" => Ok(Self::Voyage),
            other => bail!(
                "unsupported embedding provider '{other}' (expected model2vec, ollama, or voyage)"
            ),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Model2Vec => "model2vec",
            Self::Ollama => "ollama",
            Self::Voyage => "voyage",
        }
    }
}

pub trait TextEncoder: Send + Sync {
    fn model_id(&self) -> &str;
    fn encode(&self, texts: &[String], purpose: EmbedPurpose) -> Result<Vec<Vec<f32>>>;
}

pub struct Model2VecEncoder {
    model_id: String,
    model: StaticModel,
    max_length: Option<usize>,
    batch_size: usize,
}

impl Model2VecEncoder {
    pub fn from_pretrained(model_id: &str) -> Result<Self> {
        let model = StaticModel::from_pretrained(model_id, None, None, None)
            .with_context(|| format!("load Model2Vec model {model_id}"))?;
        Ok(Self {
            model_id: model_id.to_string(),
            model,
            max_length: Some(512),
            batch_size: 1024,
        })
    }
}

impl TextEncoder for Model2VecEncoder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn encode(&self, texts: &[String], _purpose: EmbedPurpose) -> Result<Vec<Vec<f32>>> {
        Ok(self
            .model
            .encode_with_args(texts, self.max_length, self.batch_size))
    }
}

pub struct OllamaEncoder {
    model_id: String,
    model: String,
    endpoint: String,
    client: reqwest::blocking::Client,
}

impl OllamaEncoder {
    pub fn new(model: &str) -> Result<Self> {
        let host = std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".into());
        let host = if host.starts_with("http://") || host.starts_with("https://") {
            host
        } else {
            format!("http://{host}")
        };
        Ok(Self {
            model_id: format!("ollama:{model}"),
            model: model.to_string(),
            endpoint: format!("{}/api/embed", host.trim_end_matches('/')),
            client: reqwest::blocking::Client::new(),
        })
    }
}

impl TextEncoder for OllamaEncoder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn encode(&self, texts: &[String], _purpose: EmbedPurpose) -> Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(64) {
            let resp: OllamaEmbedResponse = self
                .client
                .post(&self.endpoint)
                .json(&serde_json::json!({"model": self.model, "input": chunk}))
                .send()
                .with_context(|| format!("POST {}", self.endpoint))?
                .error_for_status()
                .with_context(|| format!("Ollama embedding model '{}' failed", self.model))?
                .json()
                .context("parse Ollama embedding response")?;
            if resp.embeddings.len() != chunk.len() {
                bail!(
                    "Ollama returned {} embeddings for {} inputs",
                    resp.embeddings.len(),
                    chunk.len()
                );
            }
            out.extend(resp.embeddings);
        }
        Ok(out)
    }
}

#[derive(Debug, Deserialize)]
struct OllamaEmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

pub struct VoyageEncoder {
    model_id: String,
    model: String,
    endpoint: String,
    api_key: String,
    client: reqwest::blocking::Client,
}

impl VoyageEncoder {
    pub fn new(model: &str) -> Result<Self> {
        let api_key = std::env::var("VOYAGE_API_KEY")
            .context("VOYAGE_API_KEY is required for --embedding-provider voyage")?;
        let endpoint = std::env::var("VOYAGE_ENDPOINT")
            .unwrap_or_else(|_| "https://api.voyageai.com/v1/embeddings".into());
        Ok(Self {
            model_id: format!("voyage:{model}"),
            model: model.to_string(),
            endpoint,
            api_key,
            client: reqwest::blocking::Client::new(),
        })
    }
}

impl TextEncoder for VoyageEncoder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn encode(&self, texts: &[String], purpose: EmbedPurpose) -> Result<Vec<Vec<f32>>> {
        let input_type = match purpose {
            EmbedPurpose::Document => "document",
            EmbedPurpose::Query => "query",
        };
        let mut out = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(128) {
            let resp: VoyageEmbedResponse = self
                .client
                .post(&self.endpoint)
                .bearer_auth(&self.api_key)
                .json(&serde_json::json!({
                    "model": self.model,
                    "input": chunk,
                    "input_type": input_type,
                }))
                .send()
                .with_context(|| format!("POST {}", self.endpoint))?
                .error_for_status()
                .with_context(|| format!("Voyage embedding model '{}' failed", self.model))?
                .json()
                .context("parse Voyage embedding response")?;
            let mut data = resp.data;
            data.sort_by_key(|item| item.index);
            if data.len() != chunk.len() {
                bail!(
                    "Voyage returned {} embeddings for {} inputs",
                    data.len(),
                    chunk.len()
                );
            }
            out.extend(data.into_iter().map(|item| item.embedding));
        }
        Ok(out)
    }
}

#[derive(Debug, Deserialize)]
struct VoyageEmbedResponse {
    data: Vec<VoyageEmbedding>,
}

#[derive(Debug, Deserialize)]
struct VoyageEmbedding {
    embedding: Vec<f32>,
    index: usize,
}

pub fn build_encoder(provider: &str, model: &str) -> Result<Box<dyn TextEncoder>> {
    let provider = EmbeddingProvider::parse(provider)?;
    match provider {
        EmbeddingProvider::Model2Vec => Ok(Box::new(Model2VecEncoder::from_pretrained(model)?)),
        EmbeddingProvider::Ollama => Ok(Box::new(OllamaEncoder::new(model)?)),
        EmbeddingProvider::Voyage => Ok(Box::new(VoyageEncoder::new(model)?)),
    }
}

pub fn split_model_spec(spec: &str) -> (&str, &str) {
    if let Some((provider, model)) = spec.split_once(':')
        && matches!(provider, "model2vec" | "ollama" | "voyage")
    {
        return (provider, model);
    }
    (DEFAULT_PROVIDER, spec)
}

pub struct SemanticEngine {
    index: SemanticIndex,
    encoder: Box<dyn TextEncoder>,
}

impl SemanticEngine {
    pub fn load_for_graph(index_path: &Path, graph: &KnowledgeGraph) -> Result<Self> {
        let index = read_index(index_path)?;
        let graph_fingerprint = graph_fingerprint(graph);
        validate_index_for_graph(&index, &graph_fingerprint, &index.model)?;
        let (provider, model) = split_model_spec(&index.model);
        let encoder = build_encoder(provider, model)?;
        Ok(Self { index, encoder })
    }

    pub fn query(
        &self,
        graph: &KnowledgeGraph,
        question: &str,
        top_n: usize,
    ) -> Result<Vec<SemanticMatch>> {
        let embeddings = self
            .encoder
            .encode(&[question.to_string()], EmbedPurpose::Query)?;
        let query_embedding = embeddings
            .first()
            .ok_or_else(|| anyhow!("encoder returned no query embedding"))?;
        Ok(score_hybrid(
            graph,
            &self.index,
            question,
            query_embedding,
            top_n,
        ))
    }

    pub fn model(&self) -> &str {
        &self.index.model
    }

    pub fn node_count(&self) -> usize {
        self.index.nodes.len()
    }
}

pub fn build_model2vec_index(
    graph: &KnowledgeGraph,
    root: Option<&Path>,
    model_id: &str,
) -> Result<SemanticIndex> {
    let encoder = Model2VecEncoder::from_pretrained(model_id)?;
    build_index_with_encoder(graph, root, &encoder)
}

pub fn build_semantic_index(
    graph: &KnowledgeGraph,
    root: Option<&Path>,
    provider: &str,
    model_id: &str,
) -> Result<SemanticIndex> {
    let encoder = build_encoder(provider, model_id)?;
    build_index_with_encoder(graph, root, encoder.as_ref())
}

pub fn build_index_with_encoder<E: TextEncoder + ?Sized>(
    graph: &KnowledgeGraph,
    root: Option<&Path>,
    encoder: &E,
) -> Result<SemanticIndex> {
    let entries = node_texts(graph, root);
    let texts: Vec<String> = entries.iter().map(|(_, text)| text.clone()).collect();

    let embeddings = if texts.is_empty() {
        Vec::new()
    } else {
        encoder.encode(&texts, EmbedPurpose::Document)?
    };
    if embeddings.len() != entries.len() {
        bail!(
            "encoder returned {} embeddings for {} nodes",
            embeddings.len(),
            entries.len()
        );
    }

    let dim = embeddings.first().map_or(0, Vec::len);
    let mut nodes = Vec::with_capacity(entries.len());
    for ((node_id, text), embedding) in entries.into_iter().zip(embeddings) {
        if embedding.len() != dim {
            bail!(
                "encoder returned inconsistent dimensions: expected {}, got {} for {}",
                dim,
                embedding.len(),
                node_id
            );
        }
        if embedding.iter().any(|v| !v.is_finite()) {
            bail!("encoder returned non-finite values for {node_id}");
        }
        nodes.push(IndexedNode {
            node_id,
            text,
            embedding,
        });
    }

    Ok(SemanticIndex {
        version: INDEX_VERSION,
        model: encoder.model_id().to_string(),
        graph_fingerprint: graph_fingerprint(graph),
        dim,
        nodes,
    })
}

pub fn write_index(index: &SemanticIndex, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create semantic index dir {}", parent.display()))?;
    }
    let file = File::create(path).with_context(|| format!("create {}", path.display()))?;
    serde_json::to_writer_pretty(BufWriter::new(file), index)
        .with_context(|| format!("write {}", path.display()))
}

pub fn read_index(path: &Path) -> Result<SemanticIndex> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let index: SemanticIndex = serde_json::from_reader(BufReader::new(file))
        .with_context(|| format!("parse {}", path.display()))?;
    if index.version != INDEX_VERSION {
        bail!(
            "unsupported semantic index version {}, expected {}",
            index.version,
            INDEX_VERSION
        );
    }
    Ok(index)
}

pub fn default_index_path_for_graph(graph_path: &Path) -> PathBuf {
    graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(DEFAULT_INDEX_FILE)
}

pub fn validate_index_for_graph(
    index: &SemanticIndex,
    graph_fingerprint: &str,
    requested_model: &str,
) -> std::result::Result<(), SemanticIndexError> {
    if index.model != requested_model {
        return Err(SemanticIndexError::ModelMismatch {
            index_model: index.model.clone(),
            requested_model: requested_model.to_string(),
        });
    }
    if index.graph_fingerprint != graph_fingerprint {
        return Err(SemanticIndexError::StaleIndex {
            index_fingerprint: index.graph_fingerprint.clone(),
            graph_fingerprint: graph_fingerprint.to_string(),
        });
    }
    Ok(())
}

pub fn score_hybrid(
    graph: &KnowledgeGraph,
    index: &SemanticIndex,
    question: &str,
    query_embedding: &[f32],
    top_n: usize,
) -> Vec<SemanticMatch> {
    if top_n == 0 || index.dim == 0 || query_embedding.len() != index.dim {
        return Vec::new();
    }

    let terms = query_terms(question);
    let mut matches = Vec::new();
    for indexed in &index.nodes {
        let Some(node) = graph.get_node(&indexed.node_id) else {
            continue;
        };
        if indexed.embedding.len() != index.dim {
            continue;
        }
        let semantic_score = cosine_similarity(query_embedding, &indexed.embedding);
        let lexical_score = lexical_score(node, &indexed.text, &terms);
        let degree_boost = (graph.degree(&indexed.node_id) as f32).ln_1p() * 0.03;
        let raw_score = (semantic_score * 0.50) + (lexical_score * 0.45) + degree_boost;
        let score = raw_score * source_quality_multiplier(node);
        if score.is_finite() && score > 0.0 {
            matches.push(SemanticMatch {
                node_id: indexed.node_id.clone(),
                label: node.label.clone(),
                source_file: node.source_file.clone(),
                score,
                semantic_score,
                lexical_score,
            });
        }
    }

    matches.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
    matches.truncate(top_n);
    matches
}

pub fn graph_fingerprint(graph: &KnowledgeGraph) -> String {
    let mut hasher = Sha256::new();

    let mut node_ids = graph.node_ids();
    node_ids.sort();
    for id in node_ids {
        if let Some(node) = graph.get_node(&id) {
            hasher.update(b"node\0");
            hasher.update(node.id.as_bytes());
            hasher.update(b"\0");
            hasher.update(node.label.as_bytes());
            hasher.update(b"\0");
            hasher.update(format!("{:?}", node.node_type).as_bytes());
            hasher.update(b"\0");
            hasher.update(node.source_file.as_bytes());
            hasher.update(b"\0");
            if let Some(loc) = &node.source_location {
                hasher.update(loc.as_bytes());
            }
            hasher.update(b"\0");
        }
    }

    let mut edges: Vec<_> = graph
        .edges_with_endpoints()
        .into_iter()
        .map(|(s, t, e)| (s.to_string(), t.to_string(), e.relation.clone()))
        .collect();
    edges.sort();
    for (source, target, relation) in edges {
        hasher.update(b"edge\0");
        hasher.update(source.as_bytes());
        hasher.update(b"\0");
        hasher.update(target.as_bytes());
        hasher.update(b"\0");
        hasher.update(relation.as_bytes());
        hasher.update(b"\0");
    }

    format!("{:x}", hasher.finalize())
}

fn node_texts(graph: &KnowledgeGraph, root: Option<&Path>) -> Vec<(String, String)> {
    let mut ids = graph.node_ids();
    ids.sort();

    let mut file_cache: HashMap<String, Option<Vec<String>>> = HashMap::new();
    ids.into_iter()
        .filter_map(|id| {
            let node = graph.get_node(&id)?;
            let snippet = root.and_then(|root| snippet_for_node(root, node, &mut file_cache));
            Some((id, node_text(node, snippet.as_deref())))
        })
        .collect()
}

fn node_text(node: &GraphNode, snippet: Option<&str>) -> String {
    let mut parts = vec![
        format!("label: {}", node.label),
        format!("id: {}", node.id),
        format!("type: {:?}", node.node_type),
        format!("file: {}", node.source_file),
    ];
    if let Some(location) = &node.source_location {
        parts.push(format!("location: {location}"));
    }
    if let Some(doc_text) = node.extra.get("doc_text").and_then(|v| v.as_str()) {
        parts.push(format!("document text:\n{doc_text}"));
    }

    let mut extra_keys: Vec<_> = node.extra.keys().collect();
    extra_keys.sort();
    for key in extra_keys {
        if let Some(value) = node.extra.get(key)
            && (value.is_string() || value.is_number() || value.is_boolean())
        {
            parts.push(format!("{key}: {value}"));
        }
    }
    if let Some(snippet) = snippet.filter(|s| !s.trim().is_empty()) {
        parts.push(format!("snippet:\n{snippet}"));
    }
    parts.join("\n")
}

fn snippet_for_node(
    root: &Path,
    node: &GraphNode,
    file_cache: &mut HashMap<String, Option<Vec<String>>>,
) -> Option<String> {
    let line = parse_line(node.source_location.as_deref())?;
    let lines = file_cache
        .entry(node.source_file.clone())
        .or_insert_with(|| read_source_lines(root, &node.source_file));
    let lines = lines.as_ref()?;
    if lines.is_empty() {
        return None;
    }
    let zero_based = line.saturating_sub(1);
    let start = zero_based.saturating_sub(SNIPPET_CONTEXT_LINES);
    let end = (zero_based + SNIPPET_CONTEXT_LINES + 1).min(lines.len());
    let mut snippet = lines[start..end].join("\n");
    if snippet.len() > MAX_SNIPPET_CHARS {
        snippet.truncate(MAX_SNIPPET_CHARS);
        snippet.push('…');
    }
    Some(snippet)
}

fn read_source_lines(root: &Path, source_file: &str) -> Option<Vec<String>> {
    let relative = source_file.strip_prefix("./").unwrap_or(source_file);
    let path = root.join(relative);
    let content = std::fs::read_to_string(path).ok()?;
    Some(content.lines().map(str::to_string).collect())
}

fn parse_line(location: Option<&str>) -> Option<usize> {
    let location = location?;
    let digits: String = location
        .trim_start_matches('L')
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn query_terms(question: &str) -> Vec<String> {
    question
        .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
        .map(str::to_lowercase)
        .filter(|term| term.len() > 2)
        .collect()
}

fn lexical_score(node: &GraphNode, indexed_text: &str, terms: &[String]) -> f32 {
    if terms.is_empty() {
        return 0.0;
    }
    let label = node.label.to_lowercase();
    let id = node.id.to_lowercase();
    let text = indexed_text.to_lowercase();

    let mut score = 0.0;
    for term in terms {
        if label == *term {
            score += 2.0;
        } else if label.contains(term) {
            score += 1.0;
        }
        if id.contains(term) {
            score += 0.6;
        }
        if text.contains(term) {
            score += 0.25;
        }
    }
    (score / terms.len() as f32).min(3.0)
}

fn source_quality_multiplier(node: &GraphNode) -> f32 {
    quality::node_priority(node)
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;
    for (&x, &y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom <= f32::EPSILON {
        0.0
    } else {
        dot / denom
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphify_core::confidence::Confidence;
    use graphify_core::model::{GraphEdge, GraphNode, NodeType};

    struct StubEncoder {
        embeddings: Vec<Vec<f32>>,
    }

    impl TextEncoder for StubEncoder {
        fn model_id(&self) -> &str {
            "stub-model"
        }

        fn encode(&self, texts: &[String], _purpose: EmbedPurpose) -> Result<Vec<Vec<f32>>> {
            Ok(self.embeddings.iter().take(texts.len()).cloned().collect())
        }
    }

    fn make_graph() -> KnowledgeGraph {
        let mut graph = KnowledgeGraph::new();
        graph
            .add_node(GraphNode {
                id: "kafka_sender".into(),
                label: "SendToKafka".into(),
                source_file: "./src/kafka.rs".into(),
                source_location: Some("L10".into()),
                node_type: NodeType::Function,
                community: None,
                extra: HashMap::new(),
            })
            .unwrap();
        graph
            .add_node(GraphNode {
                id: "decimal_validator".into(),
                label: "validateTournamentDecimal".into(),
                source_file: "./src/decimal.rs".into(),
                source_location: Some("L3".into()),
                node_type: NodeType::Function,
                community: None,
                extra: HashMap::new(),
            })
            .unwrap();
        graph
            .add_edge(GraphEdge {
                source: "kafka_sender".into(),
                target: "decimal_validator".into(),
                relation: "calls".into(),
                confidence: Confidence::Extracted,
                confidence_score: 1.0,
                source_file: "./src/kafka.rs".into(),
                source_location: None,
                weight: 1.0,
                extra: HashMap::new(),
            })
            .unwrap();
        graph
    }

    #[test]
    fn build_index_is_stable_and_fingerprinted() {
        let graph = make_graph();
        let encoder = StubEncoder {
            embeddings: vec![vec![1.0, 0.0], vec![0.0, 1.0]],
        };

        let index = build_index_with_encoder(&graph, None, &encoder).unwrap();

        assert_eq!(index.version, INDEX_VERSION);
        assert_eq!(index.model, "stub-model");
        assert_eq!(index.dim, 2);
        assert_eq!(index.nodes.len(), 2);
        assert_eq!(index.graph_fingerprint, graph_fingerprint(&graph));
    }

    #[test]
    fn hybrid_scoring_prefers_semantically_close_node() {
        let graph = make_graph();
        let index = SemanticIndex {
            version: INDEX_VERSION,
            model: "stub-model".into(),
            graph_fingerprint: graph_fingerprint(&graph),
            dim: 2,
            nodes: vec![
                IndexedNode {
                    node_id: "kafka_sender".into(),
                    text: "message delivery queue producer".into(),
                    embedding: vec![1.0, 0.0],
                },
                IndexedNode {
                    node_id: "decimal_validator".into(),
                    text: "decimal precision validation".into(),
                    embedding: vec![0.0, 1.0],
                },
            ],
        };

        let matches = score_hybrid(&graph, &index, "delivery backpressure", &[1.0, 0.0], 2);

        assert_eq!(matches[0].node_id, "kafka_sender");
        assert!(matches[0].semantic_score > matches[1].semantic_score);
    }

    #[test]
    fn hybrid_scoring_penalizes_test_nodes_when_signal_is_equal() {
        let mut graph = KnowledgeGraph::new();
        for (id, file) in [
            ("prod_validator", "./internal/converter/from_api.go"),
            ("test_validator", "./internal/converter/from_api_test.go"),
        ] {
            graph
                .add_node(GraphNode {
                    id: id.into(),
                    label: "validateTournamentDecimal".into(),
                    source_file: file.into(),
                    source_location: Some("L10".into()),
                    node_type: NodeType::Function,
                    community: None,
                    extra: HashMap::new(),
                })
                .unwrap();
        }
        let index = SemanticIndex {
            version: INDEX_VERSION,
            model: "stub-model".into(),
            graph_fingerprint: graph_fingerprint(&graph),
            dim: 2,
            nodes: vec![
                IndexedNode {
                    node_id: "test_validator".into(),
                    text: "tournament decimal validation".into(),
                    embedding: vec![1.0, 0.0],
                },
                IndexedNode {
                    node_id: "prod_validator".into(),
                    text: "tournament decimal validation".into(),
                    embedding: vec![1.0, 0.0],
                },
            ],
        };

        let matches = score_hybrid(
            &graph,
            &index,
            "tournament decimal validation",
            &[1.0, 0.0],
            2,
        );

        assert_eq!(matches[0].node_id, "prod_validator");
    }

    #[test]
    fn hybrid_scoring_prefers_schema_over_down_migration_when_signal_is_equal() {
        let mut graph = KnowledgeGraph::new();
        for (id, file) in [
            ("schema_mv", "./database/leaderboards/schema.sql"),
            (
                "down_mv",
                "./database/leaderboards/migrations/20240215111131_fix.down.sql",
            ),
        ] {
            graph
                .add_node(GraphNode {
                    id: id.into(),
                    label: "mark_transactions_with_tournaments_mv".into(),
                    source_file: file.into(),
                    source_location: Some("L10".into()),
                    node_type: NodeType::Function,
                    community: None,
                    extra: HashMap::new(),
                })
                .unwrap();
        }
        let index = SemanticIndex {
            version: INDEX_VERSION,
            model: "stub-model".into(),
            graph_fingerprint: graph_fingerprint(&graph),
            dim: 2,
            nodes: vec![
                IndexedNode {
                    node_id: "down_mv".into(),
                    text: "materialized view tournament transactions leaderboards".into(),
                    embedding: vec![1.0, 0.0],
                },
                IndexedNode {
                    node_id: "schema_mv".into(),
                    text: "materialized view tournament transactions leaderboards".into(),
                    embedding: vec![1.0, 0.0],
                },
            ],
        };

        let matches = score_hybrid(
            &graph,
            &index,
            "materialized view tournament transactions leaderboards",
            &[1.0, 0.0],
            2,
        );

        assert_eq!(matches[0].node_id, "schema_mv");
    }

    #[test]
    fn split_model_spec_supports_remote_prefixes() {
        assert_eq!(
            split_model_spec("ollama:embeddinggemma"),
            ("ollama", "embeddinggemma")
        );
        assert_eq!(
            split_model_spec("voyage:voyage-code-3"),
            ("voyage", "voyage-code-3")
        );
        assert_eq!(
            split_model_spec("minishlab/potion-code-16M"),
            (DEFAULT_PROVIDER, "minishlab/potion-code-16M")
        );
    }

    #[test]
    fn stale_index_is_rejected() {
        let graph = make_graph();
        let index = SemanticIndex {
            version: INDEX_VERSION,
            model: "stub-model".into(),
            graph_fingerprint: "old".into(),
            dim: 0,
            nodes: Vec::new(),
        };

        let err = validate_index_for_graph(&index, &graph_fingerprint(&graph), "stub-model")
            .expect_err("stale index should fail");

        assert!(matches!(err, SemanticIndexError::StaleIndex { .. }));
    }

    #[test]
    fn index_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DEFAULT_INDEX_FILE);
        let graph = make_graph();
        let index = SemanticIndex {
            version: INDEX_VERSION,
            model: "stub-model".into(),
            graph_fingerprint: graph_fingerprint(&graph),
            dim: 1,
            nodes: vec![IndexedNode {
                node_id: "kafka_sender".into(),
                text: "kafka".into(),
                embedding: vec![1.0],
            }],
        };

        write_index(&index, &path).unwrap();
        let loaded = read_index(&path).unwrap();

        assert_eq!(loaded.model, "stub-model");
        assert_eq!(loaded.nodes[0].node_id, "kafka_sender");
    }
}
