# SCOPE-HANDOFF — build/t5-legacy-isolation (legacy-sqlite quarantine)

(Replaces the w1d-absorb handoff, already merged at 18b13c7.)

Branch: `build/t5-legacy-isolation` (off main @ 82f0cf5). Committed locally;
nothing pushed. Mission: every line of SQLite/rusqlite code moves behind a
`legacy-sqlite` cargo feature (default ON), so the wipe is a flag flip. No
SQLite code was deleted — quarantine only. Sibling strata crates
(`crates/strata-gate`, `crates/strata-kernel`, not workspace members) untouched.

## Acceptance gates (both verified live)

- `cargo check --workspace` — SUCCESS, zero errors.
- `cargo check --workspace --no-default-features` — SUCCESS, zero errors.
- Bonus, beyond the gate: `--all-targets` in BOTH modes also passes with
  zero errors (in-file test modules and integration targets are feature-gated;
  see "tests" below).

## The feature

- `vestige-core`: `legacy-sqlite = ["dep:rusqlite"]`, in `default`
  (default = `["legacy-sqlite", "bundled-sqlite", "codebase-git"]`).
  `bundled-sqlite` and `encryption` now imply `legacy-sqlite`. `rusqlite`
  is `optional = true`.
- `vestige-mcp`: `legacy-sqlite = ["vestige-core/legacy-sqlite",
  "vestige-core/bundled-sqlite", "dep:rusqlite"]`, in `default`. The
  previously hardcoded `features = ["bundled-sqlite"]` on the core dep moved
  into this feature; core dep is now `default-features = false` with no
  unconditional features. mcp's own `rusqlite` (backup WAL checkpoint) is
  `optional = true` under the feature.
- `tests/e2e`, `tests/phase_1`: same pattern (`legacy-sqlite` pass-through,
  default ON; e2e's core dep is now `default-features = false`).

## Constructor path

`vestige_core::storage::open_storage(Option<PathBuf>)` is the sanctioned way
to open a store:

- feature ON → `SqliteMemoryStore::new` (identical behavior).
- feature OFF → returns `LegacySqliteDisabled`, message verbatim:
  `"built without legacy-sqlite; STRATA default lands in the next merge"`.
  `LegacySqliteDisabled` (thiserror) is compiled in EVERY build so surfaces
  can print it regardless of feature state (feature unification can leave
  core's feature on while mcp's is off — the mcp shells print the error type
  directly for exactly that reason).

Call sites migrated: mcp `main.rs` (serve), `bin/restore.rs`, `bin/cli.rs`
(via its local `open_storage()`). The `Storage = SqliteMemoryStore` alias and
`Storage::new` itself now exist only in feature builds.

## Everything now behind `legacy-sqlite`

vestige-core — whole files compiled out without the feature (31 files):
- `storage/sqlite/` — all 12 files (mod, actors, admin, connectors, ingest,
  lifecycle, merge, purge, records, search, sync, tests) plus
  `storage/v3_regression_tests.rs` (included via `#[path]` from sqlite/mod.rs).
- `storage/` siblings that talk rusqlite or the store type (15):
  attestation_store, blast, edges, intention_claim, intention_graph_store,
  maintenance_batches, migrations, portable, replay_store, resolver,
  session_queries, synaptic_store, trace_store, unlearning_store,
  walk_receipts.
- `fts.rs` (SQLite FTS5 sanitizers; its re-export in `search/` gated too).
- `codebase/anchor.rs` (persists/verifies anchors through the store).
- `consolidation/dream_compile.rs` (4-phase engine wired to the store).

vestige-core — partially gated files (gated items, file still compiles):
`lib.rs` (fts mod + gated storage/dream_compile re-exports + prelude split),
`storage/mod.rs` (module decls, re-exports, `Storage` alias, open_storage),
`storage/memory_store.rs` (`From<StorageError>` + one test),
`storage/receipt_attestation.rs` (CounterfactualReplay match arms),
`trace/receipt.rs` (`ReceiptEvidence::CounterfactualReplay` variant + arm),
`search/mod.rs` (fts re-export), `advanced/causal_walk.rs`
(`walk_storage`/`persist_evidence_edges` + tests), `advanced/mod.rs`,
`codebase/mod.rs` (anchor mod/re-exports), `connectors/mod.rs`
(`run_sync` + `ConnectorCursor` import + both test mods), `connectors/github.rs`
(tests), `consolidation/mod.rs` (dream_compile mod/re-exports),
`projection.rs` (`select_durable`/`project` + tests).

vestige-mcp — whole modules gated OFF (57 files): `tools/` (45), `dashboard/`
(6), `resources/` (5), `server.rs`, `cognitive.rs`, `autopilot.rs`,
`actor_surface.rs`, `trace_recorder.rs`, `protocol/http.rs`,
`protocol/stdio.rs`. Partially gated: `lib.rs`, `main.rs` (serve body behind
one cfg block; feature-off shell logs the LegacySqliteDisabled error and
exits 1), `bin/cli.rs` (30 store-backed `run_*`/helpers gated; feature-off
dispatch keeps only `update`/`sandwich` working, every store subcommand
fails with the clear error), `bin/restore.rs` (restore loop gated; parse +
`-h/-V` still work), `protocol/mod.rs`, `Cargo.toml`.

tests: `tests/phase_1` — all 4 [[test]] targets `required-features =
["legacy-sqlite"]`, own rusqlite dep optional under the feature.
`tests/e2e` — all 20 [[test]] targets `required-features = ["legacy-sqlite"]`,
lib prelude split. `vestige-mcp` integration tests (4) `required-features`.
In-file test modules gated with `#[cfg(all(test, feature = "legacy-sqlite"))]`
where they build stores.

## portable.rs verification (mission open question)

`storage/portable.rs` DOES read the SQLite database directly for export
(rusqlite Connection usage), so it took the feature — note the flip
consequence: a feature-off build has no portable export/import until STRATA
ships its own archive path. The pure archive TYPES live in the same file and
are gated with it.

## Files that still reach rusqlite with the feature OFF

Zero. Live grep for `rusqlite` outside `storage/sqlite/`, the gated storage
siblings, and `codebase/anchor.rs` returns only: comment text in
`storage/mod.rs`, and mcp files (`bin/cli.rs`, `tools/receipt.rs`,
`tools/forgotten_lesson.rs`, `dashboard/handlers.rs`, `trace_recorder.rs`)
whose every rusqlite reference sits inside `#[cfg(feature = "legacy-sqlite")]`
modules/functions, plus `tests/phase_1/domain_column_migration.rs` (target
excluded via required-features). `vestige-spacetime` never depended on
rusqlite.

## Known noise (expected, do not "fix" blindly)

- Feature-off builds emit dead-code warnings for helpers whose only callers
  are gated (e.g. `causal_walk::commit_sha_of`, `projection::CANDIDATE_LIMIT`,
  cli's `truncate`). They come back when the callers return; silencing them
  crate-wide would hide real rot.
- mcp feature-off binaries are intentional stubs that exit(1) with the
  quarantine message (only `vestige update`/`sandwich` still function in
  `bin/cli.rs`).
- The flip to default-off (post-STRATA-migration) is: remove `legacy-sqlite`
  from `default` in vestige-core, vestige-mcp, tests/e2e, tests/phase_1.
  Nothing else — that was the point.

## Verification commands (re-run before the flip)

```bash
cargo check --workspace
cargo check --workspace --no-default-features
cargo check --workspace --all-targets
cargo check --workspace --no-default-features --all-targets
```
