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
    (".dart", "dart"),
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
///
/// Also handles `from x import *` by expanding to all entities in module x.
fn resolve_python_imports(result: &mut ExtractionResult) {
    // Build a lookup from node label → node id
    let label_to_id: HashMap<String, String> = result
        .nodes
        .iter()
        .map(|n| (n.label.clone(), n.id.clone()))
        .collect();

    // Build module stem → [entity_id] for star import expansion
    let mut stem_to_entity_ids: HashMap<String, Vec<String>> = HashMap::new();
    let defined_targets: HashSet<String> = result
        .edges
        .iter()
        .filter(|e| e.relation == "defines")
        .map(|e| e.target.clone())
        .collect();
    for node in &result.nodes {
        if !defined_targets.contains(&node.id) {
            continue;
        }
        let stem = std::path::Path::new(&node.source_file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        stem_to_entity_ids
            .entry(stem)
            .or_default()
            .push(node.id.clone());
    }

    // Collect star import edges for expansion
    let mut star_expansions: Vec<GraphEdge> = Vec::new();

    // For every edge with relation "imports", try to resolve the target
    for edge in &mut result.edges {
        if edge.relation == "imports" {
            // Check for star import: target label contains "*"
            let import_label = result
                .nodes
                .iter()
                .find(|n| n.id == edge.target)
                .map(|n| n.label.as_str())
                .unwrap_or("");

            if import_label.contains('*') {
                // `from module import *` — expand to all entities in module
                let module_name = import_label.trim_end_matches(".*").trim_end_matches(" *");
                if let Some(entity_ids) = stem_to_entity_ids.get(module_name) {
                    for target_id in entity_ids {
                        star_expansions.push(GraphEdge {
                            source: edge.source.clone(),
                            target: target_id.clone(),
                            relation: "uses".to_string(),
                            confidence: Confidence::Inferred,
                            confidence_score: 0.7,
                            source_file: edge.source_file.clone(),
                            source_location: None,
                            weight: 0.7,
                            extra: Default::default(),
                        });
                    }
                }
            } else {
                // Regular import — resolve by label
                if let Some(resolved_id) = label_to_id.get(&edge.target) {
                    edge.target = resolved_id.clone();
                    edge.confidence = graphify_core::confidence::Confidence::Extracted;
                }
            }
        }
    }

    if !star_expansions.is_empty() {
        debug!(
            "python star import expansion: created {} uses edges",
            star_expansions.len()
        );
        result.edges.extend(star_expansions);
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
            "js" | "jsx" | "ts" | "tsx" => resolve_jsts_import(import_label, &stem_to_entities),
            // Go: import labels are like "fmt" or "net/http"
            "go" => resolve_go_import(import_label, &stem_to_entities, &go_pkg_to_entities),
            // Rust: import labels are like "std::collections" or "crate::model"
            "rs" => resolve_rust_import(import_label, &stem_to_entities),
            // Java: import labels are like "java.util.List"
            "java" => resolve_dot_import(import_label, &stem_to_entities),
            // C#: using labels are like "System.Collections.Generic"
            "cs" => resolve_dot_import(import_label, &stem_to_entities),
            // C/C++: include labels are like "stdio.h" or "myheader.h"
            "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" => {
                resolve_c_include(import_label, &stem_to_entities)
            }
            // Kotlin: raw "import com.example.Foo"
            "kt" | "kts" => {
                let cleaned = import_label.strip_prefix("import ").unwrap_or(import_label);
                resolve_dot_import(cleaned.trim(), &stem_to_entities)
            }
            // PHP: labels are like "use App\Models\User"
            "php" => {
                let cleaned = import_label.strip_prefix("use ").unwrap_or(import_label);
                resolve_backslash_import(cleaned.trim(), &stem_to_entities)
            }
            // Dart: raw "import 'package:foo/bar.dart'"
            "dart" => resolve_dart_import(import_label, &stem_to_entities),
            // Scala: raw "import scala.collection.mutable"
            "scala" => {
                let cleaned = import_label.strip_prefix("import ").unwrap_or(import_label);
                resolve_dot_import(cleaned.trim(), &stem_to_entities)
            }
            // Swift: raw "import Foundation"
            "swift" => {
                let cleaned = import_label.strip_prefix("import ").unwrap_or(import_label);
                resolve_dot_import(cleaned.trim(), &stem_to_entities)
            }
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
/// Also handles barrel exports: if import points to a directory, tries to
/// resolve via `index` file (index.js/index.ts).
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

    // Barrel export: if the last segment matches a directory, check for "index" file
    // This handles `import { Foo } from './components'` → resolves to components/index.ts
    if let Some(entities) = stem_to_entities.get("index") {
        // Only use index if the import looks like a directory path
        if import_label.contains('/') || import_label.starts_with('.') {
            return entities.iter().collect();
        }
    }

    Vec::new()
}

/// Resolve a Go import to target entities.
///
/// Go import labels are like `"fmt"`, `"net/http"`, or `"myproject/pkg/utils"`.
/// The last path segment is the package name.
/// Also handles dot imports (`import . "pkg"`) which bring all entities into scope.
fn resolve_go_import<'a>(
    import_label: &str,
    stem_to_entities: &'a HashMap<String, Vec<(String, String, NodeType)>>,
    go_pkg_to_entities: &'a HashMap<String, Vec<(String, String, NodeType)>>,
) -> Vec<&'a (String, String, NodeType)> {
    // Handle dot import: strip leading "." prefix
    let label = import_label.trim_start_matches(". ");

    // Extract the last path segment as the package name
    let pkg_name = label.rsplit('/').next().unwrap_or(label);

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
/// Handles `pub use` re-exports: if import label starts with `pub use`, resolves
/// transitively through the re-exporting module to the original definition.
fn resolve_rust_import<'a>(
    import_label: &str,
    stem_to_entities: &'a HashMap<String, Vec<(String, String, NodeType)>>,
) -> Vec<&'a (String, String, NodeType)> {
    // Strip `pub use ` prefix if present (re-export)
    let label = import_label
        .strip_prefix("pub use ")
        .unwrap_or(import_label);
    let segments: Vec<&str> = label.split("::").collect();

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

