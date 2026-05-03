//! SQL extraction backed by `sqlparser` dialect parsers.
//!
//! The Python graphify extractor used an optional `tree-sitter-sql` pass for
//! tables, views, functions and relationships.  In Rust we use sqlparser so the
//! parser understands dialect-specific PostgreSQL and ClickHouse syntax instead
//! of relying on regexes or a generic grammar.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use graphify_core::confidence::Confidence;
use graphify_core::id::make_id;
use graphify_core::model::{ExtractionResult, GraphEdge, GraphNode, NodeType};
use sqlparser::ast::{
    ColumnOption, CreateFunctionBody, CreateTable, CreateView, Expr, FromTable, ObjectName, Query,
    Select, SetExpr, Statement, TableConstraint, TableFactor, TableObject, TableWithJoins,
    UpdateTableFromKind,
};
use sqlparser::dialect::{ClickHouseDialect, Dialect, GenericDialect, PostgreSqlDialect};
use sqlparser::parser::Parser;
use tracing::{debug, warn};

/// SQL dialect that successfully parsed a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlDialectKind {
    ClickHouse,
    PostgreSql,
    Generic,
}

impl SqlDialectKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::ClickHouse => "clickhouse",
            Self::PostgreSql => "postgresql",
            Self::Generic => "generic",
        }
    }
}

#[derive(Debug)]
struct ParsedSql {
    dialect: SqlDialectKind,
    statements: Vec<Statement>,
}

/// Extract graph nodes and edges from SQL using sqlparser.
///
/// Dialect selection intentionally tries ClickHouse first, then PostgreSQL, then
/// the generic ANSI-ish dialect.  ClickHouse parsing must remain first because it
/// accepts ClickHouse-only clauses such as `ON CLUSTER`, `ENGINE`, `ORDER BY`,
/// `PREWHERE`, `SETTINGS`, `FORMAT`, `LIMIT BY`, and materialized-view `TO`.
pub fn extract_sql(path: &Path, source: &str) -> ExtractionResult {
    match parse_sql(source) {
        Ok(parsed) => SqlExtractor::new(path, source, parsed.dialect).extract(&parsed.statements),
        Err(error) => {
            let mut result =
                SqlExtractor::new(path, source, SqlDialectKind::ClickHouse).extract(&[]);
            if let Some(file) = result
                .nodes
                .iter_mut()
                .find(|node| node.node_type == NodeType::File)
            {
                file.extra.insert(
                    "sql_parse_error".to_string(),
                    serde_json::Value::String(error.clone()),
                );
            }
            if result.nodes.len() <= 1 {
                warn!("cannot parse SQL {}: {error}", path.display());
            } else {
                debug!(
                    "SQL AST parse failed for {}, recovered {} topology nodes using ClickHouse fallback",
                    path.display(),
                    result.nodes.len().saturating_sub(1)
                );
            }
            result
        }
    }
}

fn parse_sql(source: &str) -> Result<ParsedSql, String> {
    let attempts: &[(SqlDialectKind, &dyn Dialect)] = &[
        (SqlDialectKind::ClickHouse, &ClickHouseDialect {}),
        (SqlDialectKind::PostgreSql, &PostgreSqlDialect {}),
        (SqlDialectKind::Generic, &GenericDialect {}),
    ];

    let mut errors = Vec::new();
    for (kind, dialect) in attempts {
        match Parser::parse_sql(*dialect, source) {
            Ok(statements) => {
                debug!("parsed SQL using {} dialect", kind.as_str());
                return Ok(ParsedSql {
                    dialect: *kind,
                    statements,
                });
            }
            Err(error) => errors.push(format!("{}: {error}", kind.as_str())),
        }
    }

    // sqlparser's ClickHouse dialect understands the core grammar, but 0.61.0
    // still rejects some common CREATE TABLE tail clauses (`PARTITION BY`,
    // table-level `SETTINGS`) even though they do not affect graph topology.
    // Keep the actual extraction AST-based by sanitizing only those tail clauses
    // and retrying the ClickHouse parser before giving up.
    if looks_like_clickhouse(source) {
        let normalized = normalize_clickhouse_for_sqlparser(source);
        if normalized != source {
            match Parser::parse_sql(&ClickHouseDialect {}, &normalized) {
                Ok(statements) => {
                    debug!(
                        "parsed SQL using clickhouse dialect after topology-preserving normalization"
                    );
                    return Ok(ParsedSql {
                        dialect: SqlDialectKind::ClickHouse,
                        statements,
                    });
                }
                Err(error) => errors.push(format!("clickhouse-normalized: {error}")),
            }
        }
    }

    if let Some(parsed) = parse_sql_statement_by_statement(source) {
        return Ok(parsed);
    }

    Err(errors.join("; "))
}

