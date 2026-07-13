//! dbt project extraction.
//!
//! Invokes `dbt compile`, parses `manifest.json`, and extracts column-level lineage
//! from compiled SQL files.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use graphify_core::id::make_id;
use graphify_core::model::{ExtractionResult, GraphNode, NodeType};
use graphify_detect::DbtProject;
use tracing::{debug, warn};
use walkdir::WalkDir;

/// Controls how dbt projects are extracted.
#[derive(Debug, Clone, Copy)]
pub struct DbtOptions {
    /// When `true`, run `dbt compile` to regenerate `target/manifest.json` before
    /// parsing. This executes the project's dbt (which may connect to a live
    /// warehouse and evaluate arbitrary Jinja/Python macros), so it is **off by
    /// default**. When `false`, only an already-present manifest is parsed.
    pub compile: bool,
    /// Maximum wall-clock seconds to allow a single `dbt compile` invocation to
    /// run before it is killed. Only relevant when `compile` is `true`.
    pub compile_timeout_secs: u64,
}

impl Default for DbtOptions {
    fn default() -> Self {
        Self {
            compile: false,
            compile_timeout_secs: 120,
        }
    }
}

/// Extract all dbt projects into a combined [`ExtractionResult`].
///
/// For each project in `projects`:
/// 1. Optionally runs `dbt compile` in the project root to produce a fresh
///    `manifest.json` — **only** when [`DbtOptions::compile`] is set. By default
///    this step is skipped and an existing manifest is used as-is, keeping the
///    extractor side-effect free. If the `dbt` binary is not found or compilation
///    fails, extraction continues with whatever manifest already exists.
/// 2. Parses `target/manifest.json`, emitting [`NodeType::Relation`] nodes for every
///    `model`, `seed`, `snapshot`, and `source`, together with `defines`, `part_of`,
///    and `depends_on` edges.
/// 3. Walks `target/compiled/**/*.sql`, maps each file back to its manifest model
///    (via `compiled_path`), and feeds it through the lineage-only SQL extractor
///    ([`crate::sql::extract_sql_lineage`]) to collect column-level lineage
///    (`derives_from` edges) attributed to that model's Relation node.
///
/// Projects whose manifest is absent are skipped with a warning.
pub fn extract_dbt_projects(projects: &[DbtProject], opts: DbtOptions) -> ExtractionResult {
    let mut combined = ExtractionResult::default();

    for project in projects {
        if opts.compile {
            debug!("running dbt compile for project '{}'", project.name);
            run_dbt_compile(
                &project.name,
                &project.root,
                Duration::from_secs(opts.compile_timeout_secs),
            );
        } else {
            debug!(
                "dbt compile disabled; parsing existing manifest for '{}'",
                project.name
            );
        }

        let manifest_path = project.root.join("target/manifest.json");
        if !manifest_path.exists() {
            warn!(
                "no manifest.json found for '{}' (expected at {}); \
                 run `dbt compile` or pass --dbt-compile to generate it",
                project.name,
                manifest_path.display()
            );
            continue;
        }

        let manifest_content = match fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(e) => {
                warn!("failed to read manifest.json for '{}': {}", project.name, e);
                continue;
            }
        };

        let manifest: serde_json::Value = match serde_json::from_str(&manifest_content) {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    "failed to parse manifest.json for '{}': {}",
                    project.name, e
                );
                continue;
            }
        };

        let ManifestParse {
            mut result,
            compiled_map,
        } = parse_manifest(&project.name, &manifest, &project.root);

        // Compiled SQL pass: compiled dbt models are bare `SELECT`s (dbt injects
        // the DDL at run time), so they are fed through the lineage-only extractor
        // with the owning model's Relation ID from the manifest. Files that don't
        // map to a manifest node (macros, analyses, ephemeral leftovers) are
        // skipped — running the full SQL extractor on them would fabricate
        // Application/File nodes named after target/compiled directories.
        let compiled_dir = project.root.join("target/compiled");
        if compiled_dir.exists() {
            for entry in WalkDir::new(&compiled_dir).into_iter().flatten() {
                if entry.file_type().is_file()
                    && entry.path().extension().and_then(|e| e.to_str()) == Some("sql")
                {
                    let Some(rel_id) = compiled_map.get(entry.path()) else {
                        debug!(
                            "skipping compiled SQL with no manifest mapping: {}",
                            entry.path().display()
                        );
                        continue;
                    };
                    let source = fs::read_to_string(entry.path()).unwrap_or_default();
                    let lineage = crate::sql::extract_sql_lineage(entry.path(), &source, rel_id);
                    result.nodes.extend(lineage.nodes);
                    result.edges.extend(lineage.edges);
                }
            }
        }

        combined.nodes.extend(result.nodes);
        combined.edges.extend(result.edges);
    }

    combined
}

