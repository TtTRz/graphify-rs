//! Tree-sitter based AST extraction engine.
//!
//! Provides accurate structural extraction using native tree-sitter grammars
//! for Python, JavaScript, TypeScript, Rust, Go, Java, C, C++, Ruby, C#, and Dart.
//! Falls back gracefully to the regex-based extractor for unsupported languages.

mod handlers;
mod imports;
mod treesitter_config;

pub use treesitter_config::TsConfig;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use graphify_core::confidence::Confidence;
use graphify_core::id::make_id;
use graphify_core::model::{ExtractionResult, GraphEdge, GraphNode, NodeType};
use tracing::trace;
use tree_sitter::{Language, Node, Parser};

pub fn try_extract(path: &Path, source: &[u8], lang: &str) -> Option<ExtractionResult> {
    let (language, config) = treesitter_config::resolve_language(lang)?;
    extract_with_treesitter(path, source, language, &config, lang)
}

fn extract_with_treesitter(
    path: &Path,
    source: &[u8],
    language: Language,
    config: &TsConfig,
    lang: &str,
) -> Option<ExtractionResult> {
    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(source, None)?;
    let root = tree.root_node();

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let str_path = path.to_string_lossy();

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut raw_calls: Vec<(String, String)> = Vec::new();
    let mut ruby_bodies: Vec<(String, usize, usize)> = Vec::new();

    let file_nid = make_id(&[&str_path]);
    seen_ids.insert(file_nid.clone());
    nodes.push(GraphNode {
        id: file_nid.clone(),
        label: stem.to_string(),
        source_file: str_path.to_string(),
        source_location: None,
        node_type: NodeType::File,
        community: None,
        extra: HashMap::new(),
    });

    {
        let mut ctx = WalkContext {
            lang,
            file_nid: &file_nid,
            str_path: &str_path,
            nodes: &mut nodes,
            edges: &mut edges,
            seen_ids: &mut seen_ids,
            raw_calls: &mut raw_calls,
            ruby_bodies: &mut ruby_bodies,
        };
        walk_node(root, source, config, &mut ctx, None);
    }

    let label_to_nid: HashMap<String, String> = nodes
        .iter()
        .filter(|n| matches!(n.node_type, NodeType::Function | NodeType::Method))
        .map(|n| {
            let normalized = n
                .label
                .trim_end_matches("()")
                .trim_start_matches('.')
                .to_lowercase();
            (normalized, n.id.clone())
        })
        .collect();

    let mut seen_calls: HashSet<(String, String)> = HashSet::new();
    for (caller_nid, callee_name) in &raw_calls {
        let name_lower = callee_name.to_lowercase();
        if let Some(callee_nid) = label_to_nid.get(&name_lower) {
            if callee_nid == caller_nid {
                continue;
            }
            let key = (caller_nid.clone(), callee_nid.clone());
            if seen_calls.insert(key) {
                edges.push(GraphEdge {
                    source: caller_nid.clone(),
                    target: callee_nid.clone(),
                    relation: "calls".to_string(),
                    confidence: Confidence::Inferred,
                    confidence_score: Confidence::Inferred.default_score(),
                    source_file: str_path.to_string(),
                    source_location: None,
                    weight: 1.0,
                    extra: HashMap::new(),
                });
            }
        }
    }

    if lang == "ruby" {
        for (caller_nid, body_start, body_end) in &ruby_bodies {
            let body_text = &source[*body_start..*body_end];
            let body_str = String::from_utf8_lossy(body_text);
            let body_lower = body_str.to_lowercase();
            for (func_label, callee_nid) in &label_to_nid {
                if callee_nid == caller_nid {
                    continue;
                }
                let found = body_lower
                    .find(func_label.as_str())
                    .is_some_and(|pos| {
                        let after = pos + func_label.len();
                        if after >= body_lower.len() {
                            true
                        } else {
                            let next_ch = body_lower.as_bytes()[after];
                            !next_ch.is_ascii_alphanumeric() && next_ch != b'_'
                        }
                    });
                if found {
                    let key = (caller_nid.clone(), callee_nid.clone());
                    if seen_calls.insert(key) {
                        edges.push(GraphEdge {
                            source: caller_nid.clone(),
                            target: callee_nid.clone(),
                            relation: "calls".to_string(),
                            confidence: Confidence::Inferred,
                            confidence_score: Confidence::Inferred.default_score(),
                            source_file: str_path.to_string(),
                            source_location: None,
                            weight: 1.0,
                            extra: HashMap::new(),
                        });
                    }
                }
            }
        }
    }

    trace!(
        "treesitter({}): {} nodes, {} edges from {}",
        lang,
        nodes.len(),
        edges.len(),
        str_path
    );

    Some(ExtractionResult {
        nodes,
        edges,
        hyperedges: vec![],
    })
}

fn collect_callees(body: Node, source: &[u8], config: &TsConfig) -> Vec<String> {
    let mut callees = Vec::new();
    collect_callees_recursive(body, source, config, &mut callees);
    callees
}

fn collect_callees_recursive(
    node: Node,
    source: &[u8],
    config: &TsConfig,
    callees: &mut Vec<String>,
) {
    if config.call_types.contains(node.kind())
        && let Some(name) = extract_callee_name(node, source, config)
    {
        callees.push(name);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_callees_recursive(child, source, config, callees);
    }
}