fn parse_sql_statement_by_statement(source: &str) -> Option<ParsedSql> {
    let mut statements = Vec::new();
    let mut dialect = None;

    for statement in source.split_inclusive(';') {
        let statement = statement.trim();
        if statement.is_empty() {
            continue;
        }

        if let Some(parsed) = parse_one_sql_statement(statement) {
            dialect.get_or_insert(parsed.dialect);
            statements.extend(parsed.statements);
        }
    }

    (!statements.is_empty()).then(|| ParsedSql {
        dialect: dialect.unwrap_or(SqlDialectKind::Generic),
        statements,
    })
}

fn parse_one_sql_statement(statement: &str) -> Option<ParsedSql> {
    let attempts: &[(SqlDialectKind, &dyn Dialect)] = &[
        (SqlDialectKind::ClickHouse, &ClickHouseDialect {}),
        (SqlDialectKind::PostgreSql, &PostgreSqlDialect {}),
        (SqlDialectKind::Generic, &GenericDialect {}),
    ];

    for (kind, dialect) in attempts {
        if let Ok(statements) = Parser::parse_sql(*dialect, statement) {
            return Some(ParsedSql {
                dialect: *kind,
                statements,
            });
        }
    }

    if looks_like_clickhouse(statement) {
        let normalized = normalize_clickhouse_for_sqlparser(statement);
        if normalized != statement
            && let Ok(statements) = Parser::parse_sql(&ClickHouseDialect {}, &normalized)
        {
            return Some(ParsedSql {
                dialect: SqlDialectKind::ClickHouse,
                statements,
            });
        }
    }

    None
}

fn looks_like_clickhouse(source: &str) -> bool {
    let upper = source.to_ascii_uppercase();
    upper.contains("ENGINE =")
        || upper.contains("ENGINE=")
        || upper.contains("PREWHERE")
        || upper.contains(" ON CLUSTER ")
        || upper.contains("MATERIALIZED VIEW")
        || upper.contains("FORMAT JSONEACHROW")
}

fn normalize_clickhouse_for_sqlparser(source: &str) -> String {
    source
        .split_inclusive(';')
        .map(normalize_clickhouse_statement_for_sqlparser)
        .collect::<Vec<_>>()
        .join("")
}

fn normalize_clickhouse_statement_for_sqlparser(statement: &str) -> String {
    let trimmed = statement.trim_start();
    if !trimmed.to_ascii_uppercase().starts_with("CREATE TABLE") {
        return statement.to_string();
    }

    let mut normalized = Vec::new();
    for line in statement.lines() {
        let upper = line.trim_start().to_ascii_uppercase();
        if upper.starts_with("PARTITION BY ") || upper.starts_with("SETTINGS ") {
            if line.contains(';') {
                normalized.push(";");
            }
            continue;
        }
        normalized.push(line);
    }

    let mut result = normalized.join("\n");
    if statement.ends_with('\n') {
        result.push('\n');
    }
    result
}

struct SqlExtractor<'a> {
    path: &'a Path,
    source: &'a str,
    source_file: String,
    file_id: String,
    dialect: SqlDialectKind,
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    seen_nodes: HashSet<String>,
    seen_edges: HashSet<(String, String, String)>,
    relations: HashMap<String, String>,
}

impl<'a> SqlExtractor<'a> {
    fn new(path: &'a Path, source: &'a str, dialect: SqlDialectKind) -> Self {
        let source_file = path.to_string_lossy().into_owned();
        let file_id = make_id(&[&source_file]);
        Self {
            path,
            source,
            source_file,
            file_id,
            dialect,
            nodes: Vec::new(),
            edges: Vec::new(),
            seen_nodes: HashSet::new(),
            seen_edges: HashSet::new(),
            relations: HashMap::new(),
        }
    }

    fn extract(mut self, statements: &[Statement]) -> ExtractionResult {
        let file_node = make_file_node(self.path, Some(self.dialect));
        self.seen_nodes.insert(file_node.id.clone());
        self.nodes.push(file_node);

        for statement in statements {
            self.extract_statement(statement, &self.file_id.clone());
        }
        self.extract_clickhouse_fallback_ddl();

        ExtractionResult {
            nodes: self.nodes,
            edges: self.edges,
            hyperedges: Vec::new(),
        }
    }

