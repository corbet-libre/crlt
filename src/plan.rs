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
    conn: &crate::backend::Connection,
    sql: &str,
    params: &[libsql::Value],
) -> Result<QueryPlan> {
    let mut rows = conn
        .query(&format!("EXPLAIN QUERY PLAN {sql}"), params.to_vec())
        .await?;
    let mut details = Vec::new();
    while let Some(row) = rows.next().await? {
        details.push(row.get_str(3)?.to_owned());
    }
    Ok(QueryPlan { details })
}

#[cfg(test)]
#[path = "tests/plans.rs"]
mod tests;
