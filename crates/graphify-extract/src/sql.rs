//! SQL AST extraction using tree-sitter-sequel.
//!
//! Extracts DDL (tables, views), DML dependencies (FROM/JOIN), foreign keys,
//! and column-level lineage from plain SQL files.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use graphify_core::confidence::Confidence;
use graphify_core::id::make_id;
use graphify_core::model::{ExtractionResult, GraphEdge, GraphNode, NodeType};
use tracing::warn;
use tree_sitter::{Node, Parser};

/// Lineage of a derived column: the base-table `(catalog, schema, table, column)` sources
/// it ultimately reads from.
type ColLineage = Vec<(Option<String>, String, String, String)>;

/// A source visible in a query's `FROM` clause, keyed by the name used to qualify
/// columns (table name or alias).
#[derive(Clone)]
enum QuerySource {
    /// A concrete base table.
    Table {
        catalog: Option<String>,
        schema: String,
        table: String,
    },
    /// A derived source (CTE or subquery): output column name -> base lineage.
    Derived(HashMap<String, ColLineage>),
}

/// The set of sources visible to a single query block.
struct Scope {
    /// Lookup by table name and/or alias (a base table appears under both).
    by_key: HashMap<String, QuerySource>,
    /// One entry per `FROM`/`JOIN` source, used to resolve unqualified columns
    /// only when exactly one source is in scope.
    distinct: Vec<QuerySource>,
}

/// Main entry point for plain SQL extraction.
pub fn extract_sql(path: &Path, source: &str) -> ExtractionResult {
    let mut parser = Parser::new();
    let language = tree_sitter_sequel::LANGUAGE.into();
    if parser.set_language(&language).is_err() {
        warn!("failed to set tree-sitter-sequel language");
        return ExtractionResult::default();
    }

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return ExtractionResult::default(),
    };

    let mut result = ExtractionResult::default();

    // NOTE: Application nodes are created per-file here. When multiple SQL files
    // share the same parent directory, build() will deduplicate them using the
    // deterministic make_id, keeping the first node encountered.
    let app_name = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("default_sql_app")
        .to_string();

    let app_id = make_id(&["app", &app_name]);
    result.nodes.push(GraphNode {
        id: app_id.clone(),
        label: app_name.clone(),
        source_file: path.to_string_lossy().into_owned(),
        source_location: None,
        node_type: NodeType::Application,
        community: None,
        extra: HashMap::new(),
    });

    // File Node
    let file_id = make_id(&[
        "file",
        &path.to_string_lossy().into_owned().replace('/', "_"),
    ]);
    result.nodes.push(GraphNode {
        id: file_id.clone(),
        label: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        source_file: path.to_string_lossy().into_owned(),
        source_location: None,
        node_type: NodeType::File,
        community: None,
        extra: HashMap::new(),
    });

    let mut extractor = SqlExtractor {
        app_id,
        file_id,
        path: path.to_string_lossy().into_owned(),
        source: source.as_bytes(),
        result,
        defined_relations: HashSet::new(),
    };

    extractor.extract_pass1(tree.root_node());
    extractor.extract_pass2(tree.root_node());

    extractor.result
}

/// Extract dependencies and column-level lineage from a bare top-level `SELECT`
/// (e.g. a compiled dbt model), attributing everything to `target_rel_id`.
///
/// Compiled dbt models contain only the rendered `SELECT` — dbt injects the
/// surrounding DDL at run time — so the regular [`extract_sql`] entry point
/// (which anchors lineage on `CREATE`/`INSERT`/`MERGE` targets) extracts nothing
/// from them. This entry point instead treats every top-level statement that
/// contains a `SELECT` as the body of `target_rel_id` and emits:
///
/// - `depends_on` edges from `target_rel_id` to each referenced base table
///   (CTE references are excluded, self-references — `{{ this }}` in incremental
///   models — are skipped),
/// - `Column`/`Expression` nodes `part_of` `target_rel_id` with `derives_from`
///   edges traced through CTEs and inline subqueries.
///
/// Unlike [`extract_sql`], **no** `Application`, `File`, or `Relation` nodes are
/// emitted for the file itself: the caller (dbt manifest parsing) already owns
/// the Relation node.
pub fn extract_sql_lineage(path: &Path, source: &str, target_rel_id: &str) -> ExtractionResult {
    let mut parser = Parser::new();
    let language = tree_sitter_sequel::LANGUAGE.into();
    if parser.set_language(&language).is_err() {
        warn!("failed to set tree-sitter-sequel language");
        return ExtractionResult::default();
    }

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return ExtractionResult::default(),
    };

    let mut extractor = SqlExtractor {
        app_id: String::new(),
        file_id: String::new(),
        path: path.to_string_lossy().into_owned(),
        source: source.as_bytes(),
        result: ExtractionResult::default(),
        defined_relations: HashSet::new(),
    };

    extractor.extract_lineage(tree.root_node(), target_rel_id);
    extractor.result
}

struct SqlExtractor<'a> {
    app_id: String,
    file_id: String,
    path: String,
    source: &'a [u8],
    result: ExtractionResult,
    /// Relation IDs already defined in this file (CREATE TABLE/VIEW, DML targets),
    /// so DML target handling avoids an O(n²) scan over `result.nodes`.
    defined_relations: HashSet<String>,
}

