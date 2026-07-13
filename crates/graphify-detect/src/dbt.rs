//! dbt project detection.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::debug;
use walkdir::WalkDir;

use crate::ignore::IgnoreSet;
use crate::load_graphifyignore;

/// Internal YAML structure used to deserialize `dbt_project.yml`.
#[derive(Debug, Deserialize)]
struct DbtProjectYaml {
    name: String,
    #[serde(rename = "model-paths", default = "default_model_paths")]
    model_paths: Vec<String>,
    #[serde(rename = "snapshot-paths", default = "default_snapshot_paths")]
    snapshot_paths: Vec<String>,
}

fn default_model_paths() -> Vec<String> {
    vec!["models".to_string()]
}

fn default_snapshot_paths() -> Vec<String> {
    vec!["snapshots".to_string()]
}

/// A detected dbt project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbtProject {
    pub root: PathBuf,
    pub name: String,
    pub model_paths: Vec<String>,
    pub snapshot_paths: Vec<String>,
    pub managed_sql_paths: HashSet<PathBuf>,
}

/// Detect dbt projects in the given directory.
///
/// Honors `.graphifyignore` at `root` (in addition to skipping hidden,
/// `node_modules`, `target`, and `venv` directories) so dbt discovery stays
/// consistent with the rest of the detection pipeline.
pub fn detect_dbt_projects(root: &Path) -> Vec<DbtProject> {
    let mut projects = Vec::new();
    let ignore = IgnoreSet::new(&load_graphifyignore(root));
    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.path() == root {
                return true;
            }
            let name = e.file_name().to_str().unwrap_or("");
            if name.starts_with('.') || name == "node_modules" || name == "target" || name == "venv"
            {
                return false;
            }
            !ignore.is_ignored(e.path(), root)
        });

    for entry in walker.flatten() {
        if entry.file_type().is_file()
            && entry.file_name() == "dbt_project.yml"
            && let Some(project) = parse_dbt_project(entry.path())
        {
            debug!(
                "detected dbt project '{}' at {}",
                project.name,
                entry.path().display()
            );
            projects.push(project);
        }
    }
    projects
}

