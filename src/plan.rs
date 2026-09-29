use crate::{Error, Result};

/// The detail column of libSQL's `EXPLAIN QUERY PLAN`, without bound values.
#[derive(Debug, Clone)]
pub struct QueryPlan {
    pub(crate) details: Vec<String>,
}

impl QueryPlan {
    /// Planner details, useful for asserting a particular composite index.
    pub fn details(&self) -> &[String] {
        &self.details
    }

    /// Reject all scans except constant rows. Inserts of literal values and
    /// constant expressions need no lookup and can have an empty plan.
    /// This conservative check also rejects scans of materialized subqueries.
    pub fn assert_indexed(&self) -> Result<()> {
        for detail in &self.details {
            let words: Vec<_> = detail.split_whitespace().collect();
            if words.first() == Some(&"SCAN") {
                let constant = words.as_slice() == ["SCAN", "CONSTANT", "ROW"]
                    || (words.len() == 4
                        && words[1].parse::<u64>().is_ok()
                        && words[2..] == ["CONSTANT", "ROWS"]);
                if !constant {
                    return Err(Error::UnindexedQuery);
                }
            }
        }
        Ok(())
    }
}

pub(crate) async fn explain(
    conn: &libsql::Connection,
    sql: &str,
    params: &[libsql::Value],
) -> Result<QueryPlan> {
    let mut rows = conn
        .query(&format!("EXPLAIN QUERY PLAN {sql}"), params.to_vec())
        .await?;
    let mut details = Vec::new();
    while let Some(row) = rows.next().await? {
        details.push(row.get::<String>(3)?);
    }
    Ok(QueryPlan { details })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn detects_real_table_and_index_scans() {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        conn.execute(
            "CREATE TABLE sample (id INTEGER PRIMARY KEY, value TEXT)",
            (),
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO sample VALUES (1, 'one'), (2, 'two')", ())
            .await
            .unwrap();
        let scan = explain(
            &conn,
            "SELECT * FROM sample WHERE value = ?1",
            &[libsql::Value::Text("one".into())],
        )
        .await
        .unwrap();
        assert!(matches!(scan.assert_indexed(), Err(Error::UnindexedQuery)));
        conn.execute("CREATE INDEX by_value ON sample (value)", ())
            .await
            .unwrap();
        let search = explain(
            &conn,
            "SELECT * FROM sample WHERE value = ?1",
            &[libsql::Value::Text("one".into())],
        )
        .await
        .unwrap();
        search.assert_indexed().unwrap();
        assert!(
            search
                .details()
                .iter()
                .any(|d| d.contains("USING COVERING INDEX by_value"))
        );
        let index_scan = explain(&conn, "SELECT value FROM sample INDEXED BY by_value", &[])
            .await
            .unwrap();
        assert!(matches!(
            index_scan.assert_indexed(),
            Err(Error::UnindexedQuery)
        ));
    }
}
