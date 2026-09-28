# SCOPE-HANDOFF — causal_walk (build/w3a-causal-walk)

`causal_walk` REPLACES `backfill` as the advertised flagship. `backfill`
(module `crates/vestige-mcp/src/tools/backfill.rs`, core
`advanced/retroactive_backfill.rs`) is UNMODIFIED and stays dispatchable as a
hidden alias. Nothing else outside the files below was touched.

## Files

| File | Change |
| --- | --- |
| `crates/vestige-core/src/advanced/causal_walk.rs` | NEW — engine + storage assembly + promote + 10 unit tests |
| `crates/vestige-core/src/advanced/mod.rs` | `pub mod causal_walk;` + re-exports |
| `crates/vestige-mcp/src/tools/causal_walk.rs` | NEW — schema + execute + tests |
| `crates/vestige-mcp/src/tools/mod.rs` | `pub mod causal_walk;` |
| `crates/vestige-mcp/src/tools/compact.rs` | full-schema registry: `backfill` → `causal_walk` (registry tracks the ADVERTISED catalog; hidden names must not enter it) |
| `crates/vestige-mcp/src/server.rs` | catalog entry swapped (count stays 16), `causal_walk` dispatch arm added, `backfill` arm marked hidden alias, advertised-surface test updated |
| `crates/vestige-mcp/src/bin/cli.rs` | NEW subcommand block only: `CausalWalk` variant + dispatch arm + `run_causal_walk` handler |
| `SCOPE-HANDOFF.md` | this file |

## Mechanisms (start point → suspects)

1. `failing_test {name}` → records naming the test (test-file records) →
   commit records touching their path-tier entities → each commit's OTHER
   files are the suspects. Hop: `failing_test/co_touch`, hop text
   `<test-file> -> <other files>`.
2. `stack_frame {frame: "file:line"|"file"}` → commit records whose hunk
   spans (files / `file/symbol` symbols) cover the file → the LAST commit
   touching it before the failure anchor is the prime suspect (SZZ-lite;
   earlier touchers land in `rejected`). Hop: `stack_frame/last_toucher`.
3. `ci_run {run_id}` → `agent_traces` run via `storage.get_trace(run_id)` →
   failure channel = `SanhedrinVeto` claim text + `MemoryRetrieve`/`MemoryWrite`/
   veto `evidence_ids` records → their entities → records. Hop:
   `ci_run/failed_calls`.
4. `logged_write {node_id}` → the node → its `get_connections_for_memory`
   edges (neighbours seeded as edge evidence, surviving with 0 shared
   anchors) + records sharing its entities. Hops: `logged_write/edge`,
   `logged_write/shared_anchor`.
5. `version_range {worked_in, broke_in, repo}` → `git rev-list
   worked_in..broke_in` sha set (`git_records::parse_rev_list`) → RESTRICTOR:
   out-of-range commit seeds rejected `outside version range W..B`; in-range
   commits overlapping the anchor pool get hop `version_range/rev_list`.
   Alone (no failure start point) it yields no anchors → needs_report.
   Multiple distinct repos → needs_report (single repo per call).

## Core API (vestige-core::advanced::causal_walk)

```rust
pub const EVIDENCE_LINK_TYPE: &str = "evidence_of";
pub const MAX_HOPS: usize = 2;

pub enum StartPoint { FailingTest{name}, StackFrame{frame}, CiRun{run_id},
                      LoggedWrite{node_id}, VersionRange{worked_in,broke_in,repo} }
// serde: {"kind":"failing_test","name":...} etc. (tag="kind", snake_case)

pub struct CausalWalkRequest { pub scope: String, pub start_points: Vec<StartPoint>,
                               pub lookback_days: i64, pub scan_limit: i32 } // Default: user/30/500
pub struct PathHop { pub via: String, pub hop: String }
pub struct CausalCause { pub id: String, pub sha: Option<String>, pub score: f64,
                         pub path: Vec<PathHop>, pub shared_anchors: Vec<String>,
                         #[serde(skip)] pub evidence_to: Vec<String> }
pub struct NeedsReport { pub missing: Vec<String>, pub required_start_points: Vec<String> }
pub struct WalkRejection { pub id: String, pub reason: String, pub shared_anchors: usize }
pub struct CausalWalkResult { pub causes: Vec<CausalCause>,
                              pub needs_report: Option<NeedsReport>,
                              pub rejected: Vec<WalkRejection> }

pub fn walk_storage(storage: &Storage, req: &CausalWalkRequest) -> Result<CausalWalkResult, String>;
pub fn persist_evidence_edges(storage: &Storage, result: &CausalWalkResult)
    -> Result<Vec<(String,String)>, String>;  // cause -> evidence, link_type "evidence_of"
```

`CausalWalkOptions { lookback_days, max_causes, max_rejections }::walk(starts, records, ctx)`
is the pure engine (records = `WalkRecord`, ctx = runs/edges/rev-lists) if you
need it without a store.

## Output schema (MCP `causal_walk`)

```
{ causes: [{id, sha?, score, path: [{via, hop}], shared_anchors, content_preview}],
  needs_report?: {missing, required_start_points},
  rejected: [{id, reason, shared_anchors, content_preview}],   // top 3
  promote: {edges_persisted, edges: [[cause, evidence]], link_type: "evidence_of"},
  preview, scope, headline, evidence_status: "hypothesis", causality_verified: false }
```

No start point / unanchored handle → `needs_report` (never an error, never
guessed). Promote is the ONLY write path.

## Ranking (ported from run_trail)

score = Σ probabilistic-IDF(anchor) × tier weight (Path 1.0 / Code 0.9 /
Version 0.6 / Word 0.3, df over the in-window candidate pool) + 0.3 recency
term + 0.25 change-record bonus. Sort: score desc → change-record first →
older first. Window: candidate created_at ≤ failure anchor (= newest
start-point-anchored record, else now) and within lookback_days (default 30).

## CLI

`vestige causal-walk --failing-test NAME | --stack-frame F[:L] | --ci-run ID |
--logged-write ID | (--git-repo P --worked-in T --broke-in T)`
`[--lookback-days N] [--no-promote] [--scope S] [--json]`

## Tests

- `cargo test -p vestige-core --lib causal_walk` — 10 tests: co-touch walk,
  stack_frame last-toucher, version_range restriction (real temp git repo),
  no-start refusal, wrong-handle refusal (x2), ci_run trace walk,
  logged_write edge walk, promote vs preview, unresolvable range refusal,
  record-format parsing.
- `cargo test -p vestige-mcp --lib causal` — schema variant test + tokio
  end-to-end (temp storage): needs_report, preview writes nothing,
  promote persists `evidence_of` edges.
- Server surface: advertised count stays 16 (`causal_walk` in, `backfill`
  hidden); full-schema registry test, wire-budget test, and hints test
  updated accordingly.

## Notes for adjacent agents

- Do NOT re-add `backfill` to the advertised catalog or `compact::full_schema`
  registry — `full_schema_registry_matches_the_advertised_catalog` fails if
  catalog and registry diverge.
- Richer edge APIs: keep persisting through `save_connection` with
  `link_type = "evidence_of"` (constant `causal_walk::EVIDENCE_LINK_TYPE`) so
  `memory_connections` stays compatible; `CausalCause::evidence_to` carries
  the per-cause link targets (serde-skipped).
- Supersession is intentionally NOT followed in causal_walk (MVP bound);
  commit records upsert idempotently via `(git, repo#sha)`.
