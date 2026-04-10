//! Path traversal prevention and graph file validation.

use std::path::{Path, PathBuf};

use crate::SecurityError;

/// Ensure a path stays within an allowed directory (no `../` traversal).
///
/// Both `path` and `allowed_root` are canonicalized before comparison, so
/// symlinks and relative components are resolved.
pub fn safe_path(path: &Path, allowed_root: &Path) -> Result<PathBuf, SecurityError> {
    let canonical = path
        .canonicalize()
        .map_err(|_| SecurityError::PathTraversal(path.to_string_lossy().to_string()))?;
    let root = allowed_root
        .canonicalize()
        .map_err(|_| SecurityError::PathTraversal(allowed_root.to_string_lossy().to_string()))?;

    if canonical.starts_with(&root) {
        Ok(canonical)
    } else {
        Err(SecurityError::PathTraversal(
            path.to_string_lossy().to_string(),
        ))
    }
}

/// Validate a graph file path: must have a `.json` extension.
pub fn validate_graph_path(path: &str) -> Result<PathBuf, SecurityError> {
    let p = PathBuf::from(path);
    if p.extension().and_then(|e| e.to_str()) != Some("json") {
        return Err(SecurityError::InvalidPath(
            "graph file must be .json".into(),
        ));
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_safe_path_within_root() {
        let dir = std::env::temp_dir().join("graphify_security_test_safe");
        let _ = fs::create_dir_all(&dir);
        let file = dir.join("test.json");
        fs::write(&file, "{}").unwrap();

        let result = safe_path(&file, &dir);
        assert!(result.is_ok());

        let _ = fs::remove_file(&file);
        let _ = fs::remove_dir(&dir);
    }

    #[test]
    fn test_safe_path_traversal_blocked() {
        // Try to escape from a subdirectory to its parent
        let dir = std::env::temp_dir().join("graphify_security_test_traversal");
        let sub = dir.join("sub");
        let _ = fs::create_dir_all(&sub);
        // Create a file in the parent dir
        let file = dir.join("secret.txt");
        fs::write(&file, "secret").unwrap();

        // Attempt traversal: sub/../secret.txt should be blocked when root is sub/
        let traversal = sub.join("../secret.txt");
        let result = safe_path(&traversal, &sub);
        assert!(matches!(result, Err(SecurityError::PathTraversal(_))));

        let _ = fs::remove_file(&file);
        let _ = fs::remove_dir(&sub);
        let _ = fs::remove_dir(&dir);
    }

    #[test]
    fn test_safe_path_nonexistent_file() {
        let result = safe_path(Path::new("/nonexistent/path/file.txt"), Path::new("/tmp"));
        assert!(matches!(result, Err(SecurityError::PathTraversal(_))));
    }

    #[test]
    fn test_validate_graph_path_json() {
        let result = validate_graph_path("output/graph.json");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from("output/graph.json"));
    }

    #[test]
    fn test_validate_graph_path_non_json() {
        let result = validate_graph_path("output/graph.xml");
        assert!(matches!(result, Err(SecurityError::InvalidPath(_))));
    }

    #[test]
    fn test_validate_graph_path_no_extension() {
        let result = validate_graph_path("output/graph");
        assert!(matches!(result, Err(SecurityError::InvalidPath(_))));
    }

    #[test]
    fn test_validate_graph_path_dot_json_in_middle() {
        // "foo.json.bak" should fail — extension is "bak"
        let result = validate_graph_path("foo.json.bak");
        assert!(matches!(result, Err(SecurityError::InvalidPath(_))));
    }
}
