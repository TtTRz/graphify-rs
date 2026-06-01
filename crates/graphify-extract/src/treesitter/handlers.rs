//! Tree-sitter node handler functions.

use std::collections::HashMap;

use graphify_core::confidence::Confidence;
use graphify_core::id::make_id;
use graphify_core::model::{GraphEdge, GraphNode, NodeType};
use tree_sitter::Node;

use super::treesitter_config::TsConfig;
use super::{WalkContext, get_name, make_edge, node_text, walk_node};

pub(crate) fn handle_class_like(
    node: Node,
    source: &[u8],
    config: &TsConfig,
    ctx: &mut WalkContext,
) {
    let kind = node.kind();

    if ctx.lang == "go" && kind == "type_declaration" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "type_spec" {
                handle_go_type_spec(child, source, config, ctx);
            }
        }
        return;
    }

    if ctx.lang == "rust" && kind == "impl_item" {
        handle_rust_impl(node, source, config, ctx);
        return;
    }

    let class_field = config.class_name_field.unwrap_or(config.name_field);
    let name = match get_name(node, source, class_field) {
        Some(n) => n,
        None => return,
    };
    let line = node.start_position().row + 1;
    let class_nid = make_id(&[ctx.str_path, &name]);

    let node_type = classify_class_kind(kind, ctx.lang);

    if ctx.seen_ids.insert(class_nid.clone()) {
        ctx.nodes.push(GraphNode {
            id: class_nid.clone(),
            label: name.clone(),
            source_file: ctx.str_path.to_string(),
            source_location: Some(format!("L{line}")),
            node_type,
            community: None,
            extra: HashMap::new(),
        });
        ctx.edges.push(make_edge(
            ctx.file_nid,
            &class_nid,
            "defines",
            ctx.str_path,
            line,
        ));
    }

    if let Some(body) = node.child_by_field_name(config.body_field) {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            walk_node(child, source, config, ctx, Some(&class_nid));
        }
    }
}

pub(crate) fn classify_class_kind(kind: &str, _lang: &str) -> NodeType {
    match kind {
        "struct_item" => NodeType::Struct,
        "enum_item" => NodeType::Enum,
        "trait_item" => NodeType::Trait,
        "struct_specifier" => NodeType::Struct,
        "enum_specifier" => NodeType::Enum,
        "namespace_definition" => NodeType::Namespace,
        "struct_declaration" => NodeType::Struct,
        "enum_declaration" => NodeType::Enum,
        "interface_declaration" => NodeType::Interface,
        "mixin_declaration" | "extension_declaration" => NodeType::Class,
        "module" => NodeType::Module,
        "type_definition" => NodeType::Struct,
        _ => NodeType::Class,
    }
}

pub(crate) fn handle_go_type_spec(
    node: Node,
    source: &[u8],
    config: &TsConfig,
    ctx: &mut WalkContext,
) {
    let name = match get_name(node, source, "name") {
        Some(n) => n,
        None => return,
    };
    let line = node.start_position().row + 1;
    let nid = make_id(&[ctx.str_path, &name]);

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

    if ctx.seen_ids.insert(nid.clone()) {
        ctx.nodes.push(GraphNode {
            id: nid.clone(),
            label: name.clone(),
            source_file: ctx.str_path.to_string(),
            source_location: Some(format!("L{line}")),
            node_type,
            community: None,
            extra: HashMap::new(),
        });
        ctx.edges
            .push(make_edge(ctx.file_nid, &nid, "defines", ctx.str_path, line));
    }

    if let Some(body) = node.child_by_field_name(config.body_field) {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            walk_node(child, source, config, ctx, Some(&nid));
        }
    }
}

pub(crate) fn handle_rust_impl(
    node: Node,
    source: &[u8],
    config: &TsConfig,
    ctx: &mut WalkContext,
) {
    let type_name = node
        .child_by_field_name("type")
        .map(|n| node_text(n, source));
    let trait_name = node
        .child_by_field_name("trait")
        .map(|n| node_text(n, source));

    let impl_target_nid = type_name.as_ref().map(|tn| make_id(&[ctx.str_path, tn]));

    if let (Some(trait_n), Some(target_nid)) = (&trait_name, &impl_target_nid) {
        let line = node.start_position().row + 1;
        let trait_nid = make_id(&[ctx.str_path, trait_n]);
        ctx.edges.push(GraphEdge {
            source: target_nid.clone(),
            target: trait_nid,
            relation: "implements".to_string(),
            confidence: Confidence::Extracted,
            confidence_score: Confidence::Extracted.default_score(),
            source_file: ctx.str_path.to_string(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            extra: HashMap::new(),
        });
    }

    if let Some(body) = node.child_by_field_name(config.body_field) {
        let class_nid = impl_target_nid.as_deref();
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            walk_node(child, source, config, ctx, class_nid);
        }
    }
}

pub(crate) fn normalize_dart_function_name(lang: &str, func_name: &str) -> String {
    if lang != "dart" {
        return func_name.to_string();
    }

    let mut name = func_name;
    if name.starts_with("get ") || name.starts_with("set ") {
        name = &name[4..];
    }
    name.to_string()
}

pub(crate) fn handle_function(
    node: Node,
    source: &[u8],
    config: &TsConfig,
    ctx: &mut WalkContext,
    parent_class_nid: Option<&str>,
) {
    let func_name = match get_name(node, source, config.name_field) {
        Some(n) => n,
        None => {
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
            } else if ctx.lang == "dart" {
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

    let normalized_name = normalize_dart_function_name(ctx.lang, &func_name);
    let line = node.start_position().row + 1;

    let (func_nid, label, node_type, relation) = if let Some(class_nid) = parent_class_nid {
        let nid = make_id(&[class_nid, &normalized_name]);
        (
            nid,
            format!(".{normalized_name}()"),
            NodeType::Method,
            "defines",
        )
    } else {
        let nid = make_id(&[ctx.str_path, &normalized_name]);
        (
            nid,
            format!("{normalized_name}()"),
            NodeType::Function,
            "defines",
        )
    };

    if ctx.seen_ids.insert(func_nid.clone()) {
        ctx.nodes.push(GraphNode {
            id: func_nid.clone(),
            label,
            source_file: ctx.str_path.to_string(),
            source_location: Some(format!("L{line}")),
            node_type,
            community: None,
            extra: HashMap::new(),
        });

        let parent_nid = parent_class_nid.unwrap_or(ctx.file_nid);
        ctx.edges.push(make_edge(
            parent_nid,
            &func_nid,
            relation,
            ctx.str_path,
            line,
        ));
    }

    let body_node = node.child_by_field_name(config.body_field).unwrap_or(node);
    let callees = super::collect_callees(body_node, source, config);
    for name in callees {
        ctx.raw_calls.push((func_nid.clone(), name));
    }

    if ctx.lang == "ruby" {
        ctx.ruby_bodies
            .push((func_nid, body_node.start_byte(), body_node.end_byte()));
    }
}
