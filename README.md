# crlt

**Relational database facade over Turso/libSQL: connections, migrations, community namespaces.**

Part of `cvld`, the permanent door of the cmtymeet trust stack. Native Rust,
async with Tokio, no ORM. Interfaces may change. See [the contract](docs/CONTRACT.md)
for the isolation boundary and supported SQL.

## Dependency survey

Surveyed crates.io releases and upstream sources on 2026-09-30:

| Candidate | Choice and reason |
|---|---|
| [libsql 0.9.30](https://crates.io/crates/libsql/0.9.30) (MIT) | Latest stable. Keep its local engine (`core`); disable the legacy `remote` and `tls` features. Its generic Hrana client is private, so it cannot accept an external sender. The newer 0.10.0-pre.4 still uses hyper 0.14 and hyper-rustls 0.25, so upgrading does not fix the HTTP/TLS advisories. |
| [reqwest 0.13.5](https://docs.rs/reqwest/0.13.5/reqwest/) (MIT/Apache-2.0) | Selected maintained HTTP client with rustls 0.23 and platform certificate validation. Carries Hrana 3 HTTP pipelines with verified HTTPS, bounded responses and no automatic retries. |
| [libsql-hrana 0.9.30](https://docs.rs/libsql-hrana/0.9.30/libsql_hrana/) (MIT) | Official protocol structures and serialization. A small private adapter manages execute/status/close pipelines and transaction batons; avoids vendoring or forking the full libsql client. |
| [deadpool 0.13.1](https://docs.rs/deadpool/0.13.1/deadpool/) (MIT/Apache-2.0) | Retain bounded leases and health checks with a private local/Hrana connection adapter. deadpool-libsql 0.1.1 only constructs libsql's built-in connections, so it cannot inject the transport-independent sender. |
| [hyper-rustls 0.27](https://docs.rs/hyper-rustls/) (Apache-2.0/ISC/MIT) | Considered as a custom libsql connector. Replacing TLS alone would leave libsql's unconditional hyper 0.14/h2 0.3 dependency; using it directly would require more HTTP plumbing than reqwest. |
| [sqlparser 0.63](https://github.com/apache/datafusion-sqlparser-rs) (Apache-2.0) | Maintained SQLite parser and AST visitors; structural scoping avoids unsafe substring checks or a homegrown SQL parser. |
| [libsql_migration 0.2.2](https://github.com/prashant1k99/libsql_migration) (MIT) | Examined, not selected: its history has no community key, records execution timestamps, and script/history updates are separate operations without atomic drift validation. |
| [refinery 0.9.2](https://github.com/rust-db/refinery) (MIT) | Examined, not selected: SQLite support is through rusqlite, not this remote libsql connection; custom adaptation would still need our schema boundary. |

The remaining dependencies are Tokio (runtime deadlines/locks), `url` (URL
validation), `serde_json` (official Hrana wire types), and `thiserror` (typed
errors), all permissively licensed.
`tempfile` is test-only. The small migration ledger is facade policy; database
execution, pooling, parsing, wire encoding and TLS remain delegated to maintained libraries.
See [dependency maintenance](docs/DEPENDENCIES.md) for the audit policy.

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
uniqueness, cross-community foreign keys, triggers and views. Namespace columns
cannot have defaults. Foreign keys containing the namespace column cannot use
`SET DEFAULT` or `SET NULL` on either deletion or update; `CASCADE`, `RESTRICT`
and `NO ACTION` preserve the immutable community binding. Failed audits roll
back the DDL and ledger before exposing the schema to handles. The ledger keeps
no timestamps. Add data transformations through scoped transactions.

`Config` provides `max_connections`, `timeout`, `busy_timeout` and `max_rows`.
An error poisons a transaction; explicitly roll back or drop it. Remote
timeouts can leave write outcomes uncertain, so callers must reconcile before
retrying. No logging subscriber is installed and crlt emits no request logs;
services must also keep dependency SQL/HTTP debug tracing disabled.

## Verification

GitHub Actions runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test` and `cargo deny check advisories licenses bans sources` on stable
Rust. Unmaintained/yanked dependencies also fail the audit.
Tests use real temporary libSQL files, including pooling, rollback, migration
failure/drift, namespace isolation and index plans. Foreign-key regressions
reproduce the raw libSQL namespace transfer, reject its migration, check scoped
deletions/updates in two communities, and migrate the
[existing leaf schema snapshots](tests/fixtures/leaf_schemas/README.md).

The required remote suite runs against sqld 0.24.32 behind a TLS proxy on the
ephemeral runner, with a generated trusted CA and a disposable Ed25519 JWT.
`.ci/remote.sh` runs the explicitly ignored `remote` tests (missing fixture
configuration fails rather than skipping). It covers values, namespaces,
migrations, query plans, commit/rollback/drop, row limits, deadlines and pool
recovery, cancelled replies and uncertain writes without retries, plus rejection
of untrusted/wrong-host certificates, bad tokens, redirects, malformed session
replies and Hrana TLS downgrades. Python is used only by the CI TLS proxy;
it is not a library or production dependency.

The optional `turso` integration test skips unless both `TURSO_URL` and
`TURSO_TOKEN` are nonempty. Supply a **disposable test database** through the
environment on an approved test runner: the test applies its schema and
writes test rows. Credentials belong neither in this repository nor in CI.
Never publish this crate to a registry.

## License

Copyright 2026 Julian Y. Richard Corbet. Licensed under [LGPL-3.0-only](LICENSES/LGPL-3.0-only.txt) [WITH LGPL-3.0-linking-exception](LICENSES/LGPL-3.0-linking-exception.txt). See [LICENSE.md](LICENSE.md).

## Continuous verification

Dependency updates follow main and are tested against one CI-resolved lockfile.
Line and branch coverage target 100%; failures remain blocking. See
[the coverage contract](docs/COVERAGE.md) for measurement and exclusions.


The TLS configuration refusal test reuses Reqwest's maintained Rustls/AWS-LC
provider in a separate integration-test process. Installing its empty cipher list
exercises a real client-construction failure without replacing crypto or attempting
a network connection. This test-only dependency is already used by the transport.
