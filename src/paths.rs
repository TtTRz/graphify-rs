use std::path::{Path, PathBuf};

fn graphify_home() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".graphify-rs")
}

pub fn resolve_default_output(project_root: &Path) -> PathBuf {
    let canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let abs_str = canonical.to_string_lossy();

    let hash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        abs_str.hash(&mut h);
        format!("{:08x}", h.finish())
    };

    let dir_name = canonical
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    graphify_home().join(format!("{}-{}", dir_name, hash))
}
