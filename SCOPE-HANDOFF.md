<<<<<<< HEAD
# SCOPE-HANDOFF — build/w2b-edge-schema (typed edge vocabulary)

Branch `build/w2b-edge-schema`, base `main @ 5fbe1df`. Storage layer only;
`tools/mcp` untouched. This file lists the exact interfaces other agents
build against.

## Migration

**V39** (`crates/vestige-core/src/storage/migrations.rs`), schema version
bumped 38 → 39. Non-destructive; existing rows keep NULL for the new columns:

- `memory_connections.edge_meta TEXT` — JSON payload `{"sha","span","run_id"}`
- `memory_connections.created_by_run TEXT` — run provenance
- `idx_connections_type_target` on `(link_type, target_id)`
- new table `purge_tombstones(purged_id PK, purged_at, reason, prior_content_hash)`
  (+ `idx_purge_tombstones_purged_at`)

Vocabulary validation is app-level by design — no CHECK constraint, so legacy
link types (`semantic`, `temporal`, ...) still validate through
`save_connection`. `knowledge_nodes.superseded_by` (V14) was NOT migrated;
`supersedes` exists only as an edge type.

## Public API — `vestige_core::storage` (new re-exports)

Types: `EdgeKind`, `EdgeDirection`, `EdgeMeta`, `TypedEdge`, `PurgeTombstone`,
`TYPED_EDGE_VOCABULARY`. New error variant
`StorageError::InvalidEdge(String)` (maps to
`MemoryStoreError::InvalidInput`).

```rust
pub const TYPED_EDGE_VOCABULARY: &[&str] = &[
    "touched", "anchored_to", "derived_from", "supersedes", "corrects",
    "closed_by", "projected_to", "evidence_of",
];

pub enum EdgeKind { Touched, AnchoredTo, DerivedFrom, Supersedes, Corrects,
                    ClosedBy, ProjectedTo, EvidenceOf }
impl EdgeKind {
    pub const ALL: [EdgeKind; 8];
    pub fn as_str(self) -> &'static str;
    pub fn parse(link_type: &str) -> Option<EdgeKind>;   // None outside vocabulary
}

pub enum EdgeDirection { Outgoing, Incoming, Both }

#[derive(Serialize, Deserialize, Default)]
pub struct EdgeMeta { pub sha: Option<String>, pub span: Option<String>,
                      pub run_id: Option<String> }

pub struct TypedEdge {
    pub source_id: String, pub target_id: String,
    pub link_type: String,          // validated against vocabulary on save
    pub strength: f64,              // typed edges default to 1.0
    pub meta: EdgeMeta,             // -> edge_meta JSON column
    pub created_by_run: Option<String>,
    pub created_at: DateTime<Utc>, pub last_activated: DateTime<Utc>,
    pub activation_count: i32,
}
impl TypedEdge {
    pub fn new(source_id: impl Into<String>, target_id: impl Into<String>,
               link_type: &str) -> Self;
}

pub struct PurgeTombstone { pub purged_id: String, pub purged_at: DateTime<Utc>,
    pub reason: Option<String>, pub prior_content_hash: Option<String> }
```

## SqliteMemoryStore methods (`crates/vestige-core/src/storage/edges.rs`)

```rust
impl SqliteMemoryStore {
    /// Rejects link_type outside TYPED_EDGE_VOCABULARY (Err::InvalidEdge).
    /// INSERT OR REPLACE into memory_connections incl. edge_meta/created_by_run.
    /// FKs enforced: both endpoints must exist in knowledge_nodes.
    pub fn save_typed_edge(&self, edge: &TypedEdge) -> Result<()>;

    /// Typed (vocabulary-only) edges incident to node_id by direction.
    /// Legacy connections stay invisible here.
    pub fn edges_for(&self, node_id: &str, dir: EdgeDirection) -> Result<Vec<TypedEdge>>;

    /// Forward BFS (source->target) over the given kinds. Cycle-safe, exact,
    /// depth-bounded; returns reached ids sorted, start excluded; empty
    /// link_types => empty result.
    pub fn edge_reachability(&self, start_id: &str, link_types: &[EdgeKind],
                             max_depth: usize) -> Result<Vec<String>>;

    /// root_id + derived_from closure, sorted. READ-ONLY: deletes/writes
    /// nothing; output is the review-gate input for a retire decision.
    pub fn retire_subgraph(&self, root_id: &str) -> Result<Vec<String>>;

    /// Writes purge_tombstones row; prior_content_hash = SHA-256 of the node's
    /// content if the row still exists (simulated purge), else NULL.
    /// Re-record replaces (one row per id).
    pub fn record_tombstone(&self, purged_id: &str, reason: &str) -> Result<()>;

    pub fn get_purge_tombstone(&self, purged_id: &str) -> Result<Option<PurgeTombstone>>;
}
```

