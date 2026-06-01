//! Tree-sitter import extraction functions.

use std::collections::HashMap;

use graphify_core::confidence::Confidence;
use graphify_core::id::make_id;
use graphify_core::model::{GraphEdge, GraphNode, NodeType};
use tree_sitter::Node;

use super::node_text;

pub(crate) fn extract_import(
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
            extract_js_import(node, source, file_nid, str_path, line, edges, nodes);
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

pub(crate) fn extract_python_import(
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
        let edges_before = edges.len();
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

pub(crate) fn extract_js_import(
    node: Node,
    source: &[u8],
    file_nid: &str,
    str_path: &str,
    line: usize,
    edges: &mut Vec<GraphEdge>,
    nodes: &mut Vec<GraphNode>,
) {
    let module = node
        .child_by_field_name("source")
        .map(|n| {
            let t = node_text(n, source);
            t.trim_matches(&['"', '\''][..]).to_string()
        })
        .unwrap_or_default();

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
                                let name = spec.child_by_field_name("name").map_or_else(
                                    || node_text(spec, source),
                                    |n| node_text(n, source),
                                );
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

pub(crate) fn extract_go_import(
    node: Node,
    source: &[u8],
    file_nid: &str,
    str_path: &str,
    line: usize,
    edges: &mut Vec<GraphEdge>,
    nodes: &mut Vec<GraphNode>,
) {
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

pub(crate) fn extract_ruby_import(
    node: Node,
    source: &[u8],
    file_nid: &str,
    str_path: &str,
    line: usize,
    edges: &mut Vec<GraphEdge>,
    nodes: &mut Vec<GraphNode>,
) {
    let method_name = node
        .child_by_field_name("method")
        .map(|n| node_text(n, source))
        .unwrap_or_default();

    if method_name != "require" && method_name != "require_relative" {
        return; // Not an import call, skip
    }

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

pub(crate) fn extract_dart_import(
    node: Node,
    source: &[u8],
    file_nid: &str,
    str_path: &str,
    line: usize,
    edges: &mut Vec<GraphEdge>,
    nodes: &mut Vec<GraphNode>,
) {
    let text = node_text(node, source);
    let trimmed = text.trim().trim_end_matches(';').trim();

    let module = trimmed
        .strip_prefix("part of ")
        .or_else(|| trimmed.strip_prefix("part "))
        .or_else(|| trimmed.strip_prefix("import "))
        .or_else(|| trimmed.strip_prefix("export "))
        .unwrap_or(trimmed)
        .trim()
        .trim_matches(&['"', '\''][..])
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

/// Extract text from a tree-sitter node.
pub(crate) fn add_import_node(
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
