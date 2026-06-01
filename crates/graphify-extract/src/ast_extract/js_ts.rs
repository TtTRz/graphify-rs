use std::collections::HashMap;
use std::path::Path;

use super::{
    RE_JS_CLASS, RE_JS_FUNC, RE_JS_IMPORT, RE_JS_REQUIRE, end_line_at, infer_calls, line_of,
    make_edge, make_file_node, make_node, path_str,
};
use graphify_core::confidence::Confidence;
use graphify_core::id::make_id;
use graphify_core::model::{ExtractionResult, GraphNode, NodeType};

pub(crate) fn extract_js_ts(path: &Path, source: &str, lang: &str) -> ExtractionResult {
    let mut result = ExtractionResult::default();
    let file_node = make_file_node(path);
    let file_id = file_node.id.clone();
    result.nodes.push(file_node);

    let lines: Vec<&str> = source.lines().collect();
    let ps = path_str(path);

    for cap in RE_JS_CLASS.captures_iter(source) {
        let name = &cap[1];
        let line = line_of(source, &cap);
        let node = make_node(name, path, NodeType::Class, line);
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
    let func_matches: Vec<_> = RE_JS_FUNC.captures_iter(source).collect();

    for (i, cap) in func_matches.iter().enumerate() {
        let name = cap
            .get(1)
            .or(cap.get(2))
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let start_line = line_of(source, cap);
        let end_line = end_line_at(source, func_matches.get(i + 1));

        let node = make_node(&name, path, NodeType::Function, start_line);
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

    for cap in RE_JS_IMPORT.captures_iter(source) {
        let module = cap.get(3).or(cap.get(4)).map_or("", |m| m.as_str());
        let line = line_of(source, &cap);

        if let Some(names) = cap.get(1) {
            for name in names.as_str().split(',') {
                let name = name.trim().split(" as ").next().unwrap_or("").trim();
                if name.is_empty() {
                    continue;
                }
                let full = format!("{module}/{name}");
                let import_id = make_id(&[&ps, "import", &full]);
                result.nodes.push(GraphNode {
                    id: import_id.clone(),
                    label: full,
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
        } else if let Some(default_name) = cap.get(2) {
            let name = default_name.as_str();
            let import_id = make_id(&[&ps, "import", module]);
            result.nodes.push(GraphNode {
                id: import_id.clone(),
                label: name.to_string(),
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
    }

    if lang == "javascript" {
        for cap in RE_JS_REQUIRE.captures_iter(source) {
            let name = &cap[1];
            let module = &cap[2];
            let line = line_of(source, &cap);
            let import_id = make_id(&[&ps, "import", module]);
            result.nodes.push(GraphNode {
                id: import_id.clone(),
                label: name.to_string(),
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
    }

    let call_edges = infer_calls(&functions, &lines, path);
    result.edges.extend(call_edges);

    result
}
