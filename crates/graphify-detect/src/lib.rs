//! File discovery and classification for graphify.
//!
//! Walks a directory tree, applies `.graphifyignore` filters, skips noise
//! directories and sensitive files, and classifies each file into a
//! [`FileType`] category for downstream extraction.

pub mod changeindex;
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

pub use changeindex::{ChangeIndex, load_changeindex, save_changeindex};
pub use classify::{DetectedFile, FileType, classify_file};
pub use ignore::load_graphifyignore;
pub use sensitive::is_sensitive;

use constants::{CORPUS_UPPER_THRESHOLD, CORPUS_WARN_THRESHOLD, FILE_COUNT_UPPER, SKIP_DIRS};
use ignore::IgnoreSet;

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
    /// Relative paths that were present in a previous changeindex but are no
    /// longer on disk after filtering. Only populated by
    /// [`detect_with_changeindex`].
    #[serde(default)]
    pub deleted_files: Vec<String>,
}

/// A simple manifest that records which files were previously detected.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub files: HashMap<String, FileType>,
    /// Content hashes keyed by relative path, for incremental change detection.
    #[serde(default)]
    pub hashes: HashMap<String, String>,
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

/// Walk `root` and return a [`DetectResult`] with all discovered files.
pub fn detect(root: &Path) -> DetectResult {
    detect_inner(root, false, None, None).0
}

/// Walk `root`, apply all standard filters, then use `changeindex.db` inside
/// `changeindex_dir` to skip files whose mtime and size are unchanged.
///
/// On the first run (no `changeindex.db`) all detected files are returned and
/// the index is written.  On subsequent runs only new, modified, and deleted
/// files are included in the result; the index is updated to reflect the
/// current state of every file on disk.
///
/// `DetectResult::deleted_files` contains paths that were in the previous
/// index but are absent from the current walk (after filters).
pub fn detect_with_changeindex(root: &Path, changeindex_dir: &Path) -> DetectResult {
    // Single walk: get both the full file list and the filtered result.
    let (mut result, _, all_walked) = detect_inner(root, false, None, None);

    let index_path = changeindex_dir.join(changeindex::CHANGEINDEX_NAME);
    let old_index = changeindex::load_changeindex(&index_path);

    // Always save an up-to-date index covering every file on disk.
    let new_index = changeindex::build_from_relative(root, &all_walked);
    if let Err(e) = changeindex::save_changeindex(&index_path, &new_index) {
        warn!("failed to save changeindex: {e}");
    }

    let old = match old_index {
        None => {
            info!(
                "changeindex: first run, indexing {} files",
                all_walked.len()
            );
            return result;
        }
        Some(idx) => idx,
    };

    let (to_scan, deleted) = changeindex::diff(root, &all_walked, &old);

    let scan_set: std::collections::HashSet<String> = to_scan.into_iter().collect();
    let unchanged = all_walked.len().saturating_sub(scan_set.len());

    for vec in result.files.values_mut() {
        vec.retain(|p| scan_set.contains(p));
    }
    result.files.retain(|_, v| !v.is_empty());

    let filtered_total: usize = result.files.values().map(std::vec::Vec::len).sum();
    result.total_files = filtered_total;
    result.deleted_files = deleted;

    info!(
        "changeindex: {} to scan ({} unchanged, {} deleted)",
        filtered_total,
        unchanged,
        result.deleted_files.len(),
    );

    result
}