impl<'a> SqlExtractor<'a> {
    fn extract_pass1(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "create_table" => {
                    self.handle_create_relation(child, "table");
                }
                "create_view" => {
                    self.handle_create_relation(child, "view");
                }
                // `insert` is nested inside a `statement` wrapper in tree-sitter-sequel.
                "insert" | "merge" => {
                    self.handle_dml_target(child);
                }
                // tree-sitter-sequel parses MERGE as a bare `statement` node whose
                // first keyword child is `keyword_merge` (no dedicated `merge` node).
                "statement"
                    if self
                        .find_child_by_kind_direct(child, "keyword_merge")
                        .is_some() =>
                {
                    self.handle_dml_target(child);
                }
                _ => self.extract_pass1(child),
            }
        }
    }

    fn extract_pass2(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "create_table" => {
                    // Check for AS SELECT (CTAS)
                    if self.has_select_child(child) {
                        self.handle_query_dependencies(child);
                    }
                    self.handle_fks(child);
                }
                "create_view" => {
                    self.handle_query_dependencies(child);
                }
                "alter_table" => {
                    self.handle_fks(child);
                }
                // `insert` is nested inside a `statement` wrapper in tree-sitter-sequel.
                "insert" | "merge" => {
                    self.handle_query_dependencies(child);
                }
                // tree-sitter-sequel parses MERGE as a bare `statement` node whose
                // first keyword child is `keyword_merge` (no dedicated `merge` node).
                "statement"
                    if self
                        .find_child_by_kind_direct(child, "keyword_merge")
                        .is_some() =>
                {
                    self.handle_merge_statement(child);
                }
                _ => self.extract_pass2(child),
            }
        }
    }

    /// Lineage-only walk for bare-SELECT sources (see [`extract_sql_lineage`]):
    /// every top-level `statement` containing a `select` is attributed to `rel_id`.
    fn extract_lineage(&mut self, root: Node, rel_id: &str) {
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() != "statement" || !self.has_select_child(child) {
                continue;
            }

            // depends_on edges to base tables (CTEs excluded, self-refs skipped).
            let mut cte_map = HashSet::new();
            self.collect_ctes(child, &mut cte_map);
            let mut deps = HashSet::new();
            self.collect_dependencies(child, &cte_map, &mut deps);
            for dep_id in deps {
                if dep_id != rel_id {
                    self.result
                        .edges
                        .push(self.make_edge(rel_id, &dep_id, "depends_on"));
                }
            }

            // Column lineage: the container is the statement itself when the
            // `select` is a direct child (bare SELECT / WITH … SELECT); otherwise
            // fall back to the generic container search.
            if self.find_child_by_kind_direct(child, "select").is_some() {
                self.extract_column_lineage_in(child, rel_id);
            } else {
                self.extract_column_lineage(child, rel_id);
            }
        }
    }

    fn handle_create_relation(&mut self, node: Node, kind: &str) {
        if let Some(obj_ref) = self.find_child_by_kind_direct(node, "object_reference") {
            let (catalog, schema, name) = self.parse_object_reference(obj_ref);
            let relation_id = make_id(&["rel", catalog.as_deref().unwrap_or(""), &schema, &name]);
            self.defined_relations.insert(relation_id.clone());

            // Add Relation node
            let mut extra = HashMap::new();
            extra.insert("relation_kind".to_string(), serde_json::json!(kind));
            if let Some(ref c) = catalog {
                extra.insert("catalog".to_string(), serde_json::json!(c));
            }

            self.result.nodes.push(GraphNode {
                id: relation_id.clone(),
                label: format_relation_label(catalog.as_deref(), &schema, &name),
                source_file: self.path.clone(),
                source_location: Some(format!("L{}", node.start_position().row + 1)),
                node_type: NodeType::Relation,
                community: None,
                extra,
            });

            // File defines Relation
            self.result
                .edges
                .push(self.make_edge(&self.file_id, &relation_id, "defines"));

            // Relation part_of Application
            self.result
                .edges
                .push(self.make_edge(&relation_id, &self.app_id, "part_of"));
        }
    }

    /// Create a [`NodeType::Relation`] node for the INSERT/MERGE target table if
    /// one has not already been defined by a `CREATE TABLE`/`CREATE VIEW` in the
    /// same file.  Also emits `File defines Relation` and `Relation part_of
    /// Application` edges so the node is properly wired into the graph.
    fn handle_dml_target(&mut self, node: Node) {
        if let Some(obj_ref) = self.find_first_object_reference(node) {
            let (catalog, schema, name) = self.parse_object_reference(obj_ref);
            let relation_id = make_id(&["rel", catalog.as_deref().unwrap_or(""), &schema, &name]);

            // Only create if not already defined (e.g., CREATE TABLE in same file).
            if self.defined_relations.insert(relation_id.clone()) {
                let mut extra = HashMap::new();
                extra.insert("relation_kind".to_string(), serde_json::json!("table"));
                if let Some(ref c) = catalog {
                    extra.insert("catalog".to_string(), serde_json::json!(c));
                }

                self.result.nodes.push(GraphNode {
                    id: relation_id.clone(),
                    label: format_relation_label(catalog.as_deref(), &schema, &name),
                    source_file: self.path.clone(),
                    source_location: Some(format!("L{}", node.start_position().row + 1)),
                    node_type: NodeType::Relation,
                    community: None,
                    extra,
                });

                self.result
                    .edges
                    .push(self.make_edge(&self.file_id, &relation_id, "defines"));
                self.result
                    .edges
                    .push(self.make_edge(&relation_id, &self.app_id, "part_of"));
            }
        }
    }

    /// Handle MERGE statements whose tree structure in tree-sitter-sequel is a flat
    /// `statement` node (no dedicated `merge` child).  Emits `depends_on` edges from
    /// the MERGE target to every table referenced in the `USING` clause.
    fn handle_merge_statement(&mut self, node: Node) {
        let Some(rel_id) = self.get_enclosing_relation(node) else {
            return;
        };

        // Collect `object_reference` nodes that appear after `keyword_using`.
        // These are the source tables in the MERGE USING clause.
        let mut cursor = node.walk();
        let mut after_using = false;
        for child in node.children(&mut cursor) {
            if child.kind() == "keyword_using" {
                after_using = true;
            } else if after_using && child.kind() == "object_reference" {
                let (catalog, schema, name) = self.parse_object_reference(child);
                let dep_id = make_id(&["rel", catalog.as_deref().unwrap_or(""), &schema, &name]);
                self.result
                    .edges
                    .push(self.make_edge(&rel_id, &dep_id, "depends_on"));
                // Only the first object_reference after USING is the source table.
                after_using = false;
            }
        }
    }

    fn handle_query_dependencies(&mut self, node: Node) {
        // Build CTE map
        let mut cte_map = HashSet::new();
        self.collect_ctes(node, &mut cte_map);

        // Figure out enclosing relation if any
        let enclosing_rel_id = self.get_enclosing_relation(node);

        // Find dependencies
        let mut deps = HashSet::new();
        self.collect_dependencies(node, &cte_map, &mut deps);

        if let Some(rel_id) = &enclosing_rel_id {
            for dep_id in deps {
                self.result
                    .edges
                    .push(self.make_edge(rel_id, &dep_id, "depends_on"));
            }

            // Column-level lineage: trace each output column to its base-table
            // source columns, seeing through CTEs and subqueries.
            self.extract_column_lineage(node, rel_id);
        }
    }

    fn collect_ctes(&self, node: Node, cte_map: &mut HashSet<String>) {
        if node.kind() == "cte"
            && let Some(identifier) = self.find_child_by_kind(node, "identifier")
        {
            let name = self.node_text(identifier).to_lowercase();
            cte_map.insert(name);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.collect_ctes(child, cte_map);
        }
    }

    fn collect_dependencies(
        &mut self,
        node: Node,
        cte_map: &HashSet<String>,
        deps: &mut HashSet<String>,
    ) {
        if node.kind() == "relation"
            && let Some(obj_ref) = self.find_child_by_kind(node, "object_reference")
        {
            let (catalog, schema, name) = self.parse_object_reference(obj_ref);
            if schema.is_empty() && cte_map.contains(&name.to_lowercase()) {
                // It's a CTE, skip
            } else {
                let dep_id = make_id(&["rel", catalog.as_deref().unwrap_or(""), &schema, &name]);
                deps.insert(dep_id);
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.collect_dependencies(child, cte_map, deps);
        }
    }

    /// Handles FK extraction for both `create_table` and `alter_table` nodes.
    ///
    /// Resolves the enclosing relation, then recursively walks descendants to find
    /// `column_definition`, `table_constraint`, `add_constraint`, and `constraint`
    /// nodes, delegating FK edge extraction to [`Self::extract_fk_references`].
    fn handle_fks(&mut self, node: Node) {
        let Some(rel_id) = self.get_enclosing_relation(node) else {
            return;
        };
        self.walk_for_fk_nodes(node, &rel_id);
    }

    fn walk_for_fk_nodes(&mut self, node: Node, rel_id: &str) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "column_definition" | "table_constraint" | "add_constraint" | "constraint" => {
                    self.extract_fk_references(child, rel_id);
                }
                _ => self.walk_for_fk_nodes(child, rel_id),
            }
        }
    }

    /// Walk `node` looking for a `REFERENCES` keyword (or a child whose text is
    /// `"references"`) followed by an `object_reference` sibling, and emit a
    /// `references` edge when found.  Iterates children **once**, recursing into
    /// any child that is not itself the references trigger.
    fn extract_fk_references(&mut self, node: Node, source_rel_id: &str) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let kind = child.kind();
            let text = self.node_text(child).to_lowercase();
            if kind == "keyword_references" || text == "references" {
                // Walk forward siblings to find the referenced object_reference.
                let mut next = child.next_sibling();
                while let Some(n) = next {
                    if n.kind() == "object_reference" {
                        let (catalog, schema, name) = self.parse_object_reference(n);
                        let target_id =
                            make_id(&["rel", catalog.as_deref().unwrap_or(""), &schema, &name]);
                        self.result.edges.push(self.make_edge(
                            source_rel_id,
                            &target_id,
                            "references",
                        ));
                        return;
                    }
                    next = n.next_sibling();
                }
                return;
            }
            self.extract_fk_references(child, source_rel_id);
        }
    }

    /// Emit `Column`/`Expression` nodes and `derives_from` edges for the
    /// top-level SELECT of the statement writing to `rel_id` (CREATE VIEW, CTAS,
    /// INSERT…SELECT). Lineage is traced through CTEs and subqueries down to the
    /// underlying base-table columns via [`Self::resolve_parts`].
    fn extract_column_lineage(&mut self, stmt_node: Node, rel_id: &str) {
        let Some(container) = self.find_query_container(stmt_node) else {
            return;
        };
        self.extract_column_lineage_in(container, rel_id);
    }

    /// Same as [`Self::extract_column_lineage`], but takes the already-resolved
    /// query container directly. Used by the bare-SELECT lineage path (compiled
    /// dbt models) where the container is the top-level `statement` node itself
    /// and `find_query_container`'s recursive `select` search could otherwise
    /// land inside a CTE body.
    fn extract_column_lineage_in(&mut self, container: Node, rel_id: &str) {
        let scope = self.collect_sources(container);

        let Some(select) = self.find_child_by_kind_direct(container, "select") else {
            return;
        };
        let Some(select_expr) = self.find_child_by_kind_direct(select, "select_expression") else {
            return;
        };

        let mut cursor = select_expr.walk();
        let terms: Vec<Node> = select_expr
            .children(&mut cursor)
            .filter(|c| c.kind() == "term")
            .collect();

        for (index, term) in terms.into_iter().enumerate() {
            let Some((alias, expr, is_column)) = self.parse_term(term) else {
                continue;
            };
            // SELECT * → cannot enumerate columns without schema; skip.
            if expr.kind() == "all_fields" {
                continue;
            }

            let col_name = alias.unwrap_or_else(|| format!("select_{index}"));
            let node_type = if is_column {
                NodeType::Column
            } else {
                NodeType::Expression
            };
            let node_prefix = if is_column { "col" } else { "expr" };
            let node_id = make_id(&[node_prefix, rel_id, &col_name]);

            self.result.nodes.push(GraphNode {
                id: node_id.clone(),
                label: col_name.clone(),
                source_file: self.path.clone(),
                source_location: Some(format!("L{}", expr.start_position().row + 1)),
                node_type,
                community: None,
                extra: HashMap::new(),
            });

            self.result
                .edges
                .push(self.make_edge(&node_id, rel_id, "part_of"));

            // Resolve lineage to base-table columns and emit derives_from edges.
            let mut fields = Vec::new();
            self.collect_fields(expr, &mut fields);
            let mut seen = HashSet::new();
            for field_node in fields {
                let parts = self.parse_field(field_node);
                for (catalog, schema, table, col) in self.resolve_parts(&parts, &scope) {
                    let source_rel_id =
                        make_id(&["rel", catalog.as_deref().unwrap_or(""), &schema, &table]);
                    let source_col_id = make_id(&["col", &source_rel_id, &col]);
                    if seen.insert(source_col_id.clone()) {
                        self.result.edges.push(self.make_edge(
                            &node_id,
                            &source_col_id,
                            "derives_from",
                        ));
                    }
                }
            }
        }
    }

    fn collect_fields(&self, node: Node<'a>, fields: &mut Vec<Node<'a>>) {
        if node.kind() == "field" {
            fields.push(node);
        } else {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.collect_fields(child, fields);
            }
        }
    }

    fn parse_field(&self, node: Node) -> Vec<String> {
        // A qualified field like `t.col1` is represented as:
        //   field[ object_reference(t), '.', identifier(col1) ]
        // where the table/alias qualifier lives inside `object_reference`.
        // We flatten both `object_reference` identifiers and bare `identifier`
        // children into a single parts list so that `resolve_field` can handle
        // both unqualified (`["col"]`) and qualified (`["alias", "col"]`) forms.
        let mut parts = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "identifier" => parts.push(self.node_text(child).to_lowercase()),
                "object_reference" => {
                    // Extract all identifiers from the qualifier (e.g. schema.table or alias).
                    let mut c = child.walk();
                    for gc in child.children(&mut c) {
                        if gc.kind() == "identifier" {
                            parts.push(self.node_text(gc).to_lowercase());
                        }
                    }
                }
                _ => {}
            }
        }
        parts
    }

    /// Locate the query container (`create_query` for views/CTAS, otherwise the
    /// parent of the first `select`) whose direct `select`/`from` children form
    /// the top-level query of `stmt_node`.
    fn find_query_container<'n>(&self, stmt_node: Node<'n>) -> Option<Node<'n>> {
        if let Some(cq) = self.find_child_by_kind(stmt_node, "create_query") {
            return Some(cq);
        }
        // INSERT … SELECT and similar: the container is the select's parent.
        self.find_child_by_kind(stmt_node, "select")
            .and_then(|s| s.parent())
    }

    /// Extract the output `(alias, expression, is_column)` of a `term`.
    ///
    /// The expression is the term's first child. The alias is the `identifier`
    /// directly following `keyword_as`, or — for implicit aliasing (`expr name`)
    /// — a trailing `identifier`. `is_column` is true for bare `field`/`cast`
    /// expressions (real columns) and false for computed expressions.
    fn parse_term<'n>(&self, term: Node<'n>) -> Option<(Option<String>, Node<'n>, bool)> {
        let mut cursor = term.walk();
        let children: Vec<Node> = term.children(&mut cursor).collect();
        let expr = *children.first()?;

        let mut alias = None;
        if let Some(pos) = children.iter().position(|c| c.kind() == "keyword_as") {
            if let Some(id) = children.get(pos + 1)
                && (id.kind() == "identifier" || id.kind() == "alias")
            {
                alias = Some(self.node_text(*id));
            }
        } else if children.len() >= 2 {
            let last = children[children.len() - 1];
            if last.kind() == "identifier" || last.kind() == "alias" {
                alias = Some(self.node_text(last));
            }
        }

        let is_column = expr.kind() == "field" || expr.kind() == "cast";
        Some((alias, expr, is_column))
    }

    /// The table alias of a `relation` node: the first direct `identifier`/`alias`
    /// child (the qualifier identifiers live nested inside `object_reference`).
    fn relation_alias(&self, rel: Node) -> Option<String> {
        let mut cursor = rel.walk();
        rel.children(&mut cursor)
            .find(|c| c.kind() == "identifier" || c.kind() == "alias")
            .map(|c| self.node_text(c).to_lowercase())
    }

    /// Collect the FROM-level `relation` nodes of a query container, descending
    /// into `join` children (whose `relation` is one level deeper) but not into
    /// nested subqueries.
    fn collect_from_relations<'n>(&self, from: Node<'n>) -> Vec<Node<'n>> {
        let mut rels = Vec::new();
        let mut cursor = from.walk();
        for child in from.children(&mut cursor) {
            match child.kind() {
                "relation" => rels.push(child),
                "join" => {
                    let mut jc = child.walk();
                    for gc in child.children(&mut jc) {
                        if gc.kind() == "relation" {
                            rels.push(gc);
                        }
                    }
                }
                _ => {}
            }
        }
        rels
    }

    /// Build the [`Scope`] of sources visible to a query container: CTEs declared
    /// in its `WITH`, plus tables/subqueries in its `FROM`/`JOIN`s.
    fn collect_sources(&self, container: Node) -> Scope {
        let mut by_key: HashMap<String, QuerySource> = HashMap::new();
        let mut distinct: Vec<QuerySource> = Vec::new();

        // CTEs declared on this container, keyed by CTE name.
        let mut cte_defs: HashMap<String, HashMap<String, ColLineage>> = HashMap::new();
        let mut cursor = container.walk();
        let cte_nodes: Vec<Node> = container
            .children(&mut cursor)
            .filter(|c| c.kind() == "cte")
            .collect();
        for cte in cte_nodes {
            let Some(name_node) = self.find_child_by_kind_direct(cte, "identifier") else {
                continue;
            };
            let name = self.node_text(name_node).to_lowercase();
            // The CTE body is a `statement` holding select/from siblings.
            let inner = self
                .find_child_by_kind_direct(cte, "statement")
                .unwrap_or(cte);
            let outs = self.derive_outputs(inner);
            cte_defs.insert(name, outs);
        }

        if let Some(from) = self.find_child_by_kind_direct(container, "from") {
            for rel in self.collect_from_relations(from) {
                if let Some(sub) = self.find_child_by_kind_direct(rel, "subquery") {
                    // Inline subquery: derive its output columns and key by alias.
                    let outs = self.derive_outputs(sub);
                    if let Some(alias) = self.relation_alias(rel) {
                        let src = QuerySource::Derived(outs);
                        by_key.insert(alias, src.clone());
                        distinct.push(src);
                    }
                } else if let Some(obj) = self.find_child_by_kind_direct(rel, "object_reference") {
                    let (catalog, schema, name) = self.parse_object_reference(obj);
                    let alias = self.relation_alias(rel);
                    if schema.is_empty()
                        && let Some(outs) = cte_defs.get(&name)
                    {
                        // Reference to a previously-declared CTE.
                        let src = QuerySource::Derived(outs.clone());
                        by_key.insert(alias.unwrap_or(name), src.clone());
                        distinct.push(src);
                    } else {
                        let src = QuerySource::Table {
                            catalog,
                            schema,
                            table: name.clone(),
                        };
                        // Look up by alias and by table name; count once as distinct.
                        if let Some(a) = alias {
                            by_key.insert(a, src.clone());
                        }
                        by_key.insert(name, src.clone());
                        distinct.push(src);
                    }
                }
            }
        }

        Scope { by_key, distinct }
    }

    /// Compute the output columns of a sub-query container (CTE body or inline
    /// subquery): a map of output column name to its base-table lineage.
    fn derive_outputs(&self, container: Node) -> HashMap<String, ColLineage> {
        let scope = self.collect_sources(container);
        let mut outs = HashMap::new();

        let Some(select) = self.find_child_by_kind_direct(container, "select") else {
            return outs;
        };
        let Some(select_expr) = self.find_child_by_kind_direct(select, "select_expression") else {
            return outs;
        };

        let mut cursor = select_expr.walk();
        let terms: Vec<Node> = select_expr
            .children(&mut cursor)
            .filter(|c| c.kind() == "term")
            .collect();

        for (index, term) in terms.into_iter().enumerate() {
            let Some((alias, expr, _is_column)) = self.parse_term(term) else {
                continue;
            };
            if expr.kind() == "all_fields" {
                continue;
            }
            let col_name = alias.unwrap_or_else(|| format!("select_{index}"));

            let mut fields = Vec::new();
            self.collect_fields(expr, &mut fields);
            let mut lineage = Vec::new();
            for field_node in fields {
                let parts = self.parse_field(field_node);
                lineage.extend(self.resolve_parts(&parts, &scope));
            }
            outs.insert(col_name, lineage);
        }
        outs
    }

    /// Resolve a field's `parts` against a [`Scope`] to a list of base-table
    /// `(catalog, schema, table, column)` sources, expanding CTE/subquery references.
    ///
    /// Returns an empty list when the source is unresolvable — e.g. an unqualified
    /// column with more than one source in scope (genuinely ambiguous) — so the
    /// caller emits no `derives_from` edge and no placeholder node.
    fn resolve_parts(&self, parts: &[String], scope: &Scope) -> ColLineage {
        if parts.len() >= 2 {
            let col = parts.last().unwrap().clone();
            let qualifier = parts[parts.len() - 2].clone();
            match scope.by_key.get(&qualifier) {
                Some(QuerySource::Table {
                    catalog,
                    schema,
                    table,
                }) => {
                    vec![(catalog.clone(), schema.clone(), table.clone(), col)]
                }
                Some(QuerySource::Derived(outs)) => outs.get(&col).cloned().unwrap_or_default(),
                // Qualified by a name not in FROM (defensive): treat as a real table.
                None => {
                    if parts.len() >= 3 {
                        vec![(None, parts[parts.len() - 3].clone(), qualifier, col)]
                    } else {
                        vec![(None, String::new(), qualifier, col)]
                    }
                }
            }
        } else if parts.len() == 1 {
            let col = parts[0].clone();
            // Resolvable only when exactly one source is in scope.
            if scope.distinct.len() == 1 {
                match &scope.distinct[0] {
                    QuerySource::Table {
                        catalog,
                        schema,
                        table,
                    } => {
                        vec![(catalog.clone(), schema.clone(), table.clone(), col)]
                    }
                    QuerySource::Derived(outs) => outs.get(&col).cloned().unwrap_or_default(),
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    }

    fn get_enclosing_relation(&self, node: Node) -> Option<String> {
        let mut current = Some(node);
        while let Some(n) = current {
            if n.kind() == "create_table" || n.kind() == "create_view" || n.kind() == "alter_table"
            {
                if let Some(obj_ref) = self.find_child_by_kind_direct(n, "object_reference") {
                    let (catalog, schema, name) = self.parse_object_reference(obj_ref);
                    return Some(make_id(&[
                        "rel",
                        catalog.as_deref().unwrap_or(""),
                        &schema,
                        &name,
                    ]));
                }
            } else if n.kind() == "insert"
                || n.kind() == "merge"
                || (n.kind() == "statement"
                    && self.find_child_by_kind_direct(n, "keyword_merge").is_some())
            {
                // Use find_first_object_reference to avoid descending into the SELECT/USING body.
                if let Some(obj_ref) = self.find_first_object_reference(n) {
                    let (catalog, schema, name) = self.parse_object_reference(obj_ref);
                    return Some(make_id(&[
                        "rel",
                        catalog.as_deref().unwrap_or(""),
                        &schema,
                        &name,
                    ]));
                }
            }
            current = n.parent();
        }
        None
    }

    /// Parses an `object_reference` node into `(catalog, schema, name)`.
    ///
    /// Note on 3-part names (`catalog.schema.name`):
    /// The relation ID is intentionally derived only from `schema.name` to merge
    /// representations of identical tables across files. If two relations share
    /// `schema.name` but differ in `catalog`, they will collide on the same ID.
    /// This is an acceptable tradeoff for typical single-catalog warehouses.
    /// The `catalog` is returned here so it can be preserved in the `extra` metadata.
    fn parse_object_reference(&self, node: Node) -> (Option<String>, String, String) {
        let mut parts = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                parts.push(self.node_text(child).to_lowercase());
            }
        }

        match parts.as_slice() {
            [catalog, schema, name] | [.., catalog, schema, name] => {
                (Some(catalog.clone()), schema.clone(), name.clone())
            }
            [schema, name] => (None, schema.clone(), name.clone()),
            [name] => (None, String::new(), name.clone()),
            [] => (None, String::new(), String::new()),
        }
    }

    fn has_select_child(&self, node: Node) -> bool {
        if node.kind() == "select" {
            return true;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if self.has_select_child(child) {
                return true;
            }
        }
        false
    }

    /// Find a direct (non-recursive) child of `node` matching `kind`.
    /// Use this where only immediate children should be searched, e.g., for
    /// `object_reference` under `create_table`/`create_view`.
    fn find_child_by_kind_direct<'n>(&self, node: Node<'n>, kind: &str) -> Option<Node<'n>> {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .find(|&child| child.kind() == kind)
    }

    /// Find the first `object_reference` descendant of `node`, **skipping**
    /// `select` and `subquery` subtrees so that source-table references in
    /// the SELECT body are not mistaken for the DML write target.
    fn find_first_object_reference<'n>(&self, node: Node<'n>) -> Option<Node<'n>> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "object_reference" {
                return Some(child);
            }
            // Don't descend into select/subquery — those are sources, not targets.
            if child.kind() != "select"
                && child.kind() != "subquery"
                && let Some(found) = self.find_first_object_reference(child)
            {
                return Some(found);
            }
        }
        None
    }

    fn find_child_by_kind<'n>(&self, node: Node<'n>, kind: &str) -> Option<Node<'n>> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == kind {
                return Some(child);
            }
            if let Some(found) = self.find_child_by_kind(child, kind) {
                return Some(found);
            }
        }
        None
    }

    fn node_text(&self, node: Node) -> String {
        let start = node.start_byte();
        let end = node.end_byte();
        String::from_utf8_lossy(&self.source[start..end]).into_owned()
    }

    fn make_edge(&self, source: &str, target: &str, relation: &str) -> GraphEdge {
        make_sql_edge(source, target, relation, &self.path)
    }
}

