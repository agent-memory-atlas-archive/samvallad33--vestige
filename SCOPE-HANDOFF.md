<<<<<<< HEAD
# SCOPE-HANDOFF — core vector/embedding kill (branch `build/w1a-core-kill`)

Scope executed: deleted `src/embeddings/`, `src/embedder/`, `src/search/{vector,reranker,hyde}.rs`,
`src/advanced/adaptive_embedding.rs`, `benches/search_bench.rs`, `build.rs` + `compat/`
(isoc23 ORT shim); stripped re-exports/mod decls in `lib.rs`, `search/mod.rs`, `advanced/mod.rs`;
removed deps `fastembed`, `candle-core`, `candle-nn`, `tokenizers`, `usearch`, `lru`, `cc`
(build), `criterion` (dev) and the `[[bench]]` section; gutted features (see stubs below).
`search/` module is now UNGATED and contains only hybrid RRF fusion, temporal search, and the
`fts::sanitize_fts5_query` re-export.

`cargo check -p vestige-core` (default features) fails with EXACTLY 2 errors, both below in A.
Everything else compiles. All storage vector paths are cfg-gated OFF by default, so they compile
out until their owner deletes them.

## A. crates/vestige-core/src/embedding/ (NOT in my delete list — blocks the crate)

The singular `embedding` profile-contract module logically uses the deleted `embedder` module.
This is the only thing keeping `cargo check -p vestige-core` red.

- `embedding/profile.rs:14` — `use crate::embedder::{Embedder, EmbedderError, EmbedderResult};`
  (`ProfiledEmbedder` implements `Embedder`; returns `crate::embedder::BoxedEmbedderFuture` at
  :1131 and :1141).
- `embedding/lifecycle.rs:23` — `embedder::{Embedder, EmbedderError}` import; :676
  `crate::embedder::GraniteOnnxEmbedder::from_verified_local_artifacts`, :699
  `crate::embedder::Qwen3LocalEmbedder::from_verified_local_artifacts` (both under
  `cfg(qwen3-embeddings)`), :825-826 and :1066-1067 `EmbedderError` construction, :889-964
  `use crate::search::{VectorIndex, VectorIndexConfig}` + HNSW index build/load/verify (under
  `cfg(vector-search)`), :1336 `impl crate::embedder::EmbedderSend` (test cfg).
- Needed: this module is embedding lifecycle machinery — delete it (or gut the embedder/index
  halves). NOTE storage ungated-imports types from it, so kill both together (see B).

## B. crates/vestige-core/src/storage/ (out of my scope; currently cfg'd out by default features)

All of these are inside `#[cfg(all(feature = "embeddings", feature = "vector-search"))]` (or
`#[cfg(feature = "vector-search")]`) blocks, which now evaluate false by default. They must be
deleted when storage strips its gates:

- `storage/sqlite/mod.rs:45-50` gated `use crate::embeddings::{EMBEDDING_DIMENSIONS, Embedding,
  EmbeddingService}`; `:52-53` gated `use crate::search::{VectorIndex, VectorIndexConfig,
  reciprocal_rank_fusion}` (RRF still exists — only VectorIndex/VectorIndexConfig are gone);
  `:54-55` gated `use crate::search::hyde`. `:26-31` has an UNGATED import from `crate::embedding`
  (module still present — coordinate with A).
- `storage/sqlite/mod.rs:8` gated `use lru::LruCache` — the `lru` dependency was REMOVED from
  Cargo.toml; this gated block must go when gates are stripped.
- `storage/sqlite/embeddings.rs` — whole file; `VectorIndex::with_config` (:152), `hyde::
  classify_intent/expand_query/centroid_embedding` (:2215-2227).
- `storage/sqlite/admin.rs:711-761` `EmbeddingService::new()` + `VectorIndex::new()` (rebuild),
  `:825-1153` embedding dashboard paths.
- `storage/sqlite/{ingest,merge,search,lifecycle,purge,connectors,sync}.rs`,
  `storage/sqlite/tests.rs:7876-8098` (`MarkerEmbedder: crate::embedder::EmbedderSend`),
  `storage/v3_regression_tests.rs:229` — dozens of gated blocks referencing VectorIndex /
  EmbeddingService / hyde.

## C. crates/vestige-mcp (out of my scope)

- `Cargo.toml:15-17,35-47` — default features include `embeddings, ort-download, vector-search`;
  forwards `vestige-core/{embeddings, vector-search, ort-download, ort-dynamic,
  qwen3-embeddings, metal, cuda, cudnn}` (now EMPTY STUBS, see E). Enabling them turns the cfg
  gates in B back ON against deleted symbols — mcp will not build until it drops these features
  and its `#[cfg(feature = "embeddings")]` code.
- `src/main.rs:493-497` `vestige_core::embeddings::embedding_model_cached()` (cfg embeddings);
  `~:826` `vestige_core::search::Reranker::load_cross_encoder` (cfg embeddings).
- `src/cognitive.rs:16,90,186` `AdaptiveEmbedder`; `:43,96,192` `Reranker`/`RerankerConfig`;
  `:10` `TemporalSearcher` (still exists — no change needed).
- `src/tools/search_unified.rs` — "Reranker" mentions are comments/local BM25-like rescoring;
  no deleted imports.

## D. Other manifests / CI

- `tests/e2e/Cargo.toml:8` — requests `features = ["embeddings", "vector-search"]` (now no-op
  stubs; resolves but enables nothing). e2e code should drop them.
- `.github/workflows/release.yml:38,48` — cargo flags pass `embeddings,ort-download,
  vector-search` / `ort-dynamic,vector-search`; release.yml + ci.yml no-embeddings jobs need a
  pass once B/C land.
- Workspace root `Cargo.toml` — no embedding deps existed at workspace level; nothing removed.
  `exclude = ["fastembed-rs"]` still points at the vendored fork directory on disk — dead
  weight, candidate for deletion by the repo owner.

## E. Feature stubs left behind (deliberate, unavoidable)

`vestige-core/Cargo.toml` keeps `embeddings`, `vector-search`, `ort-download`, `ort-dynamic`,
`qwen3-embeddings`, `metal`, `cuda`, `cudnn` as EMPTY features only because cargo fails
workspace manifest resolution while C/D still forward them. They enable nothing. Delete the
stubs in the same change that removes the forwarding references.
=======
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
>>>>>>> build/w1b-search-collapse
