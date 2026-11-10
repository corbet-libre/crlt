# Production coverage and proposed exceptions

The gate requires every non-excluded emitted production line and branch arm to
execute. JSON, LCOV and annotated text come from the same native run, including
real local libSQL and authenticated sqld/TLS tests. Their full source inventories
must agree before any exception is applied. Tests live under `tests/` directories;
earlier inline planner test totals were not production-only measurements.

The gate is `ccid coverage run` (declared in `.ci/ccid.toml`): one instrumented
execution of the workspace with the sqld/TLS fixture up, its `/tests/`-filtered
reports and the emitted-source cross-check, keeping the 100 % line and branch
requirement. The locations in `.ci/ccid.toml` (`[[coverage.exclusions]]`) are
review proposals, not security acceptance. Each exact line or branch arm is bound
to its full source file and the tested Cargo.lock. Updating either invalidates the
proposal, including when CI refreshes a dependency. Executed exceptions must be
removed. A line with a condition requires separate entries for every emitted arm,
not an implicit whole-condition exclusion. The `.github/check-source-coverage.py`
copy stays only for the dispatch-only GitHub workflow; the gate itself is ccid's,
with ccid's own regression suite.

No threshold is reduced and no domain guard is removed. Reports retain the raw
unadjusted counters. Native coverage does not qualify other operating systems,
remote production providers, scalability, or allocation-exhaustion behavior.

## Local driver value conversion

`backend.rs` collects `Row::get_value` over `0..row.column_count()`. In libsql
0.9.30 `src/local/rows.rs`, `Row::get_value` calls the `FromValue` implementation
for `crate::Value`, whose entire result is `Ok(val.into())`. This concrete local
adapter cannot return the error mapped by the collection `?`. Real SQLite query,
row stepping and remote value/width failures remain exercised and gated. This
claim is specific to the locked driver, not arbitrary future row decoders.

## Transaction variant binding

The only construction of `Transaction::Remote` is the remote arm of
`Connection::transaction_with_behavior`, which stores `self.clone()`. There is
no constructor from a local connection, deserializer or mutation of the variant.
Consequently its commit/rollback `Connection::Remote` pattern cannot be false.
Both actual remote autocommit states are tested: the TLS fault proxy forwards the
real operation to sqld before altering its returned state. No transaction or
accepted server response is fabricated to exercise the impossible variant.

## Unix file conversion and allocation limits

These two line exceptions are resource-failure limitations, **not logical
unreachability**. For the native Linux target, url 2.5.8 accepts a parsed hostless
`file:` URL and creates an absolute Unix path. The decoder still uses
`Vec::try_reserve`; a real allocation failure can return an error from either
`Config::validate` or the repeated conversion in `Db::open`. Deterministically
forcing only that allocation to fail would require an allocator seam or a memory
exhaustion campaign. This bounded suite does neither. The defensive mappings stay
in production and are explicitly not claimed as exercised. Resource exhaustion
belongs in separate Crow qualification. No Windows conversion claim is made. These Linux-specific proposals also pin
the complete CI workflow; changing the runner invalidates them.

The `!path.is_absolute()` branch, separately, is unreachable on this native Unix
path: url's `file_url_segments_to_pathbuf` inserts a leading slash for every
segment and asserts absolute output. A root path with no filename is a different,
reachable refusal and is tested, as are remote hosts, tokens and malformed URLs.

## Pool construction

Deadpool 0.13.1 `managed/builder.rs::build` can only fail if a wait/create/recycle
timeout is configured without a runtime. This facade sets only `max_size`; the
builder's default timeouts are all `None`. The builder error mapper is therefore
unreachable. Actual acquisition, connection, recycle, deadline and poisoned
transaction errors remain independently exercised, not excluded.

## Hrana decoded values and HTTPS hosts

libsql-hrana 0.9.30 marks `proto::Value::None` with `serde(skip_deserializing)`;
serde_json refuses non-finite JSON numbers as `f64`. Real wire data therefore
never carries `None` or a non-finite float into the decoder. A direct unit test
exercises the decoder's own contract for both (`None` and NaN/infinite floats are
refused, a finite float is a real number), so no exception remains. The proxy's
actual malformed enum and numeric-value cases still fail at deserialization, as
they should: wire evidence is not replaced by a synthetic decoder enum.

After `url.scheme() == "https"`, `host_str().is_none()` is impossible for a
successfully parsed url 2.5.8 special-scheme HTTPS URL. A hostless non-special
`libsql:` configuration is separately refused and tested. Downgrade, credentials,
query, fragment, missing optional base URL, malformed/truncated/oversized replies
and real TLS configuration failures remain covered.

## Migration community keys

In `audit`, a foreign-key row whose source is `community_id` has already passed
the earlier target-column equality guard or returned an error. At ordinal zero,
the second target-column predicate therefore cannot be false after the first
source-column predicate passes. Mismatched source/target keys, missing parent
tables, replacement actions, unscoped unique keys, imported objects and malformed
migration/index names have real local database refusal tests.

## SQLite parser and visitor invariants

These claims concern sqlparser 0.63.0 with **only** `SQLiteDialect`:

- `parse_table_object` produces `TableName`: the insert-table-function and
  insert-table-query feature methods are false. The other insert-target variant
  and its error line cannot occur.
- `replace_into` is set only by `parse_replace`, which refuses SQLite; insert
  table aliases require `supports_insert_table_alias`, also false. Valid SQLite
  conflict clauses are separately rejected and tested.
- `parse_delete` creates `WithoutKeyword` only for BigQuery, Oracle or Generic.
  The missing-FROM variant and error line cannot occur in SQLite.
- Pipe operators and table index hints require dialect features absent from
  SQLite; table partitions require MySQL or Generic. WITH table hints are a
  distinct reachable field and are tested, not excluded.
- Before visiting an insert source, `scope` requires `SetExpr::Values`. The
  visitor rewrites expressions/table factors but never replaces this body
  discriminant. The subsequent failed destructuring and `unreachable!` cannot run.
- The visitor walks queries, assignments and expressions, never a root Statement.
  A query's `pre_visit_query` rejects CTEs and non-read bodies before descending.
  Accepted Select/Values/Query/SetOperation bodies contain no Statement node;
  nested queries repeat the same guard. Thus `pre_visit_statement` has no reachable
  invocation through the visitor; its own contract (a read passes, a nested write
  is refused) is exercised directly by a unit test, so no exception remains. Real
  nested-write/CTE refusals remain tested at earlier guards.
- Injected table filters parse a fixed SELECT/derived-table template after
  `name` has reduced the identifier to nonempty ASCII letters/digits/underscore,
  without quotes or leading digits. The resulting root is Query, its body is
  Select and its single relation is Derived. The other destructuring arms and
  panic lines cannot run. Original aliases still vary and both alias cases are
  exercised through real scoped queries.

A real left-associated multi-union containing a qualified TABLE body is tested
with the exact facade refusal. The qualification prevents upstream parse_as_table
from consuming the following UNION tokens as part of its three-token lookahead.
This distinguishes recursive `read_body` refusal from an earlier parser error;
that recursive branch has no exception. Parser or driver updates require reviewing these proofs against the
new retained graph, not merely refreshing hashes to make a red gate green.
