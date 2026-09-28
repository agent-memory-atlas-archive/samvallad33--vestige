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