    /// sqlparser gives us the rich AST where it can parse a statement. Real
    /// ClickHouse schema files also contain dictionaries, lambda UDFs, template
    /// engines, and DDL clauses that sqlparser 0.61.0 still cannot fully parse.
    /// This fallback is intentionally topology-only: it does not replace the AST
    /// path, it only fills in missing object nodes and obvious dependencies from
    /// CREATE statements so real schemas do not silently lose most objects.
    fn extract_clickhouse_fallback_ddl(&mut self) {
        let Ok(create_re) = regex::Regex::new(
            r#"(?is)^\s*(?:--[^\n]*\n\s*)*CREATE\s+(?:OR\s+REPLACE\s+)?(?:(MATERIALIZED)\s+)?(TABLE|VIEW|DICTIONARY|FUNCTION)\s+(?:IF\s+NOT\s+EXISTS\s+)?([`"]?[A-Za-z_][\w$]*[`"]?(?:\.[`"]?[A-Za-z_][\w$]*[`"]?)?)"#,
        ) else {
            return;
        };

        for statement in split_sql_statements(self.source) {
            let Some(cap) = create_re.captures(statement) else {
                continue;
            };
            let materialized = cap.get(1).is_some();
            let kind = cap
                .get(2)
                .map(|mat| mat.as_str().to_ascii_uppercase())
                .unwrap_or_default();
            let Some(raw_name) = cap.get(3).map(|mat| mat.as_str()) else {
                continue;
            };
            let label = unquote_name(raw_name);
            let line = self.line_for_name(&label);
            let sql_kind = fallback_sql_kind(&kind, materialized);
            let node_type = fallback_node_type(sql_kind);
            let object_id = self.add_sql_node(
                &label,
                node_type,
                sql_kind,
                line,
                &self.file_id.clone(),
                "defines",
                false,
            );

            self.extract_fallback_statement_dependencies(statement, &object_id, line);
        }
    }

    fn extract_fallback_statement_dependencies(
        &mut self,
        statement: &str,
        object_id: &str,
        fallback_line: usize,
    ) {
        self.extract_opaque_body_refs(statement, object_id, fallback_line);
        self.extract_fallback_to_target(statement, object_id, fallback_line);
        self.extract_fallback_as_relation(statement, object_id, fallback_line);
        self.extract_fallback_distributed_relation(statement, object_id, fallback_line);
    }

    fn extract_fallback_to_target(
        &mut self,
        statement: &str,
        object_id: &str,
        fallback_line: usize,
    ) {
        let Ok(to_re) = regex::Regex::new(
            r#"(?i)\bTO\s+([`"]?[A-Za-z_][\w$]*[`"]?(?:\.[`"]?[A-Za-z_][\w$]*[`"]?)?)"#,
        ) else {
            return;
        };
        if let Some(target) = to_re
            .captures(statement)
            .and_then(|cap| cap.get(1).map(|mat| mat.as_str()))
        {
            let object = object_name_from_str(&unquote_name(target));
            let line = self.line_for_name(target).max(fallback_line);
            let target_id = self.add_relation_node(&object, "table", line, true);
            self.add_edge(object_id, &target_id, "writes_to", line);
        }
    }

    fn extract_fallback_as_relation(
        &mut self,
        statement: &str,
        object_id: &str,
        fallback_line: usize,
    ) {
        let Ok(as_re) = regex::Regex::new(
            r#"(?i)\bAS\s+([`"]?[A-Za-z_][\w$]*[`"]?(?:\.[`"]?[A-Za-z_][\w$]*[`"]?)?)\s*(?:ENGINE|$|;)"#,
        ) else {
            return;
        };
        if let Some(source) = as_re
            .captures(statement)
            .and_then(|cap| cap.get(1).map(|mat| mat.as_str()))
        {
            let source = unquote_name(source);
            if !source.eq_ignore_ascii_case("select") {
                let object = object_name_from_str(&source);
                let line = self.line_for_name(&source).max(fallback_line);
                let source_id = self.add_relation_node(&object, "table", line, true);
                self.add_edge(object_id, &source_id, "reads_from", line);
            }
        }
    }

    fn extract_fallback_distributed_relation(
        &mut self,
        statement: &str,
        object_id: &str,
        fallback_line: usize,
    ) {
        let Ok(distributed_re) = regex::Regex::new(
            r#"(?i)Distributed\s*\([^,]+,\s*currentDatabase\s*\(\s*\)\s*,\s*'([^']+)'"#,
        ) else {
            return;
        };
        if let Some(source) = distributed_re
            .captures(statement)
            .and_then(|cap| cap.get(1).map(|mat| mat.as_str()))
        {
            let object = object_name_from_str(source);
            let line = self.line_for_name(source).max(fallback_line);
            let source_id = self.add_relation_node(&object, "table", line, true);
            self.add_edge(object_id, &source_id, "reads_from", line);
        }
    }

