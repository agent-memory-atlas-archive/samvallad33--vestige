# SCOPE-HANDOFF — `crates/strata-migrate` (`vestige migrate-to-strata`)

The one-shot migration tool: empties the Vestige SQLite store into a STRATA
append-only log. Committed on `build/t3-importer` (base: main @ 82f0cf5,
plus a merge of `build/strata-log` to bring in `crates/strata`, which was
not yet on main — conflict-free, purely additive under `crates/strata/`).

Standalone crate by design: own `Cargo.toml` with an empty `[workspace]`
table (same pattern as `crates/strata`, `crates/strata-kernel`,
`crates/strata-gate`). The root `Cargo.toml` `exclude` list now names all
four standalone strata crates so root members can path-depend on them
without cargo seeing multiple workspace roots; un-exclude when the
integration re-parents them.

## Usage

```
vestige migrate-to-strata <src> <dst>
```

`<src>` is auto-detected: a `vestige.portable.v1` archive JSON (Path A,
preferred), a raw SQLite db file (Path B: opens via vestige-core
`Storage::new` and calls the SAME export code `vestige portable-export`
uses), or a data directory containing `vestige.db`. `<dst>` is the STRATA
log directory (created if missing).

## Public API (crate `strata-migrate`)

```rust
pub fn migrate(source: &Path, strata_dir: &Path) -> Result<MigrationReport, MigrationError>;
pub fn migrate_archive(archive: &PortableArchive, strata_dir: &Path) -> Result<MigrationReport, MigrationError>;
pub struct MigrationReport {
    pub nodes: u64,               // NODE frames appended
    pub edges: u64,               // EDGE frames appended
    pub fsrs_events: u64,         // FSRS_REVIEW frames appended
    pub skipped_tables: Vec<String>, // source tables with rows and no mapping
    pub verify_passed: bool,      // kernel replay AND log tail verify
    pub duration: Duration,       // serde: "durationSeconds" f64
}
pub mod records;   // kind consts + borsh record structs + decode_* fns
pub mod snapshot;  // read_snapshot(&StrataLog) -> Snapshot (log-level reopen)
pub mod source;    // load_archive + Row walker + timestamp/tags parsing
```

## Record map (kind bytes; gate owns 1..=7, 0 reserved)

| code  | kind            | payload (borsh)                                       |
|------:|-----------------|-------------------------------------------------------|
| 0x1F  | MIGRATION_META  | `MigrationMeta` (first frame of a fresh log)           |
| 0x20  | NODE            | `NodeRecord` (kernel_id v1 + legacy UUID + content/tags/type/timestamps i64 ms) |
| 0x21  | EDGE            | `EdgeRecord` (link_type VERBATIM; strength Q32.32)     |
| 0x22  | FSRS_REVIEW     | kernel `ReviewEvent` verbatim                          |
| 0x23  | TOMBSTONE       | `TombstoneRecord` (sync_tombstones + deletion_tombstones) |
| 0x24  | SUPERSESSION    | `SupersessionRecord` (`knowledge_nodes.superseded_by`) |
| 0x25  | CHECKPOINT      | kernel `Checkpoint` verbatim                           |

Every payload starts with `u16 record_version = 1`. `kernel_id` is dense,
1-based, assigned in source row order; `legacy_id` strings are always kept.

## Semantics worth knowing before building on this

- **Legacy link types pass through verbatim.** Vocabulary enforcement is
  admission-time for NEW writes; migration never rewrites history.
- **FSRS fold is counter-exact, not float-exact.** SQLite stores only final
  FSRS state, so each card becomes `reps - lapses` rating-3 events then
  `lapses` rating-1 events. The kernel fold reproduces `review_count == reps`
  and `lapse_count == lapses` exactly; stability/difficulty are recomputed
  by the deterministic fold (legacy floats are not reconstructible from any
  log and are intentionally not carried).
- **event_seq == frame seq** (kernel requirement). Seqs are predicted from
  the log head before `append_batch` and asserted against the returned acks.
- **Finish protocol:** batch appends (1024-frame batches) → fold ALL review
  events in the log → `Checkpoint::seal(ALGO_V1, last_frame_seq, prev_hash,
  &state)` appended as the final frame → `StrataLog::seal()` → verify
  (`verify_with_head` with the in-memory checkpoint hash as anchor, plus
  `verify_tail`). The CLI refuses to declare success if verification fails.
- **Re-running into the same dir is append-only**: records duplicate, the
  checkpoint chain extends (strictly increasing log_seq; zero-append re-runs
  skip the duplicate checkpoint). Tested.
- **Unmapped tables** (node_embeddings, memory_access_log, intentions,
  sessions, composition_*, …) are reported in `skipped_tables`, not migrated.
- **Path B side effects:** opening a live store with `Storage::new` may
  create `-wal`/`-shm` siblings and runs idempotent migrations; logical
  contents are never modified.
- **Dangling edges / fsrs cards** (impossible under FK cascade, possible in
  a hand-edited archive) fail the migration with a named error — nothing is
  silently dropped.

## Files touched

- `crates/strata-migrate/` — new standalone crate (lib, records, source,
  snapshot, integration tests).
- `crates/vestige-mcp/src/bin/cli.rs` — `MigrateToStrata` variant, one
  dispatch arm, one self-contained `run_migrate_to_strata` block.
- `crates/vestige-mcp/Cargo.toml` — `strata-migrate` path dep.
- Root `Cargo.toml` — `exclude` the four standalone strata crates.
- Merge commit bringing in `crates/strata` from `build/strata-log`.

## Test status

`cd crates/strata-migrate && cargo test` — 10 passed, 0 failed (5 unit +
5 integration; integration builds a real store via the public vestige-core
API, plants fsrs_cards history, exports, migrates, reopens the log, asserts
identical node ids / edge endpoints / exact FSRS counters / checkpoint
chain). `cargo clippy --all-targets -- -D warnings` clean; `cargo fmt`
applied. Full root workspace: `cargo test --workspace` green (see commit).
CLI smoke-tested live against archive, db-file, and directory sources.
