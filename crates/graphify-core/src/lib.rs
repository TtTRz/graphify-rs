pub mod confidence;
pub mod error;
pub mod graph;
pub mod id;
pub mod model;

use std::collections::HashMap;

/// Build a reverse lookup from node ID to community ID.
pub fn build_node_to_community(
    communities: &HashMap<usize, Vec<String>>,
) -> HashMap<&str, usize> {
    let mut map = HashMap::new();
    for (&cid, members) in communities {
        for nid in members {
            map.insert(nid.as_str(), cid);
        }
    }
    map
}

/// Maximum bytes for a single filename component (excluding extension).
/// macOS HFS+/APFS limit is 255 bytes per component; we reserve 15 for extension + safety.
pub const MAX_FILENAME_BYTES: usize = 240;

/// Truncate a string to at most `max_bytes` bytes while preserving UTF-8 validity.
pub fn truncate_to_bytes(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}
