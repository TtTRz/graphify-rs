//! Integration tests for cross-file import resolution and dispatch.

use graphify_extract::{extract, language_for_path};
use std::path::Path;

// ═══════════════════════════════════════════════════════════════════════════
// Dispatch
// ═══════════════════════════════════════════════════════════════════════════

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
