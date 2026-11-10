//! The crate's native integration tests in one binary.
//!
//! Every former `tests/<name>.rs` that the host runs is a module here, so a warm edit loop links
//! one test executable instead of one per file. The test names keep their module prefix
//! (`<name>::<test>`). `remote` stays separate: the repo CI drives it by name
//! (`.ci/remote.sh` runs `--test remote` with the sqld/TLS fixture exported), and
//! its `mod common;` is why `tests/common` stays at its path.
#[path = "../common/mod.rs"]
mod common;
mod foreign_keys;
mod migrations;
mod round_trips;
mod tls_configuration;
mod transaction_identity;
mod turso;
