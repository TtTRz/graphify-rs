use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use graphify_core::model::{ExtractionResult, GraphEdge, GraphNode};
use serde::{Deserialize, Serialize};

pub const LLM_CACHE_SCHEMA_VERSION: u32 = 1;
pub const LLM_PROMPT_VERSION: &str = "graphify-semantic-cli-v1";

#[derive(Clone, Debug)]
pub struct LlmCliConfig {
    pub provider: String,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCacheMetadata {
    pub schema_version: u32,
    pub provider: String,
    pub command_fingerprint: String,
    pub prompt_version: String,
    pub source_hash: String,
    pub source_path: String,
    pub generated_at_unix_secs: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stale_preserved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCacheEntry {
    pub metadata: LlmCacheMetadata,
    pub extraction: ExtractionResult,
}

#[derive(Debug, Clone)]
pub struct LoadedLlmExtraction {
    pub extraction: ExtractionResult,
    pub stale_preserved: bool,
}

pub fn provider_cache_dir(output_dir: &Path, provider: &str) -> PathBuf {
    output_dir
        .join("llm-cache")
        .join(sanitize_cache_component(provider))
}

pub fn sanitize_cache_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let sanitized: String = sanitized.trim_matches('_').chars().take(64).collect();
    if sanitized.is_empty() {
        "cli".to_string()
    } else {
        sanitized
    }
}

pub fn command_fingerprint(command: &str) -> String {
    stable_hash_hex(command.as_bytes())
}

pub fn source_hash(path: &Path) -> Option<String> {
    graphify_cache::file_hash(path)
}

pub fn current_cache_path(path: &Path, cache_dir: &Path) -> Option<PathBuf> {
    source_hash(path).map(|hash| cache_dir.join(format!("{hash}.json")))
}

pub fn latest_cache_path(path: &Path, root: &Path, cache_dir: &Path) -> PathBuf {
    cache_dir.join("latest").join(stable_path_key(path, root))
}

pub fn all_cache_dirs(output_dir: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let root = output_dir.join("llm-cache");
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            }
        }
    }
    let legacy = output_dir.join("cache");
    if legacy.is_dir() {
        dirs.push(legacy);
    }
    dirs.sort();
    dirs
}

pub fn load_current_entry(
    path: &Path,
    root: &Path,
    cache_dir: &Path,
    cli: &LlmCliConfig,
) -> Option<ExtractionResult> {
    let source_hash = source_hash(path)?;
    let cache_path = cache_dir.join(format!("{source_hash}.json"));
    load_entry_file(&cache_path, path, root).and_then(|loaded| {
        let meta = loaded.metadata?;
        let matches = meta.schema_version == LLM_CACHE_SCHEMA_VERSION
            && meta.provider == cli.provider
            && meta.command_fingerprint == command_fingerprint(&cli.command)
            && meta.prompt_version == LLM_PROMPT_VERSION
            && meta.source_hash == source_hash;
        matches.then_some(loaded.extraction)
    })
}

pub fn load_latest_for_prompt(
    path: &Path,
    root: &Path,
    cache_dir: &Path,
) -> Option<ExtractionResult> {
    load_entry_file(&latest_cache_path(path, root, cache_dir), path, root)
        .map(|entry| entry.extraction)
}

pub fn load_current_or_latest_for_preservation(
    path: &Path,
    root: &Path,
    cache_dir: &Path,
) -> Option<LoadedLlmExtraction> {
    if let Some(current) = current_cache_path(path, cache_dir)
        && let Some(loaded) = load_entry_file(&current, path, root)
    {
        return Some(LoadedLlmExtraction {
            extraction: mark_stale(loaded.extraction, false),
            stale_preserved: false,
        });
    }
    load_entry_file(&latest_cache_path(path, root, cache_dir), path, root).map(|loaded| {
        LoadedLlmExtraction {
            extraction: mark_stale(loaded.extraction, true),
            stale_preserved: true,
        }
    })
}

pub fn load_preserved_extractions(
    doc_files: &[PathBuf],
    root: &Path,
    output_dir: &Path,
    exclude_dirs: &HashSet<PathBuf>,
) -> Vec<LoadedLlmExtraction> {
    let mut results = Vec::new();
    for cache_dir in all_cache_dirs(output_dir) {
        if exclude_dirs.contains(&cache_dir) {
            continue;
        }
        for doc_path in doc_files {
            if let Some(cached) =
                load_current_or_latest_for_preservation(doc_path, root, &cache_dir)
            {
                results.push(cached);
            }
        }
    }
    results
}

pub fn save_entry(
    path: &Path,
    root: &Path,
    cache_dir: &Path,
    cli: &LlmCliConfig,
    extraction: &ExtractionResult,
) -> bool {
    let Some(source_hash) = source_hash(path) else {
        return false;
    };
    let source_path = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();
    let entry = LlmCacheEntry {
        metadata: LlmCacheMetadata {
            schema_version: LLM_CACHE_SCHEMA_VERSION,
            provider: cli.provider.clone(),
            command_fingerprint: command_fingerprint(&cli.command),
            prompt_version: LLM_PROMPT_VERSION.to_string(),
            source_hash: source_hash.clone(),
            source_path,
            generated_at_unix_secs: now_unix_secs(),
            stale_preserved: false,
        },
        extraction: extraction.clone(),
    };
    let current = cache_dir.join(format!("{source_hash}.json"));
    write_entry(&current, &entry) && write_entry(&latest_cache_path(path, root, cache_dir), &entry)
}

