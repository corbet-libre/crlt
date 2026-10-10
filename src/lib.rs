#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod backend;
mod config;
mod error;
mod migration;
mod plan;
mod remote;
mod row;
mod sql;
mod transport;

use std::{collections::BTreeSet, future::Future, sync::Arc, time::Duration};

use backend::{Connection, Manager, Pool, PooledConnection};
use tokio::{
    sync::{OwnedRwLockReadGuard, RwLock},
    time::{Instant, timeout, timeout_at},
};

pub use config::Config;
pub use error::{Error, Result};
pub use libsql::params::IntoParams;
pub use libsql::{Value, params};
pub use migration::Migration;
pub use plan::QueryPlan;
pub use row::Row;

struct Inner {
    pool: Pool,
    schema: Arc<RwLock<BTreeSet<String>>>,
    timeout: Duration,
    busy_timeout: Duration,
    max_rows: usize,
    local: bool,
}

/// Shared connection pool and migration authority. Contains no unscoped SQL API.
#[derive(Clone)]
pub struct Db(Arc<Inner>);

/// A fixed community capability. Only the owning service can obtain another scope.
#[derive(Clone)]
pub struct Community {
    db: Db,
    key: String,
}

/// An exclusive scoped transaction. Failed or cancelled operations prevent commit.
/// Dropping rolls back through libSQL and discards the connection lease.
pub struct Transaction {
    // Field order drops the libSQL transaction before discarding its lease.
    transaction: Option<backend::Transaction>,
    lease: Option<Lease>,
    db: Db,
    schema: OwnedRwLockReadGuard<BTreeSet<String>>,
    key: String,
    deadline: Instant,
    cleanup_timeout: Duration,
    max_rows: usize,
    poisoned: bool,
}

/// Discard a lease unless an operation explicitly marks its completion.
/// This covers cancellation and unwinding without an async Drop implementation.
struct Lease(Option<PooledConnection>);

impl Lease {
    fn conn(&self) -> &Connection {
        self.0.as_ref().expect("live lease")
    }
    fn recycle(mut self) {
        drop(self.0.take());
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(connection) = self.0.take() {
            drop(PooledConnection::take(connection));
        }
    }
}

async fn bounded<T>(deadline: Instant, future: impl Future<Output = Result<T>>) -> Result<T> {
    if Instant::now() >= deadline {
        return Err(Error::Timeout);
    }
    timeout_at(deadline, future)
        .await
        .map_err(|_| Error::Timeout)?
}

impl Db {
    /// Create and health-check a pool using only the supplied URL and token.
    /// Call `migrate` before using application tables, including after reopening.
    pub async fn open(config: Config) -> Result<Self> {
        let url = config.validate()?;
        let deadline = Instant::now() + config.timeout;
        bounded(deadline, async move {
            let local = url.scheme() == "file";
            let manager = if local {
                let path = url
                    .to_file_path()
                    .map_err(|_| Error::Config("invalid file URL"))?;
                Manager::Local(libsql::Builder::new_local(path).build().await?)
            } else {
                Manager::Remote {
                    url: config.url,
                    token: config.token,
                    sender: transport::Sender::new(config.timeout)?,
                    timeout: config.timeout,
                }
            };
            let pool = Pool::builder(manager)
                .max_size(config.max_connections)
                .build()
                .map_err(|_| Error::Pool)?;
            let db = Self(Arc::new(Inner {
                pool,
                schema: Arc::new(RwLock::new(BTreeSet::new())),
                timeout: config.timeout,
                busy_timeout: config.busy_timeout,
                max_rows: config.max_rows,
                local,
            }));
            db.lease().await?.recycle();
            Ok(db)
        })
        .await
    }

    async fn lease(&self) -> Result<Lease> {
        let lease = Lease(Some(self.0.pool.get().await.map_err(|_| Error::Pool)?));
        if self.0.local {
            lease.conn().busy_timeout(self.0.busy_timeout)?;
        }
        lease.conn().execute("PRAGMA foreign_keys = ON", ()).await?;
        let mut rows = lease.conn().query("PRAGMA foreign_keys", ()).await?;
        if rows
            .next()
            .await?
            .ok_or(Error::Schema("missing foreign key setting"))?
            .get_i64(0)?
            != 1
        {
            return Err(Error::Schema("foreign keys must be enabled"));
        }
        Ok(lease)
    }

