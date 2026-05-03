//! Ignore parser and matcher for graphify file discovery.
//!
//! graphify respects repository-level Git ignore files by default and layers
//! `.graphifyignore` on top for graphify-specific overrides. The effective
//! precedence is:
//!
//! 1. `.gitignore` / `.git/info/exclude` ignore repository-local generated or
//!    bulky artifacts by default.
//! 2. `.graphifyignore` patterns are applied last, so `!path` entries can
//!    explicitly re-include a gitignored file or subtree for graph building.

use std::fs;
use std::path::Path;

use ignore_crate::gitignore::{Gitignore, GitignoreBuilder};

// ---------------------------------------------------------------------------
// load_graphifyignore
// ---------------------------------------------------------------------------

/// Read `.graphifyignore` from `root` and return the raw pattern strings.
///
/// Returns an empty vec if the file does not exist. Patterns use gitignore
/// syntax, including `!` negation to re-include paths ignored by `.gitignore`.
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

/// Pre-compiled ignore matcher for efficient repeated checks.
pub struct IgnoreSet {
    matcher: Gitignore,
    has_graphify_unignore: bool,
}

impl IgnoreSet {
    /// Build an `IgnoreSet` for a repository root.
    ///
    /// root `.gitignore` and `.git/info/exclude` are loaded first. The supplied
    /// `.graphifyignore` patterns are added last so graphify-specific `!`
    /// entries can override gitignored paths.
    pub fn new(root: &Path, graphify_patterns: &[String]) -> Self {
        let mut builder = GitignoreBuilder::new(root);

        add_ignore_file_if_exists(&mut builder, &root.join(".gitignore"));
        add_ignore_file_if_exists(&mut builder, &root.join(".git/info/exclude"));

        let graphifyignore_path = root.join(".graphifyignore");
        for pattern in graphify_patterns {
            let _ = builder.add_line(Some(graphifyignore_path.clone()), pattern);
        }

        let matcher = builder.build().unwrap_or_else(|_| {
            GitignoreBuilder::new(root)
                .build()
                .expect("empty gitignore matcher")
        });
        let has_graphify_unignore = graphify_patterns.iter().any(|pattern| {
            let trimmed = pattern.trim_start();
            trimmed.starts_with('!') && !trimmed.starts_with("\\!")
        });

        Self {
            matcher,
            has_graphify_unignore,
        }
    }

    /// Returns `true` if `path` should be ignored.
    pub fn is_ignored(&self, path: &Path, is_dir: bool) -> bool {
        self.matcher
            .matched_path_or_any_parents(path, is_dir)
            .is_ignore()
    }

    /// Returns `true` when `.graphifyignore` contains an unignore (`!`) rule.
    ///
    /// Directory pruning must be conservative in this case: an ignored parent
    /// may contain a descendant that `.graphifyignore` re-includes.
    pub fn has_graphify_unignore(&self) -> bool {
        self.has_graphify_unignore
    }
}

fn add_ignore_file_if_exists(builder: &mut GitignoreBuilder, path: &Path) {
    if path.is_file() {
        let _ = builder.add(path);
    }
}

// ---------------------------------------------------------------------------
// Convenience function
// ---------------------------------------------------------------------------

/// Check if a path is ignored given raw `.graphifyignore` patterns.
///
/// If you are checking many paths, prefer constructing an [`IgnoreSet`] once
/// and calling [`IgnoreSet::is_ignored`] in a loop.
pub fn is_ignored(path: &Path, root: &Path, patterns: &[String]) -> bool {
    let is_dir = path.is_dir();
    let set = IgnoreSet::new(root, patterns);
    set.is_ignored(path, is_dir)
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
        let set = IgnoreSet::new(&root, &patterns);

        assert!(set.is_ignored(Path::new("/project/a.tmp"), false));
        assert!(set.is_ignored(Path::new("/project/sub/b.tmp"), false));
        assert!(!set.is_ignored(Path::new("/project/main.rs"), false));
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

    #[test]
    fn graphifyignore_unignore_overrides_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("docs/references")).unwrap();
        std::fs::write(root.join(".gitignore"), "docs/references/\n").unwrap();
        std::fs::write(root.join(".graphifyignore"), "!docs/references/keep.md\n").unwrap();

        let patterns = load_graphifyignore(root);
        let set = IgnoreSet::new(root, &patterns);

        assert!(set.is_ignored(&root.join("docs/references/drop.md"), false));
        assert!(!set.is_ignored(&root.join("docs/references/keep.md"), false));
        assert!(set.has_graphify_unignore());
    }
}