/// The `extra` key used to tag edges produced by SQL/dbt extraction. Only edges
/// carrying this marker are considered by [`resolve_sql_cross_file`], so cross-file
/// stub creation never touches edges from other extractors (e.g. the semantic
/// extractor also emits `depends_on`).
pub(crate) const SQL_EDGE_ORIGIN: &str = "sql";

pub(crate) fn format_relation_label(catalog: Option<&str>, schema: &str, name: &str) -> String {
    match (catalog.filter(|c| !c.is_empty()), schema.is_empty()) {
        (Some(c), false) => format!("{}.{}.{}", c, schema, name),
        (Some(c), true) => format!("{}.{}", c, name),
        (None, false) => format!("{}.{}", schema, name),
        (None, true) => name.to_string(),
    }
}

/// Build a `GraphEdge` tagged with `origin = "sql"` so the cross-file resolver can
/// scope itself to SQL/dbt edges only.
pub(crate) fn make_sql_edge(
    source: &str,
    target: &str,
    relation: &str,
    source_file: &str,
) -> GraphEdge {
    let mut extra = HashMap::new();
    extra.insert("origin".to_string(), serde_json::json!(SQL_EDGE_ORIGIN));
    GraphEdge {
        source: source.to_string(),
        target: target.to_string(),
        relation: relation.to_string(),
        confidence: Confidence::Extracted,
        confidence_score: 1.0,
        source_file: source_file.to_string(),
        source_location: None,
        weight: 1.0,
        provenance: None,
        extra,
    }
}

