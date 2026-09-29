use std::{fmt, time::Duration};

use crate::{Error, Result};

/// Caller-owned connection settings. No environment or credential files are read.
pub struct Config {
    pub(crate) url: String,
    pub(crate) token: String,
    /// Maximum leased connections; defaults to one.
    pub max_connections: usize,
    /// Acquisition and operation deadline, or total transaction lifetime.
    pub timeout: Duration,
    /// Maximum local SQLite lock wait; defaults to one second.
    pub busy_timeout: Duration,
    /// Maximum number of rows collected by one query; defaults to 10,000.
    pub max_rows: usize,
}

impl Config {
    /// Open a TLS remote URL or an absolute `file:///` URL with a caller token.
    /// Local files require an empty token. The default deadline is ten seconds.
    pub fn new(url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            token: token.into(),
            max_connections: 1,
            timeout: Duration::from_secs(10),
            busy_timeout: Duration::from_secs(1),
            max_rows: 10_000,
        }
    }

    pub(crate) fn validate(&self) -> Result<url::Url> {
        if self.max_connections == 0 || self.max_rows == 0 || self.timeout.is_zero() {
            return Err(Error::Config(
                "pool size, timeout and row limit must be positive",
            ));
        }
        if self.max_connections > tokio::sync::Semaphore::MAX_PERMITS
            || std::time::Instant::now()
                .checked_add(self.timeout)
                .is_none()
        {
            return Err(Error::Config("pool size or timeout exceeds runtime limits"));
        }
        if self.busy_timeout > self.timeout || self.busy_timeout.as_millis() > i32::MAX as u128 {
            return Err(Error::Config(
                "busy timeout exceeds the operation or SQLite limit",
            ));
        }
        let url = url::Url::parse(&self.url).map_err(|_| Error::Config("invalid database URL"))?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(Error::Config(
                "URL credentials, queries and fragments are forbidden",
            ));
        }
        match url.scheme() {
            "libsql" | "https" if url.host_str().is_some() && !self.token.is_empty() => {}
            "file" if self.token.is_empty() && url.host_str().is_none() => {
                let path = url
                    .to_file_path()
                    .map_err(|_| Error::Config("invalid file URL"))?;
                if !path.is_absolute() || path.file_name().is_none() {
                    return Err(Error::Config(
                        "file URL must name an absolute database file",
                    ));
                }
            }
            _ => {
                return Err(Error::Config(
                    "use a TLS URL with a token or a local file URL without one",
                ));
            }
        }
        Ok(url)
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("url", &"[redacted]")
            .field("token", &"[redacted]")
            .field("max_connections", &self.max_connections)
            .field("timeout", &self.timeout)
            .field("busy_timeout", &self.busy_timeout)
            .field("max_rows", &self.max_rows)
            .finish()
    }
}