/// Run `dbt compile` for a single project with a hard wall-clock timeout.
///
/// The child is spawned with stdout/stderr suppressed and polled until it exits
/// or `timeout` elapses, at which point it is killed. All failure modes (binary
/// missing, non-zero exit, timeout) are logged and swallowed: extraction then
/// proceeds with whatever `manifest.json` already exists, so a broken or hanging
/// dbt setup can never abort or wedge the overall build.
fn run_dbt_compile(name: &str, root: &Path, timeout: Duration) {
    let mut child = match Command::new("dbt")
        .arg("compile")
        .current_dir(root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            warn!("failed to invoke dbt compile for '{}': {}", name, e);
            return;
        }
    };

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    warn!(
                        "dbt compile failed for '{}', proceeding with available manifest",
                        name
                    );
                }
                return;
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    warn!(
                        "dbt compile for '{}' exceeded {}s timeout; killing and \
                         proceeding with available manifest",
                        name,
                        timeout.as_secs()
                    );
                    let _ = child.kill();
                    let _ = child.wait();
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                warn!("error waiting on dbt compile for '{}': {}", name, e);
                return;
            }
        }
    }
}

/// Output of [`parse_manifest`]: the extracted nodes/edges plus a map from each
/// model's compiled SQL path (absolute, under `target/compiled/`) to its
/// Relation node ID, used to attribute compiled-SQL lineage to the right model.
struct ManifestParse {
    result: ExtractionResult,
    compiled_map: HashMap<PathBuf, String>,
}