/// Resolve cross-file SQL references.
///
/// After all SQL files are extracted, walks every `depends_on`, `references`, and
/// `derives_from` edge. For any target ID that is not already a known node, creates
/// a stub node so the graph has no dangling edge targets:
///
/// - `rel_*` targets → stub `Relation` node with `relation_kind = "stub"`
/// - `col_*` targets → stub `Column` node
///
/// Stub labels are wrapped in `[external: …]` / `[unknown column: …]` markers (the
/// IDs are readable slugs from `make_id`, e.g. `rel_staging_orders`) and carry
/// `extra["stub"] = true`; they exist to keep the graph structurally valid for
/// downstream consumers while staying distinguishable from real database objects.
///
/// **Scope caveat:** stubs are minted against the nodes visible in `result` at call
/// time. When extraction runs per-file (as in the CLI build pipeline), callers must
/// strip stubs after merging (see [`is_sql_stub`]) and re-run this resolver once
/// over the combined result, otherwise a stub from one file can shadow the real
/// node defined in another.
pub fn resolve_sql_cross_file(result: &mut ExtractionResult) {
    let defined_ids: HashSet<String> = result.nodes.iter().map(|n| n.id.clone()).collect();

    let mut stubs = Vec::new();
    let mut seen_stubs = HashSet::new();

    for edge in &result.edges {
        let is_sql_edge =
            edge.extra.get("origin").and_then(|v| v.as_str()) == Some(SQL_EDGE_ORIGIN);
        if is_sql_edge
            && (edge.relation == "depends_on"
                || edge.relation == "references"
                || edge.relation == "derives_from")
        {
            let target_id = &edge.target;
            if target_id.starts_with("rel_")
                && !defined_ids.contains(target_id)
                && !seen_stubs.contains(target_id)
            {
                seen_stubs.insert(target_id.clone());
                // Stub label is clearly synthetic so downstream consumers can distinguish
                // placeholder nodes from real database objects.
                stubs.push(GraphNode {
                    id: target_id.clone(),
                    label: format!("[external: {}]", target_id.trim_start_matches("rel_")),
                    source_file: "unknown".to_string(),
                    source_location: None,
                    node_type: NodeType::Relation,
                    community: None,
                    extra: {
                        let mut extra = HashMap::new();
                        extra.insert("relation_kind".to_string(), serde_json::json!("stub"));
                        extra.insert("stub".to_string(), serde_json::json!(true));
                        extra
                    },
                });
            } else if target_id.starts_with("col_")
                && !defined_ids.contains(target_id)
                && !seen_stubs.contains(target_id)
            {
                seen_stubs.insert(target_id.clone());
                stubs.push(GraphNode {
                    id: target_id.clone(),
                    label: format!("[unknown column: {}]", target_id.trim_start_matches("col_")),
                    source_file: "unknown".to_string(),
                    source_location: None,
                    node_type: NodeType::Column,
                    community: None,
                    extra: {
                        let mut extra = HashMap::new();
                        extra.insert("stub".to_string(), serde_json::json!(true));
                        extra
                    },
                });
            }
        }
    }

    result.nodes.extend(stubs);
}

