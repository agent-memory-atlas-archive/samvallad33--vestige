# strata-store — Scope Handoff

The STRATA-native memory store for the SQLite wipe: `StrataLog` is the only
source of truth; the node registry, typed-edge indexes, and FSRS fold are
derived indexes rebuilt by replay on open (proven bit-identical by
`state_digest()` across open/close/open).

Standalone crate by design: own `Cargo.toml` with an empty `[workspace]`
table. Sibling crates `strata`, `strata-kernel`, `strata-gate` were NOT
edited.

```
cd crates/strata-store && cargo test                                  # 11 passed
cd crates/strata-store && cargo clippy --all-targets -- -D warnings   # clean
```

## Vendored dependency (deviation, read first)

The mission assumed all three sibling crates on main @ 82f0cf5, but
`crates/strata` (the S1 log) lives only on `build/strata-log` (f701e18,
unmerged). This branch vendors `crates/strata` **verbatim** from that commit
(`git checkout build/strata-log -- crates/strata`) so the path dependency
resolves. When `build/strata-log` merges, the vendored copy is a no-op
duplicate — drop it in favor of the merged tree.

## Public API (crate `strata-store`)

```rust
pub struct StrataStore;
impl StrataStore {
    pub fn open(dir) -> Result<Self>;                       // default policy
    pub fn open_with_policy(dir, Policy) -> Result<Self>;   // pinned policy

    // --- vestige-core memory surface ---
    pub fn ingest(&mut self, IngestInput) -> Result<String>;            // default scope
    pub fn ingest_in_scope(&mut self, IngestInput, &str) -> Result<String>;
    pub fn get_node(&self, &str) -> Option<NodeRecord>;
    pub fn get_all_nodes_in_scope(&self, &str) -> Vec<NodeRecord>;      // live only
    pub fn set_created_at(&mut self, &str, i64) -> Result<()>;
    pub fn save_connection(&mut self, &ConnectionRecord) -> Result<()>; // vocab-validated
    pub fn get_connections_for_memory(&self, &str) -> Vec<ConnectionRecord>; // out then in
    pub fn get_edges_for(&self, &str, EdgeDirection, Option<EdgeKind>) -> Vec<ConnectionRecord>;
    pub fn supersede(&mut self, &str, &str) -> Result<()>;              // RETIRE action
    pub fn supersession_pairs(&self) -> Vec<(String, String)>;
    pub fn get_never_composed(&self, &str, usize) -> Vec<(String, String)>; // unlinked pairs
    pub fn is_failure_memory(&self, &str) -> Option<bool>;              // marker heuristic
    pub fn backup_to(&self, dest) -> Result<()>;        // seal + copy segments

    // --- FSRS (strata-kernel folds) ---
    pub fn review(&mut self, &str, rating: u8) -> Result<()>;  // explicit review
    pub fn card_state(&self, &str) -> Option<CardState>;
    pub fn retrievability(&self, &str) -> Result<Option<f64>>; // derived on read

    // --- checkpoints / verification / gate introspection ---
    pub fn seal_checkpoint(&mut self) -> Result<Checkpoint>;
    pub fn verify_checkpoint_chain(&self) -> Result<()>;        // runs on open too
    pub fn sweep(&self) -> Vec<GapRecord>;
    pub fn rederive_verdicts(&self) -> Result<Vec<(u64, Verdict)>>;
    pub fn state_digest(&self) -> [u8; 32];   // canonical blake3 over all derived maps
    pub fn policy / log / node_count / edge_count / orphan_write_count /
        checkpoints / review_event_count
}
pub fn default_policy() -> Policy;   // RETIRE -> Hold, everything else Allow
pub fn handle_of(&str) -> u64;       // blake3-derived FSRS card id
pub fn looks_like_failure(&str, &[String]) -> bool;  // vestige-core marker port
pub struct NodeRecord { id, kernel_id, scope, content, tags, node_type,
    created_at_ms, valid_from_ms, valid_until_ms, superseded_by }  // ints/strings only
pub struct IngestInput { content, node_type, tags, created_at_ms?, valid_from_ms?, valid_until_ms? }
pub struct ConnectionRecord { source_id, target_id, strength_milli, link_type,
    meta_sha?, created_at_ms, activation_count }
pub enum EdgeKind { Touched, AnchoredTo, DerivedFrom, Supersedes, Corrects,
    ClosedBy, ProjectedTo, EvidenceOf }   // same 8-type vocabulary as V39
```

