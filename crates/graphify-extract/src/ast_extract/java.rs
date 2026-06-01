use std::collections::HashMap;
use std::path::Path;

use super::{
    RE_JAVA_CLASS, RE_JAVA_IMPORT, RE_JAVA_METHOD, end_line_at, infer_calls, line_of, make_edge,
    make_file_node, make_node, path_str,
};
use graphify_core::confidence::Confidence;
use graphify_core::id::make_id;
use graphify_core::model::{ExtractionResult, GraphNode, NodeType};

pub(crate) fn extract_java(path: &Path, source: &str) -> ExtractionResult {
    let mut result = ExtractionResult::default();
    let file_node = make_file_node(path);
    let file_id = file_node.id.clone();
    result.nodes.push(file_node);

    let lines: Vec<&str> = source.lines().collect();
    let ps = path_str(path);

    for cap in RE_JAVA_CLASS.captures_iter(source) {
        let kind = &cap[1];
        let name = &cap[2];
        let line = line_of(source, &cap);
        let node_type = match kind {
            "interface" => NodeType::Interface,
            "enum" => NodeType::Enum,
            _ => NodeType::Class,
        };
        let node = make_node(name, path, node_type, line);
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

    let mut functions: Vec<(String, String, usize, usize)> = Vec::new();
    let func_matches: Vec<_> = RE_JAVA_METHOD.captures_iter(source).collect();
    for (i, cap) in func_matches.iter().enumerate() {
        let name = cap[1].to_string();
        if [
            "if", "for", "while", "switch", "catch", "return", "new", "throw",
        ]
        .contains(&name.as_str())
        {
            continue;
        }
        let start_line = line_of(source, cap);
        let end_line = end_line_at(source, func_matches.get(i + 1));

        let node = make_node(&name, path, NodeType::Method, start_line);
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

    for cap in RE_JAVA_IMPORT.captures_iter(source) {
        let module = &cap[1];
        let line = line_of(source, &cap);
        let import_id = make_id(&[&ps, "import", module]);
        result.nodes.push(GraphNode {
            id: import_id.clone(),
            label: module.to_string(),
            source_file: ps.clone(),
            source_location: Some(format!("L{line}")),
            node_type: NodeType::Package,
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
