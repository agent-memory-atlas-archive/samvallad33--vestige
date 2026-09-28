<<<<<<< HEAD
<<<<<<< HEAD
<<<<<<< HEAD
<<<<<<< HEAD
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
=======
# SCOPE-HANDOFF — build/w3c-session-sync

Base: main @ 5fbe1df. Scope held: `crates/vestige-mcp/src/tools/session_context.rs`,
`crates/vestige-mcp/src/tools/source_sync.rs`, new
`crates/vestige-core/src/storage/session_queries.rs` (+ the one-line module
wiring and re-export in `crates/vestige-core/src/storage/mod.rs`, required to
compile the new file). Nothing else touched.

## What was built

### 1. `session_queries.rs` (new, on SqliteMemoryStore)

- `open_failures_touching(changed_files: &[String]) -> Vec<OpenFailureTouching{id, content_preview, anchor}>`
  — failure-like nodes (`retroactive_backfill::looks_like_failure`, reuse) that
  are open (not superseded/suppressed, inside validity window) and whose
  recorded files intersect the changed set by EXACT path equality:
  `code_memory_anchors.file_path` first (anchor rendered `path` or
  `path:symbol`), then the `files:` line of a git-commit record (the
  ` (+N more)` suffix stripped). Prefix/suffix near-misses do not hit. Bounded
  chunked node scan (pages of 500, hard cap 10k nodes).
- `last_session_failed_calls(run_id: Option<&str>) -> Vec<FailedToolCall{run_id, tool, at, error_excerpt}>`
  — `agent_traces` rows of the given run, or the latest run by
  `agent_runs.last_at` when `None`, whose serialized `mcp.call` payload
  carries `success: false`. Last 20, chronological. Rows without a `success`
  field are not failed calls.
- `append_mcp_call_outcome(run_id, tool, success, error, at_ms)` — writes an
  `mcp.call`-shaped trace row extended with `success`/`error` (argsHash is the
  opaque constant `"outcome"`). Payload keeps the exact event shape so
  `get_trace` replay still parses it. NOTE: no production caller yet — the
  trace recorder (`trace_recorder.rs`) is out of scope and still writes
  plain `mcp.call` events, so the failed-calls section stays empty until the
  recorder adopts this method. That adoption is the natural follow-up.
- `closed_issue_nodes(source_system, scope)` / `git_commit_nodes(limit)` —
  exact-tag lookups (`state:closed`, `git-commit`) backing the closed_by link.

### 2. `session_context.rs` (session_start) — purely additive

- New optional arg `changed_files: Vec<String>` (schema + serde, camelCase
  alias). Absent = open-failures section skipped. Present but no exact match =
  no section (silence, never an empty header).
- Section `**Open failures touching changed files:**` — up to 8 lines
  `- [id] preview (anchor)`, budget-accounted like every other section.
- Section `**Last session failed calls (run):**` — automatic, latest run, up
  to 8 lines `- tool: error_excerpt`.
- Both sections degrade silently on query error; all existing sections and
  the budget-trim loop are untouched.

### 3. `source_sync.rs` — `closed_by` chain linking (github, local-only)

- Verified what the github connector fetches today: issues + comments only —
  no closing-PR sha, no timeline/cross-referenced events. Per the build
  instruction, remote fetch is SKIPPED; linking is local-only and
  deterministic.
