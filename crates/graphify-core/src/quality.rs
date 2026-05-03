//! Source-quality heuristics shared by analysis, export, and semantic search.
//!
//! These heuristics do not ignore files. They classify nodes so low-signal
//! sources (generated code, minified bundles, build outputs, tests, vendored
//! dependencies) can stay in the graph while being downranked in LLM-facing
//! summaries and semantic search.

use std::path::Path;

use crate::model::{GraphNode, NodeType};

/// Metadata attached to each graph node under `extra`.
pub const EXTRA_SOURCE_KIND: &str = "source_kind";
pub const EXTRA_SOURCE_PRIORITY: &str = "source_priority";
pub const EXTRA_SOURCE_FLAGS: &str = "source_flags";

#[derive(Debug, Clone, PartialEq)]
pub struct SourceQuality {
    pub kind: &'static str,
    pub priority: f32,
    pub flags: Vec<&'static str>,
}

impl SourceQuality {
    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.contains(&flag)
    }

    pub fn is_low_signal(&self) -> bool {
        self.priority < 0.75
    }
}

/// Classify a source path + optional text sample.
///
/// Priority is a multiplier for LLM-facing ranking. It intentionally never
/// reaches zero: graphify should retain provenance and still be able to answer
/// targeted questions about generated/test/build files.
pub fn classify_source(path: &str, label: &str, id: &str, sample: Option<&str>) -> SourceQuality {
    let normalized = normalize_path(path);
    let lower_path = normalized.to_ascii_lowercase();
    let lower_label = label.to_ascii_lowercase();
    let lower_id = id.to_ascii_lowercase();
    let mut priority = 1.0f32;
    let mut flags = Vec::new();

    if is_test_path(&lower_path, &lower_label, &lower_id) {
        priority *= 0.58;
        flags.push("test");
    }

    if is_generated_path(&lower_path) || sample.is_some_and(looks_generated) {
        priority *= 0.42;
        flags.push("generated");
    }

    if is_minified_path(&lower_path) || sample.is_some_and(looks_minified) {
        priority *= 0.30;
        flags.push("minified");
    }

    if is_build_artifact_path(&lower_path) {
        priority *= 0.35;
        flags.push("build_artifact");
    }

    if is_dependency_path(&lower_path) {
        priority *= 0.35;
        flags.push("dependency");
    }

    if lower_path.ends_with(".down.sql") || lower_path.contains(".down.") {
        priority *= 0.70;
        flags.push("down_migration");
    }

    if lower_path.ends_with("schema.sql") || lower_path.ends_with(".up.sql") {
        priority *= 1.08;
        flags.push("schema");
    }

    if is_project_context_path(&lower_path) {
        priority *= 1.25;
        flags.push("project_context");
    }

    let kind = if flags.contains(&"generated") {
        "generated"
    } else if flags.contains(&"minified") {
        "minified"
    } else if flags.contains(&"build_artifact") {
        "build_artifact"
    } else if flags.contains(&"dependency") {
        "dependency"
    } else if flags.contains(&"test") {
        "test"
    } else if flags.contains(&"project_context") {
        "project_context"
    } else if flags.contains(&"schema") {
        "schema"
    } else {
        "source"
    };

    SourceQuality {
        kind,
        priority: priority.clamp(0.05, 2.0),
        flags,
    }
}

pub fn classify_node(node: &GraphNode) -> SourceQuality {
    classify_source(&node.source_file, &node.label, &node.id, None)
}

/// Ranking multiplier for LLM-facing surfaces.
pub fn node_priority(node: &GraphNode) -> f32 {
    if let Some(value) = node
        .extra
        .get(EXTRA_SOURCE_PRIORITY)
        .and_then(|v| v.as_f64())
    {
        return value as f32;
    }
    classify_node(node).priority
}

pub fn node_source_kind(node: &GraphNode) -> String {
    node.extra
        .get(EXTRA_SOURCE_KIND)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| classify_node(node).kind.to_string())
}

pub fn node_flags(node: &GraphNode) -> Vec<String> {
    if let Some(values) = node
        .extra
        .get(EXTRA_SOURCE_FLAGS)
        .and_then(|v| v.as_array())
    {
        return values
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
    }
    classify_node(node)
        .flags
        .into_iter()
        .map(str::to_string)
        .collect()
}

pub fn is_low_signal_node(node: &GraphNode) -> bool {
    node_priority(node) < 0.75
}

pub fn is_summary_candidate(node: &GraphNode) -> bool {
    if is_low_signal_node(node) {
        return false;
    }
    if node.label.starts_with('.') {
        return false;
    }
    !matches!(node.node_type, NodeType::Package | NodeType::Module)
}

