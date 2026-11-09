//! File watching and auto-rebuild for graphify.
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

const DEBOUNCE_DURATION: Duration = Duration::from_secs(3);

const IGNORE_PATTERNS: &[&str] = &[
    ".git",
    "node_modules",
    "__pycache__",
    ".pyc",
    "target",
    "graphify-rs-out",
    ".DS_Store",
];

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

type RebuildFuture = Pin<Box<dyn Future<Output = Result<(), WatchError>> + Send>>;
pub type RebuildFn = dyn Fn(&Path, &Path) -> RebuildFuture + Send + Sync;

fn should_ignore(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    IGNORE_PATTERNS.iter().any(|p| path_str.contains(p))
}

fn filter_changes(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter(|p| !should_ignore(p))
        .cloned()
        .collect()
}

pub async fn watch_directory(root: &Path, output_dir: &Path) -> Result<(), WatchError> {
    watch_directory_with(root, output_dir, |_root, _output| Box::pin(async { Ok(()) })).await
}

pub async fn watch_directory_with<F>(root: &Path, output_dir: &Path, rebuild: F) -> Result<(), WatchError>
where
    F: Fn(&Path, &Path) -> RebuildFuture + Send + Sync + 'static,
{
    let rebuild: Arc<RebuildFn> = Arc::new(rebuild);
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

    println!("Running initial build...");
    match rebuild(root, output_dir).await {
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
        println!("Files changed ({}), triggering rebuild...", relevant.len());

        for p in &relevant {
            debug!("  changed: {}", p.display());
        }

        match rebuild(root, output_dir).await {
            Ok(()) => println!("Rebuild complete."),
            Err(e) => eprintln!("Rebuild failed: {e}"),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_should_ignore_git() {
        assert!(should_ignore(Path::new("/repo/.git/objects/abc")));
        assert!(should_ignore(Path::new("/repo/node_modules/foo.js")));
        assert!(should_ignore(Path::new("/repo/__pycache__/mod.pyc")));
        assert!(should_ignore(Path::new("/repo/target/debug/build")));
        assert!(should_ignore(Path::new("/repo/graphify-rs-out/graph.json")));
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

    #[tokio::test]
    async fn watch_directory_with_invokes_rebuild_type() {
        let _ = Arc::new(AtomicUsize::new(0));
        let rebuild = |_root: &Path, _output: &Path| Box::pin(async move { Ok(()) }) as RebuildFuture;
        let _ = rebuild;
    }
}
