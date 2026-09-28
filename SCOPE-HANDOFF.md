# SCOPE-HANDOFF — build/w3b-blast-suppress

Base: `main @ 5fbe1df`. Branch: `build/w3b-blast-suppress`. Not pushed.

## What landed

1. **`crates/vestige-core/src/storage/blast.rs` (new)** — exact blast radius on
   `SqliteMemoryStore`, built only on public storage APIs:
   - `blast_radius(root_id, open_only) -> BlastReport{affected: Vec<{id, via, depth}>, total, root_id}` —
     BFS over `memory_connections` edges with `link_type` in
     `("derived_from", "backfill_candidate", "evidence_of")`, **SOURCE→TARGET
     only** (root = cause/source side; matches backfill's promote block:
     `source_id = cause.memory_id, target_id = failure_node.id`). Depth cap
     `BLAST_MAX_DEPTH = 5`, cycle-safe (first/shallowest visit wins, diamonds
     collapse), zero scoring. Records sharing the root's `commit <sha>` line
     (parser: `commit_sha_of`, hex ≥ 6 chars) are depth-0 siblings
     (`via = "shared_commit:<sha>"`); the root itself is depth 0 (`via = "root"`).
     `open_only=true` filters non-root entries to `valid_until IS NULL or > now`.
   - `blast_radius_with_link_types(root_id, open_only, link_types)` — the
     general form; `suppress`'s cascade uses `["derived_from"]`.
   - `resolve_commit_sha_root(sha_prefix)` — newest record whose `commit <sha>`
     matches a ≥6-hex prefix.
   - `retire_affected(ids, reason) -> Vec<RetireOutcome>` — **not a delete**:
     flips each id through the existing `suppress_memory` mechanism
     (journaled `suppression_operations`, Rac1-eligible, 24h labile reversal),
     per-id outcome.
   - Wired: `mod blast;` + `pub use` in `storage/mod.rs`, root re-exports in
     `lib.rs` (`BlastReport`, `BlastAffected`, `RetireOutcome`, `BLAST_*`,
     `commit_sha_of`).
2. **`crates/vestige-mcp/src/tools/blast_radius.rs` (new)** — MCP tool.
   `action="report"` (default): `{root_id | commit_sha, open_only=true}` →
   read-only report. `action="retire"`: `{ids, reason}` → **each id routed
   through `gate_pending_memory_mutation` under the tool name `"suppress"`**
   (the same pre-call gate PR #296 used for `purge`): Fast mode → gate returns
   None → `retire_affected` suppresses via the gated storage path;
   Risk-Gated/Paranoid → one pending Memory PR per id, **nothing suppressed**,
   output carries `requiresReviewFor` + PR ids; gate errors fail closed.
3. **`tools/suppress.rs`** — new `cascade_derived_from` (default false;
   snake_case accepted via serde alias). When true, the exact `derived_from`
   blast set is computed **before** any suppression, then each target goes
   through the same per-id gate. Reverse path untouched.
4. **`server.rs`** — one appended dispatch arm `"blast_radius"`. No other
   arms touched.

## Gate discipline summary

The review gate is tool-layer (`trace_recorder::gate_pending_memory_mutation`,
called pre-dispatch in `server.rs::handle_tools_call`). It wraps the
storage-level `suppress_memory`/purge paths. Rather than edit `trace_recorder.rs`
(out of scope), the new destructive-adjacent surfaces **call the gate
directly, per id, under the `"suppress"` tool name**, mirroring #296:
- Fast mode keeps historical direct execution.
- Risk-Gated/Paranoid open one pending Memory PR per id and suppress nothing
  until `forget`/`promote`/`quarantine` decides it.
- Gate failures fail closed (no ungated suppression).
- `retire_affected` itself never deletes.

## Deferred for the integration PR (out of this branch's scope)

Advertising `blast_radius` in `tools/list` needs three coordinated changes:
1. `ToolDescription` entry in `fn tool_catalog` (server.rs),
2. `"blast_radius" => blast_radius::schema()` in `tools/compact.rs`
   `full_schema` registry (the `full_schema_registry_matches_the_advertised_catalog`
   test requires every advertised name to resolve there),
3. that test's `assert_eq!(catalog.len(), 16)` → `17`.
Until then `blast_radius` is dispatch-only (same pattern as the documented
hidden back-compat aliases). Consider also adding `"blast_radius"` to
`pending_memory_mutation`'s match in `trace_recorder.rs` so the server-level
pre-gate (not just the tool-internal loop) names it explicitly.

## Tests

- `cargo test -p vestige-core --lib storage::blast` — **11 passed**
  (direction, sha siblings, diamond, cycle+self-loop, depth cap, open_only,
  sha parser, prefix resolution, missing root, retire-not-delete, per-id errors).
- `cargo test -p vestige-mcp --lib tools::blast_radius` — **7 passed**
  (report by root/sha prefix, arg validation, retire Fast: suppresses and rows
  survive; retire Risk-Gated: 3 pending PRs, zero suppression, `requiresReviewFor`).
- `cargo test -p vestige-mcp --lib tools::suppress` — **12 passed**
  (10 pre-existing + cascade Fast vs gated + cascade default-off).
- Regression filters around the touched surfaces — all pass:
  `catalog` (4), `suppress` across the crate (23), `pre_gate` (8),
  `wire_payload` (20 KiB budget holds after the suppress schema grew),
  `interaction_flag`, `output_schema_present`.
