use std::{collections::BTreeSet, ops::ControlFlow};

use libsql::{
    Value,
    params::{IntoParams, Params},
};
use sqlparser::{
    ast::{
        AssignmentTarget, BinaryOperator, Expr, FromTable, Ident, ObjectName, ObjectNamePart,
        Query, Select, SetExpr, Statement, TableFactor, TableObject, TableWithJoins,
        Value as SqlValue, ValueWithSpan, VisitMut, VisitorMut,
    },
    dialect::SQLiteDialect,
    parser::Parser,
};

use crate::{Error, Result};

pub(crate) fn parse(sql: &str) -> Result<Vec<Statement>> {
    Parser::parse_sql(&SQLiteDialect {}, sql).map_err(|_| Error::Sql("could not parse SQLite SQL"))
}

pub(crate) fn name(name: &ObjectName) -> Result<String> {
    let [ObjectNamePart::Identifier(ident)] = name.0.as_slice() else {
        return Err(Error::Sql("use an unqualified identifier"));
    };
    let value = ident.value.to_ascii_lowercase();
    if value.is_empty()
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        || value.as_bytes()[0].is_ascii_digit()
        || value.starts_with("sqlite_")
        || value.starts_with("__crlt_")
    {
        return Err(Error::Sql("invalid or reserved identifier"));
    }
    Ok(value)
}

fn community_expr() -> Expr {
    Expr::Value(SqlValue::Placeholder("?1".into()).into())
}

fn predicate(selection: &mut Option<Expr>) {
    let scope = Expr::BinaryOp {
        left: Box::new(Expr::Identifier(Ident::new("community_id"))),
        op: BinaryOperator::Eq,
        right: Box::new(community_expr()),
    };
    *selection = Some(match selection.take() {
        Some(old) => Expr::BinaryOp {
            left: Box::new(Expr::Nested(Box::new(old))),
            op: BinaryOperator::And,
            right: Box::new(scope),
        },
        None => scope,
    });
}

pub(crate) struct ScopedSql {
    pub sql: String,
    pub params: Vec<Value>,
    pub returns_rows: bool,
}

