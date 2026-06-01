//! Regex-based AST extraction engine.
//!
//! This module implements a **working** regex-based extractor for each supported
//! language. It serves as the "Pass 1" deterministic extraction while tree-sitter
//! grammar crates are being added to the workspace.
//!
//! For each source file the extractor produces:
//! - A **file** node
//! - **Class / struct / trait / interface** nodes
//! - **Function / method** nodes with `defines` edges from their parent
//! - **Import** nodes with `imports` edges from the file
//! - **Calls** edges inferred by matching known function names within bodies

mod c_cpp;
mod csharp;
mod generic;
mod go;
mod java;
mod js_ts;
mod kotlin;
mod python;
mod ruby;
mod rust;

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use graphify_core::confidence::Confidence;
use graphify_core::id::make_id;
use graphify_core::model::{ExtractionResult, GraphEdge, GraphNode, NodeType};
use regex::Regex;

macro_rules! re {
    ($name:ident, $pattern:expr) => {
        pub(crate) static $name: LazyLock<Regex> =
            LazyLock::new(|| Regex::new($pattern).expect($pattern));
    };
}

re!(RE_PY_CLASS, r"(?m)^(\s*)class\s+(\w+)");
re!(RE_PY_CLASS_LOOKUP, r"^(\s*)class\s+(\w+)");
re!(RE_PY_FUNC, r"(?m)^(\s*)def\s+(\w+)\s*\(");
re!(
    RE_PY_IMPORT,
    r"(?m)^(?:from\s+([\w.]+)\s+)?import\s+([\w.,\s*]+)"
);

re!(
    RE_JS_CLASS,
    r"(?m)(?:export\s+)?(?:default\s+)?class\s+(\w+)"
);
re!(
    RE_JS_FUNC,
    r"(?m)(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s+(\w+)\s*\(|(?:const|let|var)\s+(\w+)\s*=\s*(?:async\s+)?(?:\([^)]*\)|[^=])\s*=>"
);
re!(
    RE_JS_IMPORT,
    r#"(?m)import\s+(?:\{([^}]+)\}|(\w+))\s+from\s+['"]([^'"]+)['"]|import\s+['"]([^'"]+)['"]"#
);
re!(
    RE_JS_REQUIRE,
    r#"(?m)(?:const|let|var)\s+(\w+)\s*=\s*require\s*\(\s*['"]([^'"]+)['"]\s*\)"#
);

re!(
    RE_RS_STRUCT,
    r"(?m)^(?:\s*pub(?:\([^)]*\))?\s+)?struct\s+(\w+)"
);
re!(RE_RS_ENUM, r"(?m)^(?:\s*pub(?:\([^)]*\))?\s+)?enum\s+(\w+)");
re!(
    RE_RS_TRAIT,
    r"(?m)^(?:\s*pub(?:\([^)]*\))?\s+)?trait\s+(\w+)"
);
re!(
    RE_RS_IMPL,
    r"(?m)^(?:\s*)impl(?:<[^>]*>)?\s+(?:(\w+)\s+for\s+)?(\w+)"
);
re!(
    RE_RS_FUNC,
    r"(?m)^(\s*)(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(?:const\s+)?fn\s+(\w+)"
);
re!(RE_RS_USE, r"(?m)^(?:\s*)(?:pub\s+)?use\s+([\w:]+)");

