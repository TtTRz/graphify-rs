use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::confidence::Confidence;

// ---------------------------------------------------------------------------
// NodeType
// ---------------------------------------------------------------------------

/// The kind of entity a graph node represents.
///
/// Serialized as lowercase strings (e.g. `"class"`, `"function"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    Class,
    Function,
    Module,
    Concept,
    Paper,
    Image,
    File,
    Method,
    Interface,
    Enum,
    Struct,
    Trait,
    Constant,
    Variable,
    Package,
    Namespace,
}

// ---------------------------------------------------------------------------
// GraphNode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub source_file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_location: Option<String>,
    pub node_type: NodeType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub community: Option<usize>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// GraphEdge
// ---------------------------------------------------------------------------

fn default_confidence_score() -> f64 {
    1.0
}

fn default_weight() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub relation: String,
    pub confidence: Confidence,
    #[serde(default = "default_confidence_score")]
    pub confidence_score: f64,
    pub source_file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_location: Option<String>,
    #[serde(default = "default_weight")]
    pub weight: f64,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Hyperedge
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hyperedge {
    pub nodes: Vec<String>,
    pub relation: String,
    pub label: String,
}

// ---------------------------------------------------------------------------
// ExtractionResult
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtractionResult {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    #[serde(default)]
    pub hyperedges: Vec<Hyperedge>,
}

// ---------------------------------------------------------------------------
// CommunityInfo
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityInfo {
    pub id: usize,
    pub nodes: Vec<String>,
    pub cohesion: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

// ---------------------------------------------------------------------------
// AnalysisResult & helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GodNode {
    pub id: String,
    pub label: String,
    pub degree: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub community: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Surprise {
    pub source: String,
    pub target: String,
    pub source_community: usize,
    pub target_community: usize,
    pub relation: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub god_nodes: Vec<GodNode>,
    pub surprises: Vec<Surprise>,
    pub questions: Vec<String>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_node() -> GraphNode {
        GraphNode {
            id: "my_class".into(),
            label: "MyClass".into(),
            source_file: "src/main.rs".into(),
            source_location: Some("L42".into()),
            node_type: NodeType::Class,
            community: None,
            extra: HashMap::new(),
        }
    }

    fn sample_edge() -> GraphEdge {
        GraphEdge {
            source: "a".into(),
            target: "b".into(),
            relation: "calls".into(),
            confidence: Confidence::Extracted,
            confidence_score: 1.0,
            source_file: "src/main.rs".into(),
            source_location: None,
            weight: 1.0,
            extra: HashMap::new(),
        }
    }

    #[test]
    fn node_type_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&NodeType::Class).unwrap(),
            r#""class""#
        );
        assert_eq!(
            serde_json::to_string(&NodeType::Function).unwrap(),
            r#""function""#
        );
        assert_eq!(
            serde_json::to_string(&NodeType::Namespace).unwrap(),
            r#""namespace""#
        );
    }

    #[test]
    fn node_roundtrip() {
        let node = sample_node();
        let json = serde_json::to_string(&node).unwrap();
        let back: GraphNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "my_class");
        assert_eq!(back.node_type, NodeType::Class);
    }

    #[test]
    fn node_skip_none_fields() {
        let mut node = sample_node();
        node.source_location = None;
        node.community = None;
        let json = serde_json::to_string(&node).unwrap();
        assert!(!json.contains("source_location"));
        assert!(!json.contains("community"));
    }

    #[test]
    fn edge_defaults() {
        let json = r#"{
            "source": "a",
            "target": "b",
            "relation": "calls",
            "confidence": "EXTRACTED",
            "source_file": "x.rs"
        }"#;
        let edge: GraphEdge = serde_json::from_str(json).unwrap();
        assert!((edge.confidence_score - 1.0).abs() < f64::EPSILON);
        assert!((edge.weight - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn edge_roundtrip() {
        let edge = sample_edge();
        let json = serde_json::to_string(&edge).unwrap();
        let back: GraphEdge = serde_json::from_str(&json).unwrap();
        assert_eq!(back.relation, "calls");
        assert_eq!(back.confidence, Confidence::Extracted);
    }

    #[test]
    fn extraction_result_default() {
        let r = ExtractionResult::default();
        assert!(r.nodes.is_empty());
        assert!(r.edges.is_empty());
        assert!(r.hyperedges.is_empty());
    }

    #[test]
    fn extra_fields_flatten() {
        let mut node = sample_node();
        node.extra
            .insert("custom".into(), serde_json::Value::Bool(true));
        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains(r#""custom":true"#));
    }

    #[test]
    fn community_info_roundtrip() {
        let ci = CommunityInfo {
            id: 0,
            nodes: vec!["a".into(), "b".into()],
            cohesion: 0.85,
            label: Some("cluster-0".into()),
        };
        let json = serde_json::to_string(&ci).unwrap();
        let back: CommunityInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, 0);
        assert_eq!(back.nodes.len(), 2);
    }

    #[test]
    fn analysis_result_default() {
        let ar = AnalysisResult::default();
        assert!(ar.god_nodes.is_empty());
        assert!(ar.surprises.is_empty());
        assert!(ar.questions.is_empty());
    }
}