fn extract_callee_name(call_node: Node, source: &[u8], config: &TsConfig) -> Option<String> {
    let func_node = call_node.child_by_field_name(config.call_function_field)?;
    extract_name_from_callee(func_node, source)
}

fn extract_name_from_callee(node: Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" | "field_identifier" => Some(node_text(node, source)),
        "attribute" => node
            .child_by_field_name("attribute")
            .map(|n| node_text(n, source)),
        "field_expression" | "member_expression" => node
            .child_by_field_name("field")
            .or_else(|| node.child_by_field_name("property"))
            .map(|n| node_text(n, source)),
        "scoped_identifier" | "qualified_identifier" => node
            .child_by_field_name("name")
            .map(|n| node_text(n, source)),
        "selector_expression" => node
            .child_by_field_name("field")
            .map(|n| node_text(n, source)),
        _ => None,
    }
}

enum ElixirCallKind {
    Import,
    Class,
    Function,
    Other,
}

fn classify_elixir_call(node: Node, source: &[u8], config: &TsConfig) -> ElixirCallKind {
    let target = node
        .child_by_field_name(config.name_field)
        .map(|n| node_text(n, source))
        .unwrap_or_default();
    match target.as_str() {
        "import" | "use" | "require" | "alias" => ElixirCallKind::Import,
        "defmodule" | "defprotocol" | "defimpl" => ElixirCallKind::Class,
        "def" | "defp" | "defmacro" | "defmacrop" | "defguard" | "defguardp" | "defdelegate" => {
            ElixirCallKind::Function
        }
        _ => ElixirCallKind::Other,
    }
}

pub(crate) fn walk_node(
    node: Node,
    source: &[u8],
    config: &TsConfig,
    ctx: &mut WalkContext,
    parent_class_nid: Option<&str>,
) {
    let kind = node.kind();

    if ctx.lang == "elixir" && kind == "call" {
        match classify_elixir_call(node, source, config) {
            ElixirCallKind::Import => {
                imports::extract_import(
                    node,
                    source,
                    ctx.file_nid,
                    ctx.str_path,
                    ctx.lang,
                    ctx.edges,
                    ctx.nodes,
                );
                return;
            }
            ElixirCallKind::Class => {
                handlers::handle_class_like(node, source, config, ctx);
                return;
            }
            ElixirCallKind::Function => {
                handlers::handle_function(node, source, config, ctx, parent_class_nid);
                return;
            }
            ElixirCallKind::Other => {}
        }
    } else if config.import_types.contains(kind) {
        if ctx.lang == "ruby" && kind == "call" {
            let method_name = node
                .child_by_field_name("method")
                .map(|n| node_text(n, source))
                .unwrap_or_default();
            if method_name == "require" || method_name == "require_relative" {
                imports::extract_import(
                    node,
                    source,
                    ctx.file_nid,
                    ctx.str_path,
                    ctx.lang,
                    ctx.edges,
                    ctx.nodes,
                );
                return;
            }
        } else {
            imports::extract_import(
                node,
                source,
                ctx.file_nid,
                ctx.str_path,
                ctx.lang,
                ctx.edges,
                ctx.nodes,
            );
            return;
        }
    } else if config.class_types.contains(kind) {
        handlers::handle_class_like(node, source, config, ctx);
        return;
    } else if config.function_types.contains(kind) {
        handlers::handle_function(node, source, config, ctx, parent_class_nid);
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_node(child, source, config, ctx, parent_class_nid);
    }
}

pub(crate) struct WalkContext<'a> {
    pub lang: &'a str,
    pub file_nid: &'a str,
    pub str_path: &'a str,
    pub nodes: &'a mut Vec<GraphNode>,
    pub edges: &'a mut Vec<GraphEdge>,
    pub seen_ids: &'a mut HashSet<String>,
    pub raw_calls: &'a mut Vec<(String, String)>,
    pub ruby_bodies: &'a mut Vec<(String, usize, usize)>,
}

pub(crate) fn node_text(node: Node, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

pub(crate) fn get_name(node: Node, source: &[u8], field: &str) -> Option<String> {
    let name_node = node.child_by_field_name(field)?;
    let text = unwrap_declarator_name(name_node, source);
    if text.is_empty() { None } else { Some(text) }
}

pub(crate) fn unwrap_declarator_name(node: Node, source: &[u8]) -> String {
    match node.kind() {
        "function_declarator"
        | "pointer_declarator"
        | "reference_declarator"
        | "parenthesized_declarator" => {
            if let Some(inner) = node.child_by_field_name("declarator") {
                return unwrap_declarator_name(inner, source);
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" || child.kind() == "field_identifier" {
                    return node_text(child, source);
                }
            }
            node_text(node, source)
        }
        "qualified_identifier" | "scoped_identifier" => {
            if let Some(name) = node.child_by_field_name("name") {
                return node_text(name, source);
            }
            node_text(node, source)
        }
        _ => node_text(node, source),
    }
}

pub(crate) fn make_edge(
    source_id: &str,
    target_id: &str,
    relation: &str,
    source_file: &str,
    line: usize,
) -> GraphEdge {
    GraphEdge {
        source: source_id.to_string(),
        target: target_id.to_string(),
        relation: relation.to_string(),
        confidence: Confidence::Extracted,
        confidence_score: Confidence::Extracted.default_score(),
        source_file: source_file.to_string(),
        source_location: Some(format!("L{line}")),
        weight: 1.0,
        extra: HashMap::new(),
    }
}