/// Returns `true` if `node` is a synthetic stub minted by [`resolve_sql_cross_file`].
///
/// Stubs are placeholder nodes created for edge targets that could not be resolved
/// within the current extraction scope. When per-file extraction results are merged
/// (e.g. in the CLI build pipeline), stubs must be stripped and resolution re-run
/// against the full merged graph — otherwise a stub from one file can shadow the
/// real node defined in another file (node dedup is first-write-wins).
///
/// Matches both the explicit `extra["stub"] = true` marker and legacy forms from
/// cache entries written before the marker existed (`relation_kind == "stub"`, or
/// a `Column` node with `source_file == "unknown"`).
pub fn is_sql_stub(node: &GraphNode) -> bool {
    if node.extra.get("stub").and_then(|v| v.as_bool()) == Some(true) {
        return true;
    }
    // Legacy cache forms (pre-marker):
    if node.extra.get("relation_kind").and_then(|v| v.as_str()) == Some("stub") {
        return true;
    }
    node.node_type == NodeType::Column && node.source_file == "unknown"
}

// NOTE: For integration-level testing of SQL routing through the main extract()
// pipeline, see tests/ast_extract.rs::sql_routes_through_extract_pipeline()
//
// Unit tests below test the comprehensive extraction logic directly.
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_extract_sql_relation() {
        let sql = "CREATE TABLE schema.my_table (id INT); CREATE VIEW my_view AS SELECT * FROM schema.my_table;";
        let path = PathBuf::from("my_app/test.sql");
        let result = extract_sql(&path, sql);

        let table_node = result
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Relation && n.label == "schema.my_table");
        assert!(table_node.is_some());
        assert_eq!(
            table_node.unwrap().extra.get("relation_kind").unwrap(),
            "table"
        );

        let view_node = result
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Relation && n.label == "my_view");
        assert!(view_node.is_some());
        assert_eq!(
            view_node.unwrap().extra.get("relation_kind").unwrap(),
            "view"
        );

        let defines_table = result
            .edges
            .iter()
            .find(|e| e.relation == "defines" && e.target == table_node.unwrap().id);
        assert!(defines_table.is_some());
    }

    #[test]
    fn test_extract_sql_fk_deps() {
        let sql = "
        CREATE TABLE my_table (
            id INT PRIMARY KEY,
            other_id INT REFERENCES other_table(id)
        );
        ";
        let path = PathBuf::from("my_app/test.sql");
        let result = extract_sql(&path, sql);

        let refs = result.edges.iter().find(|e| e.relation == "references");
        assert!(refs.is_some());
    }

    #[test]
    fn test_extract_sql_ctas() {
        let sql = "CREATE TABLE new_table AS SELECT a, b FROM old_table;";
        let path = PathBuf::from("my_app/test.sql");
        let result = extract_sql(&path, sql);

        let table_node = result.nodes.iter().find(|n| n.label == "new_table");
        assert!(table_node.is_some());

        let deps = result
            .edges
            .iter()
            .filter(|e| e.relation == "depends_on")
            .collect::<Vec<_>>();
        assert!(!deps.is_empty());
    }

    #[test]
    fn test_extract_sql_column_lineage() {
        let sql =
            "CREATE VIEW my_view AS SELECT a, b AS beta, CAST(c AS INT) as charlie FROM my_table;";
        let path = PathBuf::from("my_app/test.sql");
        let result = extract_sql(&path, sql);

        let cols = result
            .nodes
            .iter()
            .filter(|n| n.node_type == NodeType::Column || n.node_type == NodeType::Expression)
            .collect::<Vec<_>>();
        assert!(cols.len() >= 3);

        // Check part_of edges
        let part_of = result
            .edges
            .iter()
            .filter(|e| e.relation == "part_of")
            .collect::<Vec<_>>();
        assert!(!part_of.is_empty());
    }

    // C8 — Strengthened: verify specific extraction despite EXASOL-style parse errors.
    #[test]
    fn test_extract_sql_exasol_error_recovery() {
        // Multi-statement file: the first statement uses EXASOL-style OR REPLACE (may produce
        // parse errors); the second is standard SQL and must be extracted correctly regardless.
        let sql = "
            CREATE OR REPLACE TABLE my_schema.err_table (id INT);
            CREATE TABLE my_schema.good_table (id INT, name VARCHAR(100));
        ";
        let path = PathBuf::from("my_app/test.sql");
        let result = extract_sql(&path, sql);

        // good_table must be extracted as a proper Relation regardless of parse errors in err_table.
        let good_table = result
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Relation && n.label == "my_schema.good_table");
        assert!(
            good_table.is_some(),
            "good_table Relation should be extracted despite parse errors in err_table"
        );
        assert_eq!(
            good_table.unwrap().extra.get("relation_kind").unwrap(),
            "table",
            "good_table should have relation_kind = 'table'"
        );
    }

    // C1 — ALTER TABLE ADD FOREIGN KEY → `references` edge.
    #[test]
    fn test_alter_table_add_fk_references_edge() {
        let sql = "
            CREATE TABLE orders (id INT);
            CREATE TABLE items (id INT, order_id INT);
            ALTER TABLE items ADD FOREIGN KEY (order_id) REFERENCES orders(id);
        ";
        let path = PathBuf::from("my_app/test.sql");
        let result = extract_sql(&path, sql);

        let orders_node = result
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Relation && n.label == "orders");
        let items_node = result
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Relation && n.label == "items");
        assert!(orders_node.is_some(), "orders Relation should exist");
        assert!(items_node.is_some(), "items Relation should exist");

        let refs_edge = result.edges.iter().find(|e| {
            e.relation == "references"
                && e.source == items_node.unwrap().id
                && e.target == orders_node.unwrap().id
        });
        assert!(
            refs_edge.is_some(),
            "references edge from items to orders should exist"
        );
    }

    // C2 — FROM/JOIN → two `depends_on` edges.
    #[test]
    fn test_view_from_join_depends_on_edges() {
        let sql = "CREATE VIEW report AS SELECT o.id, i.name FROM orders o JOIN items i ON o.id = i.order_id;";
        let path = PathBuf::from("my_app/test.sql");
        let result = extract_sql(&path, sql);

        let report_node = result
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Relation && n.label == "report");
        assert!(report_node.is_some(), "report Relation should exist");

        let report_id = &report_node.unwrap().id;
        let orders_id = make_id(&["rel", "", "orders"]);
        let items_id = make_id(&["rel", "", "items"]);

        let dep_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.relation == "depends_on" && &e.source == report_id)
            .collect();
        assert!(
            dep_edges.iter().any(|e| e.target == orders_id),
            "report should depend_on orders"
        );
        assert!(
            dep_edges.iter().any(|e| e.target == items_id),
            "report should depend_on items"
        );
        assert!(
            dep_edges.len() >= 2,
            "at least 2 depends_on edges from report"
        );
    }

    // C3 — INSERT INTO … SELECT → `depends_on`.
    #[test]
    fn test_insert_select_depends_on() {
        let sql = "
            CREATE TABLE orders (id INT);
            CREATE TABLE archive (id INT);
            INSERT INTO archive SELECT id FROM orders;
        ";
        let path = PathBuf::from("my_app/test.sql");
        let result = extract_sql(&path, sql);

        let archive_node = result
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Relation && n.label == "archive");
        assert!(archive_node.is_some(), "archive Relation should exist");

        let orders_id = make_id(&["rel", "", "orders"]);
        let dep_edge = result.edges.iter().find(|e| {
            e.relation == "depends_on"
                && e.source == archive_node.unwrap().id
                && e.target == orders_id
        });
        assert!(
            dep_edge.is_some(),
            "depends_on edge from archive to orders should exist"
        );
    }

    // C4 — Forward reference: FK to a table defined *later* in the same file.
    #[test]
    fn test_fk_forward_reference_to_later_defined_table() {
        let sql = "
            CREATE TABLE items (id INT, order_id INT REFERENCES orders(id));
            CREATE TABLE orders (id INT);
        ";
        let path = PathBuf::from("my_app/test.sql");
        let result = extract_sql(&path, sql);

        let orders_node = result
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Relation && n.label == "orders");
        let items_node = result
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Relation && n.label == "items");
        assert!(orders_node.is_some(), "orders Relation should exist");
        assert!(items_node.is_some(), "items Relation should exist");

        let refs_edge = result.edges.iter().find(|e| {
            e.relation == "references"
                && e.source == items_node.unwrap().id
                && e.target == orders_node.unwrap().id
        });
        assert!(
            refs_edge.is_some(),
            "references edge from items to orders should exist (forward reference)"
        );

        // orders must be a proper table, not a stub (it is defined in the same file).
        assert_eq!(
            orders_node
                .unwrap()
                .extra
                .get("relation_kind")
                .and_then(|v| v.as_str()),
            Some("table"),
            "orders should be a proper table, not a stub"
        );
    }

    // C5 — Column vs Expression classification.
    #[test]
    fn test_column_vs_expression_classification() {
        let sql = "CREATE VIEW v AS SELECT a, a + b AS sum_ab, UPPER(name) AS upper_name FROM t;";
        let path = PathBuf::from("my_app/test.sql");
        let result = extract_sql(&path, sql);

        // sum_ab is a binary expression → Expression.
        let sum_node = result.nodes.iter().find(|n| n.label == "sum_ab");
        assert!(sum_node.is_some(), "sum_ab node should exist");
        assert_eq!(
            sum_node.unwrap().node_type,
            NodeType::Expression,
            "'sum_ab' should be Expression (binary_expression)"
        );

        // upper_name is a function invocation → Expression.
        let upper_node = result.nodes.iter().find(|n| n.label == "upper_name");
        assert!(upper_node.is_some(), "upper_name node should exist");
        assert_eq!(
            upper_node.unwrap().node_type,
            NodeType::Expression,
            "'upper_name' should be Expression (invocation)"
        );

        // At least one Column node must exist for the bare field reference 'a'.
        let col_nodes: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| n.node_type == NodeType::Column)
            .collect();
        assert!(
            !col_nodes.is_empty(),
            "at least one Column node should exist for bare field reference 'a'"
        );
    }

    // C6 — `derives_from` edge targets + alias resolution.
    #[test]
    fn test_derives_from_with_alias_resolution() {
        let sql = "CREATE VIEW v AS SELECT t.col1, t.col2 AS renamed FROM my_schema.my_table t;";
        let path = PathBuf::from("my_app/test.sql");
        let result = extract_sql(&path, sql);

        let my_table_rel_id = make_id(&["rel", "my_schema", "my_table"]);
        let col1_src = make_id(&["col", &my_table_rel_id, "col1"]);
        let col2_src = make_id(&["col", &my_table_rel_id, "col2"]);

        // Some Column node must derive_from my_table.col1 (alias 't' → my_schema.my_table).
        let col1_derives = result
            .edges
            .iter()
            .find(|e| e.relation == "derives_from" && e.target == col1_src);
        assert!(
            col1_derives.is_some(),
            "a node should derive_from my_table.col1 (alias 't' resolves to my_schema.my_table)"
        );

        // 'renamed' Column node must derive_from my_table.col2.
        let renamed_node = result
            .nodes
            .iter()
            .find(|n| n.label == "renamed" && n.node_type == NodeType::Column);
        assert!(renamed_node.is_some(), "'renamed' Column node should exist");

        let renamed_derives = result.edges.iter().find(|e| {
            e.relation == "derives_from"
                && e.source == renamed_node.unwrap().id
                && e.target == col2_src
        });
        assert!(
            renamed_derives.is_some(),
            "'renamed' should derive_from my_table.col2"
        );
    }

    // C7 — CTE transparency: dependencies bubble up to the enclosing Relation.
    #[test]
    fn test_cte_transparency_deps_bubble_up() {
        let sql = "
            CREATE VIEW v AS
              WITH cte AS (SELECT id FROM orders)
              SELECT id FROM cte;
        ";
        let path = PathBuf::from("my_app/test.sql");
        let result = extract_sql(&path, sql);

        let v_node = result
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Relation && n.label == "v");
        assert!(v_node.is_some(), "view 'v' should exist");

        let v_id = &v_node.unwrap().id;
        let orders_id = make_id(&["rel", "", "orders"]);
        let cte_id = make_id(&["rel", "", "cte"]);

        // v must depend_on orders (the real source, bubbled through the CTE).
        let orders_dep = result
            .edges
            .iter()
            .find(|e| e.relation == "depends_on" && &e.source == v_id && e.target == orders_id);
        assert!(
            orders_dep.is_some(),
            "v should have a depends_on edge to orders (through CTE)"
        );

        // v must NOT depend_on the CTE itself — CTEs are not external dependencies.
        let cte_dep = result
            .edges
            .iter()
            .find(|e| e.relation == "depends_on" && &e.source == v_id && e.target == cte_id);
        assert!(
            cte_dep.is_none(),
            "v should NOT have a depends_on edge to cte (CTE is not an external dependency)"
        );
    }

    // Fix 1 — CAST should be classified as Column, not Expression.
    #[test]
    fn test_cast_classified_as_column() {
        let sql = "CREATE VIEW v AS SELECT CAST(x AS INT) AS x_int FROM t;";
        let path = PathBuf::from("app/test.sql");
        let result = extract_sql(&path, sql);
        let node = result.nodes.iter().find(|n| n.label == "x_int").unwrap();
        assert_eq!(node.node_type, NodeType::Column);
    }

    // Fix 8 — IF NOT EXISTS should still produce a Relation node.
    #[test]
    fn test_create_table_if_not_exists() {
        let sql = "CREATE TABLE IF NOT EXISTS my_table (id INT);";
        let path = PathBuf::from("app/test.sql");
        let result = extract_sql(&path, sql);
        let table = result
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Relation && n.label == "my_table");
        assert!(
            table.is_some(),
            "IF NOT EXISTS should still produce a Relation node"
        );
    }

    // Fix 9 — SELECT * should not produce column/expression nodes for the view.
    #[test]
    fn test_select_star_no_column_nodes() {
        let sql = "CREATE VIEW v AS SELECT * FROM t;";
        let path = PathBuf::from("app/test.sql");
        let result = extract_sql(&path, sql);
        let view_id = result
            .nodes
            .iter()
            .find(|n| n.label == "v")
            .unwrap()
            .id
            .clone();
        let col_nodes: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| {
                (n.node_type == NodeType::Column || n.node_type == NodeType::Expression)
                    && result
                        .edges
                        .iter()
                        .any(|e| e.relation == "part_of" && e.source == n.id && e.target == view_id)
            })
            .collect();
        assert!(
            col_nodes.is_empty(),
            "SELECT * should not produce column nodes"
        );
    }

    // Fix 11 — Standalone INSERT (target not pre-defined) creates Relation + depends_on.
    #[test]
    fn test_insert_into_table_not_defined_in_file() {
        let sql = "INSERT INTO archive SELECT id FROM orders;";
        let path = PathBuf::from("my_app/test.sql");
        let result = extract_sql(&path, sql);

        // archive should be created as a Relation
        let archive = result
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Relation && n.label == "archive");
        assert!(
            archive.is_some(),
            "INSERT target 'archive' should create a Relation node"
        );

        // archive should have depends_on → orders
        let orders_id = make_id(&["rel", "", "orders"]);
        let dep = result.edges.iter().find(|e| {
            e.relation == "depends_on" && e.source == archive.unwrap().id && e.target == orders_id
        });
        assert!(dep.is_some(), "archive should depend_on orders");
    }

    // Fix 11 — MERGE INTO creates a Relation node and depends_on the USING source.
    #[test]
    fn test_merge_into_creates_relation_and_depends_on() {
        let sql = "MERGE INTO target USING source ON target.id = source.id WHEN MATCHED THEN UPDATE SET target.val = source.val;";
        let path = PathBuf::from("my_app/test.sql");
        let result = extract_sql(&path, sql);

        let target_node = result
            .nodes
            .iter()
            .find(|n| n.node_type == NodeType::Relation && n.label == "target");
        assert!(
            target_node.is_some(),
            "MERGE target should create a Relation node"
        );

        let source_id = make_id(&["rel", "", "source"]);
        let dep = result.edges.iter().find(|e| {
            e.relation == "depends_on"
                && e.source == target_node.unwrap().id
                && e.target == source_id
        });
        assert!(dep.is_some(), "MERGE target should depend_on source");
    }

    // Fix 17 — CTE column lineage transparency: view column derives_from underlying table column.
    #[test]
    fn test_cte_column_lineage_transparent() {
        // CTE columns should trace through to underlying table columns.
        let sql = "CREATE VIEW v AS WITH cte AS (SELECT a AS x FROM t) SELECT x FROM cte;";
        let path = PathBuf::from("app/test.sql");
        let result = extract_sql(&path, sql);

        let t_rel_id = make_id(&["rel", "", "t"]);
        let t_a_id = make_id(&["col", &t_rel_id, "a"]);

        // Some derives_from edge should exist from the view columns.
        // Full CTE transparency (tracing v.x → t.a through the CTE) is the
        // ideal; at minimum the extraction should produce derives_from edges.
        let derives = result
            .edges
            .iter()
            .filter(|e| e.relation == "derives_from")
            .collect::<Vec<_>>();
        assert!(
            !derives.is_empty(),
            "CTE query should produce derives_from edges"
        );

        // Full CTE column transparency: v.x traces through the CTE body to t.a.
        assert!(
            derives.iter().any(|e| e.target == t_a_id),
            "view column should derive_from t.a through the CTE (column transparency)"
        );
    }

    // Fix 10 — Inline subquery: view column traces through the subquery to t.a.
    #[test]
    fn test_inline_subquery_derives_from() {
        let sql = "CREATE VIEW v AS SELECT x FROM (SELECT a AS x FROM t) sub;";
        let path = PathBuf::from("app/test.sql");
        let result = extract_sql(&path, sql);

        let t_rel_id = make_id(&["rel", "", "t"]);
        let t_a_id = make_id(&["col", &t_rel_id, "a"]);
        let derives = result
            .edges
            .iter()
            .filter(|e| e.relation == "derives_from")
            .collect::<Vec<_>>();
        assert!(
            !derives.is_empty(),
            "inline subquery should produce derives_from edges"
        );
        assert!(
            derives.iter().any(|e| e.target == t_a_id),
            "view column should derive_from t.a through the inline subquery"
        );
    }

    // Step 6 — Ambiguous unqualified column (multiple tables in scope) must NOT
    // produce a derives_from edge nor an `unknown_table` stub.
    #[test]
    fn test_ambiguous_unqualified_column_no_edge() {
        // `name` is unqualified and could come from either orders or customers.
        let sql =
            "CREATE VIEW v AS SELECT name FROM orders JOIN customers ON orders.cid = customers.id;";
        let path = PathBuf::from("app/test.sql");
        let result = extract_sql(&path, sql);

        // No derives_from edge should target an unknown_table relation.
        let unknown_rel_id = make_id(&["rel", "", "unknown_table"]);
        let has_unknown = result
            .edges
            .iter()
            .any(|e| e.relation == "derives_from" && e.target.contains(&unknown_rel_id));
        assert!(
            !has_unknown,
            "ambiguous unqualified column must not derive_from an unknown_table"
        );

        // And no stub node should be minted for unknown_table.
        let has_unknown_node = result
            .nodes
            .iter()
            .any(|n| n.label.contains("unknown_table"));
        assert!(
            !has_unknown_node,
            "no unknown_table placeholder node should be created"
        );
    }

    // Step 5 — resolve_sql_cross_file must ignore non-SQL edges (no origin tag),
    // even when they use the same relation names and rel_/col_ id prefixes.
    #[test]
    fn test_cross_file_resolver_ignores_foreign_edges() {
        use graphify_core::model::GraphEdge;
        let mut result = ExtractionResult::default();
        // A foreign depends_on edge (e.g. from the semantic extractor) with an
        // untagged rel_ target that does not exist as a node.
        result.edges.push(GraphEdge {
            source: "some_concept".to_string(),
            target: make_id(&["rel", "", "ghost"]),
            relation: "depends_on".to_string(),
            confidence: Confidence::Extracted,
            confidence_score: 1.0,
            source_file: "semantic".to_string(),
            source_location: None,
            weight: 1.0,
            provenance: None,
            extra: HashMap::new(),
        });
        let before = result.nodes.len();
        resolve_sql_cross_file(&mut result);
        assert_eq!(
            result.nodes.len(),
            before,
            "foreign (untagged) edges must not create stub nodes"
        );
    }

    // Step 5 — tagged SQL edges with an unresolved target still get a stub.
    #[test]
    fn test_cross_file_resolver_stubs_sql_edges() {
        let mut result = ExtractionResult::default();
        result.edges.push(make_sql_edge(
            "src",
            &make_id(&["rel", "", "ghost"]),
            "depends_on",
            "x.sql",
        ));
        resolve_sql_cross_file(&mut result);
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.node_type == NodeType::Relation
                    && n.extra.get("relation_kind").and_then(|v| v.as_str()) == Some("stub")),
            "tagged SQL edge with unresolved target should create a stub relation"
        );
    }

    // resolve_sql_cross_file must be idempotent: a second run over an already
    // resolved result adds no duplicate stubs.
    #[test]
    fn test_resolve_sql_cross_file_idempotent() {
        let mut result = ExtractionResult::default();
        result.edges.push(make_sql_edge(
            "src",
            &make_id(&["rel", "", "ghost"]),
            "depends_on",
            "x.sql",
        ));
        resolve_sql_cross_file(&mut result);
        let after_first = result.nodes.len();
        resolve_sql_cross_file(&mut result);
        assert_eq!(
            result.nodes.len(),
            after_first,
            "second resolve run must not add duplicate stubs"
        );
    }

    // is_sql_stub must match freshly minted stubs AND legacy cache forms
    // (entries written before the explicit `stub` marker existed).
    #[test]
    fn test_is_sql_stub_matches_legacy_cache_forms() {
        // Fresh stubs (rel + col) via the resolver.
        let mut result = ExtractionResult::default();
        result.edges.push(make_sql_edge(
            "src",
            &make_id(&["rel", "", "ghost"]),
            "depends_on",
            "x.sql",
        ));
        result.edges.push(make_sql_edge(
            "src",
            &make_id(&["col", "rel_ghost", "c"]),
            "derives_from",
            "x.sql",
        ));
        resolve_sql_cross_file(&mut result);
        assert_eq!(
            result.nodes.len(),
            2,
            "one rel stub + one col stub expected"
        );
        assert!(
            result.nodes.iter().all(is_sql_stub),
            "freshly minted stubs must match is_sql_stub"
        );

        // Legacy relation stub: relation_kind = "stub", no `stub` marker.
        let legacy_rel = GraphNode {
            id: "rel_legacy".to_string(),
            label: "[external: legacy]".to_string(),
            source_file: "unknown".to_string(),
            source_location: None,
            node_type: NodeType::Relation,
            community: None,
            extra: {
                let mut e = HashMap::new();
                e.insert("relation_kind".to_string(), serde_json::json!("stub"));
                e
            },
        };
        assert!(is_sql_stub(&legacy_rel), "legacy rel stub must match");

        // Legacy column stub: Column node with source_file = "unknown", empty extra.
        let legacy_col = GraphNode {
            id: "col_rel_legacy_c".to_string(),
            label: "[unknown column: rel_legacy_c]".to_string(),
            source_file: "unknown".to_string(),
            source_location: None,
            node_type: NodeType::Column,
            community: None,
            extra: HashMap::new(),
        };
        assert!(is_sql_stub(&legacy_col), "legacy col stub must match");

        // Real nodes must NOT match.
        let real = GraphNode {
            id: "rel_staging_orders".to_string(),
            label: "staging.orders".to_string(),
            source_file: "a.sql".to_string(),
            source_location: None,
            node_type: NodeType::Relation,
            community: None,
            extra: {
                let mut e = HashMap::new();
                e.insert("relation_kind".to_string(), serde_json::json!("table"));
                e
            },
        };
        assert!(!is_sql_stub(&real), "real relation must not match");
    }

    // Compiled dbt model: bare SELECT with JOIN → depends_on + column lineage
    // attributed to the given target relation, and no App/File/Relation nodes.
    #[test]
    fn test_extract_sql_lineage_bare_select() {
        let target = make_id(&["rel", "analytics", "order_report"]);
        let sql = "SELECT o.id AS order_id, c.name\nFROM staging.orders o\nJOIN staging.customers c ON o.customer_id = c.id";
        let path = PathBuf::from("target/compiled/app/models/order_report.sql");
        let result = extract_sql_lineage(&path, sql, &target);

        // No Application/File/Relation nodes — only Column/Expression.
        assert!(
            result
                .nodes
                .iter()
                .all(|n| n.node_type == NodeType::Column || n.node_type == NodeType::Expression),
            "lineage extraction must not emit App/File/Relation nodes, got: {:?}",
            result
                .nodes
                .iter()
                .map(|n| (&n.node_type, &n.label))
                .collect::<Vec<_>>()
        );

        // depends_on → both base tables, attributed to the target.
        let orders_id = make_id(&["rel", "staging", "orders"]);
        let customers_id = make_id(&["rel", "staging", "customers"]);
        assert!(
            result
                .edges
                .iter()
                .any(|e| e.relation == "depends_on" && e.source == target && e.target == orders_id),
            "target should depend_on staging.orders"
        );
        assert!(
            result.edges.iter().any(|e| e.relation == "depends_on"
                && e.source == target
                && e.target == customers_id),
            "target should depend_on staging.customers"
        );

        // Column lineage: order_id derives_from staging.orders.id.
        let order_id_col = result
            .nodes
            .iter()
            .find(|n| n.label == "order_id")
            .expect("order_id column node should exist");
        let src_col = make_id(&["col", &orders_id, "id"]);
        assert!(
            result.edges.iter().any(|e| e.relation == "derives_from"
                && e.source == order_id_col.id
                && e.target == src_col),
            "order_id should derive_from staging.orders.id"
        );

        // Columns are part_of the target relation.
        assert!(
            result.edges.iter().any(|e| e.relation == "part_of"
                && e.source == order_id_col.id
                && e.target == target),
            "order_id should be part_of the target relation"
        );
    }

    // Compiled dbt model with a WITH clause: lineage traces through the CTE,
    // no depends_on to the CTE itself, self-references ({{ this }}) skipped.
    #[test]
    fn test_extract_sql_lineage_with_cte() {
        let target = make_id(&["rel", "analytics", "daily"]);
        let sql = "WITH base AS (SELECT a AS x FROM raw.events)\nSELECT x FROM base";
        let path = PathBuf::from("target/compiled/app/models/daily.sql");
        let result = extract_sql_lineage(&path, sql, &target);

        let events_id = make_id(&["rel", "raw", "events"]);
        let cte_id = make_id(&["rel", "", "base"]);

        assert!(
            result
                .edges
                .iter()
                .any(|e| e.relation == "depends_on" && e.source == target && e.target == events_id),
            "target should depend_on raw.events (through the CTE)"
        );
        assert!(
            !result
                .edges
                .iter()
                .any(|e| e.relation == "depends_on" && e.target == cte_id),
            "CTE must not appear as a dependency"
        );

        // Column transparency: x traces to raw.events.a.
        let src_col = make_id(&["col", &events_id, "a"]);
        assert!(
            result
                .edges
                .iter()
                .any(|e| e.relation == "derives_from" && e.target == src_col),
            "x should derive_from raw.events.a through the CTE"
        );

        // Self-dependency guard: a model selecting from its own relation
        // (incremental {{ this }}) emits no self-loop.
        let self_sql = "SELECT id FROM analytics.daily";
        let self_result = extract_sql_lineage(&path, self_sql, &target);
        assert!(
            !self_result
                .edges
                .iter()
                .any(|e| e.relation == "depends_on" && e.source == target && e.target == target),
            "self-reference must not create a depends_on self-loop"
        );
    }

    // Step 6 — A qualified column with a single table in scope still resolves.
    #[test]
    fn test_unqualified_column_single_table_resolves() {
        let sql = "CREATE VIEW v AS SELECT name FROM customers;";
        let path = PathBuf::from("app/test.sql");
        let result = extract_sql(&path, sql);

        let customers_rel = make_id(&["rel", "", "customers"]);
        let name_col = make_id(&["col", &customers_rel, "name"]);
        let resolved = result
            .edges
            .iter()
            .any(|e| e.relation == "derives_from" && e.target == name_col);
        assert!(
            resolved,
            "unqualified column with one table in scope should resolve to that table"
        );
    }
}