## Write path (every mutation)

`PROPOSE -> GATE (compute_inputs + policy verdict) -> EFFECT (admit()) ->
STORE_WRITE data frame -> apply to derived maps`. Four log frames per
mutation. Denied/Held verdicts stop before the EFFECT; replay applies a data
frame only when an earlier admitted EFFECT carries
`payload_digest == blake3(payload)` (unadmitted frames count as orphans).
Gate contexts are the gate-space effect seqs of referenced nodes' creating
writes, so the gate's fact-id model and `ReadNoReceipt` duty stay coherent.
Default policy: writes Allow, RETIRE Hold (supersession is review-gated).

## FSRS + checkpoints

Every ingest folds one `ReviewEvent { card_id: handle_of(id), rating: 3,
event_seq: <data frame seq> }` through `Kernel::<ReviewEvent>::for_version(
record.kernel_id)` (ALGO_V2 in v1). `seal_checkpoint()` seals over
`fsrs.applied_seq`, appends the checkpoint as frame kind 33, and anchors its
hash in `<dir>/store.meta` (verified via `verify_with_head` on every open —
anchor drift fails closed). Frame kinds: 1..=7 gate records, 32 STORE_WRITE,
33 STORE_CHECKPOINT.

## Deviations from the mission text

1. **GATE+EFFECT cannot share one `append_batch`**: admission requires the
   GATE durable in the log before `commit_effect` runs `admit()` (the gate
   runtime's single append path). Each gate frame is appended individually
   through the `StrataEventLog` adapter; the store data frame uses
   `append_batch`. The log's group commit coalesces them anyway.
2. **Gate seq space is adapter-local** (dense over gate frames only): store
   frames (32/33) interleave with gate frames, so log seqs would leave holes
   in the `EventLog` contract. All gate invariants hold within the space.
3. `NodeRecord` gains `id` + `scope` beyond the mission's field list (needed
   as registry key and for `get_all_nodes_in_scope`).
4. No clocks, no RNG: `created_at_ms` is caller data (default 0); node ids
   are `mem-<next_seq>`; no `chrono` dependency (ms i64s everywhere).
   `IngestInput` drops the float sentiment fields (no floats in persisted
   state); edge strength is `strength_milli: i64`.
5. `set_created_at` takes `i64` ms, not `DateTime<Utc>` (same reason).
6. `!Send` v1: the gate-log cache is `Rc<RefCell<..>>`; single-writer,
   single-threaded (the log's `strata.lock` enforces one store per dir).
7. Replay honors **recorded** gate verdicts, not re-evaluated ones (policy
   rotation retro-stales only future admissions — per the gate handoff);
   `rederive_verdicts()` exists for explicit re-checks.
8. Reads append nothing (mission's v1 stance). `review_events` is retained
   in memory for verification (growth documented; snapshotting is later work).
9. `store.meta` anchor mismatch fails `open` (fail-closed), including the
   tiny crash window between checkpoint append and anchor rewrite.

## Test status

`cargo test` — 11 passed, 0 failed. `cargo clippy --all-targets -- -D
warnings` clean; `cargo fmt` applied. Coverage: ingest/read round-trip (+FSRS
card, retrievability bounds); edge write + reverse/typed queries + vocabulary
rejection; open->close->open bit-identity (`state_digest` equal, sweep clean,
chain verifies); deny policy blocks effects (log holds PROPOSE+GATE only,
reopen stays empty); default-policy Hold vs permissive landing of
supersession; checkpoint chain verify + anchor tamper detection (hash byte and
log_seq byte); backup_to seal/copy/reopen match with post-backup writes
continuing; every STORE_WRITE frame has an admitting EFFECT + rederive
all-Allow; reads append nothing; failure-marker port vs vestige-core canary
cases; log-derived ids and stable handles.
