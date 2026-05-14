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

/// Build a hashmap for fast extension lookup (cached).
fn dispatch_map() -> &'static HashMap<&'static str, &'static str> {
    static MAP: std::sync::LazyLock<HashMap<&str, &str>> =
        std::sync::LazyLock::new(|| DISPATCH.iter().copied().collect());
    &MAP
}

/// Return the language name for a file extension (e.g. `".py"` → `"python"`).
pub fn language_for_path(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?;
    dispatch_map().get(&*format!(".{ext}")).copied()
}

// ---------------------------------------------------------------------------
// File collection
// ---------------------------------------------------------------------------

/// Recursively collect all supported source files under `target`.
pub fn collect_files(target: &Path) -> Vec<PathBuf> {
    let map = dispatch_map();
    let mut files = Vec::new();
    collect_files_inner(target, map, &mut files);
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
            let lang = if let Some(l) = language_for_path(path) { l } else {
                debug!("skipping unsupported file: {}", path.display());
                return None;
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
                .map_or("", |n| n.label.as_str());

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
    // Step 1: Build lookup indexes in a single pass over nodes.
    //   - id_to_label: node_id → label (for fast import label lookup)
    //   - stem_to_entities: file_stem → [(label, node_id, node_type)]
    //   - go_pkg_to_entities: go_dir_name → [(label, node_id, node_type)]
    let mut id_to_label: HashMap<String, String> = HashMap::new();
    let mut stem_to_entities: HashMap<String, Vec<(String, String, NodeType)>> = HashMap::new();
    let mut go_pkg_to_entities: HashMap<String, Vec<(String, String, NodeType)>> = HashMap::new();
    let mut source_file_to_stem: HashMap<String, String> = HashMap::new();
    let mut file_id_to_source: HashMap<String, String> = HashMap::new();

    // Collect defined entity IDs from edges (one pass)
    let defined_entity_ids: HashSet<String> = result
        .edges
        .iter()
        .filter(|e| e.relation == "defines")
        .map(|e| e.target.clone())
        .collect();

    // Build source_file → [entity_node_id] and id_to_label in one pass over edges
    let mut source_file_entities: HashMap<String, Vec<String>> = HashMap::new();
    for edge in &result.edges {
        if edge.relation == "defines" {
            source_file_entities
                .entry(edge.source_file.clone())
                .or_default()
                .push(edge.target.clone());
        }
    }

    // Single pass over nodes to build all indexes
    for node in &result.nodes {
        id_to_label.insert(node.id.clone(), node.label.clone());

        if node.node_type == NodeType::File {
            let stem = Path::new(&node.source_file)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            source_file_to_stem.insert(node.source_file.clone(), stem);
            file_id_to_source.insert(node.id.clone(), node.source_file.clone());
            continue;
        }

        if !defined_entity_ids.contains(&node.id) {
            continue;
        }

        let path = Path::new(&node.source_file);
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        stem_to_entities.entry(stem).or_default().push((
            node.label.clone(),
            node.id.clone(),
            node.node_type.clone(),
        ));

        // Go package grouping
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

    // Step 2: Resolve imports → create uses edges
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

        // O(1) lookup instead of linear scan
        let import_label = match id_to_label.get(&edge.target) {
            Some(label) => label.as_str(),
            None => continue,
        };

        if import_label.is_empty() {
            continue;
        }

        let target_entities = match ext {
            "js" | "jsx" | "ts" | "tsx" => resolve_jsts_import(import_label, &stem_to_entities),
            "go" => resolve_go_import(import_label, &stem_to_entities, &go_pkg_to_entities),
            "rs" => resolve_rust_import(import_label, &stem_to_entities),
            "java" => resolve_dot_import(import_label, &stem_to_entities),
            "cs" => resolve_dot_import(import_label, &stem_to_entities),
            "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" => {
                resolve_c_include(import_label, &stem_to_entities)
            }
            "kt" | "kts" => {
                let cleaned = import_label.strip_prefix("import ").unwrap_or(import_label);
                resolve_dot_import(cleaned.trim(), &stem_to_entities)
            }
            "php" => {
                let cleaned = import_label.strip_prefix("use ").unwrap_or(import_label);
                resolve_backslash_import(cleaned.trim(), &stem_to_entities)
            }
            "dart" => resolve_dart_import(import_label, &stem_to_entities),
            "scala" => {
                let cleaned = import_label.strip_prefix("import ").unwrap_or(import_label);
                resolve_dot_import(cleaned.trim(), &stem_to_entities)
            }
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
/// Handles aliased imports (`X as Y`), barrel exports (index files),
/// and re-exports (`export { } from`).
fn resolve_jsts_import<'a>(
    import_label: &str,
    stem_to_entities: &'a HashMap<String, Vec<(String, String, NodeType)>>,
) -> Vec<&'a (String, String, NodeType)> {
    // Strip alias: "Foo as Bar" → "Foo"
    let label = import_label.split(" as ").next().unwrap_or(import_label);

    let parts: Vec<&str> = label.split('/').collect();

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
    let simple = label.trim_start_matches("./").trim_start_matches("../");
    if let Some(entities) = stem_to_entities.get(simple) {
        return entities.iter().collect();
    }

    // Barrel export: if the last segment matches a directory, check for "index" file
    if let Some(entities) = stem_to_entities.get("index")
        && (label.contains('/') || label.starts_with('.'))
    {
        return entities.iter().collect();
    }

    Vec::new()
}

/// Resolve a Go import to target entities.
///
/// Go import labels are like `"fmt"`, `"net/http"`, or `"myproject/pkg/utils"`.
/// Handles dot imports (`import . "pkg"`), blank imports (`import _ "pkg"`),
/// and aliased imports (`import alias "pkg"`).
fn resolve_go_import<'a>(
    import_label: &str,
    stem_to_entities: &'a HashMap<String, Vec<(String, String, NodeType)>>,
    go_pkg_to_entities: &'a HashMap<String, Vec<(String, String, NodeType)>>,
) -> Vec<&'a (String, String, NodeType)> {
    // Strip dot import prefix, blank import prefix, or alias
    let label = import_label
        .trim_start_matches(". ")
        .trim_start_matches("_ ");
    // Also strip any remaining alias: `alias "path"` → `"path"`
    let label = if label.contains('"') {
        label.split('"').nth(1).unwrap_or(label)
    } else {
        label
    };

    let pkg_name = label.rsplit('/').next().unwrap_or(label);

    if let Some(entities) = go_pkg_to_entities.get(pkg_name) {
        return entities.iter().collect();
    }

    if let Some(entities) = stem_to_entities.get(pkg_name) {
        return entities.iter().collect();
    }

    Vec::new()
}

