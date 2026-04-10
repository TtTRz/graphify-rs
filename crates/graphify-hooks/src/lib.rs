//! Git hook integration for graphify.
//!
//! Installs/uninstalls post-commit and post-checkout hooks that trigger
//! incremental graph rebuilds. Port of Python `hooks.py`.

use std::fs;
use std::path::Path;

use thiserror::Error;

/// Marker delimiters used to identify the graphify hook block.
const HOOK_MARKER_START: &str = "# graphify-hook-start";
const HOOK_MARKER_END: &str = "# graphify-hook-end";

/// The hook script block injected into git hooks.
const HOOK_SCRIPT: &str = r#"
# graphify-hook-start
# Auto-run graphify AST extraction on commit (code-only, no LLM)
if command -v graphify >/dev/null 2>&1; then
  graphify build --code-only --output graphify-out &
fi
# graphify-hook-end
"#;

/// Hook names that graphify manages.
const MANAGED_HOOKS: &[&str] = &["post-commit", "post-checkout"];

/// Errors from hook management.
#[derive(Debug, Error)]
pub enum HookError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("not a git repository (missing .git/hooks): {0}")]
    NotGitRepo(String),
}

/// Install graphify git hooks in the repository at `repo_root`.
///
/// Installs post-commit and post-checkout hooks. If the hook files already
/// exist, the graphify block is appended (or replaced if already present).
pub fn install_hooks(repo_root: &Path) -> Result<String, HookError> {
    let hooks_dir = repo_root.join(".git/hooks");
    if !hooks_dir.exists() {
        return Err(HookError::NotGitRepo(repo_root.display().to_string()));
    }

    for hook_name in MANAGED_HOOKS {
        install_single_hook(&hooks_dir, hook_name)?;
    }

    Ok("Git hooks installed (post-commit, post-checkout)".to_string())
}

/// Install a single hook file, preserving any existing content.
fn install_single_hook(hooks_dir: &Path, name: &str) -> Result<(), HookError> {
    let hook_path = hooks_dir.join(name);

    let mut content = if hook_path.exists() {
        fs::read_to_string(&hook_path)?
    } else {
        "#!/bin/sh\n".to_string()
    };

    // Remove old marker block if present
    content = strip_marker_block(&content);

    // Append the new hook script
    content.push_str(HOOK_SCRIPT);

    fs::write(&hook_path, &content)?;

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755))?;
    }

    Ok(())
}

/// Uninstall graphify git hooks from the repository at `repo_root`.
///
/// Removes the graphify marker block from each managed hook file. If the
/// resulting file contains only the shebang line (or is empty), the hook
/// file is deleted.
pub fn uninstall_hooks(repo_root: &Path) -> Result<String, HookError> {
    let hooks_dir = repo_root.join(".git/hooks");
    if !hooks_dir.exists() {
        return Err(HookError::NotGitRepo(repo_root.display().to_string()));
    }

    for hook_name in MANAGED_HOOKS {
        uninstall_single_hook(&hooks_dir, hook_name)?;
    }

    Ok("Git hooks removed (post-commit, post-checkout)".to_string())
}

/// Remove the graphify block from a single hook file.
fn uninstall_single_hook(hooks_dir: &Path, name: &str) -> Result<(), HookError> {
    let hook_path = hooks_dir.join(name);
    if !hook_path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&hook_path)?;
    let cleaned = strip_marker_block(&content);
    let trimmed = cleaned.trim();

    // If only shebang remains (or empty), remove the file
    if trimmed.is_empty() || trimmed == "#!/bin/sh" || trimmed == "#!/bin/bash" {
        fs::remove_file(&hook_path)?;
    } else {
        fs::write(&hook_path, &cleaned)?;
    }

    Ok(())
}

/// Check whether graphify hooks are installed in the repository at `repo_root`.
///
/// Returns a human-readable status string.
pub fn hook_status(repo_root: &Path) -> Result<String, HookError> {
    let hooks_dir = repo_root.join(".git/hooks");
    if !hooks_dir.exists() {
        return Err(HookError::NotGitRepo(repo_root.display().to_string()));
    }

    let mut installed = Vec::new();
    let mut missing = Vec::new();

    for hook_name in MANAGED_HOOKS {
        let hook_path = hooks_dir.join(hook_name);
        if hook_path.exists() {
            let content = fs::read_to_string(&hook_path)?;
            if content.contains(HOOK_MARKER_START) {
                installed.push(*hook_name);
            } else {
                missing.push(*hook_name);
            }
        } else {
            missing.push(*hook_name);
        }
    }

    if missing.is_empty() {
        Ok(format!("All hooks installed: {}", installed.join(", ")))
    } else if installed.is_empty() {
        Ok("No graphify hooks installed".to_string())
    } else {
        Ok(format!(
            "Installed: {}; Missing: {}",
            installed.join(", "),
            missing.join(", ")
        ))
    }
}