fn normalize_path(path: &str) -> String {
    path.trim_start_matches("./").replace('\\', "/")
}

fn is_test_path(path: &str, label: &str, id: &str) -> bool {
    path.ends_with("_test.go")
        || path.ends_with(".test.ts")
        || path.ends_with(".test.tsx")
        || path.ends_with(".spec.ts")
        || path.ends_with(".spec.tsx")
        || path.contains("/test/")
        || path.contains("/tests/")
        || path.contains("/__tests__/")
        || path.contains("/fixtures/")
        || label.contains("mock")
        || id.contains("mock")
        || label.starts_with("test")
        || label.starts_with("make_test")
        || label.ends_with("_test")
}

fn is_generated_path(path: &str) -> bool {
    let name = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path);
    name.ends_with("_gen.go")
        || name.ends_with(".pb.go")
        || name.ends_with(".twirp.go")
        || name.ends_with(".generated.ts")
        || name.ends_with(".generated.tsx")
        || name.ends_with(".generated.js")
        || name.ends_with(".generated.jsx")
        || name.ends_with(".g.dart")
        || name.ends_with(".freezed.dart")
        || name == "generated.d.ts"
        || path.contains("/generated/")
        || path.contains("/gen/")
        || path.contains("/internal/proto/")
        || path.contains("/openapi/")
        || path.contains("/swagger/")
}

fn looks_generated(sample: &str) -> bool {
    let head = sample
        .chars()
        .take(4096)
        .collect::<String>()
        .to_ascii_lowercase();
    let signals = [
        "code generated",
        "auto-generated",
        "autogenerated",
        "automatically generated",
        "do not edit",
        "do not modify",
        "generated by",
        "@generated",
        "<auto-generated",
        "eslint-disable",
        "tslint:disable",
    ];
    signals
        .iter()
        .filter(|signal| head.contains(**signal))
        .count()
        >= 1
}

fn is_minified_path(path: &str) -> bool {
    let name = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path);
    name.ends_with(".min.js") || name.ends_with(".min.css") || name.ends_with(".bundle.js")
}

fn looks_minified(sample: &str) -> bool {
    let lines: Vec<&str> = sample.lines().take(20).collect();
    if lines.is_empty() {
        return false;
    }
    let non_empty: Vec<&str> = lines.into_iter().filter(|l| !l.trim().is_empty()).collect();
    if non_empty.is_empty() {
        return false;
    }
    let avg_len = non_empty.iter().map(|l| l.len()).sum::<usize>() / non_empty.len();
    let punctuation = sample
        .chars()
        .filter(|ch| matches!(ch, '{' | '}' | ';' | ',' | ':' | '(' | ')'))
        .count();
    let density = punctuation as f32 / sample.chars().count().max(1) as f32;
    avg_len > 500 || (avg_len > 220 && density > 0.18)
}

fn is_build_artifact_path(path: &str) -> bool {
    path.contains("/dist/")
        || path.starts_with("dist/")
        || path.contains("/build/")
        || path.starts_with("build/")
        || path.contains("/out/")
        || path.starts_with("out/")
        || path.contains("/.next/")
        || path.contains("/coverage/")
        || path.contains("/target/")
}

fn is_dependency_path(path: &str) -> bool {
    path.contains("/vendor/")
        || path.starts_with("vendor/")
        || path.contains("/node_modules/")
        || path.contains("/site-packages/")
}

fn is_project_context_path(path: &str) -> bool {
    path.ends_with("readme.md")
        || path.ends_with("product.md")
        || path.ends_with("architecture.md")
        || path.ends_with("agents.md")
        || path.ends_with("claude.md")
        || path.starts_with("docs/")
        || path.contains("/docs/")
        || path.starts_with(".planning/")
        || path.contains("/.planning/")
        || path.contains("/adr/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_generated_signatures() {
        let q = classify_source(
            "internal/oas/oas_schemas_gen.go",
            "oas_schemas_gen",
            "n",
            Some("// Code generated by ogen, DO NOT EDIT.\npackage oas"),
        );
        assert_eq!(q.kind, "generated");
        assert!(q.priority < 0.5);
        assert!(q.has_flag("generated"));
    }

    #[test]
    fn detects_minified_code() {
        let sample = "function a(){return 1};".repeat(80);
        let q = classify_source("public/app.min.js", "app", "n", Some(&sample));
        assert_eq!(q.kind, "minified");
        assert!(q.priority < 0.5);
    }

    #[test]
    fn project_context_is_boosted() {
        let q = classify_source(".planning/PROJECT.md", "FinTracker", "n", None);
        assert_eq!(q.kind, "project_context");
        assert!(q.priority > 1.0);
    }
}