/// Resolve a Rust `use` import to target entities.
///
/// Handles `pub use` re-exports, glob imports (`use foo::*`),
/// and specific type imports (`use crate::model::Config`).
fn resolve_rust_import<'a>(
    import_label: &str,
    stem_to_entities: &'a HashMap<String, Vec<(String, String, NodeType)>>,
) -> Vec<&'a (String, String, NodeType)> {
    let label = import_label
        .strip_prefix("pub use ")
        .unwrap_or(import_label);
    let segments: Vec<&str> = label.split("::").collect();

    // Glob import: `use module::*` → return all entities from module
    if segments.last() == Some(&"*") && segments.len() >= 2 {
        let module = segments[segments.len() - 2];
        if let Some(entities) = stem_to_entities.get(module) {
            return entities.iter().collect();
        }
    }

    // Try the last segment as a module/file stem
    if let Some(last) = segments.last()
        && *last != "*"
        && let Some(entities) = stem_to_entities.get(*last)
    {
        return entities.iter().collect();
    }

    // Try the second-to-last segment (for `crate::module::Type` patterns)
    if segments.len() >= 2 {
        let module = segments[segments.len() - 2];
        if let Some(entities) = stem_to_entities.get(module) {
            let last = segments.last().unwrap();
            let filtered: Vec<_> = entities.iter().filter(|(lbl, _, _)| lbl == last).collect();
            if !filtered.is_empty() {
                return filtered;
            }
            return entities.iter().collect();
        }
    }

    Vec::new()
}

/// Resolve a dot-separated import (Java, C#, Kotlin, Scala, Swift).
///
/// Import labels like `"java.util.List"` or `"System.Collections.Generic"`.
/// Handles aliased imports (`using X = Y`), static imports (`import static`).
fn resolve_dot_import<'a>(
    import_label: &str,
    stem_to_entities: &'a HashMap<String, Vec<(String, String, NodeType)>>,
) -> Vec<&'a (String, String, NodeType)> {
    // Strip common prefixes: "static ", alias part after " = "
    let label = import_label.strip_prefix("static ").unwrap_or(import_label);
    let label = if let Some(idx) = label.find(" = ") {
        label[idx + 3..].trim()
    } else {
        label
    };

    let segments: Vec<&str> = label.split('.').collect();

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
            let filtered: Vec<_> = entities.iter().filter(|(lbl, _, _)| lbl == last).collect();
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
    // Start with the full import label
    let mut label = import_label;

    // Strip common prefixes: "import ", "export ", "part "
    if let Some(stripped) = label.strip_prefix("import ") {
        label = stripped;
    } else if let Some(stripped) = label.strip_prefix("export ") {
        label = stripped;
    } else if let Some(stripped) = label.strip_prefix("part ") {
        label = stripped;
    }

    // Step 1: Handle aliased imports like "utils.dart' as utils"
    // Extract the path part before " as "
    let path_and_alias = label;
    let path_part = if let Some(idx) = path_and_alias.find(" as ") {
        &path_and_alias[..idx]
    } else {
        path_and_alias
    };

    // Step 2: Handle deferred imports like "heavy.dart' deferred as heavy"
    // Extract the path part before " deferred"
    let path_deferred = path_part;
    let path_no_deferred = if let Some(idx) = path_deferred.find(" deferred") {
        &path_deferred[..idx]
    } else {
        path_deferred
    };

    // Step 3: Strip quotes
    let quoted = path_no_deferred.trim();
    let unquoted = quoted
        .trim_matches('\'') // Single quote character
        .trim_matches('"');

    // Step 4: Handle relative imports with "../" - resolve up to file stem
    let normalized = if unquoted.contains("../") {
        // For relative imports, just take the last segment (filename)
        // e.g., "../models/user.dart" -> "user"
        let last_segment = unquoted.rsplit('/').next().unwrap_or(unquoted);
        last_segment.strip_suffix(".dart").unwrap_or(last_segment)
    } else {
        // Step 5: Strip "package:" prefix
        let path_part = unquoted.strip_prefix("package:").unwrap_or(unquoted);

        // Step 6: Extract last path segment (filename)
        let last_segment = path_part.rsplit('/').next().unwrap_or(path_part);

        // Step 7: Strip .dart extension
        last_segment.strip_suffix(".dart").unwrap_or(last_segment)
    };

    // Look up the stem in the entities map
    if let Some(entities) = stem_to_entities.get(normalized) {
        return entities.iter().collect();
    }

    Vec::new()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
