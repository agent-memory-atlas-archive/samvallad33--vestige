# SCOPE-HANDOFF — w1b search collapse (storage retrieval internals)

Branch: `build/w1b-search-collapse` (base: main @ 5fbe1df)
Scope touched: `crates/vestige-core/src/storage/sqlite/{search.rs,lifecycle.rs,mod.rs}` only.

Owner order implemented: vector retrieval is removed; keyword/exact retrieval is
the only retrieval until the exact resolver lands. `hybrid_search` /
`hybrid_search_filtered` keep their public signatures; `keyword_weight` and
`semantic_weight` are accepted and ignored (documented on both fns). FTS5 index
creation and all `knowledge_fts` usage are untouched (later wave).

## Compile / test status (2026-09-28)

- `cargo check -p vestige-core --lib` (default features): FAILS — 2 errors, both
  in `embeddings.rs` (cross-scope, see below; that module is being deleted by
  the parallel agent).
- `cargo test -p vestige-core --lib --no-default-features --features bundled-sqlite storage`:
  PASS — 316 passed, 0 failed (vector-era tests are cfg-excluded in this config).
- New regression tests (in `search.rs`, module `w1b_search_collapse_tests`):
  `hybrid_search_filtered_semantic_weight_is_inert` and
  `hybrid_search_filtered_semantic_weight_inert_with_type_filters` — both PASS.
  They assert `semantic_weight > 0` output is byte-identical to
  `semantic_weight = 0` (ids, scores, match_type) and that every hit is
  `MatchType::Keyword` with `semantic_score: None`.
- `cargo check -p vestige-core --lib --tests` (default features): FAILS — 8
  errors total, all cross-scope (enumerated below).

## Cross-scope breaks (file:line + need)

Hard compile errors with default features:

1. `crates/vestige-core/src/storage/sqlite/embeddings.rs:23` — references
   deleted test thread-local `VECTOR_SEARCH_DISABLED_FOR_TEST`. Need: delete
   `vector_search_disable_requested`'s test/env branch (or the whole file, per
   the embeddings-module deletion agent).
2. `crates/vestige-core/src/storage/sqlite/embeddings.rs:26` — references
   deleted const `VESTIGE_DISABLE_VECTOR_SEARCH`. Same need.
3. `crates/vestige-core/src/storage/sqlite/embeddings.rs:27` — references
   deleted fn `env_value_disables_vector_search`. Same need.
4. `crates/vestige-core/src/storage/sqlite/tests.rs:1148,1150` —
   `with_vector_search_disabled` helper flips the deleted thread-local. Need:
   delete the helper and the three runtime-gate tests that use it
   (`tests.rs:1181`, `tests.rs:1222`, `tests.rs:1246`); the kill-switch no
   longer exists, so `test_runtime_vector_gate_*` are obsolete. The
   keyword-fallback behavior they guarded is now covered unconditionally by
   `w1b_search_collapse_tests` in `search.rs`.
5. `crates/vestige-core/src/storage/sqlite/tests.rs:1164,1170` —
   `vector_search_env_value_parsing` tests the deleted env parser. Need: delete
   the test.
6. `crates/vestige-core/src/embedding/lifecycle.rs:1511` — calls the deleted
   `Storage::semantic_search("first memory", 5, -1.0)`. Need: embedding-module
   owner removes/repoints this check (keyword search or store-level probe).

For the embeddings-module deletion agent (compile-safe today, breaks once
`embeddings.rs` is removed — not caused by this branch, listed for planning):

7. `crates/vestige-core/src/storage/sqlite/ingest.rs:519,535` — predictive
   ingest calls `vector_search_available()` / `semantic_search_raw()`. Need:
   remove the semantic dedupe leg or replace with keyword/co-access lookup.
8. `crates/vestige-core/src/storage/sqlite/admin.rs:713-763` — `new()` still
   constructs `VectorIndex` via `vector_search_enabled_by_cpu()` and loads
   embeddings into it. Need: drop index construction plus the now-orphaned
   struct fields in `mod.rs` (`vector_index`, `query_cache`,
   `attached_profile_runtime`, `vector_index_watermark`).
9. `crates/vestige-core/src/storage/sqlite/sync.rs:303` — calls
   `load_embeddings_into_index()`. Need: delete the call.
10. `crates/vestige-core/src/storage/memory_store.rs:226` — the `MemoryStore`
    trait still requires `async fn vector_search`; my `mod.rs` impl is an inert
    shell returning `Ok(vec![])` (kept because the signature is required).
    Need in a later wave: remove the trait method, then the shell.

## Removed in this scope (no external callers existed)

- `Storage::semantic_search` (search.rs) — vector retrieval entry point.
- Semantic leg of `hybrid_search_filtered`: `semantic_search_raw` call,
  RRF-with-vector fusion, Park et al. rerank over fused scores.
- `keyword_search_with_scores` (search.rs) — orphaned by the above; was
  cfg-gated behind `embeddings`+`vector-search` despite being pure FTS.
- `VESTIGE_DISABLE_VECTOR_SEARCH` const, `env_value_disables_vector_search`,
  `VECTOR_SEARCH_DISABLED_FOR_TEST` thread-local (mod.rs).
- Narrative-edge vector neighbor boost in `strengthen_on_access`
  (lifecycle.rs): top-6 HNSW neighbors at similarity >= 0.7 boosted by
  `0.02 * similarity`. Deleted per owner preference; exact co-access
  reinforcement already exists in `strengthen_narrative_edges`
  (memory_connections / access log), documented in the code comment.
- `recall_in_scope`: `SearchMode::Semantic` and `SearchMode::Hybrid` now take
  the FTS5 keyword path (documented in code).