/// Parse a `dbt_project.yml` file using serde_yaml_ng, supporting both
/// inline array format (`model-paths: ["models"]`) and multi-line block
/// format (`model-paths:\n  - models`).
fn parse_dbt_project(yaml_path: &Path) -> Option<DbtProject> {
    let content = fs::read_to_string(yaml_path).ok()?;
    let config: DbtProjectYaml = serde_yaml_ng::from_str(&content).ok()?;

    if config.name.is_empty() {
        return None;
    }

    let project_root = yaml_path.parent()?.to_path_buf();
    let mut managed_sql_paths = HashSet::new();

    for path in config
        .model_paths
        .iter()
        .chain(config.snapshot_paths.iter())
    {
        let dir = project_root.join(path);
        if dir.exists() {
            for entry in WalkDir::new(&dir).into_iter().flatten() {
                if entry.file_type().is_file()
                    && entry.path().extension().and_then(|e| e.to_str()) == Some("sql")
                {
                    managed_sql_paths.insert(entry.path().to_path_buf());
                }
            }
        }
    }

    Some(DbtProject {
        root: project_root,
        name: config.name,
        model_paths: config.model_paths,
        snapshot_paths: config.snapshot_paths,
        managed_sql_paths,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_parse_dbt_project() {
        let dir = tempdir().unwrap();
        let yaml_path = dir.path().join("dbt_project.yml");

        let yaml_content = r#"
name: 'my_test_project'
version: '1.0'

model-paths: ["models", "other_models"]
snapshot-paths: ["snapshots"]
"#;
        fs::write(&yaml_path, yaml_content).unwrap();

        // Create some mock SQL files
        let models_dir = dir.path().join("models");
        fs::create_dir_all(&models_dir).unwrap();
        fs::write(models_dir.join("a.sql"), "SELECT 1").unwrap();
        fs::write(models_dir.join("b.txt"), "not sql").unwrap(); // Should be ignored

        let snapshots_dir = dir.path().join("snapshots");
        fs::create_dir_all(&snapshots_dir).unwrap();
        fs::write(snapshots_dir.join("snap.sql"), "SELECT 2").unwrap();

        let parsed = parse_dbt_project(&yaml_path).expect("Should parse project");

        assert_eq!(parsed.name, "my_test_project");
        assert_eq!(parsed.model_paths, vec!["models", "other_models"]);
        assert_eq!(parsed.snapshot_paths, vec!["snapshots"]);

        assert_eq!(parsed.managed_sql_paths.len(), 2);
        assert!(parsed.managed_sql_paths.contains(&models_dir.join("a.sql")));
        assert!(
            parsed
                .managed_sql_paths
                .contains(&snapshots_dir.join("snap.sql"))
        );
    }

    #[test]
    fn test_parse_dbt_project_multiline_yaml() {
        let dir = tempdir().unwrap();
        let yaml_path = dir.path().join("dbt_project.yml");

        let yaml_content = "\
name: 'my_multiline_project'
version: '1.0'

model-paths:
  - models
  - other_models

snapshot-paths:
  - snapshots
";
        fs::write(&yaml_path, yaml_content).unwrap();
        fs::create_dir_all(dir.path().join("other_models")).unwrap();
        fs::write(dir.path().join("other_models").join("x.sql"), "SELECT 1").unwrap();

        let parsed = parse_dbt_project(&yaml_path).expect("Should parse multi-line YAML");
        assert_eq!(parsed.name, "my_multiline_project");
        assert_eq!(parsed.model_paths, vec!["models", "other_models"]);
        assert_eq!(parsed.snapshot_paths, vec!["snapshots"]);
        // x.sql should be found under other_models
        assert!(
            parsed
                .managed_sql_paths
                .contains(&dir.path().join("other_models").join("x.sql"))
        );
    }

    #[test]
    fn test_detect_dbt_projects() {
        let dir = tempdir().unwrap();

        // Project 1
        let proj1_dir = dir.path().join("proj1");
        fs::create_dir_all(&proj1_dir).unwrap();
        fs::write(
            proj1_dir.join("dbt_project.yml"),
            "name: proj1\nmodel-paths: [\"models\"]\n",
        )
        .unwrap();
        fs::create_dir_all(proj1_dir.join("models")).unwrap();
        fs::write(proj1_dir.join("models").join("m1.sql"), "SELECT 1").unwrap();

        // Project 2 (Nested)
        let proj2_dir = dir.path().join("nested").join("proj2");
        fs::create_dir_all(&proj2_dir).unwrap();
        fs::write(
            proj2_dir.join("dbt_project.yml"),
            "name: 'proj2'\nsnapshot-paths: [\"snaps\"]\n",
        )
        .unwrap();
        fs::create_dir_all(proj2_dir.join("snaps")).unwrap();
        fs::write(proj2_dir.join("snaps").join("s1.sql"), "SELECT 2").unwrap();

        // Noise dir (should be ignored)
        let node_modules = dir.path().join("node_modules");
        fs::create_dir_all(&node_modules).unwrap();
        fs::write(node_modules.join("dbt_project.yml"), "name: ignored\n").unwrap();

        let projects = detect_dbt_projects(dir.path());
        assert_eq!(projects.len(), 2);

        let names: Vec<String> = projects.into_iter().map(|p| p.name).collect();
        assert!(names.contains(&"proj1".to_string()));
        assert!(names.contains(&"proj2".to_string()));
    }

    #[test]
    fn test_detect_dbt_projects_respects_graphifyignore() {
        let dir = tempdir().unwrap();

        // A real project that should be detected.
        let keep = dir.path().join("keep");
        fs::create_dir_all(&keep).unwrap();
        fs::write(keep.join("dbt_project.yml"), "name: keep\n").unwrap();

        // A project under a directory excluded via .graphifyignore.
        let vendor = dir.path().join("vendor");
        fs::create_dir_all(&vendor).unwrap();
        fs::write(vendor.join("dbt_project.yml"), "name: vendored\n").unwrap();

        fs::write(dir.path().join(".graphifyignore"), "vendor\n").unwrap();

        let names: Vec<String> = detect_dbt_projects(dir.path())
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert!(names.contains(&"keep".to_string()));
        assert!(
            !names.contains(&"vendored".to_string()),
            "projects under a .graphifyignore'd path must be skipped"
        );
    }
}