- `commit_closes_issue(content, issue_number)` — GitHub closing keywords
  (close/closes/closed/fix/fixes/fixed/resolve/resolves/resolved), whole-word,
  case-insensitive, and `#<number>` with exact digit boundaries (`#420` never
  satisfies #42; `abc#42`/`##42` rejected), keyword + reference on the SAME
  line.
- `link_closed_by_from_local_commits(storage, scope)` — for each live closed
  github issue in scope, writes `memory_connections` edges
  `link_type="closed_by"`, `source_id` = issue node, `target_id` = commit
  node, via the existing `save_connection` (INSERT OR REPLACE ⇒ idempotent
  re-syncs). Runs after every successful github `run_sync`; count surfaced as
  `closedByLinks` in the tool result and appended to the summary when > 0.
  Redmine is untouched.

## Tests (all green)

- `cargo test -p vestige-core --lib session_queries` — 6/6.
- `cargo test -p vestige-core --lib storage::` — 372/372.
- `cargo test -p vestige-mcp --lib session` — 27/27 (4 new:
  open-failures populate from anchors + `files:` lines, negative no-noise
  near-miss/absent-arg, failed-calls latest-run, silent without outcome rows).
- `cargo test -p vestige-mcp --lib source_sync` — 4/4 (keyword forms,
  near-miss rejects incl. `discloses`/`#420`/cross-line, edge appears +
  idempotent + open-issue/commit negatives, silent without candidates).
- Full `cargo test -p vestige-mcp --lib`: 764 passed, 2 failed —
  `server::tests::test_recall_lookup_matches_search_shape` and
  `tools::backfill::tests::live_backfill_surfaces_root_cause_through_storage`
  both REPRODUCE ON PRISTINE main @ 5fbe1df (verified via stash); pre-existing,
  not from this branch.

## Follow-ups (out of scope here)

- Recorder adoption of `append_mcp_call_outcome` in `trace_recorder.rs` /
  `server.rs` dispatch so the failed-calls section has a production producer.
- Remote closing-PR lookup if ever wanted: needs a timeline-events fetch in
  the github connector (new API surface, deliberately not added).
>>>>>>> build/w3c-session-sync
# SCOPE-HANDOFF — build/w3c-session-sync

Base: main @ 5fbe1df. Scope held: `crates/vestige-mcp/src/tools/session_context.rs`,
`crates/vestige-mcp/src/tools/source_sync.rs`, new
`crates/vestige-core/src/storage/session_queries.rs` (+ the one-line module
wiring and re-export in `crates/vestige-core/src/storage/mod.rs`, required to
compile the new file). Nothing else touched.

## What was built

### 1. `session_queries.rs` (new, on SqliteMemoryStore)

- `open_failures_touching(changed_files: &[String]) -> Vec<OpenFailureTouching{id, content_preview, anchor}>`
  — failure-like nodes (`retroactive_backfill::looks_like_failure`, reuse) that
  are open (not superseded/suppressed, inside validity window) and whose
  recorded files intersect the changed set by EXACT path equality:
  `code_memory_anchors.file_path` first (anchor rendered `path` or
  `path:symbol`), then the `files:` line of a git-commit record (the
  ` (+N more)` suffix stripped). Prefix/suffix near-misses do not hit. Bounded
  chunked node scan (pages of 500, hard cap 10k nodes).
- `last_session_failed_calls(run_id: Option<&str>) -> Vec<FailedToolCall{run_id, tool, at, error_excerpt}>`
  — `agent_traces` rows of the given run, or the latest run by
  `agent_runs.last_at` when `None`, whose serialized `mcp.call` payload
  carries `success: false`. Last 20, chronological. Rows without a `success`
  field are not failed calls.
- `append_mcp_call_outcome(run_id, tool, success, error, at_ms)` — writes an
  `mcp.call`-shaped trace row extended with `success`/`error` (argsHash is the
  opaque constant `"outcome"`). Payload keeps the exact event shape so
  `get_trace` replay still parses it. NOTE: no production caller yet — the
  trace recorder (`trace_recorder.rs`) is out of scope and still writes
  plain `mcp.call` events, so the failed-calls section stays empty until the
  recorder adopts this method. That adoption is the natural follow-up.
- `closed_issue_nodes(source_system, scope)` / `git_commit_nodes(limit)` —
  exact-tag lookups (`state:closed`, `git-commit`) backing the closed_by link.

### 2. `session_context.rs` (session_start) — purely additive

- New optional arg `changed_files: Vec<String>` (schema + serde, camelCase
  alias). Absent = open-failures section skipped. Present but no exact match =
  no section (silence, never an empty header).
- Section `**Open failures touching changed files:**` — up to 8 lines
  `- [id] preview (anchor)`, budget-accounted like every other section.
- Section `**Last session failed calls (run):**` — automatic, latest run, up
  to 8 lines `- tool: error_excerpt`.
- Both sections degrade silently on query error; all existing sections and
  the budget-trim loop are untouched.

### 3. `source_sync.rs` — `closed_by` chain linking (github, local-only)

- Verified what the github connector fetches today: issues + comments only —
  no closing-PR sha, no timeline/cross-referenced events. Per the build
  instruction, remote fetch is SKIPPED; linking is local-only and
  deterministic.
- `commit_closes_issue(content, issue_number)` — GitHub closing keywords
  (close/closes/closed/fix/fixes/fixed/resolve/resolves/resolved), whole-word,
  case-insensitive, and `#<number>` with exact digit boundaries (`#420` never
  satisfies #42; `abc#42`/`##42` rejected), keyword + reference on the SAME
  line.
- `link_closed_by_from_local_commits(storage, scope)` — for each live closed
  github issue in scope, writes `memory_connections` edges
  `link_type="closed_by"`, `source_id` = issue node, `target_id` = commit
  node, via the existing `save_connection` (INSERT OR REPLACE ⇒ idempotent
  re-syncs). Runs after every successful github `run_sync`; count surfaced as
  `closedByLinks` in the tool result and appended to the summary when > 0.
  Redmine is untouched.

## Tests (all green)

- `cargo test -p vestige-core --lib session_queries` — 6/6.
- `cargo test -p vestige-core --lib storage::` — 372/372.
- `cargo test -p vestige-mcp --lib session` — 27/27 (4 new:
  open-failures populate from anchors + `files:` lines, negative no-noise
  near-miss/absent-arg, failed-calls latest-run, silent without outcome rows).
- `cargo test -p vestige-mcp --lib source_sync` — 4/4 (keyword forms,
  near-miss rejects incl. `discloses`/`#420`/cross-line, edge appears +
  idempotent + open-issue/commit negatives, silent without candidates).
- Full `cargo test -p vestige-mcp --lib`: 764 passed, 2 failed —
  `server::tests::test_recall_lookup_matches_search_shape` and
  `tools::backfill::tests::live_backfill_surfaces_root_cause_through_storage`
  both REPRODUCE ON PRISTINE main @ 5fbe1df (verified via stash); pre-existing,
  not from this branch.

## Follow-ups (out of scope here)

- Recorder adoption of `append_mcp_call_outcome` in `trace_recorder.rs` /
  `server.rs` dispatch so the failed-calls section has a production producer.
- Remote closing-PR lookup if ever wanted: needs a timeline-events fetch in
  the github connector (new API surface, deliberately not added).
=======
# Scope Handoff — build/w3e-receipts-status

Base: `main @ 5fbe1df`. Branch: `build/w3e-receipts-status`. Not pushed.

## What was built

### 1. Walk receipts (core storage)

New file `crates/vestige-core/src/storage/walk_receipts.rs` (migration **V39**, non-destructive, `walk_receipts` table: `receipt_id PK, digest UNIQUE, canonical_json, engine_version, created_at`).

- `canonical_walk_json(params: &Value) -> Result<String>` — canonicalizes via
  `serde_json_canonicalizer` (RFC 8785 / JCS), the same helper the receipt DSSE
  chain already uses. Sorted keys, no insignificant whitespace, stable number
  formatting; rejects non-object envelopes.
- `Storage::save_walk_receipt(canonical_json: &str, params: &Value) -> Result<WalkReceiptHandle>`
  — re-canonicalizes `params` internally and **rejects** a `canonical_json` that
  does not match (digest always computable from stored bytes). Digest =
  `blake3(canonical bytes)` hex (dep already in core Cargo.toml). `receipt_id =
  "wr_" + digest[..24]` → saving the same envelope is idempotent
  (`reused_existing: true`, single row). `engine_version` =
  `env!("CARGO_PKG_VERSION")` at save time.
- `Storage::get_walk_receipt(receipt_id) -> Result<Option<StoredWalkReceipt>>`.
- `Storage::coverage_snapshot() -> Result<CoverageSnapshot>` — exact SQL
  aggregates for the coverage view (see §3).

### 2. Replay placement (scope adjustment, per task instruction)

`tools::backfill::execute` lives in the MCP layer only, so **replay lives in the
MCP layer** (`crates/vestige-mcp/src/tools/receipt.rs`), not core. The core file
contributes constants only: `WALK_RECEIPT_SCHEMA_V1`,
`WALK_RECEIPT_CLAIM_BOUNDARY` (`trace/receipt.rs`, re-exported at crate root).

`receipt` tool surface (schema `additionalProperties: false`, `deny_unknown_fields`):

- `{action: "save_walk", params: {...}}` → `{receiptId, digest, canonicalParams
  (byte-stable echo), engineVersion, reusedExisting, claimBoundary}`. No clock
  fields → byte-stable output.
- `{action: "replay", receipt_id: "wr_…" [, remove_edge]}` → dispatches on walk
  receipts (ids in `walk_receipts`); non-`wr_` ids keep the counterfactual
  capsule replay untouched. `remove_edge` (memory id or `source->target`)
  filters that candidate from the pool before the ablated run. Replay is always
  a **pure preview**: recorded `promote/git_repo/worked_in/broke_in/why_not`
  are reported as ignored, never applied (no edges written, no reinforcement).
  Runs the reach twice (baseline + ablated) with a candidate assembly mirroring
  `backfill::build_candidates` (supersession-following, commit-tag flag; no git
  version-range re-application), asserts both runs evaluated the same failure,
  and reports `verdictDelta {filterApplied, verdictChanged, baseline, ablated}`
  (ids + 2-dp scores only → deterministic serialization; failure entities are
  sorted because `extract_entities` iterates a hash set).
- Validation: `withheld_slots` xor `remove_edge`; `params` only for
  `save_walk`; `receipt_id` not for `save_walk`.

### 3. memory_status view="coverage"

`{anchorCoveragePct (COUNT(DISTINCT node_id) code_memory_anchors / COUNT(*)
knowledge_nodes, 2-dp, 0.0 on empty store), anchoredNodes, totalNodes,
edgeCountsByType (memory_connections GROUP BY link_type ORDER BY link_type),
indexFreshness {newestGitCommitRecord + AgeDays (MAX(created_at) over
json_each(tags)='git-commit' nodes), newestAgentTraceAt + AgeHours (MAX(at)
agent_traces, millis), stalenessNote (deterministic thresholds: commit >30d,
trace >72h)}}` + claim boundary.

## Canonicalization spec

RFC 8785 (JCS) via `serde_json_canonicalizer::to_vec`; digest = blake3 of the
UTF-8 canonical bytes, hex; id = `wr_` + first 24 hex chars. Same value → same
bytes → same digest → same id, regardless of input key order/whitespace.

## Test status

- `cargo test -p vestige-core --lib walk_receipt` — 3/3 pass
  (canonicalization stability incl. key-order shuffle + idempotent save +
  mismatch rejection; coverage math on seeded store).
- `cargo test -p vestige-mcp --lib tools::receipt` — 9/9 pass (save twice →
  same digest/id; replay byte-identical on unchanged store; remove_edge
  verdict delta incl. `src->tgt` form and no-op edge; promote never applied).
- `cargo test -p vestige-mcp --lib tools::memory_status` — 5/5 pass
  (incl. coverage math: 1/5 anchored = 20.0%, grouped/ordered edge counts).
- `cargo test -p vestige-core --lib migrations` — 36/36 pass (V39 registered).
- `cargo clippy -p vestige-core -p vestige-mcp --lib` — clean.

### Pre-existing failures on the CLEAN base (not touched, out of scope)

Verified by stashing this diff and re-running:

- `tools::backfill::tests::live_backfill_surfaces_root_cause_through_storage` —
  the seeded distractor ("500 Internal Server Error…") outranks the env-var
  cause under the current word-tier IDF scorer. The walk replay mirrors the
  live tool's ranking faithfully, which is why the replay tests are
  rank-agnostic (ablate whatever is ranked top).
- `server::tests::test_recall_lookup_matches_search_shape` — asserts parity
  with the removed `search` tool.

## Files changed

`crates/vestige-core/src/storage/walk_receipts.rs` (new),
`storage/migrations.rs` (V39), `storage/mod.rs` (mod + re-exports),
`trace/receipt.rs` + `trace/mod.rs` + `lib.rs` (walk constants + re-exports),
`crates/vestige-mcp/src/tools/receipt.rs`, `tools/memory_status.rs`,
`server.rs` (receipt tool description string only — it enumerates actions).
>>>>>>> build/w3e-receipts-status
=======
# Scope handoff — build/w2a-hunk-lines

Base: main @ 5fbe1df. Scope: `crates/vestige-core/src/advanced/git_records.rs` + tests only.

## Out-of-scope file touched (minimal, compile-only)

`crates/vestige-mcp/src/tools/backfill.rs` — test helper `commit_record_content`
(line ~861) constructs `GitCommit` literally. Added the three new fields with
empty values so the crate compiles:

```rust
hunks: vec![],
extra_hunks: 0,
imports: vec![],
```

No logic changed; `tools::backfill` tests pass unchanged.

## What changed in git_records.rs

- `HunkSpan { file, start, len, symbol }` — new-side `@@` spans, `MAX_HUNKS = 200`,
  overflow counted in `GitCommit::extra_hunks`.
- `GitCommit::imports: Vec<(file_in_commit, target_path, resolved)>` —
  `MAX_IMPORTS = 40`, deduped, sorted by (file, target).
- `record_content` new lines:
  - `hunks: file:start+len` comma-joined, item list capped at 400 chars
    (`MAX_HUNK_LINE`), followed by ` (+N more)` when spans were dropped or
    overflowed; emitted only when spans exist.
  - `imports: file->target` comma-joined; unresolved edges render as
    `file->?target` (target kept as written, never guessed).
- Import capture: `use a::b::Item` / `import x.y.z` (multi-segment only, so
  `import os` noise is skipped) / `from x.y import z` / `#include "p"` and
  `#include <p>`. Resolution is exact module-segment matching against the same
  commit's file list and their module dirs (`crate` roots at `src/`; Rust
  `.rs`/`mod.rs`, Python `.py`/`__init__.py`, include verbatim). File hits beat
  module-dir hits; longest path first. No fuzzy matching.
- Hunks/imports for files beyond `MAX_FILES` are dropped (same
  non-attribution rule as symbols), not counted as span overflow.

## Pre-existing test failure (not from this branch)

`tools::backfill::tests::live_backfill_surfaces_root_cause_through_storage`
(crates/vestige-mcp) fails on pristine base @ 5fbe1df — verified via stash
with these changes removed: the billing-service distractor outranks the
env-var cause. The test never touches `GitCommit`/`record_content`; the two
commits below base reworked exactly that ranking code. All other
`tools::backfill` tests pass.

## Incident note: shared stash stack race

This repo's worktrees share one `git stash` stack. During verification, a
`git stash pop` in this worktree raced sibling worktrees and pulled in two
foreign stashes. Both were re-stashed onto the shared stack as labeled
entries — find them with `git stash list | grep RESCUE`:

- `RESCUE-A(65a5ea0)` — storage/session_context/source_sync changes, owner:
  build/w2c-resolver worktree.
- `RESCUE-B(ecfcddfb)` — dashboard cinema changes (MemoryCinema, sandbox,
  storm, temporal-emissive.ts), owner: feat/dashboard-live-max worktree.

Original dropped hashes are recoverable from the stash reflog if needed.
Use hash-based `git stash apply <hash>` (resolved by message) in this repo,
never positional `stash@{n}`.
>>>>>>> build/w2a-hunk-lines
# Scope handoff — build/w2a-hunk-lines

Base: main @ 5fbe1df. Scope: `crates/vestige-core/src/advanced/git_records.rs` + tests only.

## Out-of-scope file touched (minimal, compile-only)

`crates/vestige-mcp/src/tools/backfill.rs` — test helper `commit_record_content`
(line ~861) constructs `GitCommit` literally. Added the three new fields with
empty values so the crate compiles:

```rust
hunks: vec![],
extra_hunks: 0,
imports: vec![],
```

No logic changed; `tools::backfill` tests pass unchanged.

## What changed in git_records.rs

- `HunkSpan { file, start, len, symbol }` — new-side `@@` spans, `MAX_HUNKS = 200`,
  overflow counted in `GitCommit::extra_hunks`.
- `GitCommit::imports: Vec<(file_in_commit, target_path, resolved)>` —
  `MAX_IMPORTS = 40`, deduped, sorted by (file, target).
- `record_content` new lines:
  - `hunks: file:start+len` comma-joined, item list capped at 400 chars
    (`MAX_HUNK_LINE`), followed by ` (+N more)` when spans were dropped or
    overflowed; emitted only when spans exist.
  - `imports: file->target` comma-joined; unresolved edges render as
    `file->?target` (target kept as written, never guessed).
- Import capture: `use a::b::Item` / `import x.y.z` (multi-segment only, so
  `import os` noise is skipped) / `from x.y import z` / `#include "p"` and
  `#include <p>`. Resolution is exact module-segment matching against the same
  commit's file list and their module dirs (`crate` roots at `src/`; Rust
  `.rs`/`mod.rs`, Python `.py`/`__init__.py`, include verbatim). File hits beat
  module-dir hits; longest path first. No fuzzy matching.
