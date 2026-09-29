# crlt

**Relational database facade over Turso/libSQL: connections, migrations, community namespaces.**

Part of `cvld`, the permanent door of the cmtymeet trust stack. Native Rust,
async with Tokio, no ORM. Interfaces may change. See [the contract](docs/CONTRACT.md)
for the isolation boundary and supported SQL.

## Dependency survey

Surveyed crates.io releases and upstream sources on 2026-09-30:

| Candidate | Choice and reason |
|---|---|
| [libsql 0.9.30](https://github.com/tursodatabase/libsql/tree/main/libsql) (MIT) | Required official client; local files, remote transactions and TLS. Enable only core, remote and TLS, without replication/sync. |
| [deadpool-libsql 0.1.1](https://github.com/deadpool-rs/deadpool/tree/main/libsql) (MIT/Apache-2.0) | Maintained pool adapter over the same official client; bounded leases, reuse and connection health checks. |
| [sqlparser 0.63](https://github.com/apache/datafusion-sqlparser-rs) (Apache-2.0) | Maintained SQLite parser and AST visitors; structural scoping avoids unsafe substring checks or a homegrown SQL parser. |
| [libsql_migration 0.2.2](https://github.com/prashant1k99/libsql_migration) (MIT) | Examined, not selected: its history has no community key, records execution timestamps, and script/history updates are separate operations without atomic drift validation. |
| [refinery 0.9.2](https://github.com/rust-db/refinery) (MIT) | Examined, not selected: SQLite support is through rusqlite, not this remote libsql connection; custom adaptation would still need our schema boundary. |

The remaining dependencies are Tokio (runtime deadlines/locks), `url` (URL
validation), and `thiserror` (typed errors), all permissively licensed.
`tempfile` is test-only. The small migration ledger is facade policy; database
execution, pooling, parsing and TLS remain delegated to maintained libraries.

## Use

Create a `Db` with `Config::new(url, token)`, call `migrate` with the complete
migration history, then give leaf libraries a `db.community(key)?` handle.
Deploy a separate database per community. Choosing handles is an authorization
decision belonging to the service, not to a member request.

```rust,no_run
use crlt::{Config, Db, Migration, params};

# async fn example(url: String, token: String) -> crlt::Result<()> {
let db = Db::open(Config::new(url, token)).await?;
db.migrate(&[Migration::new(1, "members", "
    CREATE TABLE members (
        community_id TEXT NOT NULL,
        id TEXT NOT NULL,
        handle TEXT NOT NULL,
        PRIMARY KEY (community_id, id),
        UNIQUE (community_id, handle)
    ) WITHOUT ROWID;
")]).await?;
let community = db.community("example")?;
let mut tx = community.tx().await?;
tx.execute("INSERT INTO members (id, handle) VALUES (?1, ?2)",
    params!["pseudonym", "member_handle"]).await?;
tx.commit().await?;

let rows = community.query("SELECT handle FROM members WHERE id = ?1",
    ["pseudonym"]).await?;
assert_eq!(rows[0].get_str(0)?, "member_handle");
community.explain("SELECT handle FROM members WHERE id = ?1",
    ["pseudonym"]).await?.assert_indexed()?;
# Ok(())
# }
```

The community filter is automatic, including joins and subqueries. Do not
include `community_id` in inserted columns or update assignments. Use numbered
positional parameters; values and SQL remain separate. Raw connections and
unscoped application queries are intentionally absent.

Migrations use trusted DDL, contiguous versions starting at one, and immutable
names/SQL. Each call is atomic and repeatable. Schema checks reject global
uniqueness, cross-community foreign keys, triggers and views. The ledger keeps
no timestamps. Add data transformations through scoped transactions.

`Config` provides `max_connections`, `timeout`, `busy_timeout` and `max_rows`.
An error poisons a transaction; explicitly roll back or drop it. Remote
timeouts can leave write outcomes uncertain, so callers must reconcile before
retrying. No logging subscriber is installed and crlt emits no request logs;
services must also keep dependency SQL/HTTP debug tracing disabled.

## Verification

GitHub Actions runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`
and `cargo test` on stable Rust. Do not run Cargo on the workstation.
Tests use real temporary libSQL files, including pooling, rollback, migration
failure/drift, namespace isolation and index plans.

The optional `turso` integration test skips unless both `TURSO_URL` and
`TURSO_TOKEN` are nonempty. Supply a **disposable test database** through the
environment on an approved test runner: the test applies its schema and
writes test rows. Credentials belong neither in this repository nor in CI.
Never publish this crate to a registry.

## License

Copyright 2026 Julian Y. Richard Corbet. Licensed under [LGPL-3.0-only](LICENSES/LGPL-3.0-only.txt) [WITH LGPL-3.0-linking-exception](LICENSES/LGPL-3.0-linking-exception.txt). See [LICENSE.md](LICENSE.md).
