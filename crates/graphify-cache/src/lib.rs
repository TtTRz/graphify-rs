//! SHA256-based semantic caching for graphify.
//!
//! Caches extraction results keyed by content hash so unchanged files are not
//! re-processed.

use std::fs;
use std::path::Path;

use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tracing::debug;

/// Default cache directory relative to the working directory.
const CACHE_DIR: &str = ".graphify/cache";

/// Errors from the cache layer.
#[derive(Debug, Error)]
pub enum CacheError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Compute the SHA256 hex digest of a file's content.
///
/// Returns `None` if the file cannot be read.
pub fn file_hash(path: &Path) -> Option<String> {
    let content = fs::read(path).ok()?;
    let hash = Sha256::digest(&content);
    Some(format!("{:x}", hash))
}

/// Build a cache filename from a file path relative to `root`.
///
/// The key is `{sha256}.json` where the hash is computed over the file content,
/// so any change in content naturally invalidates the cache entry.
fn cache_key(path: &Path, _root: &Path) -> String {
    let hash = file_hash(path).unwrap_or_default();
    format!("{hash}.json")
}

/// Load a cached extraction result for `path`, returning `None` on cache miss.
///
/// A cache miss occurs when:
/// - The source file cannot be read (hash fails).
/// - No cache entry exists for the current content hash.
/// - The cached JSON cannot be deserialized into `T`.
pub fn load_cached<T: DeserializeOwned>(path: &Path, root: &Path) -> Option<T> {
    load_cached_from(path, root, Path::new(CACHE_DIR))
}

/// Like [`load_cached`] but with an explicit cache directory.
pub fn load_cached_from<T: DeserializeOwned>(
    path: &Path,
    root: &Path,
    cache_dir: &Path,
) -> Option<T> {
    let key = cache_key(path, root);
    let cache_path = cache_dir.join(&key);
    if !cache_path.exists() {
        debug!(?cache_path, "cache miss");
        return None;
    }
    let data = fs::read_to_string(&cache_path).ok()?;
    serde_json::from_str(&data).ok()
}

/// Save an extraction result to cache.
///
/// Returns `true` on success, `false` on any I/O or serialization failure.
pub fn save_cached<T: Serialize>(path: &Path, result: &T, root: &Path) -> bool {
    save_cached_to(path, result, root, Path::new(CACHE_DIR))
}

/// Like [`save_cached`] but with an explicit cache directory.
pub fn save_cached_to<T: Serialize>(
    path: &Path,
    result: &T,
    root: &Path,
    cache_dir: &Path,
) -> bool {
    let key = cache_key(path, root);
    let cache_path = cache_dir.join(&key);

    // Ensure the cache directory exists.
    if let Some(parent) = cache_path.parent()
        && fs::create_dir_all(parent).is_err()
    {
        return false;
    }

    // Atomic write: serialise → write to .tmp → rename into place.
    let tmp = cache_path.with_extension("tmp");
    match serde_json::to_string(result) {
        Ok(json) => {
            if fs::write(&tmp, &json).is_ok() {
                debug!(?cache_path, "cache write");
                fs::rename(&tmp, &cache_path).is_ok()
            } else {
                false
            }
        }
        Err(_) => false,
    }
}

/// Remove all cached files from the default cache directory.
pub fn clear_cache() -> std::io::Result<()> {
    clear_cache_dir(Path::new(CACHE_DIR))
}

/// Remove all cached files from the given cache directory.
pub fn clear_cache_dir(cache_dir: &Path) -> std::io::Result<()> {
    if cache_dir.exists() {
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

/// Invalidate the cache entry for a specific file.
///
/// Since caching is content-hash based, changing the file already causes a
/// cache miss on the next read. This function pre-deletes entries matching
/// the *current* content hash so stale data is cleaned up eagerly. It is a
/// no-op when the file can't be read (already deleted, etc.).
pub fn invalidate_cached(path: &Path, root: &Path, cache_dir: &Path) -> bool {
    let key = cache_key(path, root);
    let cache_path = cache_dir.join(&key);
    if cache_path.exists() {
        debug!(?cache_path, "cache invalidate");
        fs::remove_file(&cache_path).is_ok()
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::fs;
    use tempfile::TempDir;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct DummyResult {
        entities: Vec<String>,
        score: f64,
    }

    fn make_dummy() -> DummyResult {
        DummyResult {
            entities: vec!["Alice".into(), "Bob".into()],
            score: 0.95,
        }
    }

    #[test]
    fn test_file_hash_consistent() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("hello.txt");
        fs::write(&file, "hello world").unwrap();

        let h1 = file_hash(&file).unwrap();
        let h2 = file_hash(&file).unwrap();
        assert_eq!(h1, h2, "hash must be deterministic");

        // SHA256 of "hello world" is well-known.
        assert_eq!(
            h1,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_file_hash_nonexistent() {
        assert!(file_hash(Path::new("/no/such/file")).is_none());
    }

    #[test]
    fn test_save_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let cache_dir = dir.path().join("cache");
        let root = dir.path();

        // Create a source file.
        let src = dir.path().join("src.rs");
        fs::write(&src, "fn main() {}").unwrap();

        let value = make_dummy();
        assert!(save_cached_to(&src, &value, root, &cache_dir));

        let loaded: Option<DummyResult> = load_cached_from(&src, root, &cache_dir);
        assert_eq!(loaded, Some(value));
    }

    #[test]
    fn test_cache_miss_returns_none() {
        let dir = TempDir::new().unwrap();
        let cache_dir = dir.path().join("cache");
        let root = dir.path();

        let src = dir.path().join("not_cached.rs");
        fs::write(&src, "let x = 1;").unwrap();

        let loaded: Option<DummyResult> = load_cached_from(&src, root, &cache_dir);
        assert!(loaded.is_none());
    }

    #[test]
    fn test_content_change_invalidates_cache() {
        let dir = TempDir::new().unwrap();
        let cache_dir = dir.path().join("cache");
        let root = dir.path();

        let src = dir.path().join("mutable.rs");
        fs::write(&src, "version 1").unwrap();

        let value = make_dummy();
        assert!(save_cached_to(&src, &value, root, &cache_dir));

        // Mutate the source file — hash changes, old cache entry is stale.
        fs::write(&src, "version 2").unwrap();

        let loaded: Option<DummyResult> = load_cached_from(&src, root, &cache_dir);
        assert!(loaded.is_none(), "modified file must not match old cache");
    }

    #[test]
    fn test_clear_cache_removes_files() {
        let dir = TempDir::new().unwrap();
        let cache_dir = dir.path().join("cache");
        let root = dir.path();

        let src = dir.path().join("f.txt");
        fs::write(&src, "data").unwrap();

        assert!(save_cached_to(&src, &make_dummy(), root, &cache_dir));
        assert!(cache_dir.exists());

        clear_cache_dir(&cache_dir).unwrap();
        assert!(!cache_dir.exists());
    }
}
