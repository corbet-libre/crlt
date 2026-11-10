# crlt agent notes

Work in a lane (`~/agents/scratch/lane-new <lane>`); never edit `~/agents/scratch/ws`.
Run commands from the lane root (`<lane>/ws`); `<lane>` below is your lane name.

Build and test (quote the verdict line, never infer from a pipe):
- `~/agents/scratch/cbuild <lane> test -p crlt` — green, exit 0
  (`tests/remote.rs` 10 skip without the ephemeral-runner fixture;
  `tests/turso.rs` skips without `TURSO_URL`+`TURSO_TOKEN`).
- `cargo fmt -p crlt -- --check` and `clippy -p crlt --all-targets -- -D warnings`
  — clean/green at HEAD.
- Full gate (locked deps, `.ci/remote.sh` against sqld behind a TLS proxy,
  `ccid coverage run`, advisories) is declared in `.ci/ccid.toml` and runs in CI
  only (`ccid job-run` → `.ci/run-cached.sh`).
  Lane `--locked` needs CI's resolve refresh.

Land: `cargo fmt` in touched repos, rebase on main (never force-push),
`cbuild <lane> xtask land --dry`, then `xtask land`.

Invariants (rule -> enforcing test):
- Callers cannot set `community_id`; conflicts cannot reach another scope
  -> `tests/round_trips.rs::inserts_cannot_choose_community_and_conflicts_cannot_reach_another_scope`,
  `omitted_filters_and_same_ids_remain_isolated`
- Aliases, joins, subqueries, unions and aggregates stay scoped; unscoped
  escape hatches refused without writes
  -> `aliases_joins_subqueries_unions_and_aggregates_are_scoped`,
  `rejects_unscoped_sql_escape_hatches`
- Foreign keys bind both communities and cannot cross them
  -> `tests/foreign_keys.rs::foreign_keys_and_cascades_cannot_cross_communities`,
  `rejects_both_unsafe_actions_on_delete_and_update_without_defaults`
- Migrations are complete, contiguous, immutable; failures roll back DDL
  and history -> `tests/migrations.rs::migration_history_is_complete_contiguous_and_immutable`,
  `failed_pending_batch_rolls_back_ddl_and_history`
- Application statements must use indexed plans; scans are refused
  -> `tests/round_trips.rs::query_plan_helper_confirms_composite_indexes`
- Transaction identity is per database/scope/handle; poisoned transactions
  refuse every follow-up
  -> `tests/transaction_identity.rs::transaction_identity_accepts_clones_and_refuses_other_databases_scopes_and_reopened_handles`,
  `tests/round_trips.rs::poisoned_transactions_refuse_every_followup_operation`
- One database per community; never enable SQL/HTTP debug tracing in a
  deployment (`README.md:81-85`); `forbid(unsafe_code)` (`src/lib.rs`).

Pointers: `src/`, `tests/`, `tests/fixtures/leaf_schemas/README.md`,
`docs/CONTRACT.md`, `docs/DEPENDENCIES.md`.
Commits: explicit paths, imperative, author per `~/agents/AGENT.md`; no AI trailers.
`publish=false`; LGPL-3.0-only WITH linking exception; no registry publication.