## Notes for downstream agents

- Edge rows share the `memory_connections` PK `(source_id, target_id)` with
  legacy connections; a typed save overwrites a legacy row with the same pair.
- Node ids for files/symbols/spans (`anchored_to`, `touched`) must be
  `knowledge_nodes` rows — decide the node-creation convention before wiring
  the MCP surface.
- Review gating for `supersedes`/`corrects` is policy for the tools layer;
  storage only guarantees vocabulary validation.
- `evidence_of` replaces `backfill_candidate` for NEW writes; nothing
  rewrites existing `backfill_candidate` rows (out of scope here).
- Tests: `cargo test -p vestige-core --lib edges` — 4 tests (typed
  save/reject, diamond+cycle reachability, non-deleting retire, tombstone on
  simulated purge). Migration replay/idempotence guards pass (35 tests).
=======
# SCOPE-HANDOFF — w3d: selftest + forgotten_lesson

Branch `build/w3d-selftest-forgotten` (base `main` @ `5fbe1df`). All additions
are append-only except three marked test-constant updates forced by parity
guards; nothing outside the listed files was touched.

## New exposed APIs

### `vestige-mcp` MCP tools

| Tool | Module | Signature |
|---|---|---|
| `selftest` | `crates/vestige-mcp/src/tools/selftest.rs` | `pub async fn execute(storage: &Arc<Storage>, args: Option<Value>) -> Result<Value, String>` — no arguments |
| `forgotten_lesson` | `crates/vestige-mcp/src/tools/forgotten_lesson.rs` | same shape; args `{failure_id (required), scope?, scan_limit?}` |

`tools::selftest::schema()`, `tools::forgotten_lesson::schema()` — full JSON
schemas; both registered in `tools::compact::full_schema` (so
`memory_status view='tools'` unfolds them).

Pure helpers other agents can reuse:

- `tools::forgotten_lesson::detect_lesson(&KnowledgeNode, &HashSet<String> failure_anchors, DateTime<Utc> failure_at) -> Option<ForgottenLesson>` — one shared detection rule (lesson tag / exact past-tense fix marker + >=1 exact shared anchor + FSRS R < 0.5 at failure time).
- `tools::forgotten_lesson::{lesson_tagged, has_fix_marker}`, `pub const FORGOTTEN_THRESHOLD: f64 = 0.5`.
- `tools::selftest` internals are private; the calibration flow is only the `execute` tool.

### CLI subcommands (`crates/vestige-mcp/src/bin/cli.rs`)

- `vestige selftest` — prints the same payload as the MCP tool.
- `vestige forgotten-lesson <FAILURE_ID> [--scope S] [--json]`.

## Calibration definition (selftest)

`gap_calibration = true` iff the 6th round (cause and failure share NO anchor)
returns `triggered=true` with an EMPTY causes list AND a non-null `gap` whose
`missing_entities` contains the withheld env-shaped anchor `planted_cause_6`.
Rounds 1-5 each isolate one planted cause (5d backdated, own scope) with a
failure sharing exactly one anchor; hit@1 = cause ranked first, hit@3 = top 3.
The live store is only read (`Storage::backup_to` / `VACUUM INTO` snapshot to a
tempdir); all mutation happens on the copy; the temp store is deleted.

## Non-append-only edits (forced by existing parity guards)

