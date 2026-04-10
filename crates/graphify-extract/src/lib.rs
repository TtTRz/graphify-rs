//! AST and semantic extraction engine for graphify.
//!
//! Implements a two-pass extraction pipeline ported from the Python `extract.py`:
//!
//! - **Pass 1** (deterministic): regex-based AST extraction of functions, classes,
//!   imports, and call relationships from source code.
//! - **Pass 2** (semantic): Claude API–based extraction of higher-level concepts
//!   from documents, papers, and images.

pub mod ast_extract;
pub mod dedup;
pub mod lang_config;
pub mod parser;
pub mod semantic;
pub mod treesitter;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use graphify_core::confidence::Confidence;
use graphify_core::model::{ExtractionResult, GraphEdge, NodeType};
use rayon::prelude::*;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Extension → language dispatch table
// ---------------------------------------------------------------------------

/// Maps file extensions to language identifiers used by the extraction engine.
pub const DISPATCH: &[(&str, &str)] = &[
    (".py", "python"),
    (".js", "javascript"),
    (".jsx", "javascript"),
    (".ts", "typescript"),
    (".tsx", "typescript"),
    (".go", "go"),
    (".rs", "rust"),
    (".java", "java"),
    (".c", "c"),
    (".h", "c"),
    (".cpp", "cpp"),
    (".cc", "cpp"),
    (".cxx", "cpp"),
    (".hpp", "cpp"),
    (".rb", "ruby"),
    (".cs", "csharp"),
    (".kt", "kotlin"),
    (".kts", "kotlin"),
    (".scala", "scala"),
    (".php", "php"),
    (".swift", "swift"),
    (".lua", "lua"),
    (".toc", "lua"),
    (".zig", "zig"),
    (".ps1", "powershell"),
    (".ex", "elixir"),
    (".exs", "elixir"),
    (".m", "objc"),
    (".mm", "objc"),
    (".jl", "julia"),
];

/// Build a hashmap for fast extension lookup.
fn dispatch_map() -> HashMap<&'static str, &'static str> {
    DISPATCH.iter().copied().collect()
}

/// Return the language name for a file extension (e.g. `".py"` → `"python"`).
pub fn language_for_path(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?;
    let dotted = format!(".{ext}");
    dispatch_map().get(dotted.as_str()).copied()
}

// ---------------------------------------------------------------------------
// File collection
// ---------------------------------------------------------------------------

/// Recursively collect all supported source files under `target`.
pub fn collect_files(target: &Path) -> Vec<PathBuf> {
    let map = dispatch_map();
    let mut files = Vec::new();
    collect_files_inner(target, &map, &mut files);
    files.sort();
    files
}

