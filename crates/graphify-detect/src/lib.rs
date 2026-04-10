//! File discovery and classification for graphify.
//!
//! Walks a directory tree, applies `.graphifyignore` filters, skips noise
//! directories and sensitive files, and classifies each file into a
//! [`FileType`] category for downstream extraction.

pub mod classify;
pub mod constants;
pub mod ignore;
pub mod sensitive;

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info, warn};
use walkdir::WalkDir;

pub use classify::{DetectedFile, FileType, classify_file};
pub use ignore::load_graphifyignore;
pub use sensitive::is_sensitive;

use constants::{CORPUS_UPPER_THRESHOLD, CORPUS_WARN_THRESHOLD, FILE_COUNT_UPPER, SKIP_DIRS};
use ignore::IgnoreSet;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during file detection.
#[derive(Debug, Error)]
pub enum DetectError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("walk error: {0}")]
    Walk(#[from] walkdir::Error),

    #[error("glob pattern error: {0}")]
    Glob(#[from] globset::Error),
}

// ---------------------------------------------------------------------------
// DetectResult
// ---------------------------------------------------------------------------

/// The outcome of a full directory scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectResult {
    /// Files grouped by type. Values are relative path strings.
    pub files: HashMap<FileType, Vec<String>>,
    /// Total number of classified files.
    pub total_files: usize,
    /// Approximate total word count across text-like files.
    pub total_words: usize,
    /// Whether the corpus is large enough to benefit from a knowledge graph.
    pub needs_graph: bool,
    /// An optional warning about corpus size.
    pub warning: Option<String>,
    /// Relative paths of files that were skipped because they look sensitive.
    pub skipped_sensitive: Vec<String>,
    /// Number of patterns loaded from `.graphifyignore`.
    pub graphifyignore_patterns: usize,
}

// ---------------------------------------------------------------------------
// Manifest (for incremental detect)
// ---------------------------------------------------------------------------

/// A simple manifest that records which files were previously detected.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub files: HashMap<String, FileType>,
}

const DEFAULT_MANIFEST_NAME: &str = ".graphify_manifest.json";

/// Load a previously-saved manifest from disk.
pub fn load_manifest(path: &Path) -> Option<Manifest> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Persist the current manifest to disk.
pub fn save_manifest(path: &Path, manifest: &Manifest) -> Result<(), DetectError> {
    let json = serde_json::to_string_pretty(manifest).map_err(std::io::Error::other)?;
    fs::write(path, json)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Core detection
// ---------------------------------------------------------------------------

/// Walk `root` and return a [`DetectResult`] with all discovered files.
pub fn detect(root: &Path) -> DetectResult {
    let ignore_patterns = load_graphifyignore(root);
    let ignore_set = IgnoreSet::new(&ignore_patterns);
    let pattern_count = ignore_patterns.len();

    let mut files: HashMap<FileType, Vec<String>> = HashMap::new();
    let mut total_words = 0usize;
    let mut skipped_sensitive = Vec::new();

    let walker = WalkDir::new(root).follow_links(false);

    for entry in walker
        .into_iter()
        .filter_entry(|e| !should_skip_entry(e, root, &ignore_set))
    {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                debug!("walk error (skipped): {err}");
                continue;
            }
        };

        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();

        // Sensitive check
        if is_sensitive(path) {
            if let Ok(rel) = path.strip_prefix(root) {
                skipped_sensitive.push(rel.to_string_lossy().into_owned());
            }
            debug!("skipping sensitive file: {}", path.display());
            continue;
        }

        // Classify
        let file_type = match classify_file(path) {
            Some(ft) => ft,
            None => continue,
        };

        // Word count (only for text-readable types)
        match file_type {
            FileType::Code | FileType::Document | FileType::Paper => {
                total_words += count_words(path);
            }
            FileType::Image => {}
        }

        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();

        files.entry(file_type).or_default().push(rel);
    }

    let total_files: usize = files.values().map(|v| v.len()).sum();

    // Determine warnings
    let warning = if total_words > CORPUS_UPPER_THRESHOLD {
        Some(format!(
            "Corpus is very large ({total_words} words, {total_files} files). \
             Consider narrowing scope with .graphifyignore."
        ))
    } else if total_words > CORPUS_WARN_THRESHOLD || total_files > FILE_COUNT_UPPER {
        Some(format!(
            "Large corpus detected ({total_words} words, {total_files} files). \
             Graph build may be slow."
        ))
    } else {
        None
    };

    let needs_graph = total_files >= 2;

    if let Some(ref w) = warning {
        warn!("{w}");
    }
    info!(
        "detect: {total_files} files, {total_words} words, {} sensitive skipped, \
         {pattern_count} ignore patterns",
        skipped_sensitive.len()
    );

    DetectResult {
        files,
        total_files,
        total_words,
        needs_graph,
        warning,
        skipped_sensitive,
        graphifyignore_patterns: pattern_count,
    }
}