    fn extract_statement(&mut self, statement: &Statement, parent_id: &str) {
        match statement {
            Statement::CreateTable(table) => self.extract_create_table(table, parent_id),
            Statement::CreateView(view) => self.extract_create_view(view, parent_id),
            Statement::CreateFunction(function) => {
                let name = object_label(&function.name);
                let line = self.line_for_name(&name);
                let function_id = self.add_sql_node(
                    &name,
                    NodeType::Function,
                    "function",
                    line,
                    parent_id,
                    "defines",
                    false,
                );
                self.extract_function_body(function.function_body.as_ref(), &function_id, line);
            }
            Statement::CreateProcedure { name, body, .. } => {
                let label = format!("{}()", object_label(name));
                let line = self.line_for_name(&label);
                let procedure_id = self.add_sql_node(
                    &label,
                    NodeType::Function,
                    "procedure",
                    line,
                    parent_id,
                    "defines",
                    false,
                );
                // sqlparser exposes procedure body as statements for dialects that support it.
                for statement in body.statements() {
                    self.extract_statement(statement, &procedure_id);
                }
            }
            Statement::Query(query) => self.extract_query_reads(query, parent_id, "reads_from"),
            Statement::Insert(insert) => {
                match &insert.table {
                    TableObject::TableName(name) => {
                        let target_id =
                            self.add_relation_node(name, "table", self.line_for_object(name), true);
                        self.add_edge(
                            parent_id,
                            &target_id,
                            "writes_to",
                            self.line_for_object(name),
                        );
                    }
                    TableObject::TableFunction(function) => {
                        let label = function.name.to_string();
                        let line = self.line_for_name(&label);
                        let fn_id = self.add_sql_node(
                            &label,
                            NodeType::Function,
                            "table_function",
                            line,
                            parent_id,
                            "uses",
                            true,
                        );
                        self.add_edge(parent_id, &fn_id, "writes_to", line);
                    }
                }
                if let Some(source) = &insert.source {
                    self.extract_query_reads(source, parent_id, "reads_from");
                }
            }
            Statement::Update(update) => {
                self.extract_table_with_joins(&update.table, parent_id, "writes_to");
                if let Some(from) = &update.from {
                    match from {
                        UpdateTableFromKind::BeforeSet(tables)
                        | UpdateTableFromKind::AfterSet(tables) => {
                            for table in tables {
                                self.extract_table_with_joins(table, parent_id, "reads_from");
                            }
                        }
                    }
                }
            }
            Statement::Delete(delete) => {
                for table in &delete.tables {
                    let line = self.line_for_object(table);
                    let table_id = self.add_relation_node(table, "table", line, true);
                    self.add_edge(parent_id, &table_id, "writes_to", line);
                }
                match &delete.from {
                    FromTable::WithFromKeyword(tables) | FromTable::WithoutKeyword(tables) => {
                        for table in tables {
                            self.extract_table_with_joins(table, parent_id, "reads_from");
                        }
                    }
                }
                if let Some(tables) = &delete.using {
                    for table in tables {
                        self.extract_table_with_joins(table, parent_id, "reads_from");
                    }
                }
            }
            Statement::AlterTable(alter) => {
                let line = self.line_for_object(&alter.name);
                let table_id = self.add_relation_node(&alter.name, "table", line, true);
                self.add_edge(parent_id, &table_id, "alters", line);
                for operation in &alter.operations {
                    if let sqlparser::ast::AlterTableOperation::AddConstraint {
                        constraint, ..
                    } = operation
                    {
                        self.extract_table_constraint(&table_id, constraint, line);
                    }
                }
            }
            _ => {}
        }
    }

    fn extract_create_table(&mut self, table: &CreateTable, parent_id: &str) {
        let line = self.line_for_object(&table.name);
        let table_id = self.add_relation_node(&table.name, "table", line, false);
        self.add_edge(parent_id, &table_id, "defines", line);

        for column in &table.columns {
            let column_label = format!("{}.{}", object_label(&table.name), column.name.value);
            let mut extra = HashMap::new();
            extra.insert(
                "sql_kind".to_string(),
                serde_json::Value::String("column".to_string()),
            );
            extra.insert(
                "sql_type".to_string(),
                serde_json::Value::String(column.data_type.to_string()),
            );
            let column_id =
                self.add_node_with_extra(&column_label, NodeType::Variable, line, extra, false);
            self.add_edge(&table_id, &column_id, "has_column", line);

            for option in &column.options {
                if let ColumnOption::ForeignKey(fk) = &option.option {
                    let ref_id = self.add_relation_node(&fk.foreign_table, "table", line, true);
                    self.add_edge(&table_id, &ref_id, "references", line);
                    self.add_edge(&column_id, &ref_id, "references", line);
                }
            }
        }

        for constraint in &table.constraints {
            self.extract_table_constraint(&table_id, constraint, line);
        }

        if let Some(query) = &table.query {
            self.extract_query_reads(query, &table_id, "reads_from");
        }
        if let Some(clone) = &table.clone {
            let clone_id =
                self.add_relation_node(clone, "table", self.line_for_object(clone), true);
            self.add_edge(&table_id, &clone_id, "clones", self.line_for_object(clone));
        }
    }

    fn extract_create_view(&mut self, view: &CreateView, parent_id: &str) {
        let line = self.line_for_object(&view.name);
        let sql_kind = if view.materialized {
            "materialized_view"
        } else {
            "view"
        };
        let view_id = self.add_relation_node(&view.name, sql_kind, line, false);
        self.add_edge(parent_id, &view_id, "defines", line);
        self.extract_query_reads(&view.query, &view_id, "reads_from");

        if let Some(target) = &view.to {
            let target_id =
                self.add_relation_node(target, "table", self.line_for_object(target), true);
            self.add_edge(
                &view_id,
                &target_id,
                "writes_to",
                self.line_for_object(target),
            );
        }
    }

