use std::collections::{HashMap, HashSet};
use std::path::Path;

use graphify_core::error::Result;
use graphify_core::graph::KnowledgeGraph;
use graphify_core::model::NodeType;
use rusqlite::Connection;
use tracing::{debug, warn};

/// CodeGraph node kind -> graphify-rs NodeType.
fn map_node_kind(codegraph_kind: &str) -> Option<NodeType> {
    match codegraph_kind {
        "class" => Some(NodeType::Class),
        "struct" => Some(NodeType::Struct),
        "interface" => Some(NodeType::Interface),
        "trait" | "protocol" => Some(NodeType::Trait),
        "function" => Some(NodeType::Function),
        "method" => Some(NodeType::Method),
        "enum" => Some(NodeType::Enum),
        "enum_member" | "variable" | "parameter" | "property" | "field" | "type_alias"
        | "export" | "import" | "decorator" => Some(NodeType::Variable),
        "constant" => Some(NodeType::Constant),
        "module" | "namespace" => Some(NodeType::Module),
        "file" => Some(NodeType::File),
        "route" | "component" => Some(NodeType::Class),
        _ => None,
    }
}

/// Normalize a file path to POSIX style: forward slashes, no leading `./`.
/// Note: does not resolve `..` segments — assumes both tools produce
/// consistently relative paths from the project root.
fn normalize_path(path: &str) -> String {
    let p = path.replace('\\', "/");
    let p = p.strip_prefix("./").unwrap_or(&p);
    p.to_string()
}

/// CodeGraph edge kind -> graphify-rs relation string.
/// Returns `None` for `contains` (skipped) and unknown kinds.
fn map_edge_kind(codegraph_kind: &str) -> Option<&'static str> {
    match codegraph_kind {
        "calls" => Some("calls"),
        "imports" => Some("imports"),
        "extends" => Some("extends"),
        "implements" => Some("implements"),
        "references" => Some("references"),
        "exports" => Some("exports"),
        "overrides" => Some("overrides"),
        "returns" | "type_of" | "instantiates" | "decorates" | "route" => Some("references"),
        "contains" => None,
        _ => None,
    }
}