- Hunks/imports for files beyond `MAX_FILES` are dropped (same
  non-attribution rule as symbols), not counted as span overflow.

## Pre-existing test failure (not from this branch)

`tools::backfill::tests::live_backfill_surfaces_root_cause_through_storage`
(crates/vestige-mcp) fails on pristine base @ 5fbe1df — verified via stash
with these changes removed: the billing-service distractor outranks the
env-var cause. The test never touches `GitCommit`/`record_content`; the two
commits below base reworked exactly that ranking code. All other
`tools::backfill` tests pass.

## Incident note: shared stash stack race

This repo's worktrees share one `git stash` stack. During verification, a
`git stash pop` in this worktree raced sibling worktrees and pulled in two
foreign stashes. Both were re-stashed onto the shared stack as labeled
entries — find them with `git stash list | grep RESCUE`:

- `RESCUE-A(65a5ea0)` — storage/session_context/source_sync changes, owner:
  build/w2c-resolver worktree.
- `RESCUE-B(ecfcddfb)` — dashboard cinema changes (MemoryCinema, sandbox,
  storm, temporal-emissive.ts), owner: feat/dashboard-live-max worktree.

Original dropped hashes are recoverable from the stash reflog if needed.
Use hash-based `git stash apply <hash>` (resolved by message) in this repo,
never positional `stash@{n}`.
=======
# SCOPE-HANDOFF — Handle-Based Recall Resolver (build/w2c-resolver)