`crates/vestige-mcp/src/server.rs` tests only: advertised-tool count 16 -> 18
(two places), read-only hint list now
`["forgotten_lesson", "memory_status", "selftest", "session_start"]`. Catalog
and dispatch arms themselves are append-only additions.

## Test status

- `cargo test -p vestige-mcp --lib selftest` → 2/2 ok.
- `cargo test -p vestige-mcp --lib forgotten` → 5/5 ok.
- Full `--lib` suite: 763 passed, 2 failed — BOTH failures
  (`server::tests::test_recall_lookup_matches_search_shape`,
  `tools::backfill::tests::live_backfill_surfaces_root_cause_through_storage`)
  reproduce on the clean base commit (verified via stash) and are pre-existing.
- `cargo clippy -p vestige-mcp --lib --bins`: clean.
>>>>>>> build/w3d-selftest-forgotten
# SCOPE-HANDOFF — w3d: selftest + forgotten_lesson

Branch `build/w3d-selftest-forgotten` (base `main` @ `5fbe1df`). All additions
are append-only except three marked test-constant updates forced by parity
guards; nothing outside the listed files was touched.

## New exposed APIs

### `vestige-mcp` MCP tools

| Tool | Module | Signature |
|---|---|---|
| `selftest` | `crates/vestige-mcp/src/tools/selftest.rs` | `pub async fn execute(storage: &Arc<Storage>, args: Option<Value>) -> Result<Value, String>` — no arguments |
| `forgotten_lesson` | `crates/vestige-mcp/src/tools/forgotten_lesson.rs` | same shape; args `{failure_id (required), scope?, scan_limit?}` |

`tools::selftest::schema()`, `tools::forgotten_lesson::schema()` — full JSON
schemas; both registered in `tools::compact::full_schema` (so
`memory_status view='tools'` unfolds them).

Pure helpers other agents can reuse:

- `tools::forgotten_lesson::detect_lesson(&KnowledgeNode, &HashSet<String> failure_anchors, DateTime<Utc> failure_at) -> Option<ForgottenLesson>` — one shared detection rule (lesson tag / exact past-tense fix marker + >=1 exact shared anchor + FSRS R < 0.5 at failure time).
- `tools::forgotten_lesson::{lesson_tagged, has_fix_marker}`, `pub const FORGOTTEN_THRESHOLD: f64 = 0.5`.
- `tools::selftest` internals are private; the calibration flow is only the `execute` tool.

### CLI subcommands (`crates/vestige-mcp/src/bin/cli.rs`)

- `vestige selftest` — prints the same payload as the MCP tool.
- `vestige forgotten-lesson <FAILURE_ID> [--scope S] [--json]`.

## Calibration definition (selftest)

`gap_calibration = true` iff the 6th round (cause and failure share NO anchor)
returns `triggered=true` with an EMPTY causes list AND a non-null `gap` whose
`missing_entities` contains the withheld env-shaped anchor `planted_cause_6`.
Rounds 1-5 each isolate one planted cause (5d backdated, own scope) with a
failure sharing exactly one anchor; hit@1 = cause ranked first, hit@3 = top 3.
The live store is only read (`Storage::backup_to` / `VACUUM INTO` snapshot to a
tempdir); all mutation happens on the copy; the temp store is deleted.

## Non-append-only edits (forced by existing parity guards)

`crates/vestige-mcp/src/server.rs` tests only: advertised-tool count 16 -> 18
(two places), read-only hint list now
`["forgotten_lesson", "memory_status", "selftest", "session_start"]`. Catalog
and dispatch arms themselves are append-only additions.

## Test status

- `cargo test -p vestige-mcp --lib selftest` → 2/2 ok.
- `cargo test -p vestige-mcp --lib forgotten` → 5/5 ok.
- Full `--lib` suite: 763 passed, 2 failed — BOTH failures
  (`server::tests::test_recall_lookup_matches_search_shape`,
  `tools::backfill::tests::live_backfill_surfaces_root_cause_through_storage`)
  reproduce on the clean base commit (verified via stash) and are pre-existing.
- `cargo clippy -p vestige-mcp --lib --bins`: clean.
