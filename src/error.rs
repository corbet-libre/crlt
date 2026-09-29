use std::fmt;

/// Errors returned by the facade. Formatting never includes SQL or credentials.
#[derive(thiserror::Error)]
pub enum Error {
    /// A result column index or value type did not match the requested accessor.
    #[error("invalid result column index or type")]
    Column,
    /// Invalid URL, community, limits, or parameter configuration.
    #[error("invalid configuration: {0}")]
    Config(&'static str),
    /// SQL outside the supported scoped subset.
    #[error("unsupported scoped SQL: {0}")]
    Sql(&'static str),
    /// Invalid or unsafe database schema.
    #[error("invalid community schema: {0}")]
    Schema(&'static str),
    /// Migration versions, names, or SQL differ from the recorded history.
    #[error("migration history must be complete, contiguous and immutable")]
    MigrationHistory,
    /// An application query would scan instead of searching an index.
    #[error("query plan contains an unbounded scan")]
    UnindexedQuery,
    /// The operation deadline elapsed; a remote write may have taken effect.
    #[error("database operation timed out; remote write outcome may be unknown")]
    Timeout,
    /// A failed or cancelled transaction operation prevents commit.
    #[error("transaction is aborted; roll back or drop it")]
    TransactionAborted,
    /// The configured row collection limit was exceeded.
    #[error("query exceeded the configured row limit")]
    RowLimit,
    /// The pool could not supply a usable connection.
    #[error("database pool could not supply a connection")]
    Pool,
    /// libSQL failed. The source is available for deliberate diagnosis.
    #[error("libSQL operation failed")]
    Database(#[from] libsql::Error),
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// The facade's result type.
pub type Result<T> = std::result::Result<T, Error>;
