use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

pub const CHANGEINDEX_NAME: &str = "changeindex.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeEntry {
    pub mtime: u64,
    pub size: u64,
    /// Cached word count (0 for images/binary files).
    pub words: u64,
    /// Content hash; empty string on old entries without hash support.
    #[serde(default)]
    pub hash: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChangeIndex {
    pub files: HashMap<String, ChangeEntry>,
}

pub fn load(path: &Path) -> Option<ChangeIndex> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn save(path: &Path, index: &ChangeIndex) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec(index).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&json)?;
        f.flush()?;
    } // drop/close before rename — required on Windows
    if fs::rename(&tmp, path).is_err() {
        // Windows: rename fails when destination exists; remove then retry.
        let _ = fs::remove_file(path);
        fs::rename(&tmp, path)?;
    }
    Ok(())
}

/// Returns `(mtime_nanos, size_bytes)` for a file without reading its content.
///
/// Nanosecond resolution prevents missed changes from rapid saves within the
/// same second (common in editor save-on-change loops).
pub fn file_meta(path: &Path) -> Option<(u64, u64)> {
    let meta = fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos() as u64;
    Some((mtime, meta.len()))
}
