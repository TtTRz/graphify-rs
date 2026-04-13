//! File watching and auto-rebuild for graphify.
//!
//! Uses `notify` + debouncing to watch for file changes and trigger
//! incremental graph rebuilds. Port of Python `watch.py`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Debounce duration before triggering a rebuild.
const DEBOUNCE_DURATION: Duration = Duration::from_secs(3);

/// Default ignore patterns for files that should not trigger rebuilds.
const IGNORE_PATTERNS: &[&str] = &[
    ".git",
    "node_modules",
    "__pycache__",
    ".pyc",
    "target",
    "graphify-out",
    ".DS_Store",
];

/// Errors from the watcher.
#[derive(Debug, Error)]
pub enum WatchError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("notify error: {0}")]
    Notify(#[from] notify::Error),

    #[error("watch setup failed: {0}")]
    Setup(String),

    #[error("rebuild failed: {0}")]
    Rebuild(String),
}

/// Check if a path should be ignored based on common patterns.
fn should_ignore(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    IGNORE_PATTERNS.iter().any(|p| path_str.contains(p))
}

/// Filter changed paths to only include relevant source files.
fn filter_changes(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter(|p| !should_ignore(p))
        .cloned()
        .collect()
}

/// Run the full pipeline: detect -> extract -> build -> cluster -> analyze -> export.
///
/// When `changed_files` is provided, only those files have their cache invalidated
/// before extraction, achieving an incremental rebuild without re-parsing unchanged files.
fn rebuild(
    root: &Path,
    output_dir: &Path,
    changed_files: Option<&[PathBuf]>,
) -> Result<(), WatchError> {
    let cache_dir = output_dir.join("cache");

    // ── Step 0: Invalidate cache for changed files ──
    if let Some(changed) = changed_files {
        for path in changed {
            let _ = graphify_cache::invalidate_cached(path, root, &cache_dir);
        }
        info!(
            "rebuild: invalidated cache for {} changed file(s)",
            changed.len()
        );
    }

    // ── Step 1: Detect files ──
    info!("rebuild: detecting files...");
    let detection = graphify_detect::detect(root);
    info!(
        "rebuild: found {} files (~{} words)",
        detection.total_files, detection.total_words
    );

    // ── Step 2: Extract AST ──
    let code_files: Vec<PathBuf> = detection
        .files
        .get(&graphify_detect::FileType::Code)
        .map(|v| v.iter().map(|f| root.join(f)).collect())
        .unwrap_or_default();

    if code_files.is_empty() {
        info!("rebuild: no code files found, skipping");
        return Ok(());
    }

    info!(
        "rebuild: extracting AST from {} code files...",
        code_files.len()
    );
    let mut ast_result = graphify_core::model::ExtractionResult::default();
    let mut cache_hits = 0usize;
    let mut errors = 0usize;
    for file_path in &code_files {
        if let Some(cached) = graphify_cache::load_cached_from::<
            graphify_core::model::ExtractionResult,
        >(file_path, root, &cache_dir)
        {
            cache_hits += 1;
            ast_result.nodes.extend(cached.nodes);
            ast_result.edges.extend(cached.edges);
            ast_result.hyperedges.extend(cached.hyperedges);
            continue;
        }
        // Extract fresh, catching panics
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            graphify_extract::extract(std::slice::from_ref(file_path))
        })) {
            Ok(fresh) => {
                let _ = graphify_cache::save_cached_to(file_path, &fresh, root, &cache_dir);
                ast_result.nodes.extend(fresh.nodes);
                ast_result.edges.extend(fresh.edges);
                ast_result.hyperedges.extend(fresh.hyperedges);
            }
            Err(_) => {
                errors += 1;
                warn!("rebuild: extraction panicked for {}", file_path.display());
            }
        }
    }
    if cache_hits > 0 {
        info!(
            "rebuild: cache {} hits, {} extracted fresh",
            cache_hits,
            code_files.len() - cache_hits
        );
    }
    if errors > 0 {
        warn!("rebuild: {} file(s) had extraction errors", errors);
    }
    info!(
        "rebuild: Pass 1 (AST): {} nodes, {} edges",
        ast_result.nodes.len(),
        ast_result.edges.len()
    );

    let extractions = vec![ast_result];

    // ── Step 3: Build graph ──
    info!("rebuild: building graph...");
    let graph = graphify_build::build(&extractions)
        .map_err(|e| WatchError::Rebuild(format!("build failed: {e}")))?;
    info!(
        "rebuild: graph has {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );

    // ── Step 4: Cluster ──
    info!("rebuild: detecting communities...");
    let communities = graphify_cluster::cluster(&graph);
    let cohesion = graphify_cluster::score_all(&graph, &communities);

    let community_labels: HashMap<usize, String> = communities
        .iter()
        .map(|(cid, nodes)| {
            let label = nodes
                .first()
                .and_then(|id| graph.get_node(id))
                .map(|n| n.label.clone())
                .unwrap_or_else(|| format!("Community {}", cid));
            (*cid, label)
        })
        .collect();
    info!("rebuild: {} communities detected", communities.len());

    // ── Step 5: Analyze ──
    info!("rebuild: analyzing...");
    let god_list = graphify_analyze::god_nodes(&graph, 10);
    let surprise_list = graphify_analyze::surprising_connections(&graph, &communities, 5);
    let questions = graphify_analyze::suggest_questions(&graph, &communities, &community_labels, 7);

    // ── Step 6: Export all formats ──
    std::fs::create_dir_all(output_dir)
        .map_err(|e| WatchError::Rebuild(format!("create output dir: {e}")))?;

    let _ = graphify_export::export_json(&graph, output_dir);
    let _ = graphify_export::export_html(&graph, &communities, &community_labels, output_dir, None);
    let _ = graphify_export::export_graphml(&graph, output_dir);
    let _ = graphify_export::export_cypher(&graph, output_dir);
    let _ = graphify_export::export_svg(&graph, &communities, output_dir);
    let _ = graphify_export::export_wiki(&graph, &communities, &community_labels, output_dir);

    // Report
    let detection_json = serde_json::json!({
        "total_files": detection.total_files,
        "total_words": detection.total_words,
        "warning": detection.warning,
    });
    let god_json: Vec<serde_json::Value> = god_list
        .iter()
        .map(|g| serde_json::json!({"label": g.label, "edges": g.degree}))
        .collect();
    let surprise_json: Vec<serde_json::Value> = surprise_list
        .iter()
        .map(|s| serde_json::to_value(s).unwrap_or_default())
        .collect();
    let question_json: Vec<serde_json::Value> = questions
        .iter()
        .map(|q| serde_json::to_value(q).unwrap_or_default())
        .collect();
    let token_cost: HashMap<String, usize> =
        HashMap::from([("input".to_string(), 0), ("output".to_string(), 0)]);

    let root_str = root.to_string_lossy();
    let report = graphify_export::generate_report(
        &graph,
        &communities,
        &cohesion,
        &community_labels,
        &god_json,
        &surprise_json,
        &detection_json,
        &token_cost,
        &root_str,
        Some(&question_json),
    );
    let report_path = output_dir.join("GRAPH_REPORT.md");
    let _ = std::fs::write(&report_path, &report);

    // Save manifest
    let manifest_path = output_dir.join(".graphify_manifest.json");
    let manifest = graphify_detect::Manifest {
        files: detection
            .files
            .iter()
            .flat_map(|(ft, paths)| paths.iter().map(move |p| (p.clone(), *ft)))
            .collect(),
    };
    let _ = graphify_detect::save_manifest(&manifest_path, &manifest);

    info!("rebuild: done");
    Ok(())
}

