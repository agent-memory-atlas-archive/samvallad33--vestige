# SCOPE-HANDOFF — build/w1d-absorb (vestige-mcp cleanup after core vector removal)

Branch: `build/w1d-absorb` (off main tip ba94298). Scope: `crates/vestige-mcp`
only; vestige-core untouched. Committed locally; nothing pushed.
(Replaces the w2b-edge-schema handoff that was already merged at 45cd598.)

## Status

- `cargo check -p vestige-mcp` — 0 errors, 0 warnings for vestige-mcp
  (1 remaining warning is vestige-core's dead `EmbeddingProfileMigrationRow`
  alias, out of scope).
- `cargo test -p vestige-mcp --lib` — 760 passed, 0 failed.
- `cargo clippy -p vestige-mcp --all-targets` — clean for vestige-mcp
  (remaining warnings are all in vestige-core).

## Error classes fixed (37 lib errors + test-target fallout)

1. `cognitive.rs` — removed `AdaptiveEmbedder` and `Reranker` import/field/init
   (cfg'd reranker arm included); `COGNITIVE_MODULE_COUNT` now 16+10 (+1
   vector-search-gated TemporalSearcher, which still exists in core); test
   updated.
2. `dashboard/handlers.rs` — deleted the entire embedding-profile section
   (6 handlers, request type, snapshot/receipt helpers, ~420 lines) and their
   6 routes in `dashboard/mod.rs`; dream builder now `embedding: None`.
3. `tools/smart_ingest.rs` — removed `ContentType` import + both
   `ContentType::detect` calls + the `warming` response block.
4. `tools/dream.rs` — `embedding: None` (content-word similarity path stays).
5. `resources/memory.rs` — `embeddingServiceReady` stubbed `false`.
6. `src/main.rs` — removed the cfg'd embeddings init block
   (embedding_model_cached / init_embeddings / generate_embeddings) and the
   cross-encoder reranker load block (Reranker::load_cross_encoder); stale
   comments updated.
7. `bin/cli.rs` — deleted the `embeddings` subcommand tree (EmbeddingCommands,
   dispatch, run_embeddings_* helpers, artifact-root/confirmation helpers), the
   `--embedding-from` Recall flag and its profile-attach logic, the
   `LEGACY_NOMIC_PROFILE_ID` preflight SQL, and all `is_embedding_ready` uses;
   backfill contrast mode and `vestige ingest` are keyword-only (cfg'd semantic
   branches removed); run_consolidate note simplified.
8. Deleted `tools/context.rs` (match_context) and `tools/warming.rs`
   (+ mod entries, server.rs dispatch arm, warming response blocks in
   search_unified/smart_ingest, their tests).
9. `server.rs` — removed the `explore_connections` hidden-alias dispatch arm
   (graph tool + graph_unified's internal use of explore stay); removed its
   three trace_recorder entries (is_retrieval_tool + two evidence maps).
10. Remaining `hybrid_search(_filtered)` call sites compile against core's
    keyword-only implementations — no mcp-side change needed.
- `tools/search_unified.rs` — fixed E0382 duplicate `rerank_candidates` move
  (leftover double `reranked_results` binding).
- `tools/cross_reference.rs` — removed the cfg'd reranker block in
  `retrieve_and_rank_candidates`.
- `tools/maintenance.rs` — `embeddingReady: false`; consolidate phase
  "embeddings" arm removed; maintain schema enum/error text updated.
- `Cargo.toml` — declared `embeddings = []` / `vector-search = []` as always-off
  stubs (mirrors core's stub) so legacy cfg sites don't trip `unexpected_cfgs`
  (78 → 0 warnings).

## Test fallout fixed

- Deleted 5 embedding-profile handler tests + test-mod imports in handlers.rs.
- `maintain.rs`: embeddings-phase test rewritten as
  `bounded_phases_preview_by_default_and_rejects_misapplied_controls`
  (lifecycle phase previews by default; "embeddings" phase now rejected).
- `server.rs::test_recall_lookup_matches_search_shape`: now compares
  `recall(default)` vs `recall(mode=lookup)` — the `search` alias was already
  a removal stub at base ba94298, so the old byte-comparison could never pass.
- `tools/backfill.rs` live test: the "shares NO entity" distractor fixture
  actually shared 4 word-tier entities under core's tiered extractor (w1c,
  commit 7bc2516) and outranked the env-var cause (4 word-idf > 1 code-idf).
  Reworded the distractor ("An outage hit the billing system last month") to
  match its documented zero-shared-entity intent; all other assertions
  unchanged.

## Notes / not finished

- The sibling branch's selftest + forgotten_lesson tools (16→18) are NOT in
  this base (ba94298 advertises 16 tools); catalog tests pin 16 here and pass.
  Nothing to keep yet — they arrive with that branch's merge.
- vestige-core still emits 3 clippy warnings (dead alias, doc-comment,
  while-let) — out of scope, untouched.
- Behavioral note: with keyword-only retrieval, four rare word-tier shared
  entities can outscore one code-tier entity in backfill ranking (see the
  backfill.rs test comment). That weighting lives in core.
