# SCOPE HANDOFF — build/w1c-ingest-flip (ingest-flip: no similarity, exact-equality nomination)

Branch: `build/w1c-ingest-flip`, base = `main @ 5fbe1df`. Not pushed.

Owner decision implemented: NO similarity anywhere; dedup/merge nomination =
exact equality only (identical content hash / same declared source key / exact
entity-set equality); edges only from declared sources.

## What changed in this branch's scope

- `crates/vestige-core/src/advanced/prediction_error.rs`
  - `evaluate` / `evaluate_with_intent` dropped the `new_embedding` param.
  - `CandidateMemory.embedding` field deleted.
  - `cosine_similarity` deleted; replaced by `content_similarity` (Dice
    coefficient over lowercased alphanumeric tokens > 2 chars). Exported via
    `advanced::content_similarity`.
  - `NEAR_IDENTICAL_THRESHOLD` recalibrated 0.92 → 0.85 for the Dice scale so
    benign near-paraphrases still Reinforce; contradiction-marker check runs
    before the threshold, so corrections never reinforce.
- `crates/vestige-core/src/advanced/merge_supersede.rs`
  - `W_EMBEDDING` removed. New weights: `W_TAGS = 0.40`, `W_TOKENS = 0.60`
    (sum = 1.0). Comment states nomination is exact-equality-first and scores
    are tie-breakers/review labels only.
  - `MatchSignals.embedding_similarity` field REMOVED (breaking API change).
  - `score_pair` signature is now `score_pair(a_tags, b_tags, a_content, b_content)`.
- `crates/vestige-core/src/advanced/dreams.rs`
  - Both cosine branches deleted; `tag_similarity*0.4 + content_word_similarity*0.6`
    is now the whole computation. Module-level `cosine_similarity` fn deleted.
  - `DreamMemory.embedding` field KEPT but inert (out-of-scope constructors
    still set it — see below).
- `crates/vestige-core/src/storage/sqlite/merge.rs`
  - `merge_candidates`: O(n²) cosine scan deleted (was embeddings+vector-search
    cfg-gated). Now UNGATED and nominates via: (1) SQL group-by on
    `COALESCE(content_hash, content)`; (2) SQL group-by on the source key at
    the same granularity as the store's UNIQUE index
    `(source_system, COALESCE(source_project,''), source_id)`; (3) exact
    equality of non-empty `retroactive_backfill::extract_entities` sets
    (in-memory compare). Classification is advisory — nominated clusters are
    surfaced even when labelled NonMatch (shared source key with diverged
    content is review-worthy; old drop-on-NonMatch was a cosine-era guard).
  - `pair_similarity` deleted (embedding-based); plan_merge / plan_supersede /
    plan_reconsolidation now call the 4-arg `score_pair` directly.
  - New test module `exact_nomination_tests` (6 tests).
- `crates/vestige-core/src/storage/sqlite/ingest.rs` (embedding paths only)
  - Smart ingest: embedding-runtime preflight, `get_document_embedding`, and
    `semantic_search_raw` candidate retrieval REMOVED. Candidate selection is
    keyword-only via `Storage::search` (FTS5/BM25, public API in search.rs —
    not modified). `get_node_embedding` per-candidate fetch removed.
  - KEPT (cfg-gated, deletion belongs to the embeddings agent):
    `generate_embedding_for_node` after raw insert, and vector invalidation +
    regeneration in `update_node_content_unchecked`. These are vector STORAGE
    hygiene, not similarity decisions; removing them broke
    `embedding::lifecycle` and `peer_content_edit_invalidates_vectors` tests
    that the embeddings-deletion agent will remove wholesale.
  - NOTE: the whole `smart_ingest*` chain is still
    `#[cfg(all(feature = "embeddings", feature = "vector-search"))]`-gated. It
    no longer needs embeddings at all — the embeddings agent should un-gate it
    when the feature flags go.
- `crates/vestige-mcp/src/tools/dedup.rs`
  - `find_duplicates` (`execute`): cosine clustering + UnionFind deleted;
    now UNGATED exact-equality grouping (content-hash identity + source key at
    UNIQUE-index granularity). `similarity_threshold` parameter removed from
    the schema (unknown args are ignored by serde, so old callers keep
    working). Output: `similarityToAnchor` → `matchRelation`
    (anchor|content|source_key|transitive); `totalWithEmbeddings` →
    `totalScanned`; `threshold`/`pairsChecked` removed.
  - plan/apply/undo/verdict/tag_*/protect/policy flows untouched.

## Forced out-of-scope compile fixes (minimal, mechanical)

- `crates/vestige-core/src/advanced/mod.rs`: re-export
  `prediction_error::cosine_similarity` → `content_similarity`.
- `crates/vestige-mcp/src/tools/merge.rs` (2 sites): dropped the
  `embeddingSimilarity` output lines referencing the removed
  `MatchSignals.embedding_similarity` field. Everything else in that file is
  untouched; NOTE its `merge_candidates_schema()` may still advertise a
  `similarity_threshold`-style knob cosmetically (not compile-relevant).

## For the embeddings-deletion agent (cross-scope notes)

- After deleting `storage/sqlite/embeddings.rs` and the `embeddings/` module:
  - `vestige_core::cosine_similarity` re-export at `lib.rs:572` must go.
  - `get_document_embedding` is already dead code (warning) — disappears with
    the module.
  - Remaining callers of `storage.get_node_embedding(...)` OUTSIDE my scope:
    `crates/vestige-mcp/src/tools/dream.rs` (~line 88) and
    `crates/vestige-mcp/src/dashboard/handlers.rs` (~line 1863) — they only
    populate the now-inert `DreamMemory.embedding`; delete those calls and the
    field.
  - `lifecycle.rs` / `search.rs` untouched per scope (search.rs `hybrid_search`
    etc. still reference `semantic_search_raw`).
  - Un-gate the `smart_ingest*` chain + merge plan/apply/undo cfg gates when
    the features are dropped (see above).
  - `knowledge_nodes.has_embedding` / `node_embeddings` /
    `embedding_profile_vectors` schema: my files no longer maintain them
    except the cfg-gated blocks noted above.

## Test status (default features)

- `cargo test -p vestige-core`: 971 passed, 0 failed (1 ignored, pre-existing).
- `cargo test -p vestige-mcp`: 759 passed, 2 failed:
  - `server::tests::test_recall_lookup_matches_search_shape`
  - `tools::backfill::tests::live_backfill_surfaces_root_cause_through_storage`
  Both are verified PRE-EXISTING on base `5fbe1df` (checked in a clean
  worktree); they are fallout from commit 7bc2516's search-tool removal and
  backfill rework, not from this branch.
- All in-scope test modules green: prediction_error (16), merge_supersede,
  dreams, sqlite::merge::exact_nomination_tests (6), mcp tools::dedup (12).
