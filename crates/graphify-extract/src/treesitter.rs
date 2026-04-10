//! Tree-sitter based AST extraction engine.
//!
//! Provides accurate structural extraction using native tree-sitter grammars
//! for Python, JavaScript, TypeScript, Rust, Go, Java, C, C++, Ruby, and C#. Falls back gracefully
//! to the regex-based extractor for unsupported languages.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use graphify_core::confidence::Confidence;
use graphify_core::id::make_id;
use graphify_core::model::{ExtractionResult, GraphEdge, GraphNode, NodeType};
use tracing::trace;
use tree_sitter::{Language, Node, Parser};

// ═══════════════════════════════════════════════════════════════════════════
// Configuration per language
// ═══════════════════════════════════════════════════════════════════════════

/// Describes which tree-sitter node kinds correspond to classes, functions,
/// imports and calls for a given language.
pub struct TsConfig {
    pub class_types: HashSet<&'static str>,
    pub function_types: HashSet<&'static str>,
    pub import_types: HashSet<&'static str>,
    pub call_types: HashSet<&'static str>,
    /// Field name used by the grammar to expose the identifier of a definition.
    pub name_field: &'static str,
    /// Optional override for class/struct name field (defaults to name_field).
    pub class_name_field: Option<&'static str>,
    /// Field name for the body block of a class/function.
    pub body_field: &'static str,
    /// Field name inside a call expression that points to the callee.
    pub call_function_field: &'static str,
}

fn python_config() -> TsConfig {
    TsConfig {
        class_types: ["class_definition"].into_iter().collect(),
        function_types: ["function_definition"].into_iter().collect(),
        import_types: ["import_statement", "import_from_statement"]
            .into_iter()
            .collect(),
        call_types: ["call"].into_iter().collect(),
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "function",
    }
}

fn js_config() -> TsConfig {
    TsConfig {
        class_types: ["class_declaration", "class"].into_iter().collect(),
        function_types: [
            "function_declaration",
            "method_definition",
            "arrow_function",
            "generator_function_declaration",
        ]
        .into_iter()
        .collect(),
        import_types: ["import_statement"].into_iter().collect(),
        call_types: ["call_expression"].into_iter().collect(),
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "function",
    }
}

fn rust_config() -> TsConfig {
    TsConfig {
        class_types: ["struct_item", "enum_item", "trait_item", "impl_item"]
            .into_iter()
            .collect(),
        function_types: ["function_item"].into_iter().collect(),
        import_types: ["use_declaration"].into_iter().collect(),
        call_types: ["call_expression"].into_iter().collect(),
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "function",
    }
}

fn go_config() -> TsConfig {
    TsConfig {
        class_types: ["type_declaration"].into_iter().collect(),
        function_types: ["function_declaration", "method_declaration"]
            .into_iter()
            .collect(),
        import_types: ["import_declaration"].into_iter().collect(),
        call_types: ["call_expression"].into_iter().collect(),
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "function",
    }
}

fn java_config() -> TsConfig {
    TsConfig {
        class_types: ["class_declaration", "interface_declaration"]
            .into_iter()
            .collect(),
        function_types: ["method_declaration", "constructor_declaration"]
            .into_iter()
            .collect(),
        import_types: ["import_declaration"].into_iter().collect(),
        call_types: ["method_invocation"].into_iter().collect(),
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "name",
    }
}

fn c_config() -> TsConfig {
    TsConfig {
        class_types: HashSet::new(),
        function_types: ["function_definition"].into_iter().collect(),
        import_types: ["preproc_include"].into_iter().collect(),
        call_types: ["call_expression"].into_iter().collect(),
        name_field: "declarator",
        class_name_field: None,
        body_field: "body",
        call_function_field: "function",
    }
}

fn cpp_config() -> TsConfig {
    TsConfig {
        class_types: ["class_specifier"].into_iter().collect(),
        function_types: ["function_definition"].into_iter().collect(),
        import_types: ["preproc_include"].into_iter().collect(),
        call_types: ["call_expression"].into_iter().collect(),
        name_field: "declarator",
        class_name_field: Some("name"),
        body_field: "body",
        call_function_field: "function",
    }
}

fn ruby_config() -> TsConfig {
    TsConfig {
        class_types: ["class"].into_iter().collect(),
        function_types: ["method", "singleton_method"].into_iter().collect(),
        import_types: HashSet::new(),
        call_types: ["call"].into_iter().collect(),
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "method",
    }
}

