//! Private connection adapter; public values, parameters and errors stay libSQL's.

use std::{ops::Deref, sync::Arc, time::Duration};

use deadpool::managed::{self, Metrics, RecycleError};
use libsql::{TransactionBehavior, params::IntoParams};
use libsql_hrana::proto;

use crate::{
    Error, Result, Row,
    remote::{Remote, decode_value},
    transport::{Sender, failure},
};

pub(crate) type Pool = managed::Pool<Manager>;
pub(crate) type PooledConnection = managed::Object<Manager>;

pub(crate) enum Manager {
    Local(libsql::Database),
    Remote {
        url: String,
        token: String,
        sender: Sender,
        timeout: Duration,
    },
}

impl managed::Manager for Manager {
    type Type = Connection;
    type Error = Error;

    async fn create(&self) -> Result<Connection> {
        let conn = match self {
            Self::Local(database) => Connection::Local(database.connect()?),
            Self::Remote {
                url,
                token,
                sender,
                timeout,
            } => Connection::Remote(Arc::new(Remote::new(url, token, sender.clone(), *timeout)?)),
        };
        conn.check().await?;
        Ok(conn)
    }

    async fn recycle(&self, conn: &mut Connection, _: &Metrics) -> managed::RecycleResult<Error> {
        conn.check().await.map_err(RecycleError::Backend)
    }
}

#[derive(Clone)]
pub(crate) enum Connection {
    Local(libsql::Connection),
    Remote(Arc<Remote>),
}

impl Connection {
    async fn check(&self) -> Result<()> {
        let reusable = match self {
            Self::Local(conn) => conn.is_autocommit(),
            Self::Remote(conn) => conn.reusable().await,
        };
        if !reusable {
            return Err(failure("unfinished or broken transaction"));
        }
        let mut rows = self.query("SELECT 1", ()).await?;
        let value = rows
            .next()
            .await?
            .ok_or_else(|| failure("missing health check row"))?
            .get_i64(0)?;
        if value != 1 {
            return Err(failure("invalid health check row"));
        }
        Ok(())
    }

    pub(crate) fn busy_timeout(&self, timeout: Duration) -> Result<()> {
        match self {
            Self::Local(conn) => Ok(conn.busy_timeout(timeout)?),
            Self::Remote(_) => Ok(()),
        }
    }

    pub(crate) async fn execute(&self, sql: &str, params: impl IntoParams) -> Result<u64> {
        match self {
            Self::Local(conn) => Ok(conn.execute(sql, params).await?),
            Self::Remote(conn) => Ok(conn
                .execute(sql, params.into_params()?, false)
                .await?
                .affected_row_count),
        }
    }

    pub(crate) async fn query(&self, sql: &str, params: impl IntoParams) -> Result<Rows> {
        match self {
            Self::Local(conn) => {
                let rows = conn.query(sql, params).await?;
                let columns = (0..rows.column_count())
                    .map(|index| rows.column_name(index).unwrap_or_default().to_owned())
                    .collect();
                Ok(Rows {
                    columns,
                    inner: RowsInner::Local(rows),
                })
            }
            Self::Remote(conn) => {
                let result = conn.execute(sql, params.into_params()?, true).await?;
                let columns = result
                    .cols
                    .into_iter()
                    .map(|col| col.name.unwrap_or_default())
                    .collect();
                Ok(Rows {
                    columns,
                    inner: RowsInner::Remote(result.rows.into_iter()),
                })
            }
        }
    }

    pub(crate) async fn transaction_with_behavior(
        &self,
        behavior: TransactionBehavior,
    ) -> Result<Transaction> {
        match self {
            Self::Local(conn) => {
                let tx = conn.transaction_with_behavior(behavior).await?;
                let conn = Self::Local((*tx).clone());
                Ok(Transaction::Local(tx, conn))
            }
            Self::Remote(conn) => {
                // The pool lease is exclusive for the entire transaction.
                let sql = match behavior {
                    TransactionBehavior::Deferred => "BEGIN DEFERRED",
                    TransactionBehavior::Immediate => "BEGIN IMMEDIATE",
                    TransactionBehavior::Exclusive => "BEGIN EXCLUSIVE",
                    TransactionBehavior::ReadOnly => "BEGIN READONLY",
                };
                self.execute(sql, ()).await?;
                if conn.reusable().await {
                    return Err(failure("BEGIN did not start a transaction"));
                }
                Ok(Transaction::Remote(self.clone()))
            }
        }
    }
}

pub(crate) struct Rows {
    columns: Arc<[String]>,
    inner: RowsInner,
}

enum RowsInner {
    Local(libsql::Rows),
    Remote(std::vec::IntoIter<proto::Row>),
}

impl Rows {
    pub(crate) async fn next(&mut self) -> Result<Option<Row>> {
        let values = match &mut self.inner {
            RowsInner::Local(rows) => match rows.next().await? {
                Some(row) => (0..row.column_count())
                    .map(|index| row.get_value(index))
                    .collect::<libsql::Result<Vec<_>>>()?,
                None => return Ok(None),
            },
            RowsInner::Remote(rows) => match rows.next() {
                Some(row) => row
                    .values
                    .into_iter()
                    .map(decode_value)
                    .collect::<Result<Vec<_>>>()?,
                None => return Ok(None),
            },
        };
        if values.len() != self.columns.len() {
            return Err(failure("invalid result row width"));
        }
        Ok(Some(Row {
            columns: self.columns.clone(),
            values,
        }))
    }
}

// Local drop rolls back via libSQL; remote drop plus lease discard sends Close.
pub(crate) enum Transaction {
    Local(libsql::Transaction, Connection),
    Remote(Connection),
}

impl Deref for Transaction {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        match self {
            Self::Local(_, conn) | Self::Remote(conn) => conn,
        }
    }
}

impl Transaction {
    pub(crate) async fn commit(self) -> Result<()> {
        match self {
            Self::Local(tx, _) => Ok(tx.commit().await?),
            Self::Remote(conn) => {
                conn.execute("COMMIT", ()).await?;
                if let Connection::Remote(remote) = conn
                    && !remote.reusable().await
                {
                    return Err(failure("COMMIT did not end the transaction"));
                }
                Ok(())
            }
        }
    }

    pub(crate) async fn rollback(self) -> Result<()> {
        match self {
            Self::Local(tx, _) => Ok(tx.rollback().await?),
            Self::Remote(conn) => {
                conn.execute("ROLLBACK", ()).await?;
                if let Connection::Remote(remote) = conn
                    && !remote.reusable().await
                {
                    return Err(failure("ROLLBACK did not end the transaction"));
                }
                Ok(())
            }
        }
    }
}
