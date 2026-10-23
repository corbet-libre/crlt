# crlt contract

## Binding library section

### crlt: relational database facade (LGPL, corbet-foss)
| | |
|---|---|
| Scope | Connections to Turso/libSQL (SQLite for tests), pooling, TLS, timeouts, migrations, community namespace injected by the caller |
| Interface | `Db::open(config)`, `db.migrate(migrations)`, `db.tx(...)`; passes SQL through (no ORM) |
| Never | Reads credentials itself (passed in by the service); logs requests |
| Accept | Migrations idempotent; two communities in one DB cannot read each other's rows (tested); query plans use indexes (tested with `EXPLAIN QUERY PLAN`) |

## Cross-library rules applied here

Rust on current stable; LGPL-3.0-only WITH LGPL-3.0-linking-exception.
Survey maintained libraries before implementation. Use the official `libsql`
crate, with no custom cryptography. Deploy one database per community; the
global service has its own database. Every table carries `community_id` and
application queries must use indexed searches. No login dates, request logs,
raw gate data, credential discovery, or registry publication. Real database
round trips run on GitHub Actions.

## Implementation decisions

- `Db` is the composition root's pool and migration authority. `Community`
  is the capability handed to a leaf library; it cannot change its scope or
  expose a raw connection. Selecting a community is the caller's authorization
  responsibility. Independent communities remain isolated even in one file.
- SQL is parsed, scoped, then executed by the local libSQL engine or remote sqld. Every base table in a read
  becomes a filtered derived table. Inserted community keys are supplied by the
  handle; updates/deletes receive an additional parenthesized predicate.
  Callers cannot supply or modify `community_id` in writes.
- Queries, joins, nested subqueries, set operations, aggregates, `INSERT VALUES`,
  simple updates/deletes, and `RETURNING` are supported. CTEs, insert-select,
  upserts, replacement writes, table functions, arbitrary scalar functions,
  attached databases, pragmas, DDL and transaction control are rejected on
  scoped handles. This deliberately bounded SQL surface is not an ORM or a
  sandbox for hostile SQL. Migration authors and database administrators are
  trusted; out-of-band schema changes while handles are live are unsupported.
- Parameters use explicit `?1`, `?2`, etc. The facade renumbers them and binds
  its own community parameter. Parameter values are never interpolated.
- Migrations are a complete, ordered sequence numbered from one. Exact SQL and
  names detect edited history without hashing. Pending DDL and history records
  commit atomically under an immediate transaction. The ledger has a fixed
  schema namespace in its community key, without execution timestamps.
- Schema migrations accept table/index creation, alteration and deletion, but
  no data manipulation, views, triggers, virtual tables or transaction control.
  The resulting schema is audited before commit: every table has a non-null
  TEXT community key leading its primary key, every unique index starts with
  that key using binary collation, and foreign keys bind both communities.
  Community columns have no defaults, and every occurrence in a foreign key
  must reference the parent's community column. `ON DELETE` and `ON UPDATE`
  cannot use `SET DEFAULT` or `SET NULL` on a foreign key containing that column.
  `CASCADE`, `RESTRICT` and `NO ACTION` remain supported: scoped writes cannot
  update the parent's community key. Rejected audits roll back both DDL and
  migration history; reopening also requires a successful audit before use.
- Application statements undergo `EXPLAIN QUERY PLAN`; unbounded scans are
  refused. Constant-row plans are exempt. The public `QueryPlan::assert_indexed`
  helper also serves regression tests. Schema introspection and DDL are the
  only metadata/maintenance exceptions to the application index requirement.
- TLS and HTTP are delegated to reqwest/rustls; remote execution uses the
  Hrana 3 pipeline and official libsql-hrana wire types. Redirects, plaintext and automatic retries are
  disabled. Only `libsql://` and `https://` remote URLs are
  accepted, with tokens passed by the caller. Local `file:///absolute/path`
  URLs are supported; use temporary files for tests. In-memory URLs are not
  supported because independent pooled connections would lose their contents.
- The default pool size is one. A configurable deadline covers acquisition,
  network operations and row collection; a transaction shares one deadline.
  Local SQLite lock waits use a busy timeout. Tokio deadlines cannot preempt
  synchronous SQLite CPU work. Failed/cancelled operations discard their pool
  lease. An errored transaction cannot commit; drop rolls it back via libSQL
  (remote cleanup is best effort, with server expiry as the fallback).
- Rows are collected before releasing a lease and bounded by a configurable
  row limit. Remote pipeline replies additionally have a 16 MiB wire-body cap
  before decoding; page larger results. A timeout on a remote write/commit can have an uncertain outcome;
  the library never automatically retries writes.
- This is a native server library. No browser/wasm target is required by its
  architecture. The optional live Turso test reads environment variables only
  in the test binary and never receives credentials through public CI.
