use std::collections::BTreeSet;

use libsql::params;
use sqlparser::ast::{ObjectType, Statement};

use crate::{Error, Result, backend::Connection, sql};

const LEDGER: &str = "CREATE TABLE IF NOT EXISTS __crlt_migrations (
    community_id TEXT NOT NULL, version INTEGER NOT NULL,
    name TEXT NOT NULL, sql TEXT NOT NULL,
    PRIMARY KEY (community_id, version)
) WITHOUT ROWID";
const SCHEMA_KEY: &str = "__crlt_schema";

/// A trusted schema migration. Supply the entire immutable history each time.
#[derive(Debug, Clone, Copy)]
pub struct Migration<'a> {
    /// Contiguous version, starting at one.
    pub version: u32,
    /// Stable, nonempty migration name.
    pub name: &'a str,
    /// Schema DDL. Application data changes use scoped transactions instead.
    pub sql: &'a str,
}

impl<'a> Migration<'a> {
    /// Define a migration from caller-supplied SQL.
    pub const fn new(version: u32, name: &'a str, sql: &'a str) -> Self {
        Self { version, name, sql }
    }
}

pub(crate) fn validate(migrations: &[Migration<'_>]) -> Result<Vec<Vec<String>>> {
    let mut scripts = Vec::new();
    for (index, migration) in migrations.iter().enumerate() {
        if migration.version as usize != index + 1 || migration.name.trim().is_empty() {
            return Err(Error::MigrationHistory);
        }
        let statements = sql::parse(migration.sql)?;
        if statements.is_empty() {
            return Err(Error::MigrationHistory);
        }
        let mut script = Vec::new();
        for statement in statements {
            match &statement {
                Statement::CreateTable(table) if !table.temporary && table.query.is_none() => {
                    sql::name(&table.name)?;
                }
                Statement::CreateIndex(index) => {
                    sql::name(&index.table_name)?;
                    sql::name(
                        index
                            .name
                            .as_ref()
                            .ok_or(Error::Schema("index must have a name"))?,
                    )?;
                }
                Statement::AlterTable(table) => {
                    sql::name(&table.name)?;
                }
                Statement::Drop {
                    object_type: ObjectType::Table | ObjectType::Index,
                    names,
                    ..
                } => {
                    for name in names {
                        sql::name(name)?;
                    }
                }
                _ => return Err(Error::Schema("migrations accept table/index DDL only")),
            }
            script.push(statement.to_string());
        }
        scripts.push(script);
    }
    Ok(scripts)
}

pub(crate) async fn apply(
    conn: &Connection,
    migrations: &[Migration<'_>],
    scripts: &[Vec<String>],
) -> Result<(usize, BTreeSet<String>)> {
    conn.execute(LEDGER, ()).await?;
    let mut rows = conn.query(
        "SELECT version, name, sql FROM __crlt_migrations WHERE community_id = ?1 ORDER BY version",
        [SCHEMA_KEY],
    ).await?;
    let mut applied = 0;
    while let Some(row) = rows.next().await? {
        let Some(migration) = migrations.get(applied) else {
            return Err(Error::MigrationHistory);
        };
        if row.get_i64(0)? != i64::from(migration.version)
            || row.get_str(1)? != migration.name
            || row.get_str(2)? != migration.sql
        {
            return Err(Error::MigrationHistory);
        }
        applied += 1;
    }
    drop(rows);
    for (migration, statements) in migrations[applied..].iter().zip(&scripts[applied..]) {
        for statement in statements {
            conn.execute(statement, ()).await?;
        }
        conn.execute(
            "INSERT INTO __crlt_migrations (community_id, version, name, sql) VALUES (?1, ?2, ?3, ?4)",
            params![SCHEMA_KEY, i64::from(migration.version), migration.name, migration.sql],
        ).await?;
    }
    let tables = audit(conn).await?;
    Ok((migrations.len() - applied, tables))
}

/// Schema introspection is provisioning metadata, not an application data query.
async fn audit(conn: &Connection) -> Result<BTreeSet<String>> {
    let mut objects = conn.query(
        "SELECT type, name, sql FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*' AND name <> '__crlt_migrations'",
        (),
    ).await?;
    let mut tables = BTreeSet::new();
    while let Some(row) = objects.next().await? {
        match row.get_str(0)? {
            "table" => {
                let ddl = sql::parse(row.get_str(2)?)?;
                let [Statement::CreateTable(table)] = ddl.as_slice() else {
                    return Err(Error::Schema("only ordinary tables are supported"));
                };
                for column in &table.columns {
                    if column.name.value.eq_ignore_ascii_case("community_id") {
                        for option in &column.options {
                            // The scope is always supplied by the handle, never by DDL.
                            if matches!(option.option, sqlparser::ast::ColumnOption::Default(_)) {
                                return Err(Error::Schema("community_id must not have a default"));
                            }
                            if let sqlparser::ast::ColumnOption::Collation(collation) =
                                &option.option
                                && sql::name(collation)? != "binary"
                            {
                                return Err(Error::Schema(
                                    "community_id must use binary collation",
                                ));
                            }
                        }
                    }
                }
                tables.insert(row.get_str(1)?.to_owned());
            }
            "index" => {}
            _ => return Err(Error::Schema("views and triggers are forbidden")),
        }
    }
    drop(objects);
    let mut normalized = BTreeSet::new();
    for table in &tables {
        let object = sqlparser::ast::ObjectName::from(vec![sqlparser::ast::Ident::new(table)]);
        normalized.insert(sql::name(&object)?);
        let mut columns = conn
            .query(&format!("PRAGMA table_xinfo(\"{table}\")"), ())
            .await?;
        let mut community_found = false;
        while let Some(row) = columns.next().await? {
            if row.get_str(1)?.eq_ignore_ascii_case("community_id") {
                community_found = row.get_str(2)?.eq_ignore_ascii_case("TEXT")
                    && row.get_i64(3)? == 1
                    && row.get_i64(5)? == 1
                    && row.get_i64(6)? == 0;
            }
        }
        if !community_found {
            return Err(Error::Schema(
                "community_id must be non-null TEXT and first in the primary key",
            ));
        }
        let mut indexes = conn
            .query(&format!("PRAGMA index_list(\"{table}\")"), ())
            .await?;
        while let Some(row) = indexes.next().await? {
            if row.get_i64(2)? == 0 {
                continue;
            }
            let index = row.get_str(1)?.replace('"', "\"\"");
            let mut columns = conn
                .query(&format!("PRAGMA index_xinfo(\"{index}\")"), ())
                .await?;
            let first = columns
                .next()
                .await?
                .ok_or(Error::Schema("empty unique index"))?;
            if !first.get_str(2)?.eq_ignore_ascii_case("community_id")
                || !first.get_str(4)?.eq_ignore_ascii_case("BINARY")
            {
                return Err(Error::Schema(
                    "every unique key must start with binary community_id",
                ));
            }
        }
        let mut foreign_keys = conn
            .query(&format!("PRAGMA foreign_key_list(\"{table}\")"), ())
            .await?;
        while let Some(row) = foreign_keys.next().await? {
            if row.get_str(3)?.eq_ignore_ascii_case("community_id") {
                // Check every component, including repeated namespace columns.
                if !row.get_str(4)?.eq_ignore_ascii_case("community_id") {
                    return Err(Error::Schema(
                        "community_id must only reference community_id",
                    ));
                }
                // SQLite applies these actions to every child-key column. An
                // indexed, scoped parent write must not replace the child scope.
                for action in [row.get_str(5)?, row.get_str(6)?] {
                    if action.eq_ignore_ascii_case("SET DEFAULT")
                        || action.eq_ignore_ascii_case("SET NULL")
                    {
                        return Err(Error::Schema(
                            "foreign key actions must not replace community_id",
                        ));
                    }
                }
            }
            if row.get_i64(1)? == 0 {
                let target = row.get_str(2)?;
                if !row.get_str(3)?.eq_ignore_ascii_case("community_id")
                    || !row.get_str(4)?.eq_ignore_ascii_case("community_id")
                    || !tables
                        .iter()
                        .any(|table| table.eq_ignore_ascii_case(target))
                {
                    return Err(Error::Schema(
                        "foreign keys must explicitly bind both community keys",
                    ));
                }
            }
        }
    }
    Ok(normalized)
}
