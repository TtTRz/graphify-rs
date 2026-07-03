//! Lightweight file change tracking using mtime and size metadata.
//!
//! Stored as gzip-compressed JSON at `<output_dir>/changeindex.db` alongside
//! `graph.json`. On the first run the file is created. On subsequent runs it
//! pre-filters the file list so only new, modified, or deleted files reach the
//! extraction pipeline — no file content reads required.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::time::UNIX_EPOCH;

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use tracing::debug;

pub const CHANGEINDEX_NAME: &str = "changeindex.db";

/// Metadata snapshot for a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeEntry {
    /// Seconds since Unix epoch at last modification.
    pub mtime: u64,
    /// File size in bytes.
    pub size: u64,
}

/// Full metadata snapshot keyed by relative path.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChangeIndex {
    pub files: HashMap<String, ChangeEntry>,
}

/// Load the changeindex from a gzip-compressed JSON file.
/// Returns `None` if the file is absent or unreadable.
pub fn load_changeindex(path: &Path) -> Option<ChangeIndex> {
    let compressed = fs::read(path).ok()?;
    let mut decoder = GzDecoder::new(compressed.as_slice());
    let mut json = String::new();
    decoder.read_to_string(&mut json).ok()?;
    serde_json::from_str(&json).ok()
}

/// Persist the changeindex as gzip-compressed JSON. Uses an atomic temp-file
/// rename to avoid corruption if the process is killed mid-write.
pub fn save_changeindex(path: &Path, index: &ChangeIndex) -> std::io::Result<()> {
    let json = serde_json::to_vec(index).map_err(std::io::Error::other)?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&json)?;
    let compressed = encoder.finish()?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, compressed)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Stat a single file and return its mtime+size entry.
/// Returns `None` if the file can't be stat'd.
pub fn file_entry(path: &Path) -> Option<ChangeEntry> {
    let meta = fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(ChangeEntry {
        mtime,
        size: meta.len(),
    })
}

/// Build a fresh `ChangeIndex` from a slice of relative paths, resolving each
/// against `root` for the stat call.  Paths that can't be stat'd are omitted.
pub fn build_from_relative(root: &Path, rel_paths: &[String]) -> ChangeIndex {
    let mut files = HashMap::with_capacity(rel_paths.len());
    for rel in rel_paths {
        if let Some(entry) = file_entry(&root.join(rel)) {
            files.insert(rel.clone(), entry);
        }
    }
    ChangeIndex { files }
}

/// Compare the current file list against a stored changeindex.
///
/// Returns `(to_scan, deleted)` where:
/// - `to_scan`  — relative paths that are new or whose mtime/size changed
/// - `deleted`  — relative paths that were in the old index but are no longer
///               present in the current (filtered) file list
pub fn diff(root: &Path, current: &[String], old: &ChangeIndex) -> (Vec<String>, Vec<String>) {
    let current_set: std::collections::HashSet<&str> =
        current.iter().map(String::as_str).collect();

    let mut to_scan: Vec<String> = Vec::new();
    let mut deleted: Vec<String> = Vec::new();

    for rel in current {
        match old.files.get(rel.as_str()) {
            None => {
                debug!("changeindex: new file {rel}");
                to_scan.push(rel.clone());
            }
            Some(old_entry) => {
                let needs_scan = file_entry(&root.join(rel)).map_or(true, |cur| {
                    cur.mtime != old_entry.mtime || cur.size != old_entry.size
                });
                if needs_scan {
                    debug!("changeindex: changed file {rel}");
                    to_scan.push(rel.clone());
                }
            }
        }
    }

    for rel in old.files.keys() {
        if !current_set.contains(rel.as_str()) {
            debug!("changeindex: deleted file {rel}");
            deleted.push(rel.clone());
        }
    }

    (to_scan, deleted)
}