Branch: `build/w2c-resolver`, base = `main` @ `5fbe1df`. Not pushed.

## Files (the entire scope)

- `crates/vestige-core/src/storage/resolver.rs` — NEW. The resolver.
- `crates/vestige-core/src/storage/mod.rs` — wiring only (`mod resolver;` + re-exports).
- `crates/vestige-mcp/src/tools/recall.rs` — thin handle-mode extension at entry.
- `SCOPE-HANDOFF.md` — this file.

## Core API (vestige-core)

```rust
// crates/vestige-core/src/storage/resolver.rs
impl SqliteMemoryStore {
    pub fn resolve_handle(&self, query: &str) -> HandleResolution;
}

pub struct HandleResolution {
    pub kind: HandleKind,                     // what the query resolved as
    pub ids: Vec<String>,                     // node/trace ids resolved
    pub exact: bool,                          // true iff equality (not prefix)
    pub candidates: Vec<(String, HandleKind)>,// when prefix-ambiguous, capped at 20
    pub handle_required: Option<String>,      // guidance when nothing matched
}

pub enum HandleKind { Memory, Commit, File, Symbol, Test, Run, ToolCall, Tag, Unknown }

pub const MAX_CANDIDATES: usize = 20;
pub const HANDLE_REQUIRED_DETAIL: &str =
    "recall is handle-based: pass a memory id, commit sha, file, symbol, test, run, or tool-call id";
```

