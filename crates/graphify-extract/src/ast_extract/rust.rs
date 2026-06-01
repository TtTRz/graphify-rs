use std::collections::HashMap;
use std::path::Path;

use super::{
    RE_RS_ENUM, RE_RS_FUNC, RE_RS_IMPL, RE_RS_STRUCT, RE_RS_TRAIT, RE_RS_USE, end_line_at,
    infer_calls, line_of, make_edge, make_file_node, make_node, path_str,
};
use graphify_core::confidence::Confidence;
use graphify_core::id::make_id;
use graphify_core::model::{ExtractionResult, GraphNode, NodeType};

pub(crate) fn extract_rust(path: &Path, source: &str) -> ExtractionResult {
    let mut result = ExtractionResult::default();
    let file_node = make_file_node(path);
    let file_id = file_node.id.clone();
    result.nodes.push(file_node);

    let lines: Vec<&str> = source.lines().collect();
    let ps = path_str(path);

    for cap in RE_RS_STRUCT.captures_iter(source) {
        let name = &cap[1];
        let line = line_of(source, &cap);
        let node = make_node(name, path, NodeType::Struct, line);
        let node_id = node.id.clone();
        result.nodes.push(node);
        result.edges.push(make_edge(
            &file_id,
            &node_id,
            "defines",
            path,
            Confidence::Extracted,
        ));
    }

    for cap in RE_RS_ENUM.captures_iter(source) {
        let name = &cap[1];
        let line = line_of(source, &cap);
        let node = make_node(name, path, NodeType::Enum, line);
        let node_id = node.id.clone();
        result.nodes.push(node);
        result.edges.push(make_edge(
            &file_id,
            &node_id,
            "defines",
            path,
            Confidence::Extracted,
        ));
    }

    for cap in RE_RS_TRAIT.captures_iter(source) {
        let name = &cap[1];
        let line = line_of(source, &cap);
        let node = make_node(name, path, NodeType::Trait, line);
        let node_id = node.id.clone();
        result.nodes.push(node);
        result.edges.push(make_edge(
            &file_id,
            &node_id,
            "defines",
            path,
            Confidence::Extracted,
        ));
    }

    for cap in RE_RS_IMPL.captures_iter(source) {
        let _trait_name = cap.get(1).map(|m| m.as_str());
        let type_name = &cap[2];
        let line = line_of(source, &cap);
        if let Some(trait_m) = cap.get(1) {
            let trait_id = make_id(&[&ps, trait_m.as_str()]);
            let type_id = make_id(&[&ps, type_name]);
            result.edges.push(make_edge(
                &type_id,
                &trait_id,
                "implements",
                path,
                Confidence::Extracted,
            ));
        }
        let _ = line;
    }

    let mut functions: Vec<(String, String, usize, usize)> = Vec::new();
    let func_matches: Vec<_> = RE_RS_FUNC.captures_iter(source).collect();
    for (i, cap) in func_matches.iter().enumerate() {
        let indent = cap[1].len();
        let name = cap[2].to_string();
        let start_line = line_of(source, cap);
        let end_line = end_line_at(source, func_matches.get(i + 1));

        let node_type = if indent > 0 {
            NodeType::Method
        } else {
            NodeType::Function
        };
        let node = make_node(&name, path, node_type, start_line);
        let node_id = node.id.clone();
        functions.push((name, node_id.clone(), start_line, end_line));
        result.nodes.push(node);
        result.edges.push(make_edge(
            &file_id,
            &node_id,
            "defines",
            path,
            Confidence::Extracted,
        ));
    }

    for cap in RE_RS_USE.captures_iter(source) {
        let module = &cap[1];
        let line = line_of(source, &cap);
        let import_id = make_id(&[&ps, "use", module]);
        result.nodes.push(GraphNode {
            id: import_id.clone(),
            label: module.to_string(),
            source_file: ps.clone(),
            source_location: Some(format!("L{line}")),
            node_type: NodeType::Module,
            community: None,
            extra: HashMap::new(),
        });
        result.edges.push(make_edge(
            &file_id,
            &import_id,
            "imports",
            path,
            Confidence::Extracted,
        ));
    }

    let call_edges = infer_calls(&functions, &lines, path);
    result.edges.extend(call_edges);

    result
}
