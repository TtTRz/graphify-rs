use std::collections::HashMap;
use std::path::Path;

use super::{
    RE_GO_FUNC, RE_GO_IMPORT_BLOCK, RE_GO_IMPORT_LINE, RE_GO_IMPORT_SINGLE, RE_GO_TYPE,
    end_line_at, full_match, infer_calls, line_of, make_edge, make_file_node, make_node, path_str,
};
use graphify_core::confidence::Confidence;
use graphify_core::id::make_id;
use graphify_core::model::{ExtractionResult, GraphNode, NodeType};

pub(crate) fn extract_go(path: &Path, source: &str) -> ExtractionResult {
    let mut result = ExtractionResult::default();
    let file_node = make_file_node(path);
    let file_id = file_node.id.clone();
    result.nodes.push(file_node);

    let lines: Vec<&str> = source.lines().collect();
    let ps = path_str(path);

    for cap in RE_GO_TYPE.captures_iter(source) {
        let name = &cap[1];
        let kind = &cap[2];
        let line = line_of(source, &cap);
        let node_type = match kind {
            "interface" => NodeType::Interface,
            _ => NodeType::Struct,
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
    let func_matches: Vec<_> = RE_GO_FUNC.captures_iter(source).collect();
    for (i, cap) in func_matches.iter().enumerate() {
        let name = cap[1].to_string();
        let start_line = line_of(source, cap);
        let end_line = end_line_at(source, func_matches.get(i + 1));

        let full_match = full_match(cap);
        let node_type = if full_match.contains('(') && full_match.find('(') < full_match.find(&name)
        {
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

    for cap in RE_GO_IMPORT_SINGLE.captures_iter(source) {
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

    for cap in RE_GO_IMPORT_BLOCK.captures_iter(source) {
        let block = &cap[1];
        let block_start = line_of(source, &cap);
        for (idx, imp_cap) in RE_GO_IMPORT_LINE.captures_iter(block).enumerate() {
            let module = &imp_cap[1];
            let import_id = make_id(&[&ps, "import", module]);
            result.nodes.push(GraphNode {
                id: import_id.clone(),
                label: module.to_string(),
                source_file: ps.clone(),
                source_location: Some(format!("L{}", block_start + idx + 1)),
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
    }

    let call_edges = infer_calls(&functions, &lines, path);
    result.edges.extend(call_edges);

    result
}