Import paths: `vestige_core::storage::{HandleKind, HandleResolution, MAX_CANDIDATES, HANDLE_REQUIRED_DETAIL}`. `resolve_handle` is a method on `SqliteMemoryStore` (`vestige_core::Storage`). Read-only: no FSRS/edge/graph writes.

## Resolution rules (EXACT or PREFIX only — no fuzzy, no lexical ranking, no FTS)

| # | Kind | Match | Prefix? | Source |
|---|------|-------|---------|--------|
| 1 | Memory | query parses as uuid, equals `knowledge_nodes.id` | no | `knowledge_nodes` |
| 2 | Commit | `commit <sha>` on content line 1 of `git-commit`-tagged nodes; case-insensitive | **yes, >= 7 hex chars**; 4–6 hex chars w/ a digit = ambiguity error; unique prefix resolves with `exact=false`; multiple = capped candidates | `knowledge_nodes` |
| 3 | File / Test | whole-token boundary match over content + tags (coarse LIKE prefilter, exact Rust verify). File if path-shaped (`/` or `.`); Test if test-shaped (`tests/`, `_test`, `test_` prefix) | no | `knowledge_nodes` |
| 4 | Symbol | query normalized camel→snake+lowercase (same `extract_entities` normalization); matched against Code-tier extracted entities only — Word/Path/Version tiers excluded so `pyvenv` can never prefix-match `pyvenv.cfg` | **yes (unique prefix)** | `knowledge_nodes` |
| 5 | Run | equals `agent_runs.run_id` or `agent_traces.run_id` | no | V18 tables |
| 6 | ToolCall | equals `agent_traces.id` (trace event id) | no | V18 tables |
| 7 | Tag | exact string equality against parsed tags JSON (case-sensitive) | no | `knowledge_nodes` |