pub(crate) fn scope(
    sql: &str,
    params: impl IntoParams,
    community: &str,
    tables: &BTreeSet<String>,
) -> Result<ScopedSql> {
    let mut statements = parse(sql)?;
    if statements.len() != 1 {
        return Err(Error::Sql("exactly one statement is required"));
    }
    let mut statement = statements.remove(0);
    let mut params = match params.into_params()? {
        Params::None => Vec::new(),
        Params::Positional(values) => values,
        Params::Named(_) => return Err(Error::Sql("use numbered positional parameters")),
    };
    let mut visitor = Scoper {
        tables,
        param_count: params.len(),
        used: BTreeSet::new(),
    };
    let returns_rows = match &mut statement {
        Statement::Query(query) => {
            if matches!(query.body.as_ref(), SetExpr::Values(_)) {
                return Err(Error::Sql("use SELECT for standalone reads"));
            }
            visitor.walk(query)?;
            true
        }
        Statement::Insert(insert) => {
            let TableObject::TableName(table) = &insert.table else {
                return Err(Error::Sql("insert target must be a table"));
            };
            visitor.table(table)?;
            if insert.columns.is_empty()
                || insert.or.is_some()
                || insert.on.is_some()
                || insert.replace_into
                || insert.table_alias.is_some()
            {
                return Err(Error::Sql(
                    "insert requires columns and no conflict/alias clauses",
                ));
            }
            for column in &insert.columns {
                if name(column)? == "community_id" {
                    return Err(Error::Sql("community_id is supplied by the handle"));
                }
            }
            let query = insert
                .source
                .as_mut()
                .ok_or(Error::Sql("insert requires VALUES"))?;
            if !matches!(query.body.as_ref(), SetExpr::Values(_)) {
                return Err(Error::Sql("insert-select is not supported"));
            }
            visitor.walk(query)?;
            visitor.walk(&mut insert.returning)?;
            let SetExpr::Values(values) = query.body.as_mut() else {
                unreachable!()
            };
            for row in &mut values.rows {
                if row.len() != insert.columns.len() {
                    return Err(Error::Sql("insert column/value counts differ"));
                }
                row.insert(0, community_expr());
            }
            insert
                .columns
                .insert(0, ObjectName::from(vec![Ident::new("community_id")]));
            insert.returning.is_some()
        }
        Statement::Update(update) => {
            visitor.target(&update.table)?;
            if update.from.is_some() || update.or.is_some() {
                return Err(Error::Sql(
                    "update-from and conflict clauses are not supported",
                ));
            }
            for assignment in &update.assignments {
                let AssignmentTarget::ColumnName(column) = &assignment.target else {
                    return Err(Error::Sql("tuple assignments are not supported"));
                };
                if name(column)? == "community_id" {
                    return Err(Error::Sql("community_id is immutable"));
                }
            }
            visitor.walk(&mut update.assignments)?;
            visitor.walk(&mut update.selection)?;
            visitor.walk(&mut update.returning)?;
            visitor.walk(&mut update.order_by)?;
            visitor.walk(&mut update.limit)?;
            predicate(&mut update.selection);
            update.returning.is_some()
        }
        Statement::Delete(delete) => {
            let FromTable::WithFromKeyword(from) = &delete.from else {
                return Err(Error::Sql("delete requires FROM"));
            };
            if from.len() != 1 || delete.using.is_some() || !delete.tables.is_empty() {
                return Err(Error::Sql("delete requires a single target table"));
            }
            visitor.target(&from[0])?;
            visitor.walk(&mut delete.selection)?;
            visitor.walk(&mut delete.returning)?;
            visitor.walk(&mut delete.order_by)?;
            visitor.walk(&mut delete.limit)?;
            predicate(&mut delete.selection);
            delete.returning.is_some()
        }
        _ => {
            return Err(Error::Sql(
                "only SELECT, INSERT, UPDATE and DELETE are supported",
            ));
        }
    };
    if visitor.used.len() != params.len() {
        return Err(Error::Sql("every supplied parameter must be used"));
    }
    params.insert(0, Value::Text(community.to_owned()));
    Ok(ScopedSql {
        sql: statement.to_string(),
        params,
        returns_rows,
    })
}

struct Scoper<'a> {
    tables: &'a BTreeSet<String>,
    param_count: usize,
    used: BTreeSet<usize>,
}

impl Scoper<'_> {
    fn walk(&mut self, node: &mut impl VisitMut) -> Result<()> {
        match node.visit(self) {
            ControlFlow::Continue(()) => Ok(()),
            ControlFlow::Break(error) => Err(error),
        }
    }

    fn table(&self, table: &ObjectName) -> Result<String> {
        let table = name(table)?;
        if !self.tables.contains(&table) {
            return Err(Error::Sql("table is not in the validated migration schema"));
        }
        Ok(table)
    }

    fn target(&self, target: &TableWithJoins) -> Result<()> {
        if !target.joins.is_empty() {
            return Err(Error::Sql("write targets cannot have joins"));
        }
        let TableFactor::Table {
            name,
            alias: None,
            args: None,
            ..
        } = &target.relation
        else {
            return Err(Error::Sql(
                "write target must be a plain table without an alias",
            ));
        };
        self.table(name)?;
        Ok(())
    }
}

