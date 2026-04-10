//! `.graphifyignore` parser and matcher.
//!
//! Follows a simplified gitignore-like format:
//! - One pattern per line
//! - Lines starting with `#` are comments
//! - Blank lines are skipped
//! - Patterns are matched using glob (fnmatch) semantics

use std::fs;
use std::path::Path;

use globset::{Glob, GlobMatcher};

// ---------------------------------------------------------------------------
// load_graphifyignore
// ---------------------------------------------------------------------------

/// Read `.graphifyignore` from `root` and return the raw pattern strings.
///
/// Returns an empty vec if the file does not exist.
pub fn load_graphifyignore(root: &Path) -> Vec<String> {
    let ignore_path = root.join(".graphifyignore");
    let content = match fs::read_to_string(&ignore_path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// IgnoreSet
// ---------------------------------------------------------------------------

/// Pre-compiled set of ignore matchers for efficient repeated checks.
pub struct IgnoreSet {
    matchers: Vec<(String, GlobMatcher)>,
}

impl IgnoreSet {
    /// Build an `IgnoreSet` from raw pattern strings.
    pub fn new(patterns: &[String]) -> Self {
        let matchers = patterns
            .iter()
            .filter_map(|p| Glob::new(p).ok().map(|g| (p.clone(), g.compile_matcher())))
            .collect();
        Self { matchers }
    }

    /// Returns `true` if `path` (relative to `root`) matches any ignore pattern.
    pub fn is_ignored(&self, path: &Path, root: &Path) -> bool {
        let rel = match path.strip_prefix(root) {
            Ok(r) => r,
            Err(_) => path,
        };

        let rel_str = rel.to_string_lossy();
        let file_name = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();

        for (pattern, matcher) in &self.matchers {
            // Match against full relative path
            if matcher.is_match(rel) {
                return true;
            }
            // Match against the relative path as a string
            if matcher.is_match(rel_str.as_ref()) {
                return true;
            }
            // Match against filename alone (for patterns without path separators)
            if !pattern.contains('/') && matcher.is_match(file_name.as_str()) {
                return true;
            }
            // Match against each path segment
            for component in rel.components() {
                if let std::path::Component::Normal(seg) = component
                    && matcher.is_match(seg)
                {
                    return true;
                }
            }
        }
        false
    }
}

// ---------------------------------------------------------------------------
// Convenience function
// ---------------------------------------------------------------------------

/// Check if a path is ignored given raw patterns (builds a fresh [`IgnoreSet`]).
///
/// If you are checking many paths, prefer constructing an [`IgnoreSet`] once
/// and calling [`IgnoreSet::is_ignored`] in a loop.
pub fn is_ignored(path: &Path, root: &Path, patterns: &[String]) -> bool {
    let set = IgnoreSet::new(patterns);
    set.is_ignored(path, root)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn load_empty_when_missing() {
        let patterns = load_graphifyignore(Path::new("/nonexistent/path"));
        assert!(patterns.is_empty());
    }

    #[test]
    fn load_parses_file() {
        let dir = std::env::temp_dir().join("graphify_test_ignorefile");
        std::fs::create_dir_all(&dir).unwrap();
        let ignore_path = dir.join(".graphifyignore");
        std::fs::write(
            &ignore_path,
            "# comment\n\n*.log\nvendor/\n  # indented comment  \ndata/*.csv\n",
        )
        .unwrap();

        let patterns = load_graphifyignore(&dir);
        assert_eq!(patterns, vec!["*.log", "vendor/", "data/*.csv"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_ignored_glob_match() {
        let root = PathBuf::from("/project");
        let patterns = vec!["*.log".to_string(), "vendor".to_string()];

        assert!(is_ignored(Path::new("/project/app.log"), &root, &patterns));
        assert!(is_ignored(
            Path::new("/project/vendor/lib.rs"),
            &root,
            &patterns,
        ));
        assert!(!is_ignored(
            Path::new("/project/src/main.rs"),
            &root,
            &patterns,
        ));
    }

    #[test]
    fn is_ignored_path_pattern() {
        let root = PathBuf::from("/project");
        let patterns = vec!["data/*.csv".to_string()];

        assert!(is_ignored(
            Path::new("/project/data/train.csv"),
            &root,
            &patterns,
        ));
        assert!(!is_ignored(
            Path::new("/project/src/data.csv"),
            &root,
            &patterns,
        ));
    }

    #[test]
    fn ignore_set_reuse() {
        let root = PathBuf::from("/project");
        let patterns = vec!["*.tmp".to_string()];
        let set = IgnoreSet::new(&patterns);

        assert!(set.is_ignored(Path::new("/project/a.tmp"), &root));
        assert!(set.is_ignored(Path::new("/project/sub/b.tmp"), &root));
        assert!(!set.is_ignored(Path::new("/project/main.rs"), &root));
    }

    #[test]
    fn empty_patterns_never_ignored() {
        let root = PathBuf::from("/project");
        assert!(!is_ignored(Path::new("/project/any.rs"), &root, &[]));
    }

    #[test]
    fn wildcard_prefix_pattern() {
        let root = PathBuf::from("/project");
        let patterns = vec!["temp_*".to_string()];
        assert!(is_ignored(
            Path::new("/project/temp_data.json"),
            &root,
            &patterns,
        ));
        assert!(!is_ignored(
            Path::new("/project/data.json"),
            &root,
            &patterns,
        ));
    }
}