/// Merge edges from a CodeGraph SQLite database into an existing [`KnowledgeGraph`].
///
/// Looks for `.codegraph/codegraph.db` under `project_root`. If not found, or if
/// no matching nodes exist, returns `Ok(0)`.
///
/// Returns the number of edges merged.
pub fn merge_codegraph_edges(kg: &mut KnowledgeGraph, project_root: &Path) -> Result<usize> {
    let db_path = project_root.join(".codegraph").join("codegraph.db");
    if !db_path.exists() {
        return Ok(0);
    }

    let conn = match Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            warn!("cannot open CodeGraph DB at {}: {e}", db_path.display());
            return Ok(0);
        }
    };

    // --- Build CodeGraph node index: (file_path, name, NodeType) -> Vec<cg_id> ---
    let mut cg_nodes: HashMap<(String, String, NodeType), Vec<String>> = HashMap::new();
    let mut cg_id_to_key: HashMap<String, (String, String, NodeType)> = HashMap::new();

    {
        let mut stmt = match conn.prepare("SELECT id, kind, name, file_path FROM nodes") {
            Ok(s) => s,
            Err(e) => {
                warn!("cannot query CodeGraph nodes: {e}");
                return Ok(0);
            }
        };

        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let kind: String = row.get(1)?;
            let name: String = row.get(2)?;
            let file_path: String = row.get(3)?;
            Ok((id, kind, name, file_path))
        });

        match rows {
            Ok(iter) => {
                for r in iter {
                    match r {
                        Ok((id, kind, name, file_path)) => {
                            let nt = map_node_kind(&kind).unwrap_or_else(|| {
                                tracing::debug!(
                                    "unknown CodeGraph node kind '{kind}', falling back to Variable"
                                );
                                NodeType::Variable
                            });
                            let fp = normalize_path(&file_path);
                            let key = (fp, name, nt);
                            cg_id_to_key.insert(id.clone(), key.clone());
                            cg_nodes.entry(key).or_default().push(id);
                        }
                        Err(e) => {
                            warn!("skipping CodeGraph node row: {e}");
                        }
                    }
                }
            }
            Err(e) => {
                warn!("cannot iterate CodeGraph nodes: {e}");
                return Ok(0);
            }
        }
    }

    if cg_id_to_key.is_empty() {
        return Ok(0);
    }

    // --- Build graphify-rs node index: (file_path, name, NodeType) -> Vec<gf_node_id> ---
    let mut gf_index: HashMap<(String, String, NodeType), Vec<String>> = HashMap::new();
    for node in kg.nodes() {
        let fp = normalize_path(&node.source_file);
        let key = (fp, node.label.clone(), node.node_type.clone());
        gf_index.entry(key).or_default().push(node.id.clone());
    }

    // --- Build existing edge set for dedup ---
    let mut existing_edges: HashSet<(String, String, String)> = HashSet::new();
    for (src, tgt, edge) in kg.edges_with_endpoints() {
        existing_edges.insert((src.to_string(), tgt.to_string(), edge.relation.clone()));
    }

    // --- Read and merge CodeGraph edges ---
    let mut merged = 0usize;
    let mut skipped_contains = 0usize;
    let mut skipped_kind = 0usize;
    let mut skipped_no_source = 0usize;
    let mut skipped_no_target = 0usize;
    let mut skipped_dedup = 0usize;

    {
        let mut stmt = match conn.prepare("SELECT source, target, kind, provenance FROM edges") {
            Ok(s) => s,
            Err(e) => {
                warn!("cannot query CodeGraph edges: {e}");
                return Ok(0);
            }
        };

        let rows = stmt.query_map([], |row| {
            let source: String = row.get(0)?;
            let target: String = row.get(1)?;
            let kind: String = row.get(2)?;
            let provenance: Option<String> = row.get(3)?;
            Ok((source, target, kind, provenance))
        });

        match rows {
            Ok(iter) => {
                for r in iter {
                    match r {
                        Ok((cg_src_id, cg_tgt_id, cg_kind, provenance)) => {
                            let Some(relation) = map_edge_kind(&cg_kind) else {
                                if cg_kind == "contains" {
                                    skipped_contains += 1;
                                } else {
                                    tracing::debug!(
                                        "skipping CodeGraph edge with unrecognized kind '{cg_kind}'"
                                    );
                                    skipped_kind += 1;
                                }
                                continue;
                            };

                            // Look up source node
                            let Some(src_key) = cg_id_to_key.get(&cg_src_id) else {
                                skipped_no_source += 1;
                                continue;
                            };
                            let Some(src_gf_ids) = gf_index.get(src_key) else {
                                skipped_no_source += 1;
                                continue;
                            };

                            // Look up target node
                            let Some(tgt_key) = cg_id_to_key.get(&cg_tgt_id) else {
                                skipped_no_target += 1;
                                continue;
                            };
                            let Some(tgt_gf_ids) = gf_index.get(tgt_key) else {
                                skipped_no_target += 1;
                                continue;
                            };

                            let gf_src = &src_gf_ids[0];
                            let gf_tgt = &tgt_gf_ids[0];

                            if src_gf_ids.len() > 1 {
                                debug!(
                                    "ambiguous CodeGraph source: key {:?} maps to {} graphify nodes, using {}",
                                    src_key,
                                    src_gf_ids.len(),
                                    gf_src
                                );
                            }
                            if tgt_gf_ids.len() > 1 {
                                debug!(
                                    "ambiguous CodeGraph target: key {:?} maps to {} graphify nodes, using {}",
                                    tgt_key,
                                    tgt_gf_ids.len(),
                                    gf_tgt
                                );
                            }

                            // Dedup check
                            let key = (gf_src.as_str(), gf_tgt.as_str(), relation);
                            if existing_edges
                                .iter()
                                .any(|(s, t, r)| s == key.0 && t == key.1 && r == key.2)
                            {
                                skipped_dedup += 1;
                                continue;
                            }

                            // Build extra metadata
                            let mut extra = HashMap::new();
                            extra.insert(
                                "merge_source".to_string(),
                                serde_json::Value::String("codegraph".to_string()),
                            );
                            extra.insert(
                                "codegraph_kind".to_string(),
                                serde_json::Value::String(cg_kind.clone()),
                            );
                            if let Some(prov) = provenance {
                                extra.insert(
                                    "codegraph_provenance".to_string(),
                                    serde_json::Value::String(prov),
                                );
                            }

                            let edge = graphify_core::model::GraphEdge {
                                source: gf_src.clone(),
                                target: gf_tgt.clone(),
                                relation: relation.to_string(),
                                confidence: graphify_core::confidence::Confidence::Extracted,
                                confidence_score: 1.0,
                                source_file: src_key.0.clone(),
                                source_location: None,
                                weight: 1.0,
                                extra,
                            };

                            if kg.add_edge(edge).is_ok() {
                                merged += 1;
                                // Also add to dedup set so we don't re-add
                                existing_edges.insert((
                                    gf_src.clone(),
                                    gf_tgt.clone(),
                                    relation.to_string(),
                                ));
                            }
                        }
                        Err(e) => {
                            warn!("skipping CodeGraph edge row: {e}");
                        }
                    }
                }
            }
            Err(e) => {
                warn!("cannot iterate CodeGraph edges: {e}");
                return Ok(0);
            }
        }
    }

    let total_skipped =
        skipped_contains + skipped_kind + skipped_no_source + skipped_no_target + skipped_dedup;
    let unmatched = skipped_no_source + skipped_no_target;
    tracing::info!(
        "CodeGraph: merged {merged} edges ({total_skipped} skipped: {unmatched} unmatched, {skipped_contains} contains, {skipped_dedup} duplicate, {skipped_kind} unsupported kind)",
    );

    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphify_core::confidence::Confidence;
    use graphify_core::model::{GraphEdge, GraphNode};
    use rusqlite::Connection;

    // --- Helper functions for tests ---

    fn populate_cg_schema(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE nodes (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                name TEXT NOT NULL,
                qualified_name TEXT NOT NULL,
                file_path TEXT NOT NULL,
                language TEXT NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                start_column INTEGER NOT NULL,
                end_column INTEGER NOT NULL,
                docstring TEXT,
                signature TEXT,
                visibility TEXT,
                is_exported INTEGER NOT NULL DEFAULT 0,
                is_async INTEGER NOT NULL DEFAULT 0,
                is_static INTEGER NOT NULL DEFAULT 0,
                is_abstract INTEGER NOT NULL DEFAULT 0,
                decorators TEXT,
                type_parameters TEXT,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE edges (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source TEXT NOT NULL,
                target TEXT NOT NULL,
                kind TEXT NOT NULL,
                metadata TEXT,
                line INTEGER,
                col INTEGER,
                provenance TEXT
            );
            CREATE TABLE files (
                path TEXT PRIMARY KEY,
                content_hash TEXT NOT NULL,
                language TEXT NOT NULL,
                size INTEGER NOT NULL,
                modified_at INTEGER NOT NULL,
                indexed_at INTEGER NOT NULL,
                node_count INTEGER DEFAULT 0,
                errors TEXT
            );
            CREATE TABLE schema_versions (
                version INTEGER PRIMARY KEY,
                applied_at INTEGER NOT NULL,
                description TEXT
            );
            CREATE TABLE project_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );",
        )
        .unwrap();
    }

    fn insert_cg_node(conn: &Connection, id: &str, kind: &str, name: &str, file_path: &str) {
        conn.execute(
            "INSERT INTO nodes (id, kind, name, qualified_name, file_path, language, start_line, end_line, start_column, end_column, is_exported, is_async, is_static, is_abstract, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'rust', 1, 10, 0, 20, 0, 0, 0, 0, 1000)",
            rusqlite::params![id, kind, name, format!("{file_path}::{name}"), file_path],
        ).unwrap();
    }

    fn insert_cg_edge(conn: &Connection, source: &str, target: &str, kind: &str) {
        conn.execute(
            "INSERT INTO edges (source, target, kind) VALUES (?1, ?2, ?3)",
            rusqlite::params![source, target, kind],
        )
        .unwrap();
    }

    fn make_graph_node(id: &str, label: &str, file: &str, nt: NodeType) -> GraphNode {
        GraphNode {
            id: id.into(),
            label: label.into(),
            source_file: file.into(),
            source_location: None,
            node_type: nt,
            community: None,
            extra: std::collections::HashMap::new(),
        }
    }

    fn make_graph_edge(src: &str, tgt: &str, relation: &str, file: &str) -> GraphEdge {
        GraphEdge {
            source: src.into(),
            target: tgt.into(),
            relation: relation.into(),
            confidence: Confidence::Extracted,
            confidence_score: 1.0,
            source_file: file.into(),
            source_location: None,
            weight: 1.0,
            extra: std::collections::HashMap::new(),
        }
    }

    // --- Unit tests for helper functions ---

    #[test]
    fn normalize_path_forward_slashes() {
        assert_eq!(normalize_path(r"src\main.rs"), "src/main.rs");
    }

    #[test]
    fn normalize_path_strips_dot_slash() {
        assert_eq!(normalize_path("./src/lib.rs"), "src/lib.rs");
    }

    #[test]
    fn normalize_path_already_clean() {
        assert_eq!(normalize_path("src/lib.rs"), "src/lib.rs");
    }

    #[test]
    fn normalize_path_empty() {
        assert_eq!(normalize_path(""), "");
    }

    #[test]
    fn map_node_kind_known_types() {
        assert_eq!(map_node_kind("class"), Some(NodeType::Class));
        assert_eq!(map_node_kind("struct"), Some(NodeType::Struct));
        assert_eq!(map_node_kind("interface"), Some(NodeType::Interface));
        assert_eq!(map_node_kind("trait"), Some(NodeType::Trait));
        assert_eq!(map_node_kind("protocol"), Some(NodeType::Trait));
        assert_eq!(map_node_kind("function"), Some(NodeType::Function));
        assert_eq!(map_node_kind("method"), Some(NodeType::Method));
        assert_eq!(map_node_kind("enum"), Some(NodeType::Enum));
        assert_eq!(map_node_kind("constant"), Some(NodeType::Constant));
        assert_eq!(map_node_kind("module"), Some(NodeType::Module));
        assert_eq!(map_node_kind("namespace"), Some(NodeType::Module));
        assert_eq!(map_node_kind("file"), Some(NodeType::File));
        assert_eq!(map_node_kind("variable"), Some(NodeType::Variable));
        assert_eq!(map_node_kind("parameter"), Some(NodeType::Variable));
        assert_eq!(map_node_kind("property"), Some(NodeType::Variable));
        assert_eq!(map_node_kind("field"), Some(NodeType::Variable));
        assert_eq!(map_node_kind("type_alias"), Some(NodeType::Variable));
        assert_eq!(map_node_kind("export"), Some(NodeType::Variable));
        assert_eq!(map_node_kind("import"), Some(NodeType::Variable));
        assert_eq!(map_node_kind("decorator"), Some(NodeType::Variable));
        assert_eq!(map_node_kind("enum_member"), Some(NodeType::Variable));
        assert_eq!(map_node_kind("route"), Some(NodeType::Class));
        assert_eq!(map_node_kind("component"), Some(NodeType::Class));
    }

    #[test]
    fn map_node_kind_unknown() {
        assert_eq!(map_node_kind("unknown_thing"), None);
        assert_eq!(map_node_kind(""), None);
    }

    #[test]
    fn map_edge_kind_known() {
        assert_eq!(map_edge_kind("calls"), Some("calls"));
        assert_eq!(map_edge_kind("imports"), Some("imports"));
        assert_eq!(map_edge_kind("extends"), Some("extends"));
        assert_eq!(map_edge_kind("implements"), Some("implements"));
        assert_eq!(map_edge_kind("references"), Some("references"));
        assert_eq!(map_edge_kind("exports"), Some("exports"));
        assert_eq!(map_edge_kind("overrides"), Some("overrides"));
    }

    #[test]
    fn map_edge_kind_references_aliases() {
        assert_eq!(map_edge_kind("returns"), Some("references"));
        assert_eq!(map_edge_kind("type_of"), Some("references"));
        assert_eq!(map_edge_kind("instantiates"), Some("references"));
        assert_eq!(map_edge_kind("decorates"), Some("references"));
        assert_eq!(map_edge_kind("route"), Some("references"));
    }

    #[test]
    fn map_edge_kind_contains_returns_none() {
        assert_eq!(map_edge_kind("contains"), None);
    }

    #[test]
    fn map_edge_kind_unknown() {
        assert_eq!(map_edge_kind("weird_edge"), None);
        assert_eq!(map_edge_kind(""), None);
    }

    // --- Integration tests ---

    #[test]
    fn no_codegraph_dir_returns_ok_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let mut kg = KnowledgeGraph::new();
        let result = merge_codegraph_edges(&mut kg, tmp.path());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn valid_db_with_no_matching_nodes() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join(".codegraph").join("codegraph.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let conn = Connection::open(&db_path).unwrap();
        populate_cg_schema(&conn);
        insert_cg_node(&conn, "cg1", "function", "foo", "src/a.rs");
        insert_cg_node(&conn, "cg2", "function", "bar", "src/b.rs");
        insert_cg_edge(&conn, "cg1", "cg2", "calls");
        drop(conn);

        let mut kg = KnowledgeGraph::new();
        let result = merge_codegraph_edges(&mut kg, tmp.path());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn merges_matching_edges() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join(".codegraph").join("codegraph.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let conn = Connection::open(&db_path).unwrap();
        populate_cg_schema(&conn);

        // CodeGraph has foo calling bar in src/lib.rs
        insert_cg_node(&conn, "cg_foo", "function", "foo", "src/lib.rs");
        insert_cg_node(&conn, "cg_bar", "function", "bar", "src/lib.rs");
        insert_cg_edge(&conn, "cg_foo", "cg_bar", "calls");
        drop(conn);

        // graphify-rs has same two functions
        let mut kg = KnowledgeGraph::new();
        kg.add_node(make_graph_node(
            "gf_foo",
            "foo",
            "src/lib.rs",
            NodeType::Function,
        ))
        .unwrap();
        kg.add_node(make_graph_node(
            "gf_bar",
            "bar",
            "src/lib.rs",
            NodeType::Function,
        ))
        .unwrap();

        let result = merge_codegraph_edges(&mut kg, tmp.path());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1);

        // Verify the edge has the correct source marker in extra
        let edges = kg.edges();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].relation, "calls");
        assert_eq!(edges[0].source, "gf_foo");
        assert_eq!(edges[0].target, "gf_bar");
        assert_eq!(
            edges[0].extra.get("merge_source").unwrap(),
            &serde_json::Value::String("codegraph".to_string())
        );
        assert_eq!(
            edges[0].extra.get("codegraph_kind").unwrap(),
            &serde_json::Value::String("calls".to_string())
        );
    }

    #[test]
    fn skips_contains_edges() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join(".codegraph").join("codegraph.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let conn = Connection::open(&db_path).unwrap();
        populate_cg_schema(&conn);

        insert_cg_node(&conn, "cg_mod", "module", "my_module", "src/lib.rs");
        insert_cg_node(&conn, "cg_foo", "function", "foo", "src/lib.rs");
        insert_cg_edge(&conn, "cg_mod", "cg_foo", "contains");
        drop(conn);

        let mut kg = KnowledgeGraph::new();
        kg.add_node(make_graph_node(
            "gf_mod",
            "my_module",
            "src/lib.rs",
            NodeType::Module,
        ))
        .unwrap();
        kg.add_node(make_graph_node(
            "gf_foo",
            "foo",
            "src/lib.rs",
            NodeType::Function,
        ))
        .unwrap();

        let result = merge_codegraph_edges(&mut kg, tmp.path());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn skips_duplicate_edges() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join(".codegraph").join("codegraph.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let conn = Connection::open(&db_path).unwrap();
        populate_cg_schema(&conn);

        insert_cg_node(&conn, "cg_foo", "function", "foo", "src/lib.rs");
        insert_cg_node(&conn, "cg_bar", "function", "bar", "src/lib.rs");
        insert_cg_edge(&conn, "cg_foo", "cg_bar", "calls");
        drop(conn);

        let mut kg = KnowledgeGraph::new();
        kg.add_node(make_graph_node(
            "gf_foo",
            "foo",
            "src/lib.rs",
            NodeType::Function,
        ))
        .unwrap();
        kg.add_node(make_graph_node(
            "gf_bar",
            "bar",
            "src/lib.rs",
            NodeType::Function,
        ))
        .unwrap();
        // graphify-rs already has this edge
        kg.add_edge(make_graph_edge("gf_foo", "gf_bar", "calls", "src/lib.rs"))
            .unwrap();

        let result = merge_codegraph_edges(&mut kg, tmp.path());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
        // Still only 1 edge total
        assert_eq!(kg.edge_count(), 1);
    }

    #[test]
    fn mixed_match_and_skip() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join(".codegraph").join("codegraph.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let conn = Connection::open(&db_path).unwrap();
        populate_cg_schema(&conn);

        // Three edges:
        // 1. foo calls bar - match
        // 2. bar calls baz - baz not in kg -> skip (no target)
        // 3. module contains foo -> skip (contains)
        insert_cg_node(&conn, "cg_foo", "function", "foo", "src/lib.rs");
        insert_cg_node(&conn, "cg_bar", "function", "bar", "src/lib.rs");
        insert_cg_node(&conn, "cg_baz", "function", "baz", "src/lib.rs");
        insert_cg_node(&conn, "cg_mod", "module", "my_mod", "src/lib.rs");
        insert_cg_edge(&conn, "cg_foo", "cg_bar", "calls");
        insert_cg_edge(&conn, "cg_bar", "cg_baz", "calls");
        insert_cg_edge(&conn, "cg_mod", "cg_foo", "contains");
        drop(conn);

        // Only foo and bar are in kg (not baz)
        let mut kg = KnowledgeGraph::new();
        kg.add_node(make_graph_node(
            "gf_foo",
            "foo",
            "src/lib.rs",
            NodeType::Function,
        ))
        .unwrap();
        kg.add_node(make_graph_node(
            "gf_bar",
            "bar",
            "src/lib.rs",
            NodeType::Function,
        ))
        .unwrap();

        let result = merge_codegraph_edges(&mut kg, tmp.path());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1);
        assert_eq!(kg.edge_count(), 1);
    }
}