    fn extract_table_constraint(
        &mut self,
        table_id: &str,
        constraint: &TableConstraint,
        line: usize,
    ) {
        if let TableConstraint::ForeignKey(fk) = constraint {
            let ref_id = self.add_relation_node(&fk.foreign_table, "table", line, true);
            self.add_edge(table_id, &ref_id, "references", line);
        }
    }

    fn extract_function_body(
        &mut self,
        body: Option<&CreateFunctionBody>,
        function_id: &str,
        line: usize,
    ) {
        match body {
            Some(CreateFunctionBody::AsReturnSelect(select)) => {
                self.extract_select_reads(select, function_id, "reads_from")
            }
            Some(CreateFunctionBody::AsBeforeOptions { body, .. })
            | Some(CreateFunctionBody::AsAfterOptions(body))
            | Some(CreateFunctionBody::Return(body))
            | Some(CreateFunctionBody::AsReturnExpr(body)) => {
                self.extract_expr_subqueries(body, function_id, "reads_from");
                self.extract_opaque_body_refs(&body.to_string(), function_id, line);
            }
            Some(CreateFunctionBody::AsBeginEnd(begin_end)) => {
                for statement in &begin_end.statements {
                    self.extract_statement(statement, function_id);
                }
            }
            None => {}
        }
    }

    fn extract_query_reads(&mut self, query: &Query, owner_id: &str, relation: &str) {
        if let Some(with) = &query.with {
            for cte in &with.cte_tables {
                self.extract_query_reads(&cte.query, owner_id, relation);
            }
        }
        self.extract_set_expr_reads(&query.body, owner_id, relation);
    }

    fn extract_set_expr_reads(&mut self, set_expr: &SetExpr, owner_id: &str, relation: &str) {
        match set_expr {
            SetExpr::Select(select) => self.extract_select_reads(select, owner_id, relation),
            SetExpr::Query(query) => self.extract_query_reads(query, owner_id, relation),
            SetExpr::SetOperation { left, right, .. } => {
                self.extract_set_expr_reads(left, owner_id, relation);
                self.extract_set_expr_reads(right, owner_id, relation);
            }
            SetExpr::Insert(statement)
            | SetExpr::Update(statement)
            | SetExpr::Delete(statement)
            | SetExpr::Merge(statement) => self.extract_statement(statement, owner_id),
            SetExpr::Table(table) => {
                if let Some(table_name) = &table.table_name {
                    let full_name = if let Some(schema_name) = &table.schema_name {
                        format!("{schema_name}.{table_name}")
                    } else {
                        table_name.clone()
                    };
                    let object = object_name_from_str(&full_name);
                    let line = self.line_for_name(&full_name);
                    let table_id = self.add_relation_node(&object, "table", line, true);
                    self.add_edge(owner_id, &table_id, relation, line);
                }
            }
            SetExpr::Values(_) => {}
        }
    }

    fn extract_select_reads(&mut self, select: &Select, owner_id: &str, relation: &str) {
        for table in &select.from {
            self.extract_table_with_joins(table, owner_id, relation);
        }
        if let Some(into) = &select.into {
            let line = self.line_for_object(&into.name);
            let table_id = self.add_relation_node(&into.name, "table", line, true);
            self.add_edge(owner_id, &table_id, "writes_to", line);
        }
        for item in &select.projection {
            self.extract_opaque_body_refs(&item.to_string(), owner_id, 1);
        }
    }

    fn extract_table_with_joins(&mut self, table: &TableWithJoins, owner_id: &str, relation: &str) {
        self.extract_table_factor(&table.relation, owner_id, relation);
        for join in &table.joins {
            self.extract_table_factor(&join.relation, owner_id, relation);
        }
    }

    fn extract_table_factor(&mut self, factor: &TableFactor, owner_id: &str, relation: &str) {
        match factor {
            TableFactor::Table { name, .. } => {
                let line = self.line_for_object(name);
                let table_id = self.add_relation_node(name, "table", line, true);
                self.add_edge(owner_id, &table_id, relation, line);
            }
            TableFactor::Derived { subquery, .. } => {
                self.extract_query_reads(subquery, owner_id, relation)
            }
            TableFactor::NestedJoin {
                table_with_joins, ..
            } => {
                self.extract_table_with_joins(table_with_joins, owner_id, relation);
            }
            TableFactor::TableFunction { expr, .. } => {
                self.extract_expr_subqueries(expr, owner_id, relation);
                let label = expr.to_string();
                let line = self.line_for_name(&label);
                let fn_id = self.add_sql_node(
                    &label,
                    NodeType::Function,
                    "table_function",
                    line,
                    owner_id,
                    "uses",
                    true,
                );
                self.add_edge(owner_id, &fn_id, relation, line);
            }
            TableFactor::Function { name, .. } => {
                let line = self.line_for_object(name);
                let fn_id = self.add_sql_node(
                    &object_label(name),
                    NodeType::Function,
                    "table_function",
                    line,
                    owner_id,
                    "uses",
                    true,
                );
                self.add_edge(owner_id, &fn_id, relation, line);
            }
            _ => {}
        }
    }