fn parse_manifest(app_name: &str, manifest: &serde_json::Value, root: &Path) -> ManifestParse {
    let mut result = ExtractionResult::default();
    let mut compiled_map: HashMap<PathBuf, String> = HashMap::new();

    let app_id = make_id(&["app", app_name]);
    result.nodes.push(GraphNode {
        id: app_id.clone(),
        label: app_name.to_string(),
        source_file: root.join("dbt_project.yml").to_string_lossy().into_owned(),
        source_location: None,
        node_type: NodeType::Application,
        community: None,
        extra: HashMap::new(),
    });

    let mut defined_models = HashMap::new();

    let mut process_entry =
        |node_id: &String, node: &serde_json::Value, resource_type: &str, fallback_field: &str| {
            let name = node.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let identifier = node
                .get(fallback_field)
                .and_then(|v| v.as_str())
                .filter(|a| !a.is_empty())
                .unwrap_or(name);

            let schema = node.get("schema").and_then(|v| v.as_str()).unwrap_or("");
            let database = node.get("database").and_then(|v| v.as_str()).unwrap_or("");
            let rel_id = make_id(&["rel", database, schema, identifier]);

            defined_models.insert(node_id.clone(), rel_id.clone());

            let mut extra = HashMap::new();
            extra.insert(
                "relation_kind".to_string(),
                serde_json::json!(resource_type),
            );
            if !database.is_empty() {
                extra.insert("catalog".to_string(), serde_json::json!(database));
            }
            extra.insert("dbt_name".to_string(), serde_json::json!(name));
            if let Some(desc) = node.get("description") {
                extra.insert("description".to_string(), desc.clone());
            }

            let original_path = node
                .get("original_file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let source_file = root.join(original_path).to_string_lossy().into_owned();

            if resource_type != "source" {
                // Map model's compiled SQL to Relation ID
                let compiled_path = node
                    .get("compiled_path")
                    .and_then(|v| v.as_str())
                    .filter(|p| !p.is_empty())
                    .map(|p| root.join(p))
                    .or_else(|| {
                        (!original_path.is_empty()).then(|| {
                            root.join("target/compiled")
                                .join(app_name)
                                .join(original_path)
                        })
                    });
                if let Some(cp) = compiled_path {
                    compiled_map.insert(cp, rel_id.clone());
                }
            }

            result.nodes.push(GraphNode {
                id: rel_id.clone(),
                label: crate::sql::format_relation_label(
                    if database.is_empty() {
                        None
                    } else {
                        Some(database)
                    },
                    schema,
                    identifier,
                ),
                source_file: source_file.clone(),
                source_location: None,
                node_type: NodeType::Relation,
                community: None,
                extra,
            });

            if resource_type != "source" {
                let file_id = make_id(&["file", &source_file.replace('/', "_")]);
                result.nodes.push(GraphNode {
                    id: file_id.clone(),
                    label: Path::new(&source_file)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    source_file: source_file.clone(),
                    source_location: None,
                    node_type: NodeType::File,
                    community: None,
                    extra: HashMap::new(),
                });
                result.edges.push(crate::sql::make_sql_edge(
                    &file_id,
                    &rel_id,
                    "defines",
                    &source_file,
                ));
            }

            result.edges.push(crate::sql::make_sql_edge(
                &rel_id,
                &app_id,
                "part_of",
                &source_file,
            ));
        };

    if let Some(nodes) = manifest.get("nodes").and_then(|n| n.as_object()) {
        for (node_id, node) in nodes {
            let resource_type = node
                .get("resource_type")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if resource_type == "model" || resource_type == "seed" || resource_type == "snapshot" {
                process_entry(node_id, node, resource_type, "alias");
            }
        }
    }

    if let Some(sources) = manifest.get("sources").and_then(|s| s.as_object()) {
        for (node_id, node) in sources {
            process_entry(node_id, node, "source", "identifier");
        }
    }

    // Pass 2: resolve depends_on edges
    if let Some(nodes) = manifest.get("nodes").and_then(|n| n.as_object()) {
        for (node_id, node) in nodes {
            if let Some(source_rel_id) = defined_models.get(node_id)
                && let Some(depends_on) = node
                    .get("depends_on")
                    .and_then(|d| d.get("nodes"))
                    .and_then(|n| n.as_array())
            {
                for dep in depends_on.iter().filter_map(|d| d.as_str()) {
                    if let Some(target_rel_id) = defined_models.get(dep) {
                        // Tagged with origin="sql" so resolve_sql_cross_file treats
                        // dbt dependencies consistently with plain-SQL edges.
                        result.edges.push(crate::sql::make_sql_edge(
                            source_rel_id,
                            target_rel_id,
                            "depends_on",
                            "manifest.json",
                        ));
                    }
                }
            }
        }
    }

    ManifestParse {
        result,
        compiled_map,
    }
}

// NOTE: DBT tests require mocking the dbt CLI and filesystem. For integration
// testing at the extract() level, a full dbt project setup is needed.
// These unit tests verify the core extraction logic in isolation.
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_manifest() {
        let manifest = json!({
            "nodes": {
                "model.my_app.my_model": {
                    "resource_type": "model",
                    "name": "my_model",
                    "schema": "public",
                    "original_file_path": "models/my_model.sql",
                    "description": "A test model",
                    "depends_on": {
                        "nodes": ["source.my_app.my_source"]
                    }
                }
            },
            "sources": {
                "source.my_app.my_source": {
                    "resource_type": "source",
                    "name": "my_source",
                    "schema": "raw",
                    "original_file_path": "models/sources.yml"
                }
            }
        });

        let result = parse_manifest("my_app", &manifest, Path::new("/my_app")).result;

        let app_node = result
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Application)
            .unwrap();
        assert_eq!(app_node.label, "my_app");

        let model_node = result
            .nodes
            .iter()
            .find(|n| n.label == "public.my_model")
            .unwrap();
        assert_eq!(model_node.extra.get("relation_kind").unwrap(), "model");
        assert_eq!(model_node.extra.get("dbt_name").unwrap(), "my_model");

        let source_node = result
            .nodes
            .iter()
            .find(|n| n.label == "raw.my_source")
            .unwrap();
        assert_eq!(source_node.extra.get("relation_kind").unwrap(), "source");

        let depends_on = result
            .edges
            .iter()
            .find(|e| e.relation == "depends_on")
            .unwrap();
        assert_eq!(depends_on.source, model_node.id);
        assert_eq!(depends_on.target, source_node.id);
    }

    /// Compiled dbt models (bare SELECTs) are mapped back to their manifest model
    /// via `compiled_path` and produce lineage attributed to that model — with no
    /// bogus Application/File nodes named after `target/compiled` directories.
    /// Runs with `compile: false`, so no dbt CLI is required.
    #[test]
    fn test_dbt_compiled_lineage_mapped_to_model() {
        use graphify_core::id::make_id;
        use graphify_detect::DbtProject;
        use std::collections::HashSet;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let target_dir = dir.path().join("target");
        let compiled_models = target_dir.join("compiled/my_app/models");
        std::fs::create_dir_all(&compiled_models).unwrap();

        let manifest = json!({
            "nodes": {
                "model.my_app.order_report": {
                    "resource_type": "model",
                    "name": "order_report",
                    "schema": "analytics",
                    "original_file_path": "models/order_report.sql",
                    "compiled_path": "target/compiled/my_app/models/order_report.sql"
                }
            }
        });
        std::fs::write(target_dir.join("manifest.json"), manifest.to_string()).unwrap();

        // Compiled model body: bare SELECT (dbt injects DDL at run time).
        std::fs::write(
            compiled_models.join("order_report.sql"),
            "SELECT o.id AS order_id FROM staging.orders o",
        )
        .unwrap();

        // A compiled file with NO manifest mapping must be skipped entirely.
        std::fs::write(compiled_models.join("orphan.sql"), "SELECT x FROM y").unwrap();

        let proj = DbtProject {
            root: dir.path().to_path_buf(),
            name: "my_app".to_string(),
            model_paths: vec![],
            snapshot_paths: vec![],
            managed_sql_paths: HashSet::new(),
        };

        let result = extract_dbt_projects(&[proj], DbtOptions::default());

        // Manifest side: Application + model Relation.
        let model_rel_id = make_id(&["rel", "analytics", "order_report"]);
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.node_type == NodeType::Application && n.label == "my_app"),
            "Application node from manifest should exist"
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.node_type == NodeType::Relation && n.id == model_rel_id),
            "model Relation node should exist"
        );

        // Compiled lineage: depends_on staging.orders attributed to the model.
        let orders_id = make_id(&["rel", "staging", "orders"]);
        assert!(
            result.edges.iter().any(|e| e.relation == "depends_on"
                && e.source == model_rel_id
                && e.target == orders_id),
            "model should depend_on staging.orders from compiled SQL"
        );

        // Column lineage: order_id part_of the model, derives_from orders.id.
        let order_id_col = result
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Column && n.label == "order_id")
            .expect("order_id Column node should exist");
        let src_col = make_id(&["col", &orders_id, "id"]);
        assert!(
            result.edges.iter().any(|e| e.relation == "derives_from"
                && e.source == order_id_col.id
                && e.target == src_col),
            "order_id should derive_from staging.orders.id"
        );

        // No pollution: no Application named after compiled dirs, no File nodes
        // under target/, and nothing extracted from the unmapped orphan.sql.
        assert!(
            !result
                .nodes
                .iter()
                .any(|n| n.node_type == NodeType::Application && n.label != "my_app"),
            "no Application nodes named after target/compiled directories"
        );
        assert!(
            !result
                .nodes
                .iter()
                .any(|n| n.node_type == NodeType::File && n.source_file.contains("compiled")),
            "no File nodes for compiled SQL files"
        );
        let y_id = make_id(&["rel", "", "y"]);
        assert!(
            !result.edges.iter().any(|e| e.target == y_id),
            "unmapped orphan.sql must be skipped entirely"
        );
    }

    /// A model with an `alias` is keyed and labelled by the alias (the
    /// warehouse-side table name), with the dbt model name preserved in extra.
    #[test]
    fn test_dbt_alias_used_for_relation_identity() {
        use graphify_core::id::make_id;

        let manifest = json!({
            "nodes": {
                "model.my_app.orders_model": {
                    "resource_type": "model",
                    "name": "orders_model",
                    "alias": "orders",
                    "schema": "analytics",
                    "original_file_path": "models/orders_model.sql"
                },
                "model.my_app.plain_model": {
                    "resource_type": "seed",
                    "name": "plain_model",
                    "schema": "analytics",
                    "original_file_path": "seeds/plain_model.csv"
                }
            }
        });

        let result = parse_manifest("my_app", &manifest, Path::new("/my_app")).result;

        // Aliased model: identity follows the alias.
        let aliased = result
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Relation && n.label == "analytics.orders")
            .expect("aliased model should be labelled by its alias");
        assert_eq!(aliased.id, make_id(&["rel", "analytics", "orders"]));
        assert_eq!(aliased.extra.get("dbt_name").unwrap(), "orders_model");
        assert_eq!(aliased.extra.get("relation_kind").unwrap(), "model");

        // No alias: falls back to the model name; resource_type is preserved.
        let plain = result
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Relation && n.label == "analytics.plain_model")
            .expect("un-aliased model keyed by name");
        assert_eq!(plain.extra.get("relation_kind").unwrap(), "seed");
    }

    // Fix 18 — Missing dbt CLI / non-existent project root should not panic.
    #[test]
    fn test_extract_dbt_projects_missing_cli() {
        use std::collections::HashSet;

        // A project whose root does not exist: dbt compile cannot run, the manifest
        // will not be present, and the project is skipped with a warning.
        // This must not panic under any circumstances.
        let proj = DbtProject {
            root: std::path::PathBuf::from("/nonexistent/path/that/does/not/exist"),
            name: "missing_cli_test".to_string(),
            model_paths: vec![],
            snapshot_paths: vec![],
            managed_sql_paths: HashSet::new(),
        };

        let result = extract_dbt_projects(
            &[proj],
            DbtOptions {
                compile: true,
                compile_timeout_secs: 60,
            },
        );
        // Either empty or contains no Application nodes produced by this project.
        assert!(
            result.nodes.is_empty()
                || !result
                    .nodes
                    .iter()
                    .any(|n| n.node_type == NodeType::Application && n.label == "missing_cli_test"),
            "Missing CLI / root should produce empty or near-empty results"
        );
    }

    // Fix 20 — Cross-project depends_on: dependency on undefined target is not emitted
    // within a single manifest parse. Cross-project linking happens when the CLI
    // pipeline (step_extract_ast) merges all project results and runs
    // sql::resolve_sql_cross_file over the combined graph.
    #[test]
    fn test_parse_manifest_cross_project_depends_on() {
        let manifest_a = json!({
            "nodes": {
                "model.app_a.orders_report": {
                    "resource_type": "model",
                    "name": "orders_report",
                    "schema": "analytics",
                    "original_file_path": "models/orders_report.sql",
                    "depends_on": {
                        // Cross-project dependency — this node is not defined in manifest_a.
                        "nodes": ["source.app_b.raw_orders"]
                    }
                }
            }
        });

        let result_a =
            parse_manifest("app_a", &manifest_a, std::path::Path::new("/repo/app_a")).result;

        // The depends_on edge target references a source that doesn’t exist in this manifest.
        // Within a single parse_manifest call, the edge should NOT be created for undefined
        // targets — cross-project resolution happens at the resolve_sql_cross_file level.
        let dep = result_a.edges.iter().find(|e| e.relation == "depends_on");
        assert!(
            dep.is_none(),
            "depends_on to undefined cross-project target should not create an edge within a \
             single manifest parse; got {:?}",
            dep
        );
    }

    // C9 — Monorepo: two separate `parse_manifest` calls with different app names produce
    // independent Application nodes. Relation IDs are schema-scoped (`schema.name`),
    // so they differ here because the schemas differ — same-schema relations would
    // intentionally merge across apps (global warehouse namespace).
    #[test]
    fn test_parse_manifest_monorepo_multiple_applications() {
        let manifest_a = json!({
            "nodes": {
                "model.app_a.table_a": {
                    "resource_type": "model",
                    "name": "table_a",
                    "schema": "schema_a",
                    "original_file_path": "models/table_a.sql"
                }
            }
        });
        let manifest_b = json!({
            "nodes": {
                "model.app_b.table_b": {
                    "resource_type": "model",
                    "name": "table_b",
                    "schema": "schema_b",
                    "original_file_path": "models/table_b.sql"
                }
            }
        });

        let result_a = parse_manifest("app_a", &manifest_a, Path::new("/repo/app_a")).result;
        let result_b = parse_manifest("app_b", &manifest_b, Path::new("/repo/app_b")).result;

        // Each result must have its own distinct Application node.
        let app_a = result_a
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Application)
            .expect("app_a should have an Application node");
        let app_b = result_b
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Application)
            .expect("app_b should have an Application node");

        assert_eq!(app_a.label, "app_a");
        assert_eq!(app_b.label, "app_b");
        assert_ne!(
            app_a.id, app_b.id,
            "Application IDs must differ across apps"
        );

        // Relation nodes must be labelled correctly and scoped to their app.
        let rel_a = result_a
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Relation)
            .expect("app_a should have a Relation node");
        let rel_b = result_b
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Relation)
            .expect("app_b should have a Relation node");

        assert_eq!(rel_a.label, "schema_a.table_a");
        assert_eq!(rel_b.label, "schema_b.table_b");
        assert_ne!(
            rel_a.id, rel_b.id,
            "Relation IDs must be scoped to their app"
        );

        // Each Relation must be part_of its own Application.
        let a_part_of = result_a
            .edges
            .iter()
            .find(|e| e.relation == "part_of" && e.source == rel_a.id && e.target == app_a.id);
        assert!(a_part_of.is_some(), "table_a should be part_of app_a");

        let b_part_of = result_b
            .edges
            .iter()
            .find(|e| e.relation == "part_of" && e.source == rel_b.id && e.target == app_b.id);
        assert!(b_part_of.is_some(), "table_b should be part_of app_b");
    }
}