/// Watch `root` for file changes and trigger rebuilds into `output_dir`.
///
/// This is an async loop that runs until cancelled. On each batch of
/// debounced file changes, it logs the changed paths and invokes an
/// incremental rebuild (only changed files have their cache invalidated).
///
/// # Arguments
/// * `root` - Directory to watch recursively.
/// * `output_dir` - Where to write rebuild output.
pub async fn watch_directory(root: &Path, output_dir: &Path) -> Result<(), WatchError> {
    let (tx, mut rx) = mpsc::channel::<Vec<PathBuf>>(100);

    let mut debouncer = new_debouncer(
        DEBOUNCE_DURATION,
        move |res: Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>| match res {
            Ok(events) => {
                let paths: Vec<PathBuf> = events.into_iter().map(|e| e.path).collect();
                if let Err(e) = tx.blocking_send(paths) {
                    warn!("Failed to send watch events: {}", e);
                }
            }
            Err(e) => {
                warn!("Watch error: {}", e);
            }
        },
    )
    .map_err(|e| WatchError::Setup(e.to_string()))?;

    debouncer.watcher().watch(root, RecursiveMode::Recursive)?;

    info!(
        "Watching {} for changes (output: {})",
        root.display(),
        output_dir.display()
    );
    println!("Watching {} for changes...", root.display());

    // Run initial build (full)
    println!("Running initial build...");
    match rebuild(root, output_dir, None) {
        Ok(()) => println!("Initial build complete."),
        Err(e) => eprintln!("Initial build failed: {e}"),
    }

    while let Some(changed_paths) = rx.recv().await {
        let relevant = filter_changes(&changed_paths);

        if relevant.is_empty() {
            debug!("Ignoring changes in excluded paths");
            continue;
        }

        info!("{} file(s) changed, triggering rebuild...", relevant.len());
        println!(
            "Files changed ({}), triggering incremental rebuild...",
            relevant.len()
        );

        for p in &relevant {
            debug!("  changed: {}", p.display());
        }

        match rebuild(root, output_dir, Some(&relevant)) {
            Ok(()) => {
                println!("Rebuild complete.");
            }
            Err(e) => {
                eprintln!("Rebuild failed: {e}");
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_should_ignore_git() {
        assert!(should_ignore(Path::new("/repo/.git/objects/abc")));
        assert!(should_ignore(Path::new("/repo/node_modules/foo.js")));
        assert!(should_ignore(Path::new("/repo/__pycache__/mod.pyc")));
        assert!(should_ignore(Path::new("/repo/target/debug/build")));
        assert!(should_ignore(Path::new("/repo/graphify-out/graph.json")));
    }

    #[test]
    fn test_should_not_ignore_source() {
        assert!(!should_ignore(Path::new("/repo/src/main.rs")));
        assert!(!should_ignore(Path::new("/repo/lib/utils.py")));
        assert!(!should_ignore(Path::new("/repo/README.md")));
    }

    #[test]
    fn test_filter_changes() {
        let paths = vec![
            PathBuf::from("/repo/src/main.rs"),
            PathBuf::from("/repo/.git/HEAD"),
            PathBuf::from("/repo/src/lib.rs"),
            PathBuf::from("/repo/node_modules/foo/index.js"),
        ];
        let filtered = filter_changes(&paths);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.contains(&PathBuf::from("/repo/src/main.rs")));
        assert!(filtered.contains(&PathBuf::from("/repo/src/lib.rs")));
    }

    #[test]
    fn test_filter_changes_all_ignored() {
        let paths = vec![
            PathBuf::from("/repo/.git/HEAD"),
            PathBuf::from("/repo/.DS_Store"),
        ];
        let filtered = filter_changes(&paths);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_changes_empty() {
        let filtered = filter_changes(&[]);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_rebuild_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        // Should succeed with empty directory (no code files)
        let result = rebuild(dir.path(), output.path(), None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_rebuild_with_code_files() {
        let dir = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("main.rs"),
            "fn main() { hello(); }\nfn hello() { println!(\"hi\"); }\n",
        )
        .unwrap();
        std::fs::write(
            src.join("lib.rs"),
            "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
        )
        .unwrap();

        let result = rebuild(dir.path(), output.path(), None);
        assert!(result.is_ok());

        // Check that output files were created
        assert!(output.path().join("graph.json").exists());
        assert!(output.path().join("graph.html").exists());
        assert!(output.path().join("GRAPH_REPORT.md").exists());
    }

    #[test]
    fn test_incremental_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("main.rs"),
            "fn main() { hello(); }\nfn hello() { println!(\"hi\"); }\n",
        )
        .unwrap();

        // Initial full build
        let result = rebuild(dir.path(), output.path(), None);
        assert!(result.is_ok());

        // Incremental rebuild with changed files
        let changed = vec![src.join("main.rs")];
        let result = rebuild(dir.path(), output.path(), Some(&changed));
        assert!(result.is_ok());
    }
}