    fn extract_expr_subqueries(&mut self, expr: &Expr, owner_id: &str, relation: &str) {
        match expr {
            Expr::Subquery(query)
            | Expr::Exists {
                subquery: query, ..
            } => {
                self.extract_query_reads(query, owner_id, relation);
            }
            Expr::InSubquery { subquery, .. } => {
                self.extract_query_reads(subquery, owner_id, relation)
            }
            Expr::BinaryOp { left, right, .. } => {
                self.extract_expr_subqueries(left, owner_id, relation);
                self.extract_expr_subqueries(right, owner_id, relation);
            }
            Expr::UnaryOp { expr, .. }
            | Expr::Nested(expr)
            | Expr::IsNull(expr)
            | Expr::IsNotNull(expr)
            | Expr::IsFalse(expr)
            | Expr::IsNotFalse(expr)
            | Expr::IsTrue(expr)
            | Expr::IsNotTrue(expr)
            | Expr::IsUnknown(expr)
            | Expr::IsNotUnknown(expr) => self.extract_expr_subqueries(expr, owner_id, relation),
            _ => {}
        }
    }

    /// PostgreSQL PL/pgSQL and some UDF bodies are opaque string expressions to
    /// sqlparser.  Keep AST parsing as the primary path, but recover useful
    /// `FROM`/`JOIN` dependencies from those opaque bodies so original graphify's
    /// SQL relationships are not lost.
    fn extract_opaque_body_refs(&mut self, text: &str, owner_id: &str, fallback_line: usize) {
        let Ok(re) = regex::Regex::new(
            r#"(?i)\b(?:FROM|JOIN)\s+([`"]?[a-z_][a-z0-9_$]*[`"]?(?:\.[`"]?[a-z_][a-z0-9_$]*[`"]?)*)"#,
        ) else {
            return;
        };
        for cap in re.captures_iter(text) {
            let Some(name) = cap.get(1).map(|m| m.as_str()) else {
                continue;
            };
            let object = object_name_from_str(name);
            let line = self.line_for_name(name).max(fallback_line);
            let table_id = self.add_relation_node(&object, "table", line, true);
            self.add_edge(owner_id, &table_id, "reads_from", line);
        }
    }

    fn add_relation_node(
        &mut self,
        name: &ObjectName,
        sql_kind: &str,
        line: usize,
        external: bool,
    ) -> String {
        let label = object_label(name);
        let id_key = normalize_object_name(&label);
        if let Some(id) = self.relations.get(&id_key) {
            return id.clone();
        }

        let node_type = match sql_kind {
            "table" => NodeType::Struct,
            "view" | "materialized_view" => NodeType::Module,
            _ => NodeType::Concept,
        };
        let id = self.add_sql_node(
            &label,
            node_type,
            sql_kind,
            line,
            &self.file_id.clone(),
            "contains",
            external,
        );
        self.relations.insert(id_key, id.clone());
        id
    }

    #[allow(clippy::too_many_arguments)]
    fn add_sql_node(
        &mut self,
        label: &str,
        node_type: NodeType,
        sql_kind: &str,
        line: usize,
        parent_id: &str,
        relation: &str,
        external: bool,
    ) -> String {
        let mut extra = HashMap::new();
        extra.insert(
            "sql_kind".to_string(),
            serde_json::Value::String(sql_kind.to_string()),
        );
        if external {
            extra.insert("external".to_string(), serde_json::Value::Bool(true));
        }
        let id = self.add_node_with_extra(label, node_type, line, extra, external);
        self.add_edge(parent_id, &id, relation, line);
        id
    }

    fn add_node_with_extra(
        &mut self,
        label: &str,
        node_type: NodeType,
        line: usize,
        extra: HashMap<String, serde_json::Value>,
        external: bool,
    ) -> String {
        let id = if external {
            make_id(&["sql", &normalize_object_name(label)])
        } else {
            make_id(&[&self.source_file, label])
        };
        if self.seen_nodes.insert(id.clone()) {
            self.nodes.push(GraphNode {
                id: id.clone(),
                label: label.to_string(),
                source_file: self.source_file.clone(),
                source_location: Some(format!("L{line}")),
                node_type,
                community: None,
                extra,
            });
        }
        id
    }

    fn add_edge(&mut self, source: &str, target: &str, relation: &str, line: usize) {
        let key = (source.to_string(), target.to_string(), relation.to_string());
        if self.seen_edges.insert(key) {
            self.edges.push(GraphEdge {
                source: source.to_string(),
                target: target.to_string(),
                relation: relation.to_string(),
                confidence: Confidence::Extracted,
                confidence_score: Confidence::Extracted.default_score(),
                source_file: self.source_file.clone(),
                source_location: Some(format!("L{line}")),
                weight: 1.0,
                extra: HashMap::new(),
            });
        }
    }

