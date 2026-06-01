use std::collections::HashMap;
use std::path::Path;

use super::{
    RE_PY_CLASS, RE_PY_CLASS_LOOKUP, RE_PY_FUNC, RE_PY_IMPORT, end_line_at, infer_calls, line_of,
    make_edge, make_file_node, make_node, path_str,
};
use graphify_core::confidence::Confidence;
use graphify_core::id::make_id;
use graphify_core::model::{ExtractionResult, GraphNode, NodeType};
use tracing::trace;

pub(crate) fn extract_python(path: &Path, source: &str) -> ExtractionResult {
    let mut result = ExtractionResult::default();
    let file_node = make_file_node(path);
    let file_id = file_node.id.clone();
    result.nodes.push(file_node);

    let lines: Vec<&str> = source.lines().collect();
    let ps = path_str(path);

    let mut class_ids: HashMap<String, String> = HashMap::new();
    for cap in RE_PY_CLASS.captures_iter(source) {
        let name = &cap[2];
        let line = line_of(source, &cap);
        let node = make_node(name, path, NodeType::Class, line);
        let node_id = node.id.clone();
        class_ids.insert(name.to_string(), node_id.clone());
        result.nodes.push(node);
        result.edges.push(make_edge(
            &file_id,
            &node_id,
            "defines",
            path,
            Confidence::Extracted,
        ));
    }

    // Functions / methods: `def foo(...):`
    let mut functions: Vec<(String, String, usize, usize)> = Vec::new();
    let func_matches: Vec<_> = RE_PY_FUNC.captures_iter(source).collect();
    for (i, cap) in func_matches.iter().enumerate() {
        let indent = cap[1].len();
        let name = cap[2].to_string();
        let start_line = line_of(source, cap);

        let node_type = if indent > 0 {
            NodeType::Method
        } else {
            NodeType::Function
        };
        let node = make_node(&name, path, node_type, start_line);
        let node_id = node.id.clone();

        let parent_id = if indent > 0 {
            let mut parent = None;
            for line_idx in (0..start_line.saturating_sub(1)).rev() {
                if let Some(line) = lines.get(line_idx)
                    && let Some(cls_cap) = RE_PY_CLASS_LOOKUP.captures(line)
                    && cls_cap[1].len() < indent
                {
                    parent = class_ids.get(&cls_cap[2]).cloned();
                    break;
                }
            }
            parent.unwrap_or_else(|| file_id.clone())
        } else {
            file_id.clone()
        };

        let end_line = end_line_at(source, func_matches.get(i + 1));

        functions.push((name.clone(), node_id.clone(), start_line, end_line));
        result.nodes.push(node);
        result.edges.push(make_edge(
            &parent_id,
            &node_id,
            "defines",
            path,
            Confidence::Extracted,
        ));
    }

    for cap in RE_PY_IMPORT.captures_iter(source) {
        let module = cap.get(1).map_or("", |m| m.as_str());
        let names_str = &cap[2];
        let line = line_of(source, &cap);

        for name in names_str.split(',') {
            let name = name.trim().split(" as ").next().unwrap_or("").trim();
            if name.is_empty() || name == "*" {
                continue;
            }
            let full_name = if module.is_empty() {
                name.to_string()
            } else {
                format!("{module}.{name}")
            };
            let import_id = make_id(&[&ps, "import", &full_name]);
            result.nodes.push(GraphNode {
                id: import_id.clone(),
                label: full_name,
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

    trace!(
        "python: {} nodes, {} edges from {}",
        result.nodes.len(),
        result.edges.len(),
        ps
    );
    result
}
