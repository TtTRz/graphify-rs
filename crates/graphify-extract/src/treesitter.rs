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
            "generator_function",
            "async_function_declaration",
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
        class_types: [
            "class_declaration",
            "interface_declaration",
            "enum_declaration",
        ]
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
        class_types: ["struct_specifier", "enum_specifier", "type_definition"]
            .into_iter()
            .collect(),
        function_types: ["function_definition"].into_iter().collect(),
        import_types: ["preproc_include"].into_iter().collect(),
        call_types: ["call_expression"].into_iter().collect(),
        name_field: "declarator",
        class_name_field: Some("name"),
        body_field: "body",
        call_function_field: "function",
    }
}

fn cpp_config() -> TsConfig {
    TsConfig {
        class_types: [
            "class_specifier",
            "struct_specifier",
            "enum_specifier",
            "namespace_definition",
        ]
        .into_iter()
        .collect(),
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
        class_types: ["class", "module"].into_iter().collect(),
        function_types: ["method", "singleton_method"].into_iter().collect(),
        import_types: ["call"].into_iter().collect(), // require/require_relative are method calls
        call_types: ["call"].into_iter().collect(),
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "method",
    }
}

fn csharp_config() -> TsConfig {
    TsConfig {
        class_types: [
            "class_declaration",
            "interface_declaration",
            "struct_declaration",
            "enum_declaration",
        ]
        .into_iter()
        .collect(),
        function_types: ["method_declaration", "constructor_declaration"]
            .into_iter()
            .collect(),
        import_types: ["using_directive"].into_iter().collect(),
        call_types: ["invocation_expression"].into_iter().collect(),
        name_field: "name",
        class_name_field: None,
        body_field: "body",
        call_function_field: "function",
    }
}

