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
}

/// Walk `root` and return a [`DetectResult`] with all discovered files.
pub fn detect(root: &Path) -> DetectResult {
    detect_inner(root)
}

/// Like [`detect`], but uses a lightweight mtime+size+words index at
/// `index_path` to skip re-reading unchanged files for word counting.
///
/// All files are still returned — the index only speeds up the walk by
/// caching word counts. The index is created on the first run and kept
/// up-to-date on every subsequent run.
pub fn detect_fast(root: &Path, index_path: &Path) -> (DetectResult, bool) {
    let old_index = changeindex::load(index_path);
    let ignore_patterns = load_graphifyignore(root);
    let ignore_set = IgnoreSet::new(&ignore_patterns);
    let pattern_count = ignore_patterns.len();

    let mut files: HashMap<FileType, Vec<String>> = HashMap::new();
    let mut total_words = 0usize;
    let mut skipped_sensitive = Vec::new();
    let mut new_index = changeindex::ChangeIndex::default();

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

        let (mtime, size) = changeindex::file_meta(path).unwrap_or((0, 0));

        let old_entry = old_index.as_ref().and_then(|i| i.files.get(&rel));
        let (words, hash) = entry_words_and_hash(path, file_type, mtime, size, old_entry);

        total_words += words as usize;
        new_index.files.insert(
            rel.clone(),
            changeindex::ChangeEntry {
                mtime,
                size,
                words,
                hash,
            },
        );
        files.entry(file_type).or_default().push(rel);
    }

    let changed = match old_index.as_ref() {
        None => true,
        Some(old) => {
            old.files.len() != new_index.files.len()
                || new_index
                    .files
                    .iter()
                    .any(|(k, v)| old.files.get(k).is_none_or(|e| e.hash != v.hash))
        }
    };

    if let Err(e) = changeindex::save(index_path, &new_index) {
        warn!("failed to save changeindex: {e}");
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

    if let Some(ref w) = warning {
        warn!("{w}");
    }
    info!(
        "detect_fast: {total_files} files, {total_words} words, {} sensitive skipped, \
         {pattern_count} ignore patterns",
        skipped_sensitive.len()
    );

    let result = DetectResult {
        files,
        total_files,
        total_words,
        needs_graph: total_files >= 2,
        warning,
        skipped_sensitive,
        graphifyignore_patterns: pattern_count,
    };

    (result, changed)
}

fn detect_inner(root: &Path) -> DetectResult {
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

        match file_type {
            FileType::Code | FileType::Document | FileType::Paper => {
                total_words += count_words(path);
            }
            FileType::Image => {}
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

/// Compute `(words, hash)` for a file, using the changeindex cache where possible.
///
/// Decision tree:
/// - mtime + size unchanged → return cached values (no I/O)
/// - size unchanged, mtime different → read file, compute hash; if hash matches cache
///   treat as a spurious touch (return cached words with updated hash); else compute fresh
/// - size changed / new file / meta unavailable → compute fresh
fn entry_words_and_hash(
    path: &Path,
    file_type: FileType,
    mtime: u64,
    size: u64,
    old: Option<&changeindex::ChangeEntry>,
) -> (u64, String) {
    if mtime != 0
        && let Some(e) = old
    {
        if mtime == e.mtime && size == e.size {
            return (e.words, e.hash.clone());
        }
        if size == e.size {
            // Same size but mtime changed — verify hash before treating as modified.
            match file_type {
                FileType::Image => {
                    let h = graphify_cache::file_hash(path).unwrap_or_default();
                    return if h == e.hash { (e.words, h) } else { (0, h) };
                }
                _ => match fs::read_to_string(path) {
                    Ok(content) => {
                        let h = graphify_cache::content_hash(content.as_bytes());
                        return if h == e.hash {
                            (e.words, h)
                        } else {
                            (content.split_whitespace().count() as u64, h)
                        };
                    }
                    Err(_) => {
                        let h = graphify_cache::file_hash(path).unwrap_or_default();
                        return (0, h);
                    }
                },
            }
        }
    }
    // New file, size changed, or meta unavailable → compute fresh.
    match file_type {
        FileType::Image => (0, graphify_cache::file_hash(path).unwrap_or_default()),
        _ => match fs::read_to_string(path) {
            Ok(content) => {
                let h = graphify_cache::content_hash(content.as_bytes());
                (content.split_whitespace().count() as u64, h)
            }
            Err(_) => (0, graphify_cache::file_hash(path).unwrap_or_default()),
        },
    }
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