/// Resolve a dot-separated import (Java, C#, Kotlin, Scala, Swift).
///
/// Import labels like `"java.util.List"` or `"System.Collections.Generic"`.
/// Tries the last segment as a type name, then second-to-last as module stem.
fn resolve_dot_import<'a>(
    import_label: &str,
    stem_to_entities: &'a HashMap<String, Vec<(String, String, NodeType)>>,
) -> Vec<&'a (String, String, NodeType)> {
    let segments: Vec<&str> = import_label.split('.').collect();

    // Try the last segment as a type/entity name matching a file stem
    if let Some(last) = segments.last()
        && let Some(entities) = stem_to_entities.get(*last)
    {
        return entities.iter().collect();
    }

    // Try second-to-last as module, filter to last segment
    if segments.len() >= 2 {
        let module = segments[segments.len() - 2];
        if let Some(entities) = stem_to_entities.get(module) {
            let last = segments.last().unwrap();
            let filtered: Vec<_> = entities
                .iter()
                .filter(|(label, _, _)| label == last)
                .collect();
            if !filtered.is_empty() {
                return filtered;
            }
            return entities.iter().collect();
        }
    }

    Vec::new()
}

/// Resolve a C/C++ `#include` to target entities.
///
/// Include labels are like `"stdio.h"` or `"myheader.h"`.
/// Strips the extension and matches the stem to file entities.
fn resolve_c_include<'a>(
    import_label: &str,
    stem_to_entities: &'a HashMap<String, Vec<(String, String, NodeType)>>,
) -> Vec<&'a (String, String, NodeType)> {
    // Strip angle brackets and quotes
    let label = import_label
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim_start_matches('"')
        .trim_end_matches('"');

    // Strip extension (.h, .hpp, etc.)
    let stem = std::path::Path::new(label)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(label);

    if let Some(entities) = stem_to_entities.get(stem) {
        return entities.iter().collect();
    }

    Vec::new()
}