/// Internal detect that optionally computes content hashes during the walk.
///
/// Parameters:
/// - `compute_hashes`: compute SHA256 for change detection (incremental mode).
/// - `old_hashes`: known-good hashes from the previous manifest; unchanged
///   files are excluded from the returned result.
/// - `metadata_skip`: if a file's mtime+size match this index, the file is
///   treated as unchanged without any file reads (fast-path for incremental).
///
/// Returns `(DetectResult, Option<hashes>, all_walked)` where `all_walked`
/// is every relative path that passed filters — regardless of hash status —
/// so callers can build a complete changeindex.
fn detect_inner(
    root: &Path,
    compute_hashes: bool,
    old_hashes: Option<&HashMap<String, String>>,
    metadata_skip: Option<&changeindex::ChangeIndex>,
) -> (DetectResult, Option<HashMap<String, String>>, Vec<String>) {
    let ignore_patterns = load_graphifyignore(root);
    let ignore_set = IgnoreSet::new(&ignore_patterns);
    let pattern_count = ignore_patterns.len();

    let mut files: HashMap<FileType, Vec<String>> = HashMap::new();
    let mut total_words = 0usize;
    let mut skipped_sensitive = Vec::new();
    let mut hashes: HashMap<String, String> = HashMap::new();
    let mut all_walked: Vec<String> = Vec::new();

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

        if is_sensitive(path) {
            if let Ok(rel) = path.strip_prefix(root) {
                skipped_sensitive.push(rel.to_string_lossy().into_owned());
            }
            debug!("skipping sensitive file: {}", path.display());
            continue;
        }

        let file_type = match classify_file(path) {
            Some(ft) => ft,
            None => continue,
        };

        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();

        all_walked.push(rel.clone());

        // Metadata fast-path: if mtime+size match the changeindex, skip all
        // file reads and treat this file as unchanged (no SHA256 needed).
        if let Some(idx) = metadata_skip {
            if let Some(old_meta) = idx.files.get(&rel) {
                if let Some(cur_meta) = changeindex::file_entry(path) {
                    if cur_meta.mtime == old_meta.mtime && cur_meta.size == old_meta.size {
                        if compute_hashes {
                            if let Some(old_h) = old_hashes.and_then(|h| h.get(&rel)) {
                                hashes.insert(rel.clone(), old_h.clone());
                            }
                        }
                        files.entry(file_type).or_default();
                        continue;
                    }
                }
            }
        }

        if compute_hashes {
            if let Some(old) = old_hashes.and_then(|h| h.get(&rel)) {
                let full_path = root.join(&rel);
                match fs::read_to_string(&full_path) {
                    Ok(content) => {
                        let hash = graphify_cache::content_hash(content.as_bytes());
                        hashes.insert(rel.clone(), hash.clone());
                        if old == &hash {
                            total_words += content.split_whitespace().count();
                            files.entry(file_type).or_default();
                            continue;
                        }
                        total_words += content.split_whitespace().count();
                    }
                    Err(_) => {
                        let hash = graphify_cache::file_hash(path).unwrap_or_default();
                        hashes.insert(rel.clone(), hash.clone());
                        if old == &hash {
                            files.entry(file_type).or_default();
                            continue;
                        }
                    }
                }
            } else {
                let full_path = root.join(&rel);
                match fs::read_to_string(&full_path) {
                    Ok(content) => {
                        hashes.insert(
                            rel.clone(),
                            graphify_cache::content_hash(content.as_bytes()),
                        );
                        match file_type {
                            FileType::Code | FileType::Document | FileType::Paper => {
                                total_words += content.split_whitespace().count();
                            }
                            FileType::Image => {}
                        }
                    }
                    Err(_) => {
                        hashes.insert(
                            rel.clone(),
                            graphify_cache::file_hash(path).unwrap_or_default(),
                        );
                    }
                }
            }
        } else {
            match file_type {
                FileType::Code | FileType::Document | FileType::Paper => {
                    total_words += count_words(path);
                }
                FileType::Image => {}
            }
        }

        files.entry(file_type).or_default().push(rel);
    }

    let total_files: usize = files.values().map(std::vec::Vec::len).sum();

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

    let result = DetectResult {
        files,
        total_files,
        total_words,
        needs_graph,
        warning,
        skipped_sensitive,
        graphifyignore_patterns: pattern_count,
        deleted_files: Vec::new(),
    };

    (
        result,
        if compute_hashes { Some(hashes) } else { None },
        all_walked,
    )
}

