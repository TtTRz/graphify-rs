use std::path::{Path, PathBuf};

pub fn resolve_default_output(project_root: &Path) -> PathBuf {
    let canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    canonical.join("graphify-rs-out")
}