    fn line_for_object(&self, name: &ObjectName) -> usize {
        self.line_for_name(&object_label(name))
    }

    fn line_for_name(&self, name: &str) -> usize {
        if name.is_empty() {
            return 1;
        }
        let candidates = [
            name.to_string(),
            unquote_name(name),
            name.replace('.', "\\."),
        ];
        for candidate in candidates.iter().filter(|candidate| !candidate.is_empty()) {
            if let Some(pos) = self.source.find(candidate) {
                return self.source[..pos].lines().count() + 1;
            }
        }
        1
    }
}

fn split_sql_statements(source: &str) -> Vec<&str> {
    source
        .split_inclusive(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .collect()
}

fn fallback_sql_kind(kind: &str, materialized: bool) -> &'static str {
    match (kind, materialized) {
        ("VIEW", true) => "materialized_view",
        ("VIEW", false) => "view",
        ("TABLE", _) => "table",
        ("DICTIONARY", _) => "dictionary",
        ("FUNCTION", _) => "function",
        _ => "sql_object",
    }
}

fn fallback_node_type(sql_kind: &str) -> NodeType {
    match sql_kind {
        "table" => NodeType::Struct,
        "view" | "materialized_view" => NodeType::Module,
        "function" => NodeType::Function,
        "dictionary" => NodeType::Concept,
        _ => NodeType::Concept,
    }
}

fn make_file_node(path: &Path, dialect: Option<SqlDialectKind>) -> GraphNode {
    let source_file = path.to_string_lossy().into_owned();
    let mut extra = HashMap::new();
    extra.insert(
        "language".to_string(),
        serde_json::Value::String("sql".to_string()),
    );
    if let Some(dialect) = dialect {
        extra.insert(
            "sql_dialect".to_string(),
            serde_json::Value::String(dialect.as_str().to_string()),
        );
    }
    GraphNode {
        id: make_id(&[&source_file]),
        label: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown.sql")
            .to_string(),
        source_file,
        source_location: None,
        node_type: NodeType::File,
        community: None,
        extra,
    }
}

fn object_label(name: &ObjectName) -> String {
    unquote_name(&name.to_string())
}

fn normalize_object_name(name: &str) -> String {
    unquote_name(name).to_lowercase()
}

fn unquote_name(name: &str) -> String {
    name.replace(['"', '`', '[', ']'], "")
}