/// Incremental detection: returns only changed / new files by comparing SHA256
/// content hashes against the previous manifest.
///
/// When a `changeindex.db` exists alongside the manifest, files whose mtime
/// and size are unchanged bypass the SHA256 read entirely (metadata fast-path),
/// making subsequent runs significantly faster on large repos. The changeindex
/// is always updated so future runs stay in sync.
pub fn detect_incremental(root: &Path, manifest_path: Option<&str>) -> DetectResult {
    let manifest_file = if std::path::Path::new(manifest_path.unwrap_or("")).is_absolute() {
        std::path::PathBuf::from(manifest_path.unwrap())
    } else {
        root.join(manifest_path.unwrap_or(DEFAULT_MANIFEST_NAME))
    };
    let old_manifest = load_manifest(&manifest_file).unwrap_or_default();

    // Changeindex lives alongside the manifest.
    let changeindex_path = manifest_file
        .parent()
        .unwrap_or(root)
        .join(changeindex::CHANGEINDEX_NAME);
    let old_changeindex = changeindex::load_changeindex(&changeindex_path);

    let (result, new_hashes, all_walked) = detect_inner(
        root,
        true,
        Some(&old_manifest.hashes),
        old_changeindex.as_ref(),
    );
    let new_hashes = new_hashes.unwrap_or_default();

    let mut new_manifest = Manifest {
        files: result
            .files
            .iter()
            .flat_map(|(ft, paths)| paths.iter().map(|p| (p.clone(), *ft)))
            .collect(),
        hashes: new_hashes.clone(),
    };

    for (rel, ft) in &old_manifest.files {
        if !new_manifest.files.contains_key(rel) {
            new_manifest.files.insert(rel.clone(), *ft);
        }
        if !new_manifest.hashes.contains_key(rel)
            && let Some(h) = old_manifest.hashes.get(rel)
        {
            new_manifest.hashes.insert(rel.clone(), h.clone());
        }
    }

    let filtered_total: usize = result.files.values().map(std::vec::Vec::len).sum();

    if let Err(e) = save_manifest(&manifest_file, &new_manifest) {
        warn!("failed to save manifest: {e}");
    }

    // Save an updated changeindex covering all files currently on disk.
    let new_changeindex = changeindex::build_from_relative(root, &all_walked);
    if let Err(e) = changeindex::save_changeindex(&changeindex_path, &new_changeindex) {
        warn!("failed to save changeindex: {e}");
    }

    info!(
        "detect_incremental: {filtered_total} new/changed files (total {total} on disk)",
        total = all_walked.len(),
    );

    result
}

/// Returns `true` if this entry should be pruned from the walk.
fn should_skip_entry(entry: &walkdir::DirEntry, root: &Path, ignore_set: &IgnoreSet) -> bool {
    if entry.file_type().is_dir()
        && let Some(name) = entry.file_name().to_str()
    {
        if is_noise_dir(name) {
            return true;
        }
        if name.starts_with('.') && entry.path() != root {
            return true;
        }
    }

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
        fs::write(root.join("logo.png"), [0x89, 0x50, 0x4E, 0x47]).unwrap();

        // Sensitive
        fs::write(root.join(".env"), "SECRET=foo").unwrap();

        // Noise dir
        fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        fs::write(root.join("node_modules/pkg/index.js"), "// noise").unwrap();

        // Hidden dir
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join(".hidden/secret.rs"), "// hidden").unwrap();

        // Unknown
        fs::write(root.join("data.parquet"), [0u8; 16]).unwrap();

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

        let r1 = detect_incremental(root, None);
        assert!(r1.total_files >= 3);

        let r2 = detect_incremental(root, None);
        let r2_new_count: usize = r2.files.values().map(|v| v.len()).sum();
        assert_eq!(
            r2_new_count, 0,
            "no new/changed files expected on second run"
        );

        fs::write(root.join("new_file.ts"), "const x = 1;").unwrap();
        let r3 = detect_incremental(root, None);
        let r3_new_count: usize = r3.files.values().map(|v| v.len()).sum();
        assert_eq!(r3_new_count, 1, "expected exactly 1 new file");
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