fn collect_files_inner(dir: &Path, map: &HashMap<&str, &str>, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            warn!("cannot read directory {}: {e}", dir.display());
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden dirs and common vendor dirs
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.')
                || name == "node_modules"
                || name == "__pycache__"
                || name == "target"
                || name == "vendor"
                || name == "venv"
                || name == ".git"
            {
                continue;
            }
            collect_files_inner(&path, map, out);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let dotted = format!(".{ext}");
            if map.contains_key(dotted.as_str()) {
                out.push(path);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Main extraction entry point
// ---------------------------------------------------------------------------

/// Run Pass 1 extraction on a set of file paths.
///
/// Dispatches each file to the appropriate regex-based extractor, collects all
/// nodes and edges, deduplicates, and runs cross-file import resolution for Python.
///
/// Files are processed in parallel using rayon for improved throughput on
/// multi-core machines.
pub fn extract(paths: &[PathBuf]) -> ExtractionResult {
    let results: Vec<ExtractionResult> = paths
        .par_iter()
        .filter_map(|path| {
            let lang = match language_for_path(path) {
                Some(l) => l,
                None => {
                    debug!("skipping unsupported file: {}", path.display());
                    return None;
                }
            };

            let source = match std::fs::read(path) {
                Ok(s) => s,
                Err(e) => {
                    warn!("cannot read {}: {e}", path.display());
                    return None;
                }
            };

            debug!("extracting {} ({})", path.display(), lang);

            // Try tree-sitter first, fall back to regex
            let mut result = if let Some(ts_result) = treesitter::try_extract(path, &source, lang) {
                debug!("used tree-sitter for {} ({})", path.display(), lang);
                ts_result
            } else {
                let source_str = String::from_utf8_lossy(&source);
                ast_extract::extract_file(path, source_str.as_ref(), lang)
            };
            dedup::dedup_file(&mut result);

            Some(result)
        })
        .collect();

    let mut combined = ExtractionResult::default();
    for r in results {
        combined.nodes.extend(r.nodes);
        combined.edges.extend(r.edges);
        combined.hyperedges.extend(r.hyperedges);
    }

    // Cross-file import resolution for Python
    resolve_python_imports(&mut combined);

    // Cross-file import resolution for JS/TS, Go, and Rust
    resolve_cross_file_imports(&mut combined);

    info!(
        "extraction complete: {} nodes, {} edges",
        combined.nodes.len(),
        combined.edges.len()
    );

    combined
}

/// Resolve Python `import` / `from ... import` edges to actual module/function
/// nodes discovered across files.
fn resolve_python_imports(result: &mut ExtractionResult) {
    // Build a lookup from node label → node id
    let label_to_id: HashMap<String, String> = result
        .nodes
        .iter()
        .map(|n| (n.label.clone(), n.id.clone()))
        .collect();

    // For every edge with relation "imports", try to resolve the target
    for edge in &mut result.edges {
        if edge.relation == "imports" {
            // target is currently a raw import name – see if we have a matching node
            if let Some(resolved_id) = label_to_id.get(&edge.target) {
                edge.target = resolved_id.clone();
                edge.confidence = graphify_core::confidence::Confidence::Extracted;
            }
            // Otherwise, leave target as-is (unresolved external import)
        }
    }
}

/// Resolve cross-file imports for JS/TS, Go, and Rust.
///
/// For each `imports` edge, tries to match the imported module name to a file
/// stem and then creates `uses` edges from entities in the importing file to
/// entities defined in the target module. This turns file-level import edges
/// into entity-level relationship edges.
fn resolve_cross_file_imports(result: &mut ExtractionResult) {
    // Step 1: Build file stem → [(node_label, node_id, node_type)] for entities
    //         defined in each file. We key by the file stem (e.g. "utils" for "utils.ts").
    let mut stem_to_entities: HashMap<String, Vec<(String, String, NodeType)>> = HashMap::new();
    // Also build: file source_file → file stem
    let mut source_file_to_stem: HashMap<String, String> = HashMap::new();

    // Collect which node IDs are "entity" definitions (classes, functions, structs, etc.)
    // by looking at "defines" edges: file --defines--> entity
    let mut file_id_to_source: HashMap<String, String> = HashMap::new();
    let defined_entity_ids: HashSet<String> = result
        .edges
        .iter()
        .filter(|e| e.relation == "defines")
        .map(|e| e.target.clone())
        .collect();

    for node in &result.nodes {
        if node.node_type == NodeType::File {
            let stem = Path::new(&node.source_file)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            source_file_to_stem.insert(node.source_file.clone(), stem.clone());
            file_id_to_source.insert(node.id.clone(), node.source_file.clone());
        }
    }

    // Build stem → entities map from nodes that are defined (have a "defines" edge)
    for node in &result.nodes {
        if !defined_entity_ids.contains(&node.id) {
            continue;
        }
        let stem = Path::new(&node.source_file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        stem_to_entities.entry(stem).or_default().push((
            node.label.clone(),
            node.id.clone(),
            node.node_type.clone(),
        ));
    }

    // Step 2: Build a map of Go package directories → entities
    // Go files in the same directory share a package. Map dir name → entities.
    let mut go_pkg_to_entities: HashMap<String, Vec<(String, String, NodeType)>> = HashMap::new();
    for node in &result.nodes {
        if !defined_entity_ids.contains(&node.id) {
            continue;
        }
        let path = Path::new(&node.source_file);
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext == "go"
            && let Some(dir) = path
                .parent()
                .and_then(|d| d.file_name())
                .and_then(|d| d.to_str())
        {
            go_pkg_to_entities
                .entry(dir.to_string())
                .or_default()
                .push((node.label.clone(), node.id.clone(), node.node_type.clone()));
        }
    }

    // Step 3: For each file, collect its own entity IDs (the entities defined in that file)
    let mut file_source_to_entity_ids: HashMap<String, Vec<String>> = HashMap::new();
    for edge in &result.edges {
        if edge.relation == "defines" {
            file_source_to_entity_ids
                .entry(edge.source_file.clone())
                .or_default()
                .push(edge.source.clone()); // file_id is the source of "defines"
            // Actually we need source_file's entity IDs (the targets of "defines")
        }
    }
    // Rebuild correctly: source_file → [entity_node_id]
    let mut source_file_entities: HashMap<String, Vec<String>> = HashMap::new();
    for edge in &result.edges {
        if edge.relation == "defines" {
            source_file_entities
                .entry(edge.source_file.clone())
                .or_default()
                .push(edge.target.clone());
        }
    }

    // Step 4: Find import edges and resolve targets to create cross-file uses edges
    let mut new_edges: Vec<GraphEdge> = Vec::new();
    let mut seen = HashSet::new();

    for edge in &result.edges {
        if edge.relation != "imports" {
            continue;
        }

        let source_file = &edge.source_file;
        let ext = Path::new(source_file)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        // Determine the target module stem from the import node label
        // The import node's target is an import-node ID; find its label
        let import_label = result
            .nodes
            .iter()
            .find(|n| n.id == edge.target)
            .map(|n| n.label.as_str())
            .unwrap_or("");

        if import_label.is_empty() {
            continue;
        }

        let target_entities = match ext {
            // JS/TS: import labels are like "module/Name" or just "Name"
            // The module part of the path maps to a file stem
            "js" | "jsx" | "ts" | "tsx" => resolve_jsts_import(import_label, &stem_to_entities),
            // Go: import labels are like "fmt" or "net/http" – last segment is the package name
            "go" => resolve_go_import(import_label, &stem_to_entities, &go_pkg_to_entities),
            // Rust: import labels are like "std::collections" or "crate::model"
            "rs" => resolve_rust_import(import_label, &stem_to_entities),
            _ => continue,
        };

        if target_entities.is_empty() {
            continue;
        }

        // Get the importing file's own entities
        let local_entities = match source_file_entities.get(source_file) {
            Some(ids) => ids,
            None => continue,
        };

        // Create uses edges: each entity in the importing file → each entity in the target module
        for local_id in local_entities {
            for (_, target_id, _) in &target_entities {
                if local_id == target_id {
                    continue;
                }
                let key = (local_id.clone(), target_id.clone());
                if seen.contains(&key) {
                    continue;
                }
                seen.insert(key);
                new_edges.push(GraphEdge {
                    source: local_id.clone(),
                    target: target_id.clone(),
                    relation: "uses".to_string(),
                    confidence: Confidence::Inferred,
                    confidence_score: 0.8,
                    source_file: source_file.clone(),
                    source_location: None,
                    weight: 0.8,
                    extra: Default::default(),
                });
            }
        }
    }

    if !new_edges.is_empty() {
        debug!(
            "cross-file import resolution: created {} inferred uses edges",
            new_edges.len()
        );
    }

    result.edges.extend(new_edges);
}

/// Resolve a JS/TS import label to target entities.
///
/// Import labels can be:
/// - `"module/ExportedName"` (named import from module)
/// - `"DefaultName"` (default import, label is the local binding name)
/// - `"./relative/path"` module path
///
/// We try to match the module part (or the last path segment) to a file stem.
fn resolve_jsts_import<'a>(
    import_label: &str,
    stem_to_entities: &'a HashMap<String, Vec<(String, String, NodeType)>>,
) -> Vec<&'a (String, String, NodeType)> {
    // For named imports like "utils/parseDate", the stem is "utils"
    // For path imports like "./components/Button", the stem is "Button"
    let parts: Vec<&str> = import_label.split('/').collect();

    // Try the first segment as module stem (for "module/Name" patterns)
    if parts.len() >= 2 {
        let module_stem = parts[0].trim_start_matches('.');
        if let Some(entities) = stem_to_entities.get(module_stem) {
            return entities.iter().collect();
        }
    }

    // Try the last segment as file stem (for path-style imports)
    if let Some(last) = parts.last() {
        let stem = last.trim_start_matches('.');
        if let Some(entities) = stem_to_entities.get(stem) {
            return entities.iter().collect();
        }
    }

    // Try the whole label as a stem (for simple imports like "React")
    let simple = import_label
        .trim_start_matches("./")
        .trim_start_matches("../");
    if let Some(entities) = stem_to_entities.get(simple) {
        return entities.iter().collect();
    }

    Vec::new()
}

/// Resolve a Go import to target entities.
///
/// Go import labels are like `"fmt"`, `"net/http"`, or `"myproject/pkg/utils"`.
/// The last path segment is the package name.
fn resolve_go_import<'a>(
    import_label: &str,
    stem_to_entities: &'a HashMap<String, Vec<(String, String, NodeType)>>,
    go_pkg_to_entities: &'a HashMap<String, Vec<(String, String, NodeType)>>,
) -> Vec<&'a (String, String, NodeType)> {
    // Extract the last path segment as the package name
    let pkg_name = import_label.rsplit('/').next().unwrap_or(import_label);

    // Try matching against Go package directory names
    if let Some(entities) = go_pkg_to_entities.get(pkg_name) {
        return entities.iter().collect();
    }

    // Fall back to file stem matching
    if let Some(entities) = stem_to_entities.get(pkg_name) {
        return entities.iter().collect();
    }

    Vec::new()
}