    /// Apply all pending schema DDL and its ledger atomically; return the applied count.
    /// Changed or incomplete history is rejected. Other handles wait for this call.
    pub async fn migrate(&self, migrations: &[Migration<'_>]) -> Result<usize> {
        let scripts = migration::validate(migrations)?;
        let deadline = Instant::now() + self.0.timeout;
        bounded(deadline, async {
            let mut schema = self.0.schema.write().await;
            let lease = self.lease().await?;
            let tx = lease
                .conn()
                .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
                .await?;
            let result = migration::apply(&tx, migrations, &scripts).await;
            match result {
                Ok((count, tables)) => {
                    tx.commit().await?;
                    *schema = tables;
                    lease.recycle();
                    Ok(count)
                }
                Err(error) => {
                    if tx.rollback().await.is_ok() {
                        lease.recycle();
                    }
                    Err(error)
                }
            }
        })
        .await
    }

    /// Create a fixed scope. Authorization and one-database-per-community routing
    /// belong to the caller. Keys are nonempty UTF-8 strings, compared bytewise.
    pub fn community(&self, key: impl Into<String>) -> Result<Community> {
        let key = key.into();
        if key.is_empty() || key.contains('\0') {
            return Err(Error::Config(
                "community key must be nonempty and contain no NUL",
            ));
        }
        Ok(Community {
            db: self.clone(),
            key,
        })
    }

    /// Begin an immediate transaction for a caller-selected community.
    pub async fn tx(&self, community: impl Into<String>) -> Result<Transaction> {
        self.community(community)?.tx().await
    }
}

impl Community {
    /// Begin a transaction with one pool lease and a shared operation deadline.
    pub async fn tx(&self) -> Result<Transaction> {
        let deadline = Instant::now() + self.db.0.timeout;
        bounded(deadline, async {
            let schema = self.db.0.schema.clone().read_owned().await;
            let lease = self.db.lease().await?;
            let transaction = lease
                .conn()
                .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
                .await?;
            Ok(Transaction {
                transaction: Some(transaction),
                lease: Some(lease),
                db: self.db.clone(),
                schema,
                key: self.key.clone(),
                deadline,
                cleanup_timeout: self.db.0.timeout,
                max_rows: self.db.0.max_rows,
                poisoned: false,
            })
        })
        .await
    }

    /// Execute a scoped write without RETURNING and return affected rows.
    pub async fn execute(&self, sql: &str, params: impl IntoParams) -> Result<u64> {
        let deadline = Instant::now() + self.db.0.timeout;
        bounded(deadline, async {
            let schema = self.db.0.schema.read().await;
            let scoped = sql::scope(sql, params, &self.key, &schema)?;
            let lease = self.db.lease().await?;
            let count = execute(lease.conn(), &scoped).await?;
            lease.recycle();
            Ok(count)
        })
        .await
    }

    /// Query scoped SQL, collecting rows before returning the pool lease.
    /// For writes with RETURNING, prefer a transaction so row-limit errors roll
    /// back the write instead of leaving an uncertain autocommit outcome.
    pub async fn query(&self, sql: &str, params: impl IntoParams) -> Result<Vec<Row>> {
        let deadline = Instant::now() + self.db.0.timeout;
        bounded(deadline, async {
            let schema = self.db.0.schema.read().await;
            let scoped = sql::scope(sql, params, &self.key, &schema)?;
            let lease = self.db.lease().await?;
            let rows = query(lease.conn(), &scoped, self.db.0.max_rows).await?;
            lease.recycle();
            Ok(rows)
        })
        .await
    }