pub fn save_legacy_entry(
    path: &Path,
    root: &Path,
    cache_dir: &Path,
    provider: &str,
    extraction: &ExtractionResult,
) -> bool {
    let cli = LlmCliConfig {
        provider: provider.to_string(),
        command: provider.to_string(),
    };
    save_entry(path, root, cache_dir, &cli, extraction)
}

fn load_entry_file(path: &Path, source_path: &Path, root: &Path) -> Option<LoadedEntry> {
    let data = std::fs::read_to_string(path).ok()?;
    if let Ok(entry) = serde_json::from_str::<LlmCacheEntry>(&data) {
        return Some(LoadedEntry {
            metadata: Some(entry.metadata),
            extraction: entry.extraction,
        });
    }
    // Backward compatibility with the pre-metadata MVP and legacy Anthropic cache.
    if let Ok(extraction) = serde_json::from_str::<ExtractionResult>(&data) {
        let source_hash = source_hash(source_path).unwrap_or_default();
        let source_path = source_path
            .strip_prefix(root)
            .unwrap_or(source_path)
            .to_string_lossy()
            .to_string();
        return Some(LoadedEntry {
            metadata: Some(LlmCacheMetadata {
                schema_version: 0,
                provider: "legacy".to_string(),
                command_fingerprint: "legacy".to_string(),
                prompt_version: "legacy".to_string(),
                source_hash,
                source_path,
                generated_at_unix_secs: 0,
                stale_preserved: false,
            }),
            extraction,
        });
    }
    None
}

struct LoadedEntry {
    metadata: Option<LlmCacheMetadata>,
    extraction: ExtractionResult,
}

fn write_entry(path: &Path, entry: &LlmCacheEntry) -> bool {
    if let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return false;
    }
    match serde_json::to_string(entry) {
        Ok(json) => std::fs::write(path, json).is_ok(),
        Err(_) => false,
    }
}

fn mark_stale(mut extraction: ExtractionResult, stale: bool) -> ExtractionResult {
    for node in &mut extraction.nodes {
        set_stale_node(node, stale);
    }
    for edge in &mut extraction.edges {
        set_stale_edge(edge, stale);
    }
    extraction
}

fn set_stale_node(node: &mut GraphNode, stale: bool) {
    node.extra
        .insert("llm_stale_preserved".into(), serde_json::json!(stale));
}

fn set_stale_edge(edge: &mut GraphEdge, stale: bool) {
    edge.extra
        .insert("llm_stale_preserved".into(), serde_json::json!(stale));
}

fn stable_path_key(path: &Path, root: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy();
    format!("{}.json", stable_hash_hex(rel.as_bytes()))
}

fn stable_hash_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphify_core::model::{GraphNode, NodeType};
    use std::collections::HashMap;

    fn extraction(label: &str) -> ExtractionResult {
        ExtractionResult {
            nodes: vec![GraphNode {
                id: label.into(),
                label: label.into(),
                source_file: "doc.md".into(),
                source_location: None,
                node_type: NodeType::Concept,
                community: None,
                extra: HashMap::new(),
            }],
            edges: Vec::new(),
            hyperedges: Vec::new(),
        }
    }

    #[test]
    fn sanitize_provider_is_stable_and_non_empty() {
        assert_eq!(sanitize_cache_component("codex cli/v1"), "codex_cli_v1");
        assert_eq!(sanitize_cache_component("////"), "cli");
    }

    #[test]
    fn current_cache_requires_matching_command_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let doc = root.join("doc.md");
        std::fs::write(&doc, "hello").unwrap();
        let cache = provider_cache_dir(&root.join(".graphify"), "codex");
        let cli = LlmCliConfig {
            provider: "codex".into(),
            command: "codex-a".into(),
        };
        assert!(save_entry(&doc, root, &cache, &cli, &extraction("A")));
        assert!(load_current_entry(&doc, root, &cache, &cli).is_some());
        let other = LlmCliConfig {
            provider: "codex".into(),
            command: "codex-b".into(),
        };
        assert!(load_current_entry(&doc, root, &cache, &other).is_none());
    }

    #[test]
    fn latest_preservation_marks_stale_after_source_change() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let doc = root.join("doc.md");
        std::fs::write(&doc, "v1").unwrap();
        let cache = provider_cache_dir(&root.join(".graphify"), "codex");
        let cli = LlmCliConfig {
            provider: "codex".into(),
            command: "codex".into(),
        };
        assert!(save_entry(&doc, root, &cache, &cli, &extraction("A")));
        std::fs::write(&doc, "v2").unwrap();
        let loaded = load_current_or_latest_for_preservation(&doc, root, &cache).unwrap();
        assert!(loaded.stale_preserved);
        assert_eq!(
            loaded.extraction.nodes[0].extra["llm_stale_preserved"],
            true
        );
    }
}