/// Strip the graphify marker block from hook content.
///
/// Removes everything between (and including) the start and end markers,
/// plus any surrounding blank lines.
fn strip_marker_block(content: &str) -> String {
    if let Some(start_idx) = content.find(HOOK_MARKER_START) {
        if let Some(end_marker_start) = content[start_idx..].find(HOOK_MARKER_END) {
            let end_idx = start_idx + end_marker_start + HOOK_MARKER_END.len();
            // Also consume the trailing newline if present
            let end_idx = if content[end_idx..].starts_with('\n') {
                end_idx + 1
            } else {
                end_idx
            };
            // Strip leading newline before marker if present
            let start_idx = if start_idx > 0 && content.as_bytes()[start_idx - 1] == b'\n' {
                start_idx - 1
            } else {
                start_idx
            };
            let mut result = String::with_capacity(content.len());
            result.push_str(&content[..start_idx]);
            result.push_str(&content[end_idx..]);
            result
        } else {
            // Malformed: start without end, remove from start to end of file
            content[..start_idx].to_string()
        }
    } else {
        content.to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_fake_repo(dir: &Path) {
        let hooks_dir = dir.join(".git/hooks");
        fs::create_dir_all(&hooks_dir).unwrap();
    }

    #[test]
    fn test_strip_marker_block_empty() {
        assert_eq!(strip_marker_block("no markers here"), "no markers here");
    }

    #[test]
    fn test_strip_marker_block() {
        let input = "#!/bin/sh\n# graphify-hook-start\nsome stuff\n# graphify-hook-end\nother";
        let result = strip_marker_block(input);
        assert_eq!(result, "#!/bin/shother");

        // With trailing newline after end marker
        let input2 = "#!/bin/sh\n\n# graphify-hook-start\nsome stuff\n# graphify-hook-end\nother";
        let result2 = strip_marker_block(input2);
        assert_eq!(result2, "#!/bin/sh\nother");
    }

    #[test]
    fn test_strip_marker_block_no_end() {
        let input = "#!/bin/sh\n# graphify-hook-start\norphan";
        let result = strip_marker_block(input);
        assert_eq!(result, "#!/bin/sh\n");
    }

    #[test]
    fn test_install_not_git_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let result = install_hooks(tmp.path());
        assert!(matches!(result, Err(HookError::NotGitRepo(_))));
    }

    #[test]
    fn test_install_and_status() {
        let tmp = tempfile::tempdir().unwrap();
        setup_fake_repo(tmp.path());

        let msg = install_hooks(tmp.path()).unwrap();
        assert!(msg.contains("installed"));

        // Verify files exist
        let post_commit = tmp.path().join(".git/hooks/post-commit");
        assert!(post_commit.exists());
        let content = fs::read_to_string(&post_commit).unwrap();
        assert!(content.contains(HOOK_MARKER_START));
        assert!(content.contains(HOOK_MARKER_END));
        assert!(content.starts_with("#!/bin/sh"));

        // Status should report all installed
        let status = hook_status(tmp.path()).unwrap();
        assert!(status.contains("All hooks installed"));
    }

    #[test]
    fn test_install_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        setup_fake_repo(tmp.path());

        install_hooks(tmp.path()).unwrap();
        install_hooks(tmp.path()).unwrap();

        let content = fs::read_to_string(tmp.path().join(".git/hooks/post-commit")).unwrap();
        // Should only contain one copy of the marker
        let count = content.matches(HOOK_MARKER_START).count();
        assert_eq!(count, 1, "Hook block should not be duplicated");
    }

    #[test]
    fn test_install_preserves_existing() {
        let tmp = tempfile::tempdir().unwrap();
        setup_fake_repo(tmp.path());

        // Write an existing hook
        let hook_path = tmp.path().join(".git/hooks/post-commit");
        fs::write(&hook_path, "#!/bin/sh\necho 'existing'\n").unwrap();

        install_hooks(tmp.path()).unwrap();

        let content = fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains("echo 'existing'"));
        assert!(content.contains(HOOK_MARKER_START));
    }

    #[test]
    fn test_uninstall() {
        let tmp = tempfile::tempdir().unwrap();
        setup_fake_repo(tmp.path());

        install_hooks(tmp.path()).unwrap();
        let msg = uninstall_hooks(tmp.path()).unwrap();
        assert!(msg.contains("removed"));

        // Hook files with only shebang should be deleted
        let post_commit = tmp.path().join(".git/hooks/post-commit");
        assert!(!post_commit.exists());

        // Status should report none installed
        let status = hook_status(tmp.path()).unwrap();
        assert!(status.contains("No graphify hooks installed"));
    }

    #[test]
    fn test_uninstall_preserves_other_content() {
        let tmp = tempfile::tempdir().unwrap();
        setup_fake_repo(tmp.path());

        let hook_path = tmp.path().join(".git/hooks/post-commit");
        fs::write(&hook_path, "#!/bin/sh\necho 'keep me'\n").unwrap();

        install_hooks(tmp.path()).unwrap();
        uninstall_hooks(tmp.path()).unwrap();

        // File should still exist with the original content
        assert!(hook_path.exists());
        let content = fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains("echo 'keep me'"));
        assert!(!content.contains(HOOK_MARKER_START));
    }

    #[test]
    fn test_hook_status_not_git_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let result = hook_status(tmp.path());
        assert!(matches!(result, Err(HookError::NotGitRepo(_))));
    }
}