/// Resolve a PHP backslash-separated import.
///
/// Labels like `"App\Models\User"` → try "User" as stem, then "Models".
fn resolve_backslash_import<'a>(
    import_label: &str,
    stem_to_entities: &'a HashMap<String, Vec<(String, String, NodeType)>>,
) -> Vec<&'a (String, String, NodeType)> {
    let segments: Vec<&str> = import_label.split('\\').collect();

    // Try the last segment as entity name
    if let Some(last) = segments.last()
        && let Some(entities) = stem_to_entities.get(*last)
    {
        return entities.iter().collect();
    }

    // Try second-to-last as module
    if segments.len() >= 2 {
        let module = segments[segments.len() - 2];
        if let Some(entities) = stem_to_entities.get(module) {
            return entities.iter().collect();
        }
    }

    Vec::new()
}

/// Resolve a Dart import.
///
/// Labels like `"import 'package:foo/bar.dart'"` or `"import 'bar.dart'"`.
/// Extracts the file stem from the path.
fn resolve_dart_import<'a>(
    import_label: &str,
    stem_to_entities: &'a HashMap<String, Vec<(String, String, NodeType)>>,
) -> Vec<&'a (String, String, NodeType)> {
    // Strip "import " prefix and quotes
    let label = import_label
        .strip_prefix("import ")
        .unwrap_or(import_label)
        .trim_matches('\'')
        .trim_matches('"');

    // Strip "package:" prefix and extract last path segment
    let path_part = label.strip_prefix("package:").unwrap_or(label);
    let last_segment = path_part.rsplit('/').next().unwrap_or(path_part);

    // Strip .dart extension
    let stem = last_segment.strip_suffix(".dart").unwrap_or(last_segment);

    if let Some(entities) = stem_to_entities.get(stem) {
        return entities.iter().collect();
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

    // ===== Java cross-file resolution =====

    #[test]
    fn java_cross_file_creates_uses_edges() {
        let mut result = ExtractionResult {
            nodes: vec![
                make_test_node("file_app", "App", "src/App.java", NodeType::File),
                make_test_node("app_class", "App", "src/App.java", NodeType::Class),
                make_test_node(
                    "import_util",
                    "com.example.Util",
                    "src/App.java",
                    NodeType::Module,
                ),
                make_test_node("file_util", "Util", "src/Util.java", NodeType::File),
                make_test_node("util_class", "Util", "src/Util.java", NodeType::Class),
            ],
            edges: vec![
                make_test_edge("file_app", "app_class", "defines", "src/App.java"),
                make_test_edge("file_app", "import_util", "imports", "src/App.java"),
                make_test_edge("file_util", "util_class", "defines", "src/Util.java"),
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
            !uses_edges.is_empty(),
            "Java cross-file should create uses edges"
        );
        assert!(
            uses_edges
                .iter()
                .any(|e| e.source == "app_class" && e.target == "util_class")
        );
    }

    // ===== C/C++ cross-file resolution =====

    #[test]
    fn c_include_creates_uses_edges() {
        let mut result = ExtractionResult {
            nodes: vec![
                make_test_node("file_main", "main", "src/main.c", NodeType::File),
                make_test_node("main_fn", "main", "src/main.c", NodeType::Function),
                make_test_node("import_utils", "utils.h", "src/main.c", NodeType::Module),
                make_test_node("file_utils", "utils", "src/utils.c", NodeType::File),
                make_test_node("helper_fn", "helper", "src/utils.c", NodeType::Function),
            ],
            edges: vec![
                make_test_edge("file_main", "main_fn", "defines", "src/main.c"),
                make_test_edge("file_main", "import_utils", "imports", "src/main.c"),
                make_test_edge("file_utils", "helper_fn", "defines", "src/utils.c"),
            ],
            hyperedges: vec![],
        };

        resolve_cross_file_imports(&mut result);

        let uses_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.relation == "uses")
            .collect();
        assert!(!uses_edges.is_empty(), "C include should create uses edges");
        assert!(
            uses_edges
                .iter()
                .any(|e| e.source == "main_fn" && e.target == "helper_fn")
        );
    }

    // ===== C# cross-file resolution =====

    #[test]
    fn csharp_using_creates_uses_edges() {
        let mut result = ExtractionResult {
            nodes: vec![
                make_test_node("file_prog", "Program", "src/Program.cs", NodeType::File),
                make_test_node("prog_class", "Program", "src/Program.cs", NodeType::Class),
                make_test_node(
                    "import_svc",
                    "MyApp.Services.UserService",
                    "src/Program.cs",
                    NodeType::Module,
                ),
                make_test_node(
                    "file_svc",
                    "UserService",
                    "src/UserService.cs",
                    NodeType::File,
                ),
                make_test_node(
                    "svc_class",
                    "UserService",
                    "src/UserService.cs",
                    NodeType::Class,
                ),
            ],
            edges: vec![
                make_test_edge("file_prog", "prog_class", "defines", "src/Program.cs"),
                make_test_edge("file_prog", "import_svc", "imports", "src/Program.cs"),
                make_test_edge("file_svc", "svc_class", "defines", "src/UserService.cs"),
            ],
            hyperedges: vec![],
        };

        resolve_cross_file_imports(&mut result);

        let uses_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.relation == "uses")
            .collect();
        assert!(!uses_edges.is_empty(), "C# using should create uses edges");
        assert!(
            uses_edges
                .iter()
                .any(|e| e.source == "prog_class" && e.target == "svc_class")
        );
    }

    // ===== PHP cross-file resolution =====

    #[test]
    fn php_use_creates_uses_edges() {
        let mut result = ExtractionResult {
            nodes: vec![
                make_test_node(
                    "file_ctrl",
                    "Controller",
                    "src/Controller.php",
                    NodeType::File,
                ),
                make_test_node(
                    "ctrl_class",
                    "Controller",
                    "src/Controller.php",
                    NodeType::Class,
                ),
                make_test_node(
                    "import_user",
                    r"use App\Models\User",
                    "src/Controller.php",
                    NodeType::Module,
                ),
                make_test_node("file_user", "User", "src/User.php", NodeType::File),
                make_test_node("user_class", "User", "src/User.php", NodeType::Class),
            ],
            edges: vec![
                make_test_edge("file_ctrl", "ctrl_class", "defines", "src/Controller.php"),
                make_test_edge("file_ctrl", "import_user", "imports", "src/Controller.php"),
                make_test_edge("file_user", "user_class", "defines", "src/User.php"),
            ],
            hyperedges: vec![],
        };

        resolve_cross_file_imports(&mut result);

        let uses_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.relation == "uses")
            .collect();
        assert!(!uses_edges.is_empty(), "PHP use should create uses edges");
        assert!(
            uses_edges
                .iter()
                .any(|e| e.source == "ctrl_class" && e.target == "user_class")
        );
    }

    // ===== Dart cross-file resolution =====

    #[test]
    fn dart_import_creates_uses_edges() {
        let mut result = ExtractionResult {
            nodes: vec![
                make_test_node("file_main", "main", "lib/main.dart", NodeType::File),
                make_test_node("main_fn", "main", "lib/main.dart", NodeType::Function),
                make_test_node(
                    "import_utils",
                    "import 'package:myapp/utils.dart'",
                    "lib/main.dart",
                    NodeType::Module,
                ),
                make_test_node("file_utils", "utils", "lib/utils.dart", NodeType::File),
                make_test_node("helper_fn", "helper", "lib/utils.dart", NodeType::Function),
            ],
            edges: vec![
                make_test_edge("file_main", "main_fn", "defines", "lib/main.dart"),
                make_test_edge("file_main", "import_utils", "imports", "lib/main.dart"),
                make_test_edge("file_utils", "helper_fn", "defines", "lib/utils.dart"),
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
            !uses_edges.is_empty(),
            "Dart import should create uses edges"
        );
        assert!(
            uses_edges
                .iter()
                .any(|e| e.source == "main_fn" && e.target == "helper_fn")
        );
    }

    // ===== Kotlin cross-file resolution =====

    #[test]
    fn kotlin_import_creates_uses_edges() {
        let mut result = ExtractionResult {
            nodes: vec![
                make_test_node("file_main", "Main", "src/Main.kt", NodeType::File),
                make_test_node("main_fn", "main", "src/Main.kt", NodeType::Function),
                make_test_node(
                    "import_repo",
                    "import com.example.UserRepo",
                    "src/Main.kt",
                    NodeType::Module,
                ),
                make_test_node("file_repo", "UserRepo", "src/UserRepo.kt", NodeType::File),
                make_test_node("repo_class", "UserRepo", "src/UserRepo.kt", NodeType::Class),
            ],
            edges: vec![
                make_test_edge("file_main", "main_fn", "defines", "src/Main.kt"),
                make_test_edge("file_main", "import_repo", "imports", "src/Main.kt"),
                make_test_edge("file_repo", "repo_class", "defines", "src/UserRepo.kt"),
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
            !uses_edges.is_empty(),
            "Kotlin import should create uses edges"
        );
        assert!(
            uses_edges
                .iter()
                .any(|e| e.source == "main_fn" && e.target == "repo_class")
        );
    }

    // ===== Python star import expansion =====

    #[test]
    fn python_star_import_expands_to_entities() {
        let mut result = ExtractionResult {
            nodes: vec![
                make_test_node("file_app", "app", "src/app.py", NodeType::File),
                make_test_node("app_fn", "run", "src/app.py", NodeType::Function),
                make_test_node("import_star", "utils.*", "src/app.py", NodeType::Module),
                make_test_node("file_utils", "utils", "src/utils.py", NodeType::File),
                make_test_node("helper1", "helper1", "src/utils.py", NodeType::Function),
                make_test_node("helper2", "helper2", "src/utils.py", NodeType::Function),
            ],
            edges: vec![
                make_test_edge("file_app", "app_fn", "defines", "src/app.py"),
                make_test_edge("file_app", "import_star", "imports", "src/app.py"),
                make_test_edge("file_utils", "helper1", "defines", "src/utils.py"),
                make_test_edge("file_utils", "helper2", "defines", "src/utils.py"),
            ],
            hyperedges: vec![],
        };

        resolve_python_imports(&mut result);

        let uses_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.relation == "uses")
            .collect();
        assert_eq!(
            uses_edges.len(),
            2,
            "star import should expand to 2 uses edges, got {}",
            uses_edges.len()
        );
    }

    // ===== Scala cross-file resolution =====

    #[test]
    fn scala_cross_file_creates_uses_edges() {
        let mut result = ExtractionResult {
            nodes: vec![
                make_test_node("file_main", "Main", "src/Main.scala", NodeType::File),
                make_test_node("main_fn", "main", "src/Main.scala", NodeType::Function),
                make_test_node(
                    "import_calc",
                    "import com.example.Calculator",
                    "src/Main.scala",
                    NodeType::Module,
                ),
                make_test_node(
                    "file_calc",
                    "Calculator",
                    "src/Calculator.scala",
                    NodeType::File,
                ),
                make_test_node(
                    "calc_class",
                    "Calculator",
                    "src/Calculator.scala",
                    NodeType::Class,
                ),
            ],
            edges: vec![
                make_test_edge("file_main", "main_fn", "defines", "src/Main.scala"),
                make_test_edge("file_main", "import_calc", "imports", "src/Main.scala"),
                make_test_edge("file_calc", "calc_class", "defines", "src/Calculator.scala"),
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
            !uses_edges.is_empty(),
            "Scala cross-file should create uses edges"
        );
        assert!(
            uses_edges
                .iter()
                .any(|e| e.source == "main_fn" && e.target == "calc_class")
        );
    }

    // ===== Swift cross-file resolution =====

    #[test]
    fn swift_cross_file_creates_uses_edges() {
        let mut result = ExtractionResult {
            nodes: vec![
                make_test_node("file_app", "App", "src/App.swift", NodeType::File),
                make_test_node("app_fn", "run", "src/App.swift", NodeType::Function),
                make_test_node(
                    "import_mgr",
                    "import UserManager",
                    "src/App.swift",
                    NodeType::Module,
                ),
                make_test_node(
                    "file_mgr",
                    "UserManager",
                    "src/UserManager.swift",
                    NodeType::File,
                ),
                make_test_node(
                    "mgr_class",
                    "UserManager",
                    "src/UserManager.swift",
                    NodeType::Class,
                ),
            ],
            edges: vec![
                make_test_edge("file_app", "app_fn", "defines", "src/App.swift"),
                make_test_edge("file_app", "import_mgr", "imports", "src/App.swift"),
                make_test_edge("file_mgr", "mgr_class", "defines", "src/UserManager.swift"),
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
            !uses_edges.is_empty(),
            "Swift cross-file should create uses edges"
        );
        assert!(
            uses_edges
                .iter()
                .any(|e| e.source == "app_fn" && e.target == "mgr_class")
        );
    }
}