Unresolved / empty → `kind=Unknown`, `handle_required=Some(HANDLE_REQUIRED_DETAIL)`.

## MCP surface (recall)

New schema property `handle` (string). Semantics in `recall::execute`:

- args WITHOUT a `handle` key → legacy mode dispatch, byte-identical behavior
  (hot-path invariant preserved; existing tests pass unchanged). **This gate
  (`handle_flow` returning None) is the single flip point for making recall
  handle-only by default.**
- args WITH `handle: "<handle>"` → resolve, then:
  - resolved: `{"handle", "kind", "exact", "nodes":[{id,type,content,tags}], "neighbors":[{from,to,link_type,strength,direction,node}]}` — neighbors are one hop over `memory_connections` (both directions, strength-desc, deduped, capped at 20 edges).
  - ambiguous: `{"error":"ambiguous", "detail", "handle", "kind", "candidates":[{id,kind}]}`.
  - nothing: `{"error":"handle_required", "detail", "candidates":[]}` (`detail` carries the resolver's specific too-short-sha message when applicable).
- args WITH `handle: ""` (or non-string) → free-text branch: same
  `handle_required` payload, with candidates mined by running the resolver on
  the whole `query` text and its first 8 identifier-shaped tokens (exact/prefix
  results only, deduped, capped at 20).

## Integrator notes

- Scope-agnostic by design: handles are globally unique, so resolution ignores
  `scope`. Filter client-side if a scoped surface is needed.
- Symbol prefix scans all nodes and runs `extract_entities` per row (no entity
  index yet). Correctness-first; add an entity index before high-QPS use.
- Commit resolution reads shas only from `git-commit`-tagged records in the
  canonical `git_records::record_content` shape.
- Known deviation from the letter of the design: File and Test share one
  boundary-exact step (file before symbol per the order); a test-shaped query
  resolves as kind `Test` even before the symbol step would have matched.
- English hex words without digits (`face`, `added`) are not treated as
  too-short shas; 4–6 char hex WITH a digit is the ambiguity error.

## Tests

- `cargo test -p vestige-core --lib resolver` → 8 passed (uuid exact/prefix
  paths, full/prefix/ambiguous/too-short shas, file exact + NO-fuzzy +
  NO-prefix receipts (`pyvenv` and `pyvenv.c` both fail against `pyvenv.cfg`),
  symbol camel/snake/env normalization + prefix, test names, tags exact,
  run/toolcall exact, free prose unresolved).
- `cargo test -p vestige-mcp --lib tools::recall` → 7 passed (schema, legacy
  default, contradictions, uuid + one-hop neighbors ordering, sha exact +
  ambiguous, handle_required with/without candidates, no-handle legacy path).
- `cargo test -p vestige-core --lib "storage::sqlite"` → 220 passed (no
  regressions in the neighboring suite).
- Pre-existing failure, NOT from this branch (fails on clean `main` @
  `5fbe1df` too): `vestige-mcp server::tests::test_recall_lookup_matches_search_shape`
  — it calls the removed `search` tool and compares against its error text.
  `server.rs` is outside this task's file scope.
>>>>>>> build/w2c-resolver
# SCOPE-HANDOFF — Handle-Based Recall Resolver (build/w2c-resolver)

Branch: `build/w2c-resolver`, base = `main` @ `5fbe1df`. Not pushed.

## Files (the entire scope)

- `crates/vestige-core/src/storage/resolver.rs` — NEW. The resolver.
- `crates/vestige-core/src/storage/mod.rs` — wiring only (`mod resolver;` + re-exports).
- `crates/vestige-mcp/src/tools/recall.rs` — thin handle-mode extension at entry.
- `SCOPE-HANDOFF.md` — this file.

## Core API (vestige-core)

```rust
// crates/vestige-core/src/storage/resolver.rs
impl SqliteMemoryStore {
    pub fn resolve_handle(&self, query: &str) -> HandleResolution;
}

pub struct HandleResolution {
    pub kind: HandleKind,                     // what the query resolved as
    pub ids: Vec<String>,                     // node/trace ids resolved
    pub exact: bool,                          // true iff equality (not prefix)
    pub candidates: Vec<(String, HandleKind)>,// when prefix-ambiguous, capped at 20
    pub handle_required: Option<String>,      // guidance when nothing matched
}

pub enum HandleKind { Memory, Commit, File, Symbol, Test, Run, ToolCall, Tag, Unknown }

pub const MAX_CANDIDATES: usize = 20;
pub const HANDLE_REQUIRED_DETAIL: &str =
    "recall is handle-based: pass a memory id, commit sha, file, symbol, test, run, or tool-call id";
```

Import paths: `vestige_core::storage::{HandleKind, HandleResolution, MAX_CANDIDATES, HANDLE_REQUIRED_DETAIL}`. `resolve_handle` is a method on `SqliteMemoryStore` (`vestige_core::Storage`). Read-only: no FSRS/edge/graph writes.

## Resolution rules (EXACT or PREFIX only — no fuzzy, no lexical ranking, no FTS)

| # | Kind | Match | Prefix? | Source |
|---|------|-------|---------|--------|
| 1 | Memory | query parses as uuid, equals `knowledge_nodes.id` | no | `knowledge_nodes` |
| 2 | Commit | `commit <sha>` on content line 1 of `git-commit`-tagged nodes; case-insensitive | **yes, >= 7 hex chars**; 4–6 hex chars w/ a digit = ambiguity error; unique prefix resolves with `exact=false`; multiple = capped candidates | `knowledge_nodes` |
| 3 | File / Test | whole-token boundary match over content + tags (coarse LIKE prefilter, exact Rust verify). File if path-shaped (`/` or `.`); Test if test-shaped (`tests/`, `_test`, `test_` prefix) | no | `knowledge_nodes` |
| 4 | Symbol | query normalized camel→snake+lowercase (same `extract_entities` normalization); matched against Code-tier extracted entities only — Word/Path/Version tiers excluded so `pyvenv` can never prefix-match `pyvenv.cfg` | **yes (unique prefix)** | `knowledge_nodes` |
| 5 | Run | equals `agent_runs.run_id` or `agent_traces.run_id` | no | V18 tables |
| 6 | ToolCall | equals `agent_traces.id` (trace event id) | no | V18 tables |
| 7 | Tag | exact string equality against parsed tags JSON (case-sensitive) | no | `knowledge_nodes` |

Unresolved / empty → `kind=Unknown`, `handle_required=Some(HANDLE_REQUIRED_DETAIL)`.

## MCP surface (recall)

New schema property `handle` (string). Semantics in `recall::execute`:

- args WITHOUT a `handle` key → legacy mode dispatch, byte-identical behavior
  (hot-path invariant preserved; existing tests pass unchanged). **This gate
  (`handle_flow` returning None) is the single flip point for making recall
  handle-only by default.**
- args WITH `handle: "<handle>"` → resolve, then:
  - resolved: `{"handle", "kind", "exact", "nodes":[{id,type,content,tags}], "neighbors":[{from,to,link_type,strength,direction,node}]}` — neighbors are one hop over `memory_connections` (both directions, strength-desc, deduped, capped at 20 edges).
  - ambiguous: `{"error":"ambiguous", "detail", "handle", "kind", "candidates":[{id,kind}]}`.
  - nothing: `{"error":"handle_required", "detail", "candidates":[]}` (`detail` carries the resolver's specific too-short-sha message when applicable).
- args WITH `handle: ""` (or non-string) → free-text branch: same
  `handle_required` payload, with candidates mined by running the resolver on
  the whole `query` text and its first 8 identifier-shaped tokens (exact/prefix
  results only, deduped, capped at 20).

## Integrator notes

- Scope-agnostic by design: handles are globally unique, so resolution ignores
  `scope`. Filter client-side if a scoped surface is needed.
- Symbol prefix scans all nodes and runs `extract_entities` per row (no entity
  index yet). Correctness-first; add an entity index before high-QPS use.
- Commit resolution reads shas only from `git-commit`-tagged records in the
  canonical `git_records::record_content` shape.
- Known deviation from the letter of the design: File and Test share one
  boundary-exact step (file before symbol per the order); a test-shaped query
  resolves as kind `Test` even before the symbol step would have matched.
- English hex words without digits (`face`, `added`) are not treated as
  too-short shas; 4–6 char hex WITH a digit is the ambiguity error.

## Tests

- `cargo test -p vestige-core --lib resolver` → 8 passed (uuid exact/prefix
  paths, full/prefix/ambiguous/too-short shas, file exact + NO-fuzzy +
  NO-prefix receipts (`pyvenv` and `pyvenv.c` both fail against `pyvenv.cfg`),
  symbol camel/snake/env normalization + prefix, test names, tags exact,
  run/toolcall exact, free prose unresolved).
- `cargo test -p vestige-mcp --lib tools::recall` → 7 passed (schema, legacy
  default, contradictions, uuid + one-hop neighbors ordering, sha exact +
  ambiguous, handle_required with/without candidates, no-handle legacy path).
- `cargo test -p vestige-core --lib "storage::sqlite"` → 220 passed (no
  regressions in the neighboring suite).
- Pre-existing failure, NOT from this branch (fails on clean `main` @
  `5fbe1df` too): `vestige-mcp server::tests::test_recall_lookup_matches_search_shape`
  — it calls the removed `search` tool and compares against its error text.
  `server.rs` is outside this task's file scope.