    /// Inspect the plan for the exact scoped statement without executing it.
    pub async fn explain(&self, sql: &str, params: impl IntoParams) -> Result<QueryPlan> {
        let deadline = Instant::now() + self.db.0.timeout;
        bounded(deadline, async {
            let schema = self.db.0.schema.read().await;
            let scoped = sql::scope(sql, params, &self.key, &schema)?;
            let lease = self.db.lease().await?;
            let plan = plan::explain(lease.conn(), &scoped.sql, &scoped.params).await?;
            lease.recycle();
            Ok(plan)
        })
        .await
    }
}

impl Transaction {
    /// Whether this transaction belongs to the exact database handle and scope.
    /// Database/Community clones share identity; separately opened handles do
    /// not, even when their URLs are equal. No database identifier is exposed.
    pub fn belongs_to(&self, community: &Community) -> bool {
        self.key == community.key && Arc::ptr_eq(&self.db.0, &community.db.0)
    }

    fn begin_operation(&mut self) -> Result<()> {
        if self.poisoned {
            return Err(Error::TransactionAborted);
        }
        // Set before any await so cancellation cannot leave a committable transaction.
        self.poisoned = true;
        Ok(())
    }

    /// Execute a scoped write on this transaction's exclusive connection.
    pub async fn execute(&mut self, sql: &str, params: impl IntoParams) -> Result<u64> {
        self.begin_operation()?;
        let scoped = sql::scope(sql, params, &self.key, &self.schema)?;
        let tx = self.transaction.as_ref().expect("live transaction");
        let result = bounded(self.deadline, execute(tx, &scoped)).await;
        self.poisoned = result.is_err();
        result
    }

    /// Collect a scoped read or write with RETURNING inside the transaction.
    pub async fn query(&mut self, sql: &str, params: impl IntoParams) -> Result<Vec<Row>> {
        self.begin_operation()?;
        let scoped = sql::scope(sql, params, &self.key, &self.schema)?;
        let tx = self.transaction.as_ref().expect("live transaction");
        let result = bounded(self.deadline, query(tx, &scoped, self.max_rows)).await;
        self.poisoned = result.is_err();
        result
    }

    /// Inspect a scoped plan within the current transaction.
    pub async fn explain(&mut self, sql: &str, params: impl IntoParams) -> Result<QueryPlan> {
        self.begin_operation()?;
        let scoped = sql::scope(sql, params, &self.key, &self.schema)?;
        let tx = self.transaction.as_ref().expect("live transaction");
        let result = bounded(
            self.deadline,
            plan::explain(tx, &scoped.sql, &scoped.params),
        )
        .await;
        self.poisoned = result.is_err();
        result
    }

    /// Commit successful operations. No automatic retry occurs on timeout.
    pub async fn commit(mut self) -> Result<()> {
        if self.poisoned {
            return Err(Error::TransactionAborted);
        }
        let tx = self.transaction.take().expect("live transaction");
        bounded(self.deadline, tx.commit()).await?;
        self.lease.take().expect("live lease").recycle();
        Ok(())
    }

    /// Explicitly roll back, even after a deadline or an operation failure.
    pub async fn rollback(mut self) -> Result<()> {
        let tx = self.transaction.take().expect("live transaction");
        timeout(self.cleanup_timeout, tx.rollback())
            .await
            .map_err(|_| Error::Timeout)??;
        self.lease.take().expect("live lease").recycle();
        Ok(())
    }
}

async fn execute(conn: &Connection, scoped: &sql::ScopedSql) -> Result<u64> {
    if scoped.returns_rows {
        return Err(Error::Sql("use query for statements returning rows"));
    }
    plan::explain(conn, &scoped.sql, &scoped.params)
        .await?
        .assert_indexed()?;
    conn.execute(&scoped.sql, scoped.params.clone()).await
}

async fn query(conn: &Connection, scoped: &sql::ScopedSql, max_rows: usize) -> Result<Vec<Row>> {
    if !scoped.returns_rows {
        return Err(Error::Sql("use execute for statements without rows"));
    }
    plan::explain(conn, &scoped.sql, &scoped.params)
        .await?
        .assert_indexed()?;
    let mut rows = conn.query(&scoped.sql, scoped.params.clone()).await?;
    let mut result = Vec::new();
    while let Some(row) = rows.next().await? {
        if result.len() == max_rows {
            return Err(Error::RowLimit);
        }
        result.push(row);
    }
    Ok(result)
}