/// Incremental detection: compares against a stored manifest and returns only
/// changed / new files.
pub fn detect_incremental(root: &Path, manifest_path: Option<&str>) -> DetectResult {
    let manifest_file = root.join(manifest_path.unwrap_or(DEFAULT_MANIFEST_NAME));
    let old_manifest = load_manifest(&manifest_file).unwrap_or_default();

    let result = detect(root);

    // Build a new manifest from the result
    let mut new_manifest = Manifest::default();
    for (ft, paths) in &result.files {
        for p in paths {
            new_manifest.files.insert(p.clone(), *ft);
        }
    }

    // Filter to only new or changed files
    let mut filtered_files: HashMap<FileType, Vec<String>> = HashMap::new();
    for (ft, paths) in &result.files {
        for p in paths {
            if !old_manifest.files.contains_key(p) {
                filtered_files.entry(*ft).or_default().push(p.clone());
            }
        }
    }

    let filtered_total: usize = filtered_files.values().map(|v| v.len()).sum();

    // Save the new manifest
    if let Err(e) = save_manifest(&manifest_file, &new_manifest) {
        warn!("failed to save manifest: {e}");
    }

    info!(
        "detect_incremental: {filtered_total} new files (total {total} on disk)",
        total = result.total_files,
    );

    DetectResult {
        files: filtered_files,
        total_files: filtered_total,
        total_words: result.total_words,
        needs_graph: result.needs_graph,
        warning: result.warning,
        skipped_sensitive: result.skipped_sensitive,
        graphifyignore_patterns: result.graphifyignore_patterns,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns `true` if this entry should be pruned from the walk.
fn should_skip_entry(entry: &walkdir::DirEntry, root: &Path, ignore_set: &IgnoreSet) -> bool {
    // Only filter directories here (files are checked individually).
    if entry.file_type().is_dir()
        && let Some(name) = entry.file_name().to_str() {
            if is_noise_dir(name) {
                return true;
            }
            // Skip hidden directories (except the root itself).
            if name.starts_with('.') && entry.path() != root {
                return true;
            }
        }

    // Check .graphifyignore patterns
    if ignore_set.is_ignored(entry.path(), root) {
        return true;
    }

    false
}

/// Returns `true` if a directory name is a known "noise" directory.
fn is_noise_dir(name: &str) -> bool {
    SKIP_DIRS.contains(&name)
        || name.ends_with("_venv")
        || name.ends_with("_env")
        || name.ends_with(".egg-info")
}

/// Approximate word count for a file by splitting on whitespace.
///
/// Returns 0 for files that can't be read as UTF-8 (binary, PDF, etc.).
fn count_words(path: &Path) -> usize {
    match fs::read_to_string(path) {
        Ok(content) => content.split_whitespace().count(),
        Err(_) => 0,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a temporary project tree for integration tests.
    fn make_test_tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // Code files
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/main.rs"),
            "fn main() { println!(\"hello\"); }",
        )
        .unwrap();
        fs::write(root.join("src/lib.py"), "def hello():\n    pass\n").unwrap();

        // Doc
        fs::write(root.join("README.md"), "# Project\n\nSome documentation.").unwrap();

        // Image
        fs::write(root.join("logo.png"), &[0x89, 0x50, 0x4E, 0x47]).unwrap();

        // Sensitive
        fs::write(root.join(".env"), "SECRET=foo").unwrap();

        // Noise dir
        fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        fs::write(root.join("node_modules/pkg/index.js"), "// noise").unwrap();

        // Hidden dir
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join(".hidden/secret.rs"), "// hidden").unwrap();

        // Unknown
        fs::write(root.join("data.parquet"), &[0u8; 16]).unwrap();

        dir
    }

    #[test]
    fn detect_walks_tree() {
        let dir = make_test_tree();
        let result = detect(dir.path());

        assert!(
            result.total_files >= 3,
            "expected at least 3 files, got {}",
            result.total_files
        );

        // Code files should be found
        let code = result
            .files
            .get(&FileType::Code)
            .expect("expected code files");
        assert!(code.iter().any(|p| p.ends_with("main.rs")));
        assert!(code.iter().any(|p| p.ends_with("lib.py")));

        // Document
        let docs = result
            .files
            .get(&FileType::Document)
            .expect("expected doc files");
        assert!(docs.iter().any(|p| p.contains("README.md")));

        // Image
        let imgs = result
            .files
            .get(&FileType::Image)
            .expect("expected image files");
        assert!(imgs.iter().any(|p| p.contains("logo.png")));

        // Sensitive files should be skipped
        assert!(!result.skipped_sensitive.is_empty());
        assert!(result.skipped_sensitive.iter().any(|p| p.contains(".env")));

        // node_modules should be skipped
        let all_paths: Vec<&String> = result.files.values().flat_map(|v| v.iter()).collect();
        assert!(
            !all_paths.iter().any(|p| p.contains("node_modules")),
            "node_modules should be skipped"
        );

        // .hidden should be skipped
        assert!(
            !all_paths.iter().any(|p| p.contains(".hidden")),
            ".hidden dir should be skipped"
        );

        // Unknown extensions should not appear
        assert!(
            !all_paths.iter().any(|p| p.contains("parquet")),
            "unknown extensions should be skipped"
        );
    }

    #[test]
    fn detect_with_graphifyignore() {
        let dir = make_test_tree();
        fs::write(dir.path().join(".graphifyignore"), "*.py\nREADME.md\n").unwrap();

        let result = detect(dir.path());

        let all_paths: Vec<&String> = result.files.values().flat_map(|v| v.iter()).collect();
        assert!(
            !all_paths.iter().any(|p| p.ends_with(".py")),
            ".py files should be ignored"
        );
        assert!(
            !all_paths.iter().any(|p| p.contains("README.md")),
            "README.md should be ignored"
        );
        assert_eq!(result.graphifyignore_patterns, 2);
    }

    #[test]
    fn detect_incremental_filters_known() {
        let dir = make_test_tree();
        let root = dir.path();

        // First run: populates the manifest
        let r1 = detect_incremental(root, None);
        assert!(r1.total_files >= 3);

        // Second run: nothing new
        let r2 = detect_incremental(root, None);
        assert_eq!(r2.total_files, 0, "no new files expected on second run");

        // Add a new file
        fs::write(root.join("new_file.ts"), "const x = 1;").unwrap();
        let r3 = detect_incremental(root, None);
        assert_eq!(r3.total_files, 1, "expected exactly 1 new file");
        let code = r3.files.get(&FileType::Code).expect("expected code");
        assert!(code.iter().any(|p| p.contains("new_file.ts")));
    }

    #[test]
    fn is_noise_dir_known() {
        assert!(is_noise_dir("node_modules"));
        assert!(is_noise_dir(".git"));
        assert!(is_noise_dir("__pycache__"));
        assert!(is_noise_dir("venv"));
        assert!(is_noise_dir("target"));
    }

    #[test]
    fn is_noise_dir_suffix_patterns() {
        assert!(is_noise_dir("my_venv"));
        assert!(is_noise_dir("project_env"));
        assert!(is_noise_dir("foo.egg-info"));
    }

    #[test]
    fn is_noise_dir_false_for_normal() {
        assert!(!is_noise_dir("src"));
        assert!(!is_noise_dir("lib"));
        assert!(!is_noise_dir("docs"));
    }

    #[test]
    fn count_words_basic() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("test.txt");
        fs::write(&p, "hello world foo bar baz").unwrap();
        assert_eq!(count_words(&p), 5);
    }

    #[test]
    fn count_words_returns_zero_for_missing() {
        assert_eq!(count_words(Path::new("/nonexistent/file.txt")), 0);
    }

    #[test]
    fn manifest_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("manifest.json");

        let mut m = Manifest::default();
        m.files.insert("src/main.rs".into(), FileType::Code);
        m.files.insert("README.md".into(), FileType::Document);

        save_manifest(&p, &m).unwrap();
        let loaded = load_manifest(&p).unwrap();
        assert_eq!(loaded.files.len(), 2);
        assert_eq!(loaded.files["src/main.rs"], FileType::Code);
    }

    #[test]
    fn needs_graph_with_few_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("only.rs"), "fn main() {}").unwrap();

        let result = detect(root);
        assert!(!result.needs_graph, "single file should not need graph");
    }

    #[test]
    fn make_id_compat() {
        // Verify graphify_core::id::make_id is accessible and works as expected
        assert_eq!(
            graphify_core::id::make_id(&["detect", "file.rs"]),
            "detect_file_rs"
        );
        assert_eq!(
            graphify_core::id::make_id(&["__init__", "MyClass"]),
            "init_myclass"
        );
    }
}
