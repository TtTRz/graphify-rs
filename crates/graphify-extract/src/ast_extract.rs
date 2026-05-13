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

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use graphify_core::confidence::Confidence;
use graphify_core::id::make_id;
use graphify_core::model::{ExtractionResult, GraphEdge, GraphNode, NodeType};
use regex::Regex;
use tracing::trace;

macro_rules! re {
    ($name:ident, $pattern:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pattern).expect($pattern));
    };
}

// Pre-compiled regex patterns — initialized once, never panic at runtime
re!(RE_PY_CLASS, r"(?m)^(\s*)class\s+(\w+)");
re!(RE_PY_CLASS_LOOKUP, r"^(\s*)class\s+(\w+)");
re!(RE_PY_FUNC, r"(?m)^(\s*)def\s+(\w+)\s*\(");
re!(RE_PY_IMPORT, r"(?m)^(?:from\s+([\w.]+)\s+)?import\s+([\w.,\s*]+)");

re!(RE_JS_CLASS, r"(?m)(?:export\s+)?(?:default\s+)?class\s+(\w+)");
re!(RE_JS_FUNC, r"(?m)(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s+(\w+)\s*\(|(?:const|let|var)\s+(\w+)\s*=\s*(?:async\s+)?(?:\([^)]*\)|[^=])\s*=>");
re!(RE_JS_IMPORT, r#"(?m)import\s+(?:\{([^}]+)\}|(\w+))\s+from\s+['"]([^'"]+)['"]|import\s+['"]([^'"]+)['"]"#);
re!(RE_JS_REQUIRE, r#"(?m)(?:const|let|var)\s+(\w+)\s*=\s*require\s*\(\s*['"]([^'"]+)['"]\s*\)"#);

re!(RE_RS_STRUCT, r"(?m)^(?:\s*pub(?:\([^)]*\))?\s+)?struct\s+(\w+)");
re!(RE_RS_ENUM, r"(?m)^(?:\s*pub(?:\([^)]*\))?\s+)?enum\s+(\w+)");
re!(RE_RS_TRAIT, r"(?m)^(?:\s*pub(?:\([^)]*\))?\s+)?trait\s+(\w+)");
re!(RE_RS_IMPL, r"(?m)^(?:\s*)impl(?:<[^>]*>)?\s+(?:(\w+)\s+for\s+)?(\w+)");
re!(RE_RS_FUNC, r"(?m)^(\s*)(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(?:const\s+)?fn\s+(\w+)");
re!(RE_RS_USE, r"(?m)^(?:\s*)(?:pub\s+)?use\s+([\w:]+)");

re!(RE_GO_TYPE, r"(?m)^type\s+(\w+)\s+(struct|interface)");
re!(RE_GO_FUNC, r"(?m)^func\s+(?:\([^)]+\)\s+)?(\w+)\s*\(");
re!(RE_GO_IMPORT_SINGLE, r#"(?m)^import\s+"([^"]+)""#);
re!(RE_GO_IMPORT_BLOCK, r"(?s)import\s*\(([^)]+)\)");
re!(RE_GO_IMPORT_LINE, r#""([^"]+)""#);

re!(RE_JAVA_CLASS, r"(?m)(?:public\s+|private\s+|protected\s+)?(?:abstract\s+|static\s+|final\s+)*(class|interface|enum)\s+(\w+)");
re!(RE_JAVA_METHOD, r"(?m)^\s+(?:public\s+|private\s+|protected\s+)?(?:static\s+)?(?:final\s+)?(?:synchronized\s+)?(?:abstract\s+)?(?:\w+(?:<[^>]*>)?)\s+(\w+)\s*\(");
re!(RE_JAVA_IMPORT, r"(?m)^import\s+(?:static\s+)?([\w.]+)\s*;");

re!(RE_C_INCLUDE, r#"(?m)^#include\s+[<"]([^>"]+)[>"]"#);
re!(RE_CPP_CLASS, r"(?m)^(?:\s*)(?:class|struct|namespace)\s+(\w+)");
re!(RE_C_STRUCT, r"(?m)^(?:typedef\s+)?struct\s+(\w+)");
re!(RE_C_FUNC, r"(?m)^(?:static\s+)?(?:inline\s+)?(?:extern\s+)?(?:const\s+)?(?:unsigned\s+)?(?:signed\s+)?(?:\w+(?:\s*\*\s*|\s+))(\w+)\s*\([^;]*\)\s*\{");

re!(RE_RB_CLASS, r"(?m)^\s*(class|module)\s+(\w+(?:::\w+)*)");
re!(RE_RB_FUNC, r"(?m)^\s*def\s+(self\.)?(\w+[?!=]?)");
re!(RE_RB_REQUIRE, r#"(?m)^\s*require(?:_relative)?\s+['"]([^'"]+)['"]"#);

re!(RE_CS_CLASS, r"(?m)(?:public\s+|private\s+|protected\s+|internal\s+)?(?:abstract\s+|static\s+|sealed\s+|partial\s+)*(class|interface|struct|enum)\s+(\w+)");
re!(RE_CS_METHOD, r"(?m)^\s+(?:public\s+|private\s+|protected\s+|internal\s+)?(?:static\s+)?(?:virtual\s+)?(?:override\s+)?(?:async\s+)?(?:\w+(?:<[^>]*>)?)\s+(\w+)\s*\(");
re!(RE_CS_USING, r"(?m)^using\s+([\w.]+)\s*;");

re!(RE_KT_CLASS, r"(?m)(?:open\s+|abstract\s+|data\s+|sealed\s+)?(?:class|object|interface)\s+(\w+)");
re!(RE_KT_FUNC, r"(?m)^\s*(?:(?:private|public|protected|internal|override|open|suspend)\s+)*fun\s+(?:<[^>]+>\s+)?(\w+)\s*\(");
re!(RE_KT_IMPORT, r"(?m)^import\s+([\w.]+)");

re!(RE_GEN_CLASS, r"(?m)^\s*(?:(?:pub|public|private|protected|internal|open|abstract|sealed|partial|static|final|export)\s+)*(?:class|struct|module|object|interface|trait|protocol|enum|defmodule)\s+(\w+(?:::\w+)*)");
re!(RE_GEN_FUNC, r"(?m)^\s*(?:(?:pub|public|private|protected|internal|open|override|suspend|static|async|export|def|defp)\s+)*(?:func|function|fn|def|defp|fun|sub)\s+(\w+[?!]?)\s*[\(<]");
re!(RE_GEN_IMPORT, r#"(?m)^\s*(?:import|use|using|require|include|from)\s+['"]?([\w./:-]+)['"]?"#);

// ═══════════════════════════════════════════════════════════════════════════
// Public entry point
// ═══════════════════════════════════════════════════════════════════════════

/// Extract graph nodes and edges from a single source file.
pub fn extract_file(path: &Path, source: &str, lang: &str) -> ExtractionResult {
    match lang {
        "python" => extract_python(path, source),
        "javascript" | "typescript" => extract_js_ts(path, source, lang),
        "rust" => extract_rust(path, source),
        "go" => extract_go(path, source),
        "java" => extract_java(path, source),
        "c" | "cpp" => extract_c_cpp(path, source, lang),
        "ruby" => extract_ruby(path, source),
        "csharp" => extract_csharp(path, source),
        "kotlin" => extract_kotlin(path, source),
        _ => extract_generic(path, source, lang),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn make_file_node(path: &Path) -> GraphNode {
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

fn make_node(name: &str, path: &Path, node_type: NodeType, line: usize) -> GraphNode {
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

fn make_edge(
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

/// Simple call-graph inference: for each function body, look for occurrences
/// of other known function names.
fn infer_calls(
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
            // Check if callee_name appears in caller body as a call (name followed by `(`)
            let pattern = format!(r"\b{}\s*\(", regex::escape(callee_name));
            if let Ok(re) = Regex::new(&pattern)
                && re.is_match(&body)
            {
                edges.push(make_edge(
                    caller_id,
                    callee_id,
                    "calls",
                    path,
                    Confidence::Inferred,
                ));
            }
        }
    }
    edges
}

// ═══════════════════════════════════════════════════════════════════════════
// Python
// ═══════════════════════════════════════════════════════════════════════════

fn extract_python(path: &Path, source: &str) -> ExtractionResult {
    let mut result = ExtractionResult::default();
    let file_node = make_file_node(path);
    let file_id = file_node.id.clone();
    result.nodes.push(file_node);

    let lines: Vec<&str> = source.lines().collect();
    let ps = path_str(path);

    // Classes: `class Foo(Bar):`  or `class Foo:`
    let mut class_ids: HashMap<String, String> = HashMap::new();
    for cap in RE_PY_CLASS.captures_iter(source) {
        let name = &cap[2];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
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
        let start_line = source[..cap.get(0).unwrap().start()].lines().count() + 1;

        let node_type = if indent > 0 {
            NodeType::Method
        } else {
            NodeType::Function
        };
        let node = make_node(&name, path, node_type, start_line);
        let node_id = node.id.clone();

        // Determine parent: if indented, belong to nearest class above with less indent
        let parent_id = if indent > 0 {
            // Find enclosing class by checking lines above for `class` with less indent
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

        // End line: next function at same or lower indent, or end of file
        let end_line = if i + 1 < func_matches.len() {
            source[..func_matches[i + 1].get(0).unwrap().start()]
                .lines()
                .count()
        } else {
            lines.len()
        };

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

    // Imports: `import X` / `from X import Y`
    for cap in RE_PY_IMPORT.captures_iter(source) {
        let module = cap.get(1).map_or("", |m| m.as_str());
        let names_str = &cap[2];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;

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

    // Infer calls
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

// ═══════════════════════════════════════════════════════════════════════════
// JavaScript / TypeScript
// ═══════════════════════════════════════════════════════════════════════════

fn extract_js_ts(path: &Path, source: &str, lang: &str) -> ExtractionResult {
    let mut result = ExtractionResult::default();
    let file_node = make_file_node(path);
    let file_id = file_node.id.clone();
    result.nodes.push(file_node);

    let lines: Vec<&str> = source.lines().collect();
    let ps = path_str(path);

    // Classes: `class Foo` / `export class Foo`
    for cap in RE_JS_CLASS.captures_iter(source) {
        let name = &cap[1];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
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

    // Functions: `function foo(` / `const foo = (` / `const foo = async (`
    // Also: `export function foo(` / `export default function foo(`
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
        let start_line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
        let end_line = if i + 1 < func_matches.len() {
            source[..func_matches[i + 1].get(0).unwrap().start()]
                .lines()
                .count()
        } else {
            lines.len()
        };

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

    // Imports: `import { X } from 'Y'` / `import X from 'Y'` / `import 'Y'`
    for cap in RE_JS_IMPORT.captures_iter(source) {
        let module = cap.get(3).or(cap.get(4)).map(|m| m.as_str()).unwrap_or("");
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;

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

    // Also handle require() for JS
    if lang == "javascript" {
        for cap in RE_JS_REQUIRE.captures_iter(source) {
            let name = &cap[1];
            let module = &cap[2];
            let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
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

// ═══════════════════════════════════════════════════════════════════════════
// Rust
// ═══════════════════════════════════════════════════════════════════════════

fn extract_rust(path: &Path, source: &str) -> ExtractionResult {
    let mut result = ExtractionResult::default();
    let file_node = make_file_node(path);
    let file_id = file_node.id.clone();
    result.nodes.push(file_node);

    let lines: Vec<&str> = source.lines().collect();
    let ps = path_str(path);

    // Structs: `pub struct Foo` / `struct Foo`
    for cap in RE_RS_STRUCT.captures_iter(source) {
        let name = &cap[1];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
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

    // Enums: `pub enum Foo` / `enum Foo`
    for cap in RE_RS_ENUM.captures_iter(source) {
        let name = &cap[1];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
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

    // Traits: `pub trait Foo` / `trait Foo`
    for cap in RE_RS_TRAIT.captures_iter(source) {
        let name = &cap[1];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
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

    // Impl blocks: `impl Foo` / `impl Trait for Foo`
    for cap in RE_RS_IMPL.captures_iter(source) {
        let _trait_name = cap.get(1).map(|m| m.as_str());
        let type_name = &cap[2];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
        // Create an "implements" edge if impl Trait for Type
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

    // Functions: `pub fn foo(` / `fn foo(` / `pub(crate) fn foo(`
    // Also methods inside impl blocks
    let mut functions: Vec<(String, String, usize, usize)> = Vec::new();
    let func_matches: Vec<_> = RE_RS_FUNC.captures_iter(source).collect();
    for (i, cap) in func_matches.iter().enumerate() {
        let indent = cap[1].len();
        let name = cap[2].to_string();
        let start_line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
        let end_line = if i + 1 < func_matches.len() {
            source[..func_matches[i + 1].get(0).unwrap().start()]
                .lines()
                .count()
        } else {
            lines.len()
        };

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

    // Use statements
    for cap in RE_RS_USE.captures_iter(source) {
        let module = &cap[1];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
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

// ═══════════════════════════════════════════════════════════════════════════
// Go
// ═══════════════════════════════════════════════════════════════════════════

fn extract_go(path: &Path, source: &str) -> ExtractionResult {
    let mut result = ExtractionResult::default();
    let file_node = make_file_node(path);
    let file_id = file_node.id.clone();
    result.nodes.push(file_node);

    let lines: Vec<&str> = source.lines().collect();
    let ps = path_str(path);

    // Type definitions: `type Foo struct {` / `type Foo interface {`
    for cap in RE_GO_TYPE.captures_iter(source) {
        let name = &cap[1];
        let kind = &cap[2];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
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

    // Functions and methods: `func Foo(` / `func (r *Recv) Foo(`
    let mut functions: Vec<(String, String, usize, usize)> = Vec::new();
    let func_matches: Vec<_> = RE_GO_FUNC.captures_iter(source).collect();
    for (i, cap) in func_matches.iter().enumerate() {
        let name = cap[1].to_string();
        let start_line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
        let end_line = if i + 1 < func_matches.len() {
            source[..func_matches[i + 1].get(0).unwrap().start()]
                .lines()
                .count()
        } else {
            lines.len()
        };

        // Methods have a receiver
        let full_match = cap.get(0).unwrap().as_str();
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

    // Imports: `import "fmt"` / `import ( "fmt" "os" )`
    for cap in RE_GO_IMPORT_SINGLE.captures_iter(source) {
        let module = &cap[1];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
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
        let block_start = source[..cap.get(0).unwrap().start()].lines().count() + 1;
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

// ═══════════════════════════════════════════════════════════════════════════
// Java
// ═══════════════════════════════════════════════════════════════════════════

fn extract_java(path: &Path, source: &str) -> ExtractionResult {
    let mut result = ExtractionResult::default();
    let file_node = make_file_node(path);
    let file_id = file_node.id.clone();
    result.nodes.push(file_node);

    let lines: Vec<&str> = source.lines().collect();
    let ps = path_str(path);

    // Classes / interfaces / enums
    for cap in RE_JAVA_CLASS.captures_iter(source) {
        let kind = &cap[1];
        let name = &cap[2];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
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

    // Methods: `public void foo(` / `private static int bar(`
    let mut functions: Vec<(String, String, usize, usize)> = Vec::new();
    let func_matches: Vec<_> = RE_JAVA_METHOD.captures_iter(source).collect();
    for (i, cap) in func_matches.iter().enumerate() {
        let name = cap[1].to_string();
        // Skip common false positives
        if [
            "if", "for", "while", "switch", "catch", "return", "new", "throw",
        ]
        .contains(&name.as_str())
        {
            continue;
        }
        let start_line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
        let end_line = if i + 1 < func_matches.len() {
            source[..func_matches[i + 1].get(0).unwrap().start()]
                .lines()
                .count()
        } else {
            lines.len()
        };

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

    // Imports
    for cap in RE_JAVA_IMPORT.captures_iter(source) {
        let module = &cap[1];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
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

// ═══════════════════════════════════════════════════════════════════════════
// C / C++
// ═══════════════════════════════════════════════════════════════════════════

fn extract_c_cpp(path: &Path, source: &str, lang: &str) -> ExtractionResult {
    let mut result = ExtractionResult::default();
    let file_node = make_file_node(path);
    let file_id = file_node.id.clone();
    result.nodes.push(file_node);

    let lines: Vec<&str> = source.lines().collect();
    let ps = path_str(path);

    // #include directives
    for cap in RE_C_INCLUDE.captures_iter(source) {
        let header = &cap[1];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
        let import_id = make_id(&[&ps, "include", header]);
        result.nodes.push(GraphNode {
            id: import_id.clone(),
            label: header.to_string(),
            source_file: ps.clone(),
            source_location: Some(format!("L{line}")),
            node_type: NodeType::Module,
            community: None,
            extra: HashMap::new(),
        });
        result.edges.push(make_edge(
            &file_id,
            &import_id,
            "includes",
            path,
            Confidence::Extracted,
        ));
    }

    // C++ classes / structs / namespaces
    if lang == "cpp" {
        for cap in RE_CPP_CLASS.captures_iter(source) {
            let name = &cap[1];
            let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
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
    }

    // C structs
    if lang == "c" {
        for cap in RE_C_STRUCT.captures_iter(source) {
            let name = &cap[1];
            let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
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
    }

    // Functions: `type name(` at start of line (heuristic)
    let mut functions: Vec<(String, String, usize, usize)> = Vec::new();
    let func_matches: Vec<_> = RE_C_FUNC.captures_iter(source).collect();
    for (i, cap) in func_matches.iter().enumerate() {
        let name = cap[1].to_string();
        if ["if", "for", "while", "switch", "return", "sizeof"].contains(&name.as_str()) {
            continue;
        }
        let start_line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
        let end_line = if i + 1 < func_matches.len() {
            source[..func_matches[i + 1].get(0).unwrap().start()]
                .lines()
                .count()
        } else {
            lines.len()
        };

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

    let call_edges = infer_calls(&functions, &lines, path);
    result.edges.extend(call_edges);

    result
}

// ═══════════════════════════════════════════════════════════════════════════
// Ruby
// ═══════════════════════════════════════════════════════════════════════════

fn extract_ruby(path: &Path, source: &str) -> ExtractionResult {
    let mut result = ExtractionResult::default();
    let file_node = make_file_node(path);
    let file_id = file_node.id.clone();
    result.nodes.push(file_node);

    let lines: Vec<&str> = source.lines().collect();
    let ps = path_str(path);

    // Classes and modules
    for cap in RE_RB_CLASS.captures_iter(source) {
        let name = &cap[2];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
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

    // Methods
    let mut functions: Vec<(String, String, usize, usize)> = Vec::new();
    let func_matches: Vec<_> = RE_RB_FUNC.captures_iter(source).collect();
    for (i, cap) in func_matches.iter().enumerate() {
        let name = cap[2].to_string();
        let start_line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
        let end_line = if i + 1 < func_matches.len() {
            source[..func_matches[i + 1].get(0).unwrap().start()]
                .lines()
                .count()
        } else {
            lines.len()
        };

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

    // require / require_relative
    for cap in RE_RB_REQUIRE.captures_iter(source) {
        let module = &cap[1];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
        let import_id = make_id(&[&ps, "require", module]);
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

// ═══════════════════════════════════════════════════════════════════════════
// C#
// ═══════════════════════════════════════════════════════════════════════════

fn extract_csharp(path: &Path, source: &str) -> ExtractionResult {
    let mut result = ExtractionResult::default();
    let file_node = make_file_node(path);
    let file_id = file_node.id.clone();
    result.nodes.push(file_node);

    let lines: Vec<&str> = source.lines().collect();
    let ps = path_str(path);

    // Classes / interfaces / structs / enums
    for cap in RE_CS_CLASS.captures_iter(source) {
        let kind = &cap[1];
        let name = &cap[2];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
        let node_type = match kind {
            "interface" => NodeType::Interface,
            "struct" => NodeType::Struct,
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

    // Methods
    let mut functions: Vec<(String, String, usize, usize)> = Vec::new();
    let func_matches: Vec<_> = RE_CS_METHOD.captures_iter(source).collect();
    for (i, cap) in func_matches.iter().enumerate() {
        let name = cap[1].to_string();
        if [
            "if", "for", "while", "switch", "catch", "return", "new", "throw",
        ]
        .contains(&name.as_str())
        {
            continue;
        }
        let start_line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
        let end_line = if i + 1 < func_matches.len() {
            source[..func_matches[i + 1].get(0).unwrap().start()]
                .lines()
                .count()
        } else {
            lines.len()
        };

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

    // using directives
    for cap in RE_CS_USING.captures_iter(source) {
        let ns = &cap[1];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
        let import_id = make_id(&[&ps, "using", ns]);
        result.nodes.push(GraphNode {
            id: import_id.clone(),
            label: ns.to_string(),
            source_file: ps.clone(),
            source_location: Some(format!("L{line}")),
            node_type: NodeType::Namespace,
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

// ═══════════════════════════════════════════════════════════════════════════
// Kotlin
// ═══════════════════════════════════════════════════════════════════════════

fn extract_kotlin(path: &Path, source: &str) -> ExtractionResult {
    let mut result = ExtractionResult::default();
    let file_node = make_file_node(path);
    let file_id = file_node.id.clone();
    result.nodes.push(file_node);

    let lines: Vec<&str> = source.lines().collect();
    let ps = path_str(path);

    // Classes / objects / interfaces
    for cap in RE_KT_CLASS.captures_iter(source) {
        let name = &cap[1];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
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

    // Functions: `fun foo(`
    let mut functions: Vec<(String, String, usize, usize)> = Vec::new();
    let func_matches: Vec<_> = RE_KT_FUNC.captures_iter(source).collect();
    for (i, cap) in func_matches.iter().enumerate() {
        let name = cap[1].to_string();
        let start_line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
        let end_line = if i + 1 < func_matches.len() {
            source[..func_matches[i + 1].get(0).unwrap().start()]
                .lines()
                .count()
        } else {
            lines.len()
        };

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

    // Imports
    for cap in RE_KT_IMPORT.captures_iter(source) {
        let module = &cap[1];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
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

// ═══════════════════════════════════════════════════════════════════════════
// Generic fallback (Scala, PHP, Swift, Lua, Zig, PowerShell, Elixir, ObjC, Julia)
// ═══════════════════════════════════════════════════════════════════════════

fn extract_generic(path: &Path, source: &str, _lang: &str) -> ExtractionResult {
    let mut result = ExtractionResult::default();
    let file_node = make_file_node(path);
    let file_id = file_node.id.clone();
    result.nodes.push(file_node);

    let lines: Vec<&str> = source.lines().collect();
    let ps = path_str(path);

    // Generic class/struct/module pattern
    for cap in RE_GEN_CLASS.captures_iter(source) {
        let name = &cap[1];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
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

    // Generic function pattern
    let mut functions: Vec<(String, String, usize, usize)> = Vec::new();
    let func_matches: Vec<_> = RE_GEN_FUNC.captures_iter(source).collect();
    for (i, cap) in func_matches.iter().enumerate() {
        let name = cap[1].to_string();
        let start_line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
        let end_line = if i + 1 < func_matches.len() {
            source[..func_matches[i + 1].get(0).unwrap().start()]
                .lines()
                .count()
        } else {
            lines.len()
        };

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

    // Generic import pattern
    for cap in RE_GEN_IMPORT.captures_iter(source) {
        let module = &cap[1];
        let line = source[..cap.get(0).unwrap().start()].lines().count() + 1;
        let import_id = make_id(&[&ps, "import", module]);
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

// Tests moved to tests/ast_extract.rs (integration tests)