/// Resolve a Rust `use` import to target entities.
///
/// Rust import labels are like `"std::collections"`, `"crate::model"`, or `"super::utils"`.
/// We try to match the last segment of the path to a file stem / module name.
fn resolve_rust_import<'a>(
    import_label: &str,
    stem_to_entities: &'a HashMap<String, Vec<(String, String, NodeType)>>,
) -> Vec<&'a (String, String, NodeType)> {
    let segments: Vec<&str> = import_label.split("::").collect();

    // Try the last segment as a module/file stem
    if let Some(last) = segments.last()
        && let Some(entities) = stem_to_entities.get(*last)
    {
        return entities.iter().collect();
    }

    // Try the second-to-last segment (for `crate::module::Type` patterns)
    if segments.len() >= 2 {
        let module = segments[segments.len() - 2];
        if let Some(entities) = stem_to_entities.get(module) {
            // Filter to only return entities whose label matches the last segment
            let last = segments.last().unwrap();
            let filtered: Vec<_> = entities
                .iter()
                .filter(|(label, _, _)| label == last)
                .collect();
            if !filtered.is_empty() {
                return filtered;
            }
            // If no exact match, return all entities from the module
            return entities.iter().collect();
        }
    }

    Vec::new()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use graphify_core::model::{GraphEdge, GraphNode};

    #[test]
    fn dispatch_table_covers_all_languages() {
        let map = dispatch_map();
        assert_eq!(map.get(".py"), Some(&"python"));
        assert_eq!(map.get(".rs"), Some(&"rust"));
        assert_eq!(map.get(".go"), Some(&"go"));
        assert_eq!(map.get(".tsx"), Some(&"typescript"));
        assert_eq!(map.get(".jl"), Some(&"julia"));
        assert_eq!(map.get(".mm"), Some(&"objc"));
    }

    #[test]
    fn language_for_path_works() {
        assert_eq!(language_for_path(Path::new("foo/bar.py")), Some("python"));
        assert_eq!(language_for_path(Path::new("main.rs")), Some("rust"));
        assert_eq!(language_for_path(Path::new("readme.md")), None);
    }

    #[test]
    fn extract_empty_paths() {
        let result = extract(&[]);
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    // -----------------------------------------------------------------------
    // Helpers for cross-file import resolution tests
    // -----------------------------------------------------------------------

    fn make_test_node(id: &str, label: &str, source_file: &str, node_type: NodeType) -> GraphNode {
        GraphNode {
            id: id.to_string(),
            label: label.to_string(),
            source_file: source_file.to_string(),
            source_location: None,
            node_type,
            community: None,
            extra: Default::default(),
        }
    }

    fn make_test_edge(source: &str, target: &str, relation: &str, source_file: &str) -> GraphEdge {
        GraphEdge {
            source: source.to_string(),
            target: target.to_string(),
            relation: relation.to_string(),
            confidence: Confidence::Extracted,
            confidence_score: 1.0,
            source_file: source_file.to_string(),
            source_location: None,
            weight: 1.0,
            extra: Default::default(),
        }
    }

    // -----------------------------------------------------------------------
    // JS/TS cross-file resolution
    // -----------------------------------------------------------------------

    #[test]
    fn jsts_cross_file_creates_uses_edges() {
        // File: src/app.ts defines AppController, imports from "utils"
        // File: src/utils.ts defines parseDate, formatDate
        let mut result = ExtractionResult {
            nodes: vec![
                make_test_node("file_app", "app", "src/app.ts", NodeType::File),
                make_test_node("app_ctrl", "AppController", "src/app.ts", NodeType::Class),
                make_test_node(
                    "import_utils",
                    "utils/parseDate",
                    "src/app.ts",
                    NodeType::Module,
                ),
                make_test_node("file_utils", "utils", "src/utils.ts", NodeType::File),
                make_test_node(
                    "parse_date",
                    "parseDate",
                    "src/utils.ts",
                    NodeType::Function,
                ),
                make_test_node(
                    "format_date",
                    "formatDate",
                    "src/utils.ts",
                    NodeType::Function,
                ),
            ],
            edges: vec![
                make_test_edge("file_app", "app_ctrl", "defines", "src/app.ts"),
                make_test_edge("file_app", "import_utils", "imports", "src/app.ts"),
                make_test_edge("file_utils", "parse_date", "defines", "src/utils.ts"),
                make_test_edge("file_utils", "format_date", "defines", "src/utils.ts"),
            ],
            hyperedges: vec![],
        };

        resolve_cross_file_imports(&mut result);

        let uses_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.relation == "uses")
            .collect();

        // AppController should use both parseDate and formatDate
        assert_eq!(
            uses_edges.len(),
            2,
            "expected 2 uses edges, got {}",
            uses_edges.len()
        );
        assert!(
            uses_edges
                .iter()
                .any(|e| e.source == "app_ctrl" && e.target == "parse_date")
        );
        assert!(
            uses_edges
                .iter()
                .any(|e| e.source == "app_ctrl" && e.target == "format_date")
        );

        // All uses edges should be Inferred with weight 0.8
        for edge in &uses_edges {
            assert_eq!(edge.confidence, Confidence::Inferred);
            assert!((edge.weight - 0.8).abs() < f64::EPSILON);
            assert!((edge.confidence_score - 0.8).abs() < f64::EPSILON);
        }
    }

    // -----------------------------------------------------------------------
    // Go cross-file resolution
    // -----------------------------------------------------------------------

    #[test]
    fn go_cross_file_creates_uses_edges() {
        // File: cmd/main.go defines Server, imports "myproject/pkg/utils"
        // File: pkg/utils/helpers.go defines ParseConfig, Validate
        let mut result = ExtractionResult {
            nodes: vec![
                make_test_node("file_main", "main", "cmd/main.go", NodeType::File),
                make_test_node("server", "Server", "cmd/main.go", NodeType::Struct),
                make_test_node(
                    "import_utils",
                    "myproject/pkg/utils",
                    "cmd/main.go",
                    NodeType::Package,
                ),
                make_test_node(
                    "file_helpers",
                    "helpers",
                    "pkg/utils/helpers.go",
                    NodeType::File,
                ),
                make_test_node(
                    "parse_config",
                    "ParseConfig",
                    "pkg/utils/helpers.go",
                    NodeType::Function,
                ),
                make_test_node(
                    "validate",
                    "Validate",
                    "pkg/utils/helpers.go",
                    NodeType::Function,
                ),
            ],
            edges: vec![
                make_test_edge("file_main", "server", "defines", "cmd/main.go"),
                make_test_edge("file_main", "import_utils", "imports", "cmd/main.go"),
                make_test_edge(
                    "file_helpers",
                    "parse_config",
                    "defines",
                    "pkg/utils/helpers.go",
                ),
                make_test_edge(
                    "file_helpers",
                    "validate",
                    "defines",
                    "pkg/utils/helpers.go",
                ),
            ],
            hyperedges: vec![],
        };

        resolve_cross_file_imports(&mut result);

        let uses_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.relation == "uses")
            .collect();

        // Server should use both ParseConfig and Validate
        assert_eq!(
            uses_edges.len(),
            2,
            "expected 2 uses edges, got {}",
            uses_edges.len()
        );
        assert!(
            uses_edges
                .iter()
                .any(|e| e.source == "server" && e.target == "parse_config")
        );
        assert!(
            uses_edges
                .iter()
                .any(|e| e.source == "server" && e.target == "validate")
        );

        for edge in &uses_edges {
            assert_eq!(edge.confidence, Confidence::Inferred);
        }
    }

    // -----------------------------------------------------------------------
    // Rust cross-file resolution
    // -----------------------------------------------------------------------

    #[test]
    fn rust_cross_file_creates_uses_edges() {
        // File: src/main.rs defines App, imports "crate::model"
        // File: src/model.rs defines Config, Database
        let mut result = ExtractionResult {
            nodes: vec![
                make_test_node("file_main", "main", "src/main.rs", NodeType::File),
                make_test_node("app", "App", "src/main.rs", NodeType::Struct),
                make_test_node(
                    "import_model",
                    "crate::model",
                    "src/main.rs",
                    NodeType::Module,
                ),
                make_test_node("file_model", "model", "src/model.rs", NodeType::File),
                make_test_node("config", "Config", "src/model.rs", NodeType::Struct),
                make_test_node("database", "Database", "src/model.rs", NodeType::Struct),
            ],
            edges: vec![
                make_test_edge("file_main", "app", "defines", "src/main.rs"),
                make_test_edge("file_main", "import_model", "imports", "src/main.rs"),
                make_test_edge("file_model", "config", "defines", "src/model.rs"),
                make_test_edge("file_model", "database", "defines", "src/model.rs"),
            ],
            hyperedges: vec![],
        };

        resolve_cross_file_imports(&mut result);

        let uses_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.relation == "uses")
            .collect();

        // App should use both Config and Database
        assert_eq!(
            uses_edges.len(),
            2,
            "expected 2 uses edges, got {}",
            uses_edges.len()
        );
        assert!(
            uses_edges
                .iter()
                .any(|e| e.source == "app" && e.target == "config")
        );
        assert!(
            uses_edges
                .iter()
                .any(|e| e.source == "app" && e.target == "database")
        );

        for edge in &uses_edges {
            assert_eq!(edge.confidence, Confidence::Inferred);
            assert!((edge.weight - 0.8).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn rust_cross_file_resolves_specific_type() {
        // `use crate::model::Config` should prefer Config over all entities in model
        let mut result = ExtractionResult {
            nodes: vec![
                make_test_node("file_main", "main", "src/main.rs", NodeType::File),
                make_test_node("app", "App", "src/main.rs", NodeType::Struct),
                make_test_node(
                    "import_config",
                    "crate::model::Config",
                    "src/main.rs",
                    NodeType::Module,
                ),
                make_test_node("file_model", "model", "src/model.rs", NodeType::File),
                make_test_node("config", "Config", "src/model.rs", NodeType::Struct),
                make_test_node("database", "Database", "src/model.rs", NodeType::Struct),
            ],
            edges: vec![
                make_test_edge("file_main", "app", "defines", "src/main.rs"),
                make_test_edge("file_main", "import_config", "imports", "src/main.rs"),
                make_test_edge("file_model", "config", "defines", "src/model.rs"),
                make_test_edge("file_model", "database", "defines", "src/model.rs"),
            ],
            hyperedges: vec![],
        };

        resolve_cross_file_imports(&mut result);

        let uses_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.relation == "uses")
            .collect();

        // Should only create edge to Config, not Database
        assert_eq!(
            uses_edges.len(),
            1,
            "expected 1 uses edge, got {}",
            uses_edges.len()
        );
        assert_eq!(uses_edges[0].source, "app");
        assert_eq!(uses_edges[0].target, "config");
    }

    #[test]
    fn cross_file_no_duplicate_edges() {
        // Two imports from the same module shouldn't create duplicate uses edges
        let mut result = ExtractionResult {
            nodes: vec![
                make_test_node("file_app", "app", "src/app.ts", NodeType::File),
                make_test_node("ctrl", "Controller", "src/app.ts", NodeType::Class),
                make_test_node("import1", "utils/foo", "src/app.ts", NodeType::Module),
                make_test_node("import2", "utils/bar", "src/app.ts", NodeType::Module),
                make_test_node("file_utils", "utils", "src/utils.ts", NodeType::File),
                make_test_node("helper", "Helper", "src/utils.ts", NodeType::Class),
            ],
            edges: vec![
                make_test_edge("file_app", "ctrl", "defines", "src/app.ts"),
                make_test_edge("file_app", "import1", "imports", "src/app.ts"),
                make_test_edge("file_app", "import2", "imports", "src/app.ts"),
                make_test_edge("file_utils", "helper", "defines", "src/utils.ts"),
            ],
            hyperedges: vec![],
        };

        resolve_cross_file_imports(&mut result);

        let uses_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.relation == "uses")
            .collect();

        // Only one edge Controller → Helper even though there are two imports from utils
        assert_eq!(
            uses_edges.len(),
            1,
            "expected 1 uses edge (no dups), got {}",
            uses_edges.len()
        );
    }

    #[test]
    fn cross_file_unresolved_import_creates_no_edges() {
        // Import from external module (not in our files) should create no uses edges
        let mut result = ExtractionResult {
            nodes: vec![
                make_test_node("file_main", "main", "src/main.rs", NodeType::File),
                make_test_node("app", "App", "src/main.rs", NodeType::Struct),
                make_test_node(
                    "import_serde",
                    "serde::Deserialize",
                    "src/main.rs",
                    NodeType::Module,
                ),
            ],
            edges: vec![
                make_test_edge("file_main", "app", "defines", "src/main.rs"),
                make_test_edge("file_main", "import_serde", "imports", "src/main.rs"),
            ],
            hyperedges: vec![],
        };

        resolve_cross_file_imports(&mut result);

        let uses_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.relation == "uses")
            .collect();

        assert!(
            uses_edges.is_empty(),
            "external imports should not create uses edges"
        );
    }

    #[test]
    fn python_resolver_not_broken_by_cross_file() {
        // Ensure the Python resolver still works independently
        let mut result = ExtractionResult {
            nodes: vec![
                make_test_node("file_a", "module_a", "src/a.py", NodeType::File),
                make_test_node("my_class", "MyClass", "src/a.py", NodeType::Class),
            ],
            edges: vec![make_test_edge("file_a", "MyClass", "imports", "src/a.py")],
            hyperedges: vec![],
        };

        resolve_python_imports(&mut result);

        // The import edge target should resolve to the node ID "my_class"
        assert_eq!(result.edges[0].target, "my_class");
    }
}