impl VisitorMut for Scoper<'_> {
    type Break = Error;

    fn pre_visit_query(&mut self, query: &mut Query) -> ControlFlow<Error> {
        if query.with.is_some() || !query.locks.is_empty() || !query.pipe_operators.is_empty() {
            return ControlFlow::Break(Error::Sql("CTEs, query locks and pipes are not supported"));
        }
        fn read_body(body: &SetExpr) -> bool {
            match body {
                SetExpr::Select(_) | SetExpr::Values(_) | SetExpr::Query(_) => true,
                SetExpr::SetOperation { left, right, .. } => read_body(left) && read_body(right),
                _ => false,
            }
        }
        if !read_body(&query.body) {
            return ControlFlow::Break(Error::Sql("query body must be a read or VALUES"));
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_select(&mut self, select: &mut Select) -> ControlFlow<Error> {
        if select.into.is_some() {
            return ControlFlow::Break(Error::Sql("SELECT INTO is not supported"));
        }
        ControlFlow::Continue(())
    }

    fn post_visit_select(&mut self, select: &mut Select) -> ControlFlow<Error> {
        if select.from.is_empty() {
            let guard = Expr::IsNotNull(Box::new(community_expr()));
            select.selection = Some(match select.selection.take() {
                Some(old) => Expr::BinaryOp {
                    left: Box::new(Expr::Nested(Box::new(old))),
                    op: BinaryOperator::And,
                    right: Box::new(guard),
                },
                None => guard,
            });
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_statement(&mut self, statement: &mut Statement) -> ControlFlow<Error> {
        if !matches!(statement, Statement::Query(_)) {
            return ControlFlow::Break(Error::Sql("nested writes are not supported"));
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_value(&mut self, value: &mut ValueWithSpan) -> ControlFlow<Error> {
        if let SqlValue::Placeholder(placeholder) = &mut value.value {
            let index = placeholder
                .strip_prefix('?')
                .and_then(|s| s.parse::<usize>().ok());
            let Some(index) = index.filter(|&i| i > 0 && i <= self.param_count) else {
                return ControlFlow::Break(Error::Sql("parameters must be numbered ?1 through ?N"));
            };
            self.used.insert(index);
            *placeholder = format!("?{}", index + 1);
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<Error> {
        if let Expr::Function(function) = expr {
            let allowed = [
                "abs",
                "avg",
                "coalesce",
                "count",
                "hex",
                "ifnull",
                "instr",
                "json",
                "json_array",
                "json_extract",
                "json_object",
                "json_type",
                "json_valid",
                "length",
                "lower",
                "ltrim",
                "max",
                "min",
                "nullif",
                "replace",
                "round",
                "rtrim",
                "substr",
                "substring",
                "sum",
                "total",
                "trim",
                "typeof",
                "upper",
            ];
            if !name(&function.name).is_ok_and(|n| allowed.contains(&n.as_str())) {
                return ControlFlow::Break(Error::Sql(
                    "scalar function is not in the supported set",
                ));
            }
        }
        ControlFlow::Continue(())
    }

    fn post_visit_table_factor(&mut self, factor: &mut TableFactor) -> ControlFlow<Error> {
        let result = (|| -> Result<()> {
            match factor {
                TableFactor::Table {
                    name,
                    alias,
                    args: None,
                    with_hints,
                    version: None,
                    with_ordinality: false,
                    partitions,
                    json_path: None,
                    sample: None,
                    index_hints,
                } if with_hints.is_empty() && partitions.is_empty() && index_hints.is_empty() => {
                    let table = self.table(name)?;
                    let mut parsed = parse(&format!(
                        "SELECT * FROM (SELECT * FROM \"{table}\" WHERE community_id = ?1) AS \"{table}\""
                    ))?;
                    let Statement::Query(query) = parsed.remove(0) else {
                        unreachable!()
                    };
                    let SetExpr::Select(mut select) = *query.body else {
                        unreachable!()
                    };
                    let mut replacement = select.from.remove(0).relation;
                    if let TableFactor::Derived {
                        alias: new_alias, ..
                    } = &mut replacement
                        && alias.is_some()
                    {
                        *new_alias = alias.clone();
                    }
                    // Post-order replacement avoids visiting the injected filter again.
                    *factor = replacement;
                }
                TableFactor::Derived {
                    lateral: false,
                    sample: None,
                    ..
                } => {}
                _ => {
                    return Err(Error::Sql(
                        "only plain tables and derived reads are supported",
                    ));
                }
            }
            Ok(())
        })();
        match result {
            Ok(()) => ControlFlow::Continue(()),
            Err(error) => ControlFlow::Break(error),
        }
    }
}

#[cfg(test)]
#[path = "tests/scope.rs"]
mod tests;
