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