fn csharp_config() -> TsConfig {
    TsConfig {
        class_types: ["class_declaration", "interface_declaration"]
            .into_iter()
            .collect(),
        function_types: ["method_declaration"].into_iter().collect(),
        import_types: ["using_directive"].into_iter().collect(),
        call_types: ["invocation_expression"].into_iter().collect(),
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "function",
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Public entry point
// ═══════════════════════════════════════════════════════════════════════════

/// Try tree-sitter extraction for a supported language.
/// Returns `None` if the language is not supported by tree-sitter grammars.
pub fn try_extract(path: &Path, source: &[u8], lang: &str) -> Option<ExtractionResult> {
    let (language, config) = match lang {
        "python" => (tree_sitter_python::LANGUAGE.into(), python_config()),
        "javascript" => (tree_sitter_javascript::LANGUAGE.into(), js_config()),
        "typescript" => (
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            js_config(),
        ),
        "rust" => (tree_sitter_rust::LANGUAGE.into(), rust_config()),
        "go" => (tree_sitter_go::LANGUAGE.into(), go_config()),
        "java" => (tree_sitter_java::LANGUAGE.into(), java_config()),
        "c" => (tree_sitter_c::LANGUAGE.into(), c_config()),
        "cpp" => (tree_sitter_cpp::LANGUAGE.into(), cpp_config()),
        "ruby" => (tree_sitter_ruby::LANGUAGE.into(), ruby_config()),
        "csharp" => (tree_sitter_c_sharp::LANGUAGE.into(), csharp_config()),
        _ => return None,
    };
    extract_with_treesitter(path, source, language, &config, lang)
}

// ═══════════════════════════════════════════════════════════════════════════
// Core extraction
// ═══════════════════════════════════════════════════════════════════════════

/// Extract graph nodes and edges from a single file using tree-sitter.
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

    let stem = path.file_stem()?.to_str()?;
    let str_path = path.to_string_lossy();

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut seen_ids = HashSet::new();
    // For the call-graph pass we record (caller_nid, body_start_byte, body_end_byte)
    let mut function_bodies: Vec<(String, usize, usize)> = Vec::new();

    // File node
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

    // Walk the AST
    walk_node(
        root,
        source,
        config,
        lang,
        &file_nid,
        stem,
        &str_path,
        &mut nodes,
        &mut edges,
        &mut seen_ids,
        &mut function_bodies,
        None,
    );

    // ---- Call-graph pass ----
    // Build label → nid mapping for known functions
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
    for (caller_nid, body_start, body_end) in &function_bodies {
        let body_text = &source[*body_start..*body_end];
        let body_str = String::from_utf8_lossy(body_text);
        for (func_label, callee_nid) in &label_to_nid {
            if callee_nid == caller_nid {
                continue;
            }
            // Simple heuristic: look for `func_name(` in body
            if body_str.to_lowercase().contains(&format!("{func_label}(")) {
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

// ═══════════════════════════════════════════════════════════════════════════
// AST walking
// ═══════════════════════════════════════════════════════════════════════════

#[allow(clippy::too_many_arguments)]
fn walk_node(
    node: Node,
    source: &[u8],
    config: &TsConfig,
    lang: &str,
    file_nid: &str,
    stem: &str,
    str_path: &str,
    nodes: &mut Vec<GraphNode>,
    edges: &mut Vec<GraphEdge>,
    seen_ids: &mut HashSet<String>,
    function_bodies: &mut Vec<(String, usize, usize)>,
    parent_class_nid: Option<&str>,
) {
    let kind = node.kind();

    // ---- Imports ----
    if config.import_types.contains(kind) {
        extract_import(node, source, file_nid, str_path, lang, edges, nodes);
        return; // Don't recurse into import children
    }

    // ---- Classes / Structs / Enums / Traits ----
    if config.class_types.contains(kind) {
        handle_class_like(
            node,
            source,
            config,
            lang,
            file_nid,
            stem,
            str_path,
            nodes,
            edges,
            seen_ids,
            function_bodies,
        );
        return;
    }

    // ---- Functions / Methods ----
    if config.function_types.contains(kind) {
        handle_function(
            node,
            source,
            config,
            lang,
            file_nid,
            stem,
            str_path,
            nodes,
            edges,
            seen_ids,
            function_bodies,
            parent_class_nid,
        );
        return;
    }

    // ---- Default: recurse into children ----
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_node(
            child,
            source,
            config,
            lang,
            file_nid,
            stem,
            str_path,
            nodes,
            edges,
            seen_ids,
            function_bodies,
            parent_class_nid,
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Class-like handler (class, struct, enum, trait, impl, type_declaration)
// ═══════════════════════════════════════════════════════════════════════════

#[allow(clippy::too_many_arguments)]
fn handle_class_like(
    node: Node,
    source: &[u8],
    config: &TsConfig,
    lang: &str,
    file_nid: &str,
    stem: &str,
    str_path: &str,
    nodes: &mut Vec<GraphNode>,
    edges: &mut Vec<GraphEdge>,
    seen_ids: &mut HashSet<String>,
    function_bodies: &mut Vec<(String, usize, usize)>,
) {
    let kind = node.kind();

    // For Go type_declaration, we need to dig into the type_spec child
    if lang == "go" && kind == "type_declaration" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "type_spec" {
                handle_go_type_spec(
                    child,
                    source,
                    config,
                    lang,
                    file_nid,
                    stem,
                    str_path,
                    nodes,
                    edges,
                    seen_ids,
                    function_bodies,
                );
            }
        }
        return;
    }

    // Rust impl_item: extract methods inside, create "implements" edges
    if lang == "rust" && kind == "impl_item" {
        handle_rust_impl(
            node,
            source,
            config,
            lang,
            file_nid,
            stem,
            str_path,
            nodes,
            edges,
            seen_ids,
            function_bodies,
        );
        return;
    }

    // Standard class/struct/enum/trait
    let class_field = config.class_name_field.unwrap_or(config.name_field);
    let name = match get_name(node, source, class_field) {
        Some(n) => n,
        None => return,
    };
    let line = node.start_position().row + 1;
    let class_nid = make_id(&[str_path, &name]);

    let node_type = classify_class_kind(kind, lang);

    if seen_ids.insert(class_nid.clone()) {
        nodes.push(GraphNode {
            id: class_nid.clone(),
            label: name.clone(),
            source_file: str_path.to_string(),
            source_location: Some(format!("L{line}")),
            node_type,
            community: None,
            extra: HashMap::new(),
        });
        edges.push(make_edge(file_nid, &class_nid, "defines", str_path, line));
    }

    // Recurse into body to find methods
    if let Some(body) = node.child_by_field_name(config.body_field) {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            walk_node(
                child,
                source,
                config,
                lang,
                file_nid,
                stem,
                str_path,
                nodes,
                edges,
                seen_ids,
                function_bodies,
                Some(&class_nid),
            );
        }
    }
}

fn classify_class_kind(kind: &str, lang: &str) -> NodeType {
    match (kind, lang) {
        ("struct_item", "rust") => NodeType::Struct,
        ("enum_item", "rust") => NodeType::Enum,
        ("trait_item", "rust") => NodeType::Trait,
        _ => NodeType::Class,
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_go_type_spec(
    node: Node,
    source: &[u8],
    config: &TsConfig,
    lang: &str,
    file_nid: &str,
    stem: &str,
    str_path: &str,
    nodes: &mut Vec<GraphNode>,
    edges: &mut Vec<GraphEdge>,
    seen_ids: &mut HashSet<String>,
    function_bodies: &mut Vec<(String, usize, usize)>,
) {
    let name = match get_name(node, source, "name") {
        Some(n) => n,
        None => return,
    };
    let line = node.start_position().row + 1;
    let nid = make_id(&[str_path, &name]);

    // Determine struct vs interface by looking at the type child
    let node_type = {
        let mut nt = NodeType::Struct;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "interface_type" => {
                    nt = NodeType::Interface;
                    break;
                }
                "struct_type" => {
                    nt = NodeType::Struct;
                    break;
                }
                _ => {}
            }
        }
        nt
    };

    if seen_ids.insert(nid.clone()) {
        nodes.push(GraphNode {
            id: nid.clone(),
            label: name.clone(),
            source_file: str_path.to_string(),
            source_location: Some(format!("L{line}")),
            node_type,
            community: None,
            extra: HashMap::new(),
        });
        edges.push(make_edge(file_nid, &nid, "defines", str_path, line));
    }

    // Recurse into body for any child methods (Go doesn't nest methods in struct body,
    // but interfaces have method specs)
    if let Some(body) = node.child_by_field_name(config.body_field) {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            walk_node(
                child,
                source,
                config,
                lang,
                file_nid,
                stem,
                str_path,
                nodes,
                edges,
                seen_ids,
                function_bodies,
                Some(&nid),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_rust_impl(
    node: Node,
    source: &[u8],
    config: &TsConfig,
    lang: &str,
    file_nid: &str,
    stem: &str,
    str_path: &str,
    nodes: &mut Vec<GraphNode>,
    edges: &mut Vec<GraphEdge>,
    seen_ids: &mut HashSet<String>,
    function_bodies: &mut Vec<(String, usize, usize)>,
) {
    // `impl [Trait for] Type { ... }`
    // The type is the `type` field, the trait is the `trait` field
    let type_name = node
        .child_by_field_name("type")
        .map(|n| node_text(n, source));
    let trait_name = node
        .child_by_field_name("trait")
        .map(|n| node_text(n, source));

    let impl_target_nid = type_name.as_ref().map(|tn| make_id(&[str_path, tn]));

    // Create an "implements" edge if trait impl
    if let (Some(trait_n), Some(target_nid)) = (&trait_name, &impl_target_nid) {
        let line = node.start_position().row + 1;
        let trait_nid = make_id(&[str_path, trait_n]);
        edges.push(GraphEdge {
            source: target_nid.clone(),
            target: trait_nid,
            relation: "implements".to_string(),
            confidence: Confidence::Extracted,
            confidence_score: Confidence::Extracted.default_score(),
            source_file: str_path.to_string(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            extra: HashMap::new(),
        });
    }

    // Recurse into body to find methods, treating them as methods of the impl target
    if let Some(body) = node.child_by_field_name(config.body_field) {
        let class_nid = impl_target_nid.as_deref();
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            walk_node(
                child,
                source,
                config,
                lang,
                file_nid,
                stem,
                str_path,
                nodes,
                edges,
                seen_ids,
                function_bodies,
                class_nid,
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Function handler
// ═══════════════════════════════════════════════════════════════════════════

#[allow(clippy::too_many_arguments)]
fn handle_function(
    node: Node,
    source: &[u8],
    config: &TsConfig,
    _lang: &str,
    file_nid: &str,
    _stem: &str,
    str_path: &str,
    nodes: &mut Vec<GraphNode>,
    edges: &mut Vec<GraphEdge>,
    seen_ids: &mut HashSet<String>,
    function_bodies: &mut Vec<(String, usize, usize)>,
    parent_class_nid: Option<&str>,
) {
    // For JS arrow functions assigned to a variable, the name is on the parent
    // `variable_declarator` node. But for function_declaration, method_definition,
    // etc., the name is directly on the node.
    let func_name = match get_name(node, source, config.name_field) {
        Some(n) => n,
        None => {
            // For JS arrow functions, try to get name from parent variable_declarator
            if node.kind() == "arrow_function" {
                if let Some(parent) = node.parent() {
                    if parent.kind() == "variable_declarator" {
                        match get_name(parent, source, "name") {
                            Some(n) => n,
                            None => return,
                        }
                    } else {
                        return;
                    }
                } else {
                    return;
                }
            } else {
                return;
            }
        }
    };

    let line = node.start_position().row + 1;

    let (func_nid, label, node_type, relation) = if let Some(class_nid) = parent_class_nid {
        let nid = make_id(&[class_nid, &func_name]);
        (
            nid,
            format!(".{}()", func_name),
            NodeType::Method,
            "defines",
        )
    } else {
        let nid = make_id(&[str_path, &func_name]);
        (
            nid,
            format!("{}()", func_name),
            NodeType::Function,
            "defines",
        )
    };

    if seen_ids.insert(func_nid.clone()) {
        nodes.push(GraphNode {
            id: func_nid.clone(),
            label,
            source_file: str_path.to_string(),
            source_location: Some(format!("L{line}")),
            node_type,
            community: None,
            extra: HashMap::new(),
        });

        let parent_nid = parent_class_nid.unwrap_or(file_nid);
        edges.push(make_edge(parent_nid, &func_nid, relation, str_path, line));
    }

    // Record the function body bytes for call-graph inference
    if let Some(body) = node.child_by_field_name(config.body_field) {
        function_bodies.push((func_nid, body.start_byte(), body.end_byte()));
    } else {
        // Fallback: use the whole node as body
        function_bodies.push((func_nid, node.start_byte(), node.end_byte()));
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Import handler
// ═══════════════════════════════════════════════════════════════════════════

fn extract_import(
    node: Node,
    source: &[u8],
    file_nid: &str,
    str_path: &str,
    lang: &str,
    edges: &mut Vec<GraphEdge>,
    nodes: &mut Vec<GraphNode>,
) {
    let line = node.start_position().row + 1;
    let import_text = node_text(node, source);

    match lang {
        "python" => extract_python_import(node, source, file_nid, str_path, line, edges, nodes),
        "javascript" | "typescript" => {
            extract_js_import(node, source, file_nid, str_path, line, edges, nodes)
        }
        "rust" => {
            // `use foo::bar::Baz;` → module = full text after "use"
            let module = import_text
                .strip_prefix("use ")
                .unwrap_or(&import_text)
                .trim_end_matches(';')
                .trim();
            add_import_node(
                nodes,
                edges,
                file_nid,
                str_path,
                line,
                module,
                NodeType::Module,
            );
        }
        "go" => {
            extract_go_import(node, source, file_nid, str_path, line, edges, nodes);
        }
        "java" => {
            // `import java.util.List;` → extract path after "import"
            let text = node_text(node, source);
            let module = text
                .trim()
                .strip_prefix("import ")
                .unwrap_or(&text)
                .strip_prefix("static ")
                .unwrap_or_else(|| text.trim().strip_prefix("import ").unwrap_or(&text))
                .trim_end_matches(';')
                .trim();
            add_import_node(
                nodes,
                edges,
                file_nid,
                str_path,
                line,
                module,
                NodeType::Module,
            );
        }
        "c" | "cpp" => {
            // `#include <stdio.h>` or `#include "myheader.h"`
            let text = node_text(node, source);
            let module = text
                .trim()
                .strip_prefix("#include")
                .unwrap_or(&text)
                .trim()
                .trim_matches(&['<', '>', '"'][..])
                .trim();
            add_import_node(
                nodes,
                edges,
                file_nid,
                str_path,
                line,
                module,
                NodeType::Module,
            );
        }
        "csharp" => {
            // `using System.Collections.Generic;`
            let text = node_text(node, source);
            let module = text
                .trim()
                .strip_prefix("using ")
                .unwrap_or(&text)
                .trim_end_matches(';')
                .trim();
            add_import_node(
                nodes,
                edges,
                file_nid,
                str_path,
                line,
                module,
                NodeType::Module,
            );
        }
        _ => {
            add_import_node(
                nodes,
                edges,
                file_nid,
                str_path,
                line,
                &import_text,
                NodeType::Module,
            );
        }
    }
}

fn extract_python_import(
    node: Node,
    source: &[u8],
    file_nid: &str,
    str_path: &str,
    line: usize,
    edges: &mut Vec<GraphEdge>,
    nodes: &mut Vec<GraphNode>,
) {
    // `import_statement`: `import os` → child "dotted_name"
    // `import_from_statement`: `from pathlib import Path` → module_name + name children
    let kind = node.kind();

    if kind == "import_from_statement" {
        let module = node
            .child_by_field_name("module_name")
            .map(|n| node_text(n, source))
            .unwrap_or_default();
        // Iterate over named import children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "dotted_name" || child.kind() == "aliased_import" {
                let name_node = if child.kind() == "aliased_import" {
                    child.child_by_field_name("name")
                } else {
                    Some(child)
                };
                if let Some(nn) = name_node {
                    let name = node_text(nn, source);
                    if name != module {
                        let full = if module.is_empty() {
                            name
                        } else {
                            format!("{module}.{name}")
                        };
                        add_import_node(
                            nodes,
                            edges,
                            file_nid,
                            str_path,
                            line,
                            &full,
                            NodeType::Module,
                        );
                    }
                }
            }
        }
        // If no names were added (e.g. `from x import *`), add the module
        let import_count = edges.iter().filter(|e| e.relation == "imports").count();
        if import_count == 0 && !module.is_empty() {
            add_import_node(
                nodes,
                edges,
                file_nid,
                str_path,
                line,
                &module,
                NodeType::Module,
            );
        }
    } else {
        // `import os`, `import os.path`
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "dotted_name" || child.kind() == "aliased_import" {
                let name_node = if child.kind() == "aliased_import" {
                    child.child_by_field_name("name")
                } else {
                    Some(child)
                };
                if let Some(nn) = name_node {
                    let name = node_text(nn, source);
                    add_import_node(
                        nodes,
                        edges,
                        file_nid,
                        str_path,
                        line,
                        &name,
                        NodeType::Module,
                    );
                }
            }
        }
    }
}

fn extract_js_import(
    node: Node,
    source: &[u8],
    file_nid: &str,
    str_path: &str,
    line: usize,
    edges: &mut Vec<GraphEdge>,
    nodes: &mut Vec<GraphNode>,
) {
    // JS import: `import { X, Y } from 'module'` or `import X from 'module'`
    // The source/module is in the `source` field
    let module = node
        .child_by_field_name("source")
        .map(|n| {
            let t = node_text(n, source);
            t.trim_matches(&['"', '\''][..]).to_string()
        })
        .unwrap_or_default();

    // Collect imported identifiers
    let mut found_names = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_clause" {
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                match inner.kind() {
                    "identifier" => {
                        let name = node_text(inner, source);
                        let full = format!("{module}/{name}");
                        add_import_node(
                            nodes,
                            edges,
                            file_nid,
                            str_path,
                            line,
                            &full,
                            NodeType::Module,
                        );
                        found_names = true;
                    }
                    "named_imports" => {
                        let mut spec_cursor = inner.walk();
                        for spec in inner.children(&mut spec_cursor) {
                            if spec.kind() == "import_specifier" {
                                let name = spec
                                    .child_by_field_name("name")
                                    .map(|n| node_text(n, source))
                                    .unwrap_or_else(|| node_text(spec, source));
                                let full = format!("{module}/{name}");
                                add_import_node(
                                    nodes,
                                    edges,
                                    file_nid,
                                    str_path,
                                    line,
                                    &full,
                                    NodeType::Module,
                                );
                                found_names = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    if !found_names && !module.is_empty() {
        add_import_node(
            nodes,
            edges,
            file_nid,
            str_path,
            line,
            &module,
            NodeType::Module,
        );
    }
}

fn extract_go_import(
    node: Node,
    source: &[u8],
    file_nid: &str,
    str_path: &str,
    line: usize,
    edges: &mut Vec<GraphEdge>,
    nodes: &mut Vec<GraphNode>,
) {
    // Go imports: `import "fmt"` or `import ( "fmt" \n "os" )`
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_spec" => {
                if let Some(path_node) = child.child_by_field_name("path") {
                    let module = node_text(path_node, source).trim_matches('"').to_string();
                    let spec_line = child.start_position().row + 1;
                    add_import_node(
                        nodes,
                        edges,
                        file_nid,
                        str_path,
                        spec_line,
                        &module,
                        NodeType::Package,
                    );
                }
            }
            "import_spec_list" => {
                let mut inner = child.walk();
                for spec in child.children(&mut inner) {
                    if spec.kind() == "import_spec" {
                        if let Some(path_node) = spec.child_by_field_name("path") {
                            let module = node_text(path_node, source).trim_matches('"').to_string();
                            let spec_line = spec.start_position().row + 1;
                            add_import_node(
                                nodes,
                                edges,
                                file_nid,
                                str_path,
                                spec_line,
                                &module,
                                NodeType::Package,
                            );
                        }
                    }
                }
            }
            "interpreted_string_literal" => {
                // Single import: `import "fmt"`
                let module = node_text(child, source).trim_matches('"').to_string();
                add_import_node(
                    nodes,
                    edges,
                    file_nid,
                    str_path,
                    line,
                    &module,
                    NodeType::Package,
                );
            }
            _ => {}
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Extract text from a tree-sitter node.
fn node_text(node: Node, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

/// Get the name of a definition node via its field name.
fn get_name(node: Node, source: &[u8], field: &str) -> Option<String> {
    let name_node = node.child_by_field_name(field)?;
    // For C/C++ declarators, unwrap nested declarators to find the identifier
    let text = unwrap_declarator_name(name_node, source);
    if text.is_empty() { None } else { Some(text) }
}

/// Recursively unwrap C/C++ declarators (function_declarator, pointer_declarator, etc.)
/// to find the underlying identifier name.
fn unwrap_declarator_name(node: Node, source: &[u8]) -> String {
    match node.kind() {
        "function_declarator"
        | "pointer_declarator"
        | "reference_declarator"
        | "parenthesized_declarator" => {
            // The actual name is in the "declarator" field or first named child
            if let Some(inner) = node.child_by_field_name("declarator") {
                return unwrap_declarator_name(inner, source);
            }
            // Fallback: look for an identifier child
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" || child.kind() == "field_identifier" {
                    return node_text(child, source);
                }
            }
            node_text(node, source)
        }
        "qualified_identifier" | "scoped_identifier" => {
            // C++ qualified names like `Foo::bar` — use the "name" field
            if let Some(name) = node.child_by_field_name("name") {
                return node_text(name, source);
            }
            node_text(node, source)
        }
        _ => node_text(node, source),
    }
}

fn add_import_node(
    nodes: &mut Vec<GraphNode>,
    edges: &mut Vec<GraphEdge>,
    file_nid: &str,
    str_path: &str,
    line: usize,
    module: &str,
    node_type: NodeType,
) {
    let import_id = make_id(&[str_path, "import", module]);
    nodes.push(GraphNode {
        id: import_id.clone(),
        label: module.to_string(),
        source_file: str_path.to_string(),
        source_location: Some(format!("L{line}")),
        node_type,
        community: None,
        extra: HashMap::new(),
    });
    edges.push(GraphEdge {
        source: file_nid.to_string(),
        target: import_id,
        relation: "imports".to_string(),
        confidence: Confidence::Extracted,
        confidence_score: Confidence::Extracted.default_score(),
        source_file: str_path.to_string(),
        source_location: Some(format!("L{line}")),
        weight: 1.0,
        extra: HashMap::new(),
    });
}

fn make_edge(
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

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // ----- Python -----

    #[test]
    fn ts_python_extracts_class_and_methods() {
        let source = br#"
class MyClass:
    def __init__(self):
        pass

    def greet(self, name):
        return f"Hello {name}"

def standalone():
    pass
"#;
        let result = try_extract(Path::new("test.py"), source, "python").unwrap();

        let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
        assert!(
            labels.iter().any(|l| l.contains("MyClass")),
            "missing MyClass: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("__init__")),
            "missing __init__: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("greet")),
            "missing greet: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("standalone")),
            "missing standalone: {labels:?}"
        );
        assert!(result.nodes.iter().any(|n| n.node_type == NodeType::File));
        assert!(result.nodes.iter().any(|n| n.node_type == NodeType::Class));
    }

    #[test]
    fn ts_python_extracts_imports() {
        let source = br#"
import os
from pathlib import Path
from collections import defaultdict, OrderedDict
"#;
        let result = try_extract(Path::new("test.py"), source, "python").unwrap();
        let import_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.relation == "imports")
            .collect();
        assert!(
            import_edges.len() >= 2,
            "expected >= 2 import edges, got {}",
            import_edges.len()
        );
    }

    #[test]
    fn ts_python_infers_calls() {
        let source = br#"
def foo():
    bar()

def bar():
    pass
"#;
        let result = try_extract(Path::new("test.py"), source, "python").unwrap();
        let call_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.relation == "calls")
            .collect();
        assert!(!call_edges.is_empty(), "expected call edges");
    }

    // ----- Rust -----

    #[test]
    fn ts_rust_extracts_structs_and_functions() {
        let source = br#"
use std::collections::HashMap;

pub struct Config {
    name: String,
}

pub enum Status {
    Active,
    Inactive,
}

pub trait Runnable {
    fn run(&self);
}

impl Runnable for Config {
    fn run(&self) {
        println!("{}", self.name);
    }
}

pub fn main() {
    let c = Config { name: "test".into() };
    c.run();
}
"#;
        let result = try_extract(Path::new("lib.rs"), source, "rust").unwrap();
        let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
        assert!(
            labels.iter().any(|l| l.contains("Config")),
            "missing Config: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("Status")),
            "missing Status: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("Runnable")),
            "missing Runnable: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("main")),
            "missing main: {labels:?}"
        );
        assert!(result.nodes.iter().any(|n| n.node_type == NodeType::Struct));
        assert!(result.nodes.iter().any(|n| n.node_type == NodeType::Enum));
        assert!(result.nodes.iter().any(|n| n.node_type == NodeType::Trait));
        assert!(
            result.edges.iter().any(|e| e.relation == "implements"),
            "missing implements edge"
        );
    }

    // ----- JavaScript -----

    #[test]
    fn ts_js_extracts_functions_and_classes() {
        let source = br#"
import { useState } from 'react';
import axios from 'axios';

export class ApiClient {
    constructor(baseUrl) {
        this.baseUrl = baseUrl;
    }
}

export function fetchData(url) {
    return axios.get(url);
}
"#;
        let result = try_extract(Path::new("api.js"), source, "javascript").unwrap();
        let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
        assert!(
            labels.iter().any(|l| l.contains("ApiClient")),
            "missing ApiClient: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("fetchData")),
            "missing fetchData: {labels:?}"
        );

        let import_count = result
            .edges
            .iter()
            .filter(|e| e.relation == "imports")
            .count();
        assert!(
            import_count >= 2,
            "expected >=2 imports, got {import_count}"
        );
    }

    // ----- Go -----

    #[test]
    fn ts_go_extracts_types_and_functions() {
        let source = br#"
package main

import (
    "fmt"
    "os"
)

type Server struct {
    host string
    port int
}

type Handler interface {
    Handle()
}

func (s *Server) Start() {
    fmt.Println("starting")
}

func main() {
    s := Server{host: "localhost", port: 8080}
    s.Start()
}
"#;
        let result = try_extract(Path::new("main.go"), source, "go").unwrap();
        let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
        assert!(
            labels.iter().any(|l| l.contains("Server")),
            "missing Server: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("Handler")),
            "missing Handler: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("Start")),
            "missing Start: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("main")),
            "missing main: {labels:?}"
        );
        assert!(result.nodes.iter().any(|n| n.node_type == NodeType::Struct));
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.node_type == NodeType::Interface)
        );
    }

    // ----- Unsupported language returns None -----

    #[test]
    fn ts_unsupported_returns_none() {
        assert!(try_extract(Path::new("test.pl"), b"sub foo { 1 }", "perl").is_none());
    }

    // ----- Tree-sitter at least matches regex node count -----

    #[test]
    fn ts_python_at_least_as_many_nodes_as_regex() {
        let source_str = r#"
class MyClass:
    def __init__(self):
        pass

    def greet(self, name):
        return f"Hello {name}"

def standalone():
    pass
"#;
        let regex_result =
            crate::ast_extract::extract_file(Path::new("test.py"), source_str, "python");
        let ts_result = try_extract(Path::new("test.py"), source_str.as_bytes(), "python").unwrap();

        assert!(
            ts_result.nodes.len() >= regex_result.nodes.len(),
            "tree-sitter ({}) should produce >= nodes than regex ({})",
            ts_result.nodes.len(),
            regex_result.nodes.len()
        );
    }

    #[test]
    fn all_edges_have_source_file() {
        let source = b"def foo():\n    bar()\ndef bar():\n    pass\n";
        let result = try_extract(Path::new("x.py"), source, "python").unwrap();
        for edge in &result.edges {
            assert!(!edge.source_file.is_empty());
        }
    }

    #[test]
    fn node_ids_are_deterministic() {
        let source = b"def foo():\n    pass\n";
        let r1 = try_extract(Path::new("test.py"), source, "python").unwrap();
        let r2 = try_extract(Path::new("test.py"), source, "python").unwrap();
        assert_eq!(r1.nodes.len(), r2.nodes.len());
        for (a, b) in r1.nodes.iter().zip(r2.nodes.iter()) {
            assert_eq!(a.id, b.id);
        }
    }

    // ----- Java -----

    #[test]
    fn ts_java_extracts_class_and_methods() {
        let source = br#"
import java.util.List;

public class Foo {
    public void bar() {}
    public int baz(String s) { return 0; }
}
"#;
        let result = try_extract(Path::new("Foo.java"), source, "java").unwrap();
        let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
        assert!(
            labels.iter().any(|l| l.contains("Foo")),
            "missing Foo: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("bar")),
            "missing bar: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("baz")),
            "missing baz: {labels:?}"
        );
        let import_count = result
            .edges
            .iter()
            .filter(|e| e.relation == "imports")
            .count();
        assert!(
            import_count >= 1,
            "expected >=1 imports, got {import_count}"
        );
    }

    #[test]
    fn ts_java_extracts_interface() {
        let source = br#"
public interface Runnable {
    void run();
}
"#;
        let result = try_extract(Path::new("Runnable.java"), source, "java").unwrap();
        let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
        assert!(
            labels.iter().any(|l| l.contains("Runnable")),
            "missing Runnable: {labels:?}"
        );
    }

    // ----- C -----

    #[test]
    fn ts_c_extracts_functions() {
        let source = br#"
#include <stdio.h>

int main(int argc, char **argv) {
    printf("hello\n");
    return 0;
}

void helper(void) {}
"#;
        let result = try_extract(Path::new("main.c"), source, "c").unwrap();
        let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
        assert!(
            labels.iter().any(|l| l.contains("main")),
            "missing main: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("helper")),
            "missing helper: {labels:?}"
        );
        let import_count = result
            .edges
            .iter()
            .filter(|e| e.relation == "imports")
            .count();
        assert!(
            import_count >= 1,
            "expected >=1 imports, got {import_count}"
        );
    }

    // ----- C++ -----

    #[test]
    fn ts_cpp_extracts_class_and_functions() {
        let source = br#"
#include <iostream>

class Greeter {
public:
    void greet() {
        std::cout << "hello" << std::endl;
    }
};

int main() {
    Greeter g;
    g.greet();
    return 0;
}
"#;
        let result = try_extract(Path::new("main.cpp"), source, "cpp").unwrap();
        let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
        assert!(
            labels.iter().any(|l| l.contains("Greeter")),
            "missing Greeter: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("main")),
            "missing main: {labels:?}"
        );
    }

    // ----- Ruby -----

    #[test]
    fn ts_ruby_extracts_class_and_methods() {
        let source = br#"
class Dog
  def initialize(name)
    @name = name
  end

  def bark
    puts "Woof!"
  end
end
"#;
        let result = try_extract(Path::new("dog.rb"), source, "ruby").unwrap();
        let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
        assert!(
            labels.iter().any(|l| l.contains("Dog")),
            "missing Dog: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("initialize")),
            "missing initialize: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("bark")),
            "missing bark: {labels:?}"
        );
    }

    // ----- C# -----

    #[test]
    fn ts_csharp_extracts_class_and_methods() {
        let source = br#"
using System;
using System.Collections.Generic;

public class Calculator {
    public int Add(int a, int b) {
        return a + b;
    }

    public int Subtract(int a, int b) {
        return a - b;
    }
}
"#;
        let result = try_extract(Path::new("Calculator.cs"), source, "csharp").unwrap();
        let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
        assert!(
            labels.iter().any(|l| l.contains("Calculator")),
            "missing Calculator: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("Add")),
            "missing Add: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("Subtract")),
            "missing Subtract: {labels:?}"
        );
        let import_count = result
            .edges
            .iter()
            .filter(|e| e.relation == "imports")
            .count();
        assert!(
            import_count >= 2,
            "expected >=2 imports, got {import_count}"
        );
    }
}