re!(RE_GO_TYPE, r"(?m)^type\s+(\w+)\s+(struct|interface)");
re!(RE_GO_FUNC, r"(?m)^func\s+(?:\([^)]+\)\s+)?(\w+)\s*\(");
re!(RE_GO_IMPORT_SINGLE, r#"(?m)^import\s+"([^"]+)""#);
re!(RE_GO_IMPORT_BLOCK, r"(?s)import\s*\(([^)]+)\)");
re!(RE_GO_IMPORT_LINE, r#""([^"]+)""#);

re!(
    RE_JAVA_CLASS,
    r"(?m)(?:public\s+|private\s+|protected\s+)?(?:abstract\s+|static\s+|final\s+)*(class|interface|enum)\s+(\w+)"
);
re!(
    RE_JAVA_METHOD,
    r"(?m)^\s+(?:public\s+|private\s+|protected\s+)?(?:static\s+)?(?:final\s+)?(?:synchronized\s+)?(?:abstract\s+)?(?:\w+(?:<[^>]*>)?)\s+(\w+)\s*\("
);
re!(RE_JAVA_IMPORT, r"(?m)^import\s+(?:static\s+)?([\w.]+)\s*;");

re!(RE_C_INCLUDE, r#"(?m)^#include\s+[<"]([^>"]+)[>"]"#);
re!(
    RE_CPP_CLASS,
    r"(?m)^(?:\s*)(?:class|struct|namespace)\s+(\w+)"
);
re!(RE_C_STRUCT, r"(?m)^(?:typedef\s+)?struct\s+(\w+)");
re!(
    RE_C_FUNC,
    r"(?m)^(?:static\s+)?(?:inline\s+)?(?:extern\s+)?(?:const\s+)?(?:unsigned\s+)?(?:signed\s+)?(?:\w+(?:\s*\*\s*|\s+))(\w+)\s*\([^;]*\)\s*\{"
);

re!(RE_RB_CLASS, r"(?m)^\s*(class|module)\s+(\w+(?:::\w+)*)");
re!(RE_RB_FUNC, r"(?m)^\s*def\s+(self\.)?(\w+[?!=]?)");
re!(
    RE_RB_REQUIRE,
    r#"(?m)^\s*require(?:_relative)?\s+['"]([^'"]+)['"]"#
);

re!(
    RE_CS_CLASS,
    r"(?m)(?:public\s+|private\s+|protected\s+|internal\s+)?(?:abstract\s+|static\s+|sealed\s+|partial\s+)*(class|interface|struct|enum)\s+(\w+)"
);
re!(
    RE_CS_METHOD,
    r"(?m)^\s+(?:public\s+|private\s+|protected\s+|internal\s+)?(?:static\s+)?(?:virtual\s+)?(?:override\s+)?(?:async\s+)?(?:\w+(?:<[^>]*>)?)\s+(\w+)\s*\("
);
re!(RE_CS_USING, r"(?m)^using\s+([\w.]+)\s*;");

re!(
    RE_KT_CLASS,
    r"(?m)(?:open\s+|abstract\s+|data\s+|sealed\s+)?(?:class|object|interface)\s+(\w+)"
);
re!(
    RE_KT_FUNC,
    r"(?m)^\s*(?:(?:private|public|protected|internal|override|open|suspend)\s+)*fun\s+(?:<[^>]+>\s+)?(\w+)\s*\("
);
re!(RE_KT_IMPORT, r"(?m)^import\s+([\w.]+)");

re!(
    RE_GEN_CLASS,
    r"(?m)^\s*(?:(?:pub|public|private|protected|internal|open|abstract|sealed|partial|static|final|export)\s+)*(?:class|struct|module|object|interface|trait|protocol|enum|defmodule)\s+(\w+(?:::\w+)*)"
);
re!(
    RE_GEN_FUNC,
    r"(?m)^\s*(?:(?:pub|public|private|protected|internal|open|override|suspend|static|async|export|def|defp)\s+)*(?:func|function|fn|def|defp|fun|sub)\s+(\w+[?!]?)\s*[\(<]"
);
re!(
    RE_GEN_IMPORT,
    r#"(?m)^\s*(?:import|use|using|require|include|from)\s+['"]?([\w./:-]+)['"]?"#
);

/// Extract graph nodes and edges from a single source file.
pub fn extract_file(path: &Path, source: &str, lang: &str) -> ExtractionResult {
    match lang {
        "python" => python::extract_python(path, source),
        "javascript" | "typescript" => js_ts::extract_js_ts(path, source, lang),
        "rust" => rust::extract_rust(path, source),
        "go" => go::extract_go(path, source),
        "java" => java::extract_java(path, source),
        "c" | "cpp" => c_cpp::extract_c_cpp(path, source, lang),
        "ruby" => ruby::extract_ruby(path, source),
        "csharp" => csharp::extract_csharp(path, source),
        "kotlin" => kotlin::extract_kotlin(path, source),
        _ => generic::extract_generic(path, source, lang),
    }
}

pub(crate) fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

pub(crate) fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub(crate) fn make_file_node(path: &Path) -> GraphNode {
    let ps = path_str(path);
    GraphNode {
        id: make_id(&[&ps]),
        label: file_stem(path),
        source_file: ps,
        source_location: None,
        node_type: NodeType::File,
        community: None,
        extra: HashMap::new(),
    }
}

pub(crate) fn make_node(name: &str, path: &Path, node_type: NodeType, line: usize) -> GraphNode {
    let ps = path_str(path);
    GraphNode {
        id: make_id(&[&ps, name]),
        label: name.to_string(),
        source_file: ps,
        source_location: Some(format!("L{line}")),
        node_type,
        community: None,
        extra: HashMap::new(),
    }
}

pub(crate) fn make_edge(
    source_id: &str,
    target_id: &str,
    relation: &str,
    path: &Path,
    confidence: Confidence,
) -> GraphEdge {
    GraphEdge {
        source: source_id.to_string(),
        target: target_id.to_string(),
        relation: relation.to_string(),
        confidence: confidence.clone(),
        confidence_score: confidence.default_score(),
        source_file: path_str(path),
        source_location: None,
        weight: 1.0,
        extra: HashMap::new(),
    }
}

/// Line number (1-based) where a regex capture starts in `source`.
pub(crate) fn line_of(source: &str, cap: &regex::Captures<'_>) -> usize {
    source[..cap.get(0).unwrap().start()].lines().count() + 1
}

/// 1-based end line of `source` at byte offset of the next capture's start,
/// or end of file if this is the last capture.
pub(crate) fn end_line_at(source: &str, next: Option<&regex::Captures<'_>>) -> usize {
    match next {
        Some(n) => source[..n.get(0).unwrap().start()].lines().count(),
        None => source.lines().count(),
    }
}

/// Full matched string of capture group 0.
pub(crate) fn full_match<'a>(cap: &regex::Captures<'a>) -> &'a str {
    cap.get(0).unwrap().as_str()
}

/// Simple call-graph inference: for each function body, look for occurrences
/// of other known function names.
pub(crate) fn infer_calls(
    functions: &[(String, String, usize, usize)], // (name, id, start_line, end_line)
    source_lines: &[&str],
    path: &Path,
) -> Vec<GraphEdge> {
    let mut edges = Vec::new();
    for (_caller_name, caller_id, start, end) in functions {
        let body = source_lines
            .get(*start..*end)
            .unwrap_or_default()
            .join("\n");
        for (callee_name, callee_id, _, _) in functions {
            if caller_id == callee_id {
                continue;
            }
            let pattern = format!(r"\b{}\s*\(", regex::escape(callee_name));
            if let Ok(re) = regex::Regex::new(&pattern) {
                for mat in re.find_iter(&body) {
                    let start = mat.start();
                    if start > 0 && body.as_bytes()[start - 1] == b'.' {
                        continue;
                    }
                    edges.push(make_edge(
                        caller_id,
                        callee_id,
                        "calls",
                        path,
                        Confidence::Inferred,
                    ));
                    break;
                }
            }
        }
    }
    edges
}