fn dart_config() -> TsConfig {
    TsConfig {
        class_types: [
            "class_definition",
            "enum_declaration",
            "mixin_declaration",
            "extension_declaration",
        ]
        .into_iter()
        .collect(),
        function_types: [
            "function_signature",
            "method_signature",
            "function_body",
            "function_declaration",
            "method_definition",
        ]
        .into_iter()
        .collect(),
        import_types: ["import_or_export", "part_directive", "part_of_directive"]
            .into_iter()
            .collect(),
        call_types: ["method_invocation", "function_expression_invocation"]
            .into_iter()
            .collect(),
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
        "dart" => (tree_sitter_dart::LANGUAGE.into(), dart_config()),
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
        let body_lower = body_str.to_lowercase();
        for (func_label, callee_nid) in &label_to_nid {
            if callee_nid == caller_nid {
                continue;
            }
            // Heuristic: look for `func_name(` in body, or for Ruby-style no-parens calls
            let has_paren_call = body_lower.contains(&format!("{func_label}("));
            let has_noparen_call = if lang == "ruby" {
                // Ruby allows `func arg` or `func\n` — check if func_label appears
                // as a standalone word (not part of a longer identifier)
                body_lower.find(func_label.as_str()).is_some_and(|pos| {
                    let after = pos + func_label.len();
                    if after >= body_lower.len() {
                        true // at end of body
                    } else {
                        let next_ch = body_lower.as_bytes()[after];
                        // Must be followed by non-alphanumeric (space, newline, paren, etc.)
                        !next_ch.is_ascii_alphanumeric() && next_ch != b'_'
                    }
                })
            } else {
                false
            };
            if has_paren_call || has_noparen_call {
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
        // For Ruby, `call` is in both import_types and call_types.
        // Only treat require/require_relative as imports; let other calls recurse normally.
        if lang == "ruby" && kind == "call" {
            let method_name = node
                .child_by_field_name("method")
                .map(|n| node_text(n, source))
                .unwrap_or_default();
            if method_name == "require" || method_name == "require_relative" {
                extract_import(node, source, file_nid, str_path, lang, edges, nodes);
                return;
            }
            // Not a require call, fall through to normal processing
        } else {
            extract_import(node, source, file_nid, str_path, lang, edges, nodes);
            return; // Don't recurse into import children
        }
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
    match kind {
        // Rust
        "struct_item" => NodeType::Struct,
        "enum_item" => NodeType::Enum,
        "trait_item" => NodeType::Trait,
        // C / C++
        "struct_specifier" => NodeType::Struct,
        "enum_specifier" => NodeType::Enum,
        "namespace_definition" => NodeType::Namespace,
        // C#
        "struct_declaration" => NodeType::Struct,
        "enum_declaration" => match lang {
            "csharp" | "java" | "dart" => NodeType::Enum,
            _ => NodeType::Enum,
        },
        // Java / C#
        "interface_declaration" => NodeType::Interface,
        // Dart
        "mixin_declaration" | "extension_declaration" => NodeType::Class,
        // Ruby
        "module" => NodeType::Module,
        // C (type_definition is used for typedef'd structs/enums)
        "type_definition" => NodeType::Struct,
        // Default
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
/// Normalize Dart function names: handle getters/setters and named constructors
fn normalize_dart_function_name(lang: &str, func_name: &str) -> String {
    if lang != "dart" {
        return func_name.to_string();
    }

    let mut name = func_name;

    // Strip "get "/"set " prefix for Dart getters/setters (e.g., "get value" -> "value")
    if name.starts_with("get ") || name.starts_with("set ") {
        name = &name[4..];
    }

    name.to_string()
}

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
            } else if _lang == "dart" {
                // Dart function_signature/method_signature may not have a name field;
                // try to find the first identifier child as the function name
                let mut found = None;
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "identifier" {
                        found = Some(node_text(child, source));
                        break;
                    }
                }
                match found {
                    Some(n) if !n.is_empty() => n,
                    _ => return,
                }
            } else {
                return;
            }
        }
    };

    let normalized_name = normalize_dart_function_name(_lang, &func_name);
    let line = node.start_position().row + 1;

    let (func_nid, label, node_type, relation) = if let Some(class_nid) = parent_class_nid {
        let nid = make_id(&[class_nid, &normalized_name]);
        (
            nid,
            format!(".{}()", normalized_name),
            NodeType::Method,
            "defines",
        )
    } else {
        let nid = make_id(&[str_path, &normalized_name]);
        (
            nid,
            format!("{}()", normalized_name),
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
            // `import java.util.List;` or `import static java.util.Arrays.asList;`
            let text = node_text(node, source);
            let after_import = text.trim().strip_prefix("import ").unwrap_or(text.trim());
            let module = after_import
                .strip_prefix("static ")
                .unwrap_or(after_import)
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
        "ruby" => {
            extract_ruby_import(node, source, file_nid, str_path, line, edges, nodes);
        }
        "dart" => {
            extract_dart_import(node, source, file_nid, str_path, line, edges, nodes);
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
        // Track how many edges existed before this statement
        let edges_before = edges.len();
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
        // If no names were added by this statement (e.g. `from x import *`), add the module
        let new_edges = edges.len() - edges_before;
        if new_edges == 0 && !module.is_empty() {
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
                    if spec.kind() == "import_spec"
                        && let Some(path_node) = spec.child_by_field_name("path")
                    {
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

fn extract_ruby_import(
    node: Node,
    source: &[u8],
    file_nid: &str,
    str_path: &str,
    line: usize,
    edges: &mut Vec<GraphEdge>,
    nodes: &mut Vec<GraphNode>,
) {
    // Ruby imports are method calls: `require 'json'`, `require_relative 'helper'`
    // The tree-sitter node is a `call` with method=identifier("require"/"require_relative")
    // and arguments containing a string.
    let method_name = node
        .child_by_field_name("method")
        .map(|n| node_text(n, source))
        .unwrap_or_default();

    if method_name != "require" && method_name != "require_relative" {
        return; // Not an import call, skip
    }

    // Extract the argument string
    if let Some(args) = node.child_by_field_name("arguments") {
        let mut cursor = args.walk();
        for child in args.children(&mut cursor) {
            let kind = child.kind();
            if kind == "string" || kind == "string_literal" {
                let raw = node_text(child, source);
                let module = raw.trim_matches(&['"', '\''][..]).to_string();
                if !module.is_empty() {
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
                return;
            }
        }
    }

    // Fallback: try parsing from the raw text
    let text = node_text(node, source);
    let module = text
        .trim()
        .strip_prefix("require_relative ")
        .or_else(|| text.trim().strip_prefix("require "))
        .unwrap_or(&text)
        .trim_matches(&['"', '\'', ' '][..]);
    if !module.is_empty() {
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
}

fn extract_dart_import(
    node: Node,
    source: &[u8],
    file_nid: &str,
    str_path: &str,
    line: usize,
    edges: &mut Vec<GraphEdge>,
    nodes: &mut Vec<GraphNode>,
) {
    // Dart: `import 'dart:async';`, `part 'src/models.dart';`
    let text = node_text(node, source);
    let trimmed = text.trim().trim_end_matches(';').trim();

    // Strip keyword prefix
    let module = trimmed
        .strip_prefix("part of ")
        .or_else(|| trimmed.strip_prefix("part "))
        .or_else(|| trimmed.strip_prefix("import "))
        .or_else(|| trimmed.strip_prefix("export "))
        .unwrap_or(trimmed)
        .trim()
        .trim_matches(&['"', '\''][..])
        // Remove `deferred as X`, `as X`, `show X`, `hide X` suffixes
        .split(" deferred ")
        .next()
        .unwrap_or("")
        .split(" as ")
        .next()
        .unwrap_or("")
        .split(" show ")
        .next()
        .unwrap_or("")
        .split(" hide ")
        .next()
        .unwrap_or("")
        .trim();

    if !module.is_empty() {
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

// Tests moved to tests/treesitter.rs (integration tests)