fn object_name_from_str(name: &str) -> ObjectName {
    ObjectName(
        name.split('.')
            .map(|part| {
                sqlparser::ast::ObjectNamePart::Identifier(sqlparser::ast::Ident::new(part))
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(result: &ExtractionResult) -> HashSet<String> {
        result.nodes.iter().map(|node| node.label.clone()).collect()
    }

    fn relations(result: &ExtractionResult) -> HashSet<String> {
        result
            .edges
            .iter()
            .map(|edge| edge.relation.clone())
            .collect()
    }

    #[test]
    fn extracts_postgres_tables_views_functions_and_relationships() {
        let source = r#"
CREATE TABLE organizations (
  id SERIAL PRIMARY KEY,
  name TEXT NOT NULL
);

CREATE TABLE users (
  id SERIAL PRIMARY KEY,
  email TEXT NOT NULL,
  org_id INT REFERENCES organizations(id)
);

CREATE VIEW active_users AS
  SELECT * FROM users WHERE active = true;

CREATE FUNCTION get_user(user_id INT) RETURNS users AS $$
  BEGIN
    RETURN QUERY SELECT * FROM users WHERE id = user_id;
  END;
$$ LANGUAGE plpgsql;
"#;

        let result = extract_sql(Path::new("schema.sql"), source);
        let labels = labels(&result);
        assert!(labels.contains("organizations"), "labels: {labels:?}");
        assert!(labels.contains("users"), "labels: {labels:?}");
        assert!(labels.contains("active_users"), "labels: {labels:?}");
        assert!(labels.contains("get_user"), "labels: {labels:?}");

        let relations = relations(&result);
        assert!(relations.contains("references"), "relations: {relations:?}");
        assert!(relations.contains("reads_from"), "relations: {relations:?}");
        assert_no_dangling_edges(&result);
    }

    #[test]
    fn extracts_clickhouse_specific_tables_views_and_reads() {
        let source = r#"
CREATE TABLE events ON CLUSTER analytics
(
    ts DateTime64(3),
    user_id UInt64,
    event_name LowCardinality(String),
    day Date MATERIALIZED toDate(ts)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(ts)
ORDER BY (user_id, ts)
SETTINGS index_granularity = 8192;

CREATE MATERIALIZED VIEW daily_events_mv TO daily_events AS
SELECT
    user_id,
    count() AS cnt
FROM events
PREWHERE ts >= now() - INTERVAL 1 DAY
GROUP BY ALL WITH TOTALS
SETTINGS max_threads = 4
FORMAT JSONEachRow;
"#;

        let result = extract_sql(Path::new("clickhouse.sql"), source);
        let file = result
            .nodes
            .iter()
            .find(|node| node.node_type == NodeType::File)
            .unwrap();
        assert_eq!(
            file.extra
                .get("sql_dialect")
                .and_then(|value| value.as_str()),
            Some("clickhouse")
        );

        let labels = labels(&result);
        assert!(labels.contains("events"), "labels: {labels:?}");
        assert!(labels.contains("daily_events_mv"), "labels: {labels:?}");
        assert!(labels.contains("daily_events"), "labels: {labels:?}");

        let relations = relations(&result);
        assert!(relations.contains("reads_from"), "relations: {relations:?}");
        assert!(relations.contains("writes_to"), "relations: {relations:?}");
        assert_no_dangling_edges(&result);
    }

    #[test]
    fn parses_valid_statements_even_when_one_statement_is_unsupported() {
        let source = r#"
CREATE TABLE users (id INT);
CREATE FUNCTION unsupported_body() RETURNS users LANGUAGE SQL RETURN SELECT * FROM users WHERE id = 1;
CREATE VIEW user_view AS SELECT * FROM users;
"#;
        let result = extract_sql(Path::new("mixed.sql"), source);
        let labels = labels(&result);
        assert!(labels.contains("users"), "labels: {labels:?}");
        assert!(labels.contains("user_view"), "labels: {labels:?}");
        assert!(relations(&result).contains("reads_from"));
        assert_no_dangling_edges(&result);
    }

    #[test]
    fn clickhouse_fallback_fills_commented_materialized_views_dictionaries_and_functions() {
        let source = r#"
CREATE FUNCTION IF NOT EXISTS normalizeKey AS (x) -> lower(x);

-- dictionary is not fully represented by sqlparser's ClickHouse AST yet
CREATE DICTIONARY entity_data_by_id_dict
(
    entity_id UUID,
    title String
)
PRIMARY KEY entity_id
SOURCE(CLICKHOUSE(TABLE 'entity_settings_global'))
LAYOUT(HASHED())
LIFETIME(300);

-- leading comments must not hide the following CREATE statement
-- from topology extraction.
CREATE MATERIALIZED VIEW assign_batch_id_mv TO processed_events_local AS
SELECT * FROM raw_events_null;
"#;

        let result = extract_sql(Path::new("clickhouse.sql"), source);
        let labels = labels(&result);
        assert!(labels.contains("normalizeKey"), "labels: {labels:?}");
        assert!(
            labels.contains("entity_data_by_id_dict"),
            "labels: {labels:?}"
        );
        assert!(labels.contains("assign_batch_id_mv"), "labels: {labels:?}");
        assert!(
            labels.contains("processed_events_local"),
            "labels: {labels:?}"
        );
        assert!(labels.contains("raw_events_null"), "labels: {labels:?}");

        let relations = relations(&result);
        assert!(relations.contains("defines"), "relations: {relations:?}");
        assert!(relations.contains("reads_from"), "relations: {relations:?}");
        assert!(relations.contains("writes_to"), "relations: {relations:?}");
        assert_no_dangling_edges(&result);
    }

    #[test]
    fn parse_sql_falls_back_to_postgres_for_pg_only_function() {
        let source = "CREATE FUNCTION one() RETURNS int LANGUAGE SQL RETURN 1;";
        let parsed = parse_sql(source).expect("postgresql function should parse");
        assert!(matches!(
            parsed.dialect,
            SqlDialectKind::ClickHouse | SqlDialectKind::PostgreSql
        ));
    }

    #[test]
    fn parse_failure_still_extracts_clickhouse_create_topology() {
        let source = r#"
CREATE TABLE processed_events_local
(
    id UUID CODEC(ZSTD(1)),
    amount Decimal(18, 2) CODEC(DoubleDelta, ZSTD)
)
ENGINE = MergeTree
ORDER BY id;
"#;

        let result = extract_sql(Path::new("clickhouse_codec.sql"), source);
        let file = result
            .nodes
            .iter()
            .find(|node| node.node_type == NodeType::File)
            .unwrap();
        assert!(
            file.extra.contains_key("sql_parse_error"),
            "expected parse error metadata for fallback-only extraction"
        );

        let labels = labels(&result);
        assert!(
            labels.contains("processed_events_local"),
            "labels: {labels:?}"
        );

        let relations = relations(&result);
        assert!(relations.contains("defines"), "relations: {relations:?}");
        assert_no_dangling_edges(&result);
    }

    fn assert_no_dangling_edges(result: &ExtractionResult) {
        let node_ids: HashSet<&str> = result.nodes.iter().map(|node| node.id.as_str()).collect();
        for edge in &result.edges {
            assert!(
                node_ids.contains(edge.source.as_str()),
                "dangling source: {edge:?}"
            );
            assert!(
                node_ids.contains(edge.target.as_str()),
                "dangling target: {edge:?}"
            );
        }
    }
}
