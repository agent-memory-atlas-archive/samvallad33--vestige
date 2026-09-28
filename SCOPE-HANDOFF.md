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
