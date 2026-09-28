# W1E Test Cleanup — Scope Handoff

Branch `build/w1e-tests-cleanup` (base: main @ 5fbe1df). Test hygiene ONLY, per
owner order. Source modules still present in this tree; parallel agents
(w1a-core-kill, w1b-search-collapse, w1c-ingest-flip, w1d-mcp-surface) own the
source deletions.

## Deleted (clearly vector/embedding-only)

- `tests/e2e/src/mocks/mock_embedding.rs` (whole file; no test used
  `MockEmbeddingService` directly — only re-exports in `tests/e2e/src/lib.rs`
  and `tests/e2e/src/mocks/mod.rs`, both cleaned).
- `tests/e2e/tests/extreme/proof_of_superiority.rs`:
  `test_proof_hippocampal_indexing_efficiency` (embedding-compression bench).
- `tests/e2e/tests/cognitive/comparative_benchmarks.rs`:
  `test_two_phase_vs_flat_search`, `test_index_compression_ratio`.
- `crates/vestige-core/src/storage/v3_regression_tests.rs`:
  `v3_delayed_embedding_cannot_resurrect_a_purged_memory` (gated).
- `crates/vestige-core/src/storage/sqlite/tests.rs` (27 tests + 5 helpers):
  runtime vector-gate tests (`vector_search_env_value_parsing`,
  `disabling_vector_search_...`, `test_runtime_vector_gate_env_...` x2),
  embedding identity/regeneration/dirty-embedding tests, embedding profile
  lifecycle tests (`init_embeddings_permits/rejects_...`,
  `reopening_after_qwen_pointer_...`, `embedding_profiles_keep_vectors_isolated`,
  `activation_rejects_ready_...`, `migration_vector_and_node_checkpoint_...`,
  `purge_removes_vectors_from_every_embedding_profile` + `ready_profile_manifest`
  helper), `non_256_active_profile_...`, all peer/vector-journal tests
  (`peer_process_write_...`, `peer_reembedding_...`, `peer_purge_...`,
  `peer_content_edit_...`, `own_writes_...`, `a_journal_pruned_...`,
  `vector_journal_prune_...`), `embedding_maintenance_preview_...`,
  `embedding_write_rejects_stale_...`,
  `purge_and_profile_activation_do_not_deadlock`, and orphaned helpers
  (`with_vector_search_disabled`, `persist_test_vector`, `index_contains`,
  `nearest`, `ingest_plain`).
- `crates/vestige-mcp/tests/e2e_real_binary.rs`: embedding warm-up test,
  `the_real_embedding_runtime_...`, `tag_prefix_filtering_..._hybrid_path`
  (keyword-path sibling kept), `approved_purge_removes_the_stored_embedding`,
  `embeddings_and_semantic_retrieval_survive_a_restart`,
  `corrupt_fts_rebuild_preserves_embeddings`; removed the
  `node_embeddings`-count assertion from the keyword purge-tombstone test;
  trimmed `embeddingsCompiledIn`/`embeddingReady` from the memory_status
  health shape assertion.
- `crates/vestige-mcp/tests/e2e_failure_cases.rs`:
  `conflicting_ingest_during_a_labile_window_...` (vector-gated supersede path
  by its own doc).
- `crates/vestige-mcp/tests/common/mod.rs`: removed `wait_for_embeddings`,
  `ingest_embedded`, `ingest_inner`'s embedding assert, `wait_for_log_notification`,
  `EMBEDDING_TIMEOUT`, `EMBEDDINGS_READY`.

## Adapted (kept green WITHOUT embeddings at runtime)

- `proof_of_superiority.rs` `test_proof_multi_hop_beats_similarity`: local
  mock-vector similarity half removed; now asserts structural results only
  (4-hop membership + path). Comprehensive summary test renumbered to 5
  capabilities (hippocampal compression block removed).
- `e2e_real_binary.rs` `contradictions_...` and `correction_...`: converted
  from #[ignore]d real-embedding-runtime tests to default-suite keyword-path
  tests (their subjects are lexical by construction). Verified green.

## Borderline — left gated / left as-is for the integration pass

All still behind `#[cfg(all(feature = "embeddings", feature = "vector-search"))]`
in `crates/vestige-core/src/storage/sqlite/tests.rs`; their subject code is
itself feature-gated, so decide at integration (dedup/merge tool SURVIVES in
the 15-tool surface — if merge code is un-gated rather than deleted, these need
keyword-path fixtures, not deletion):

- Merge/supersede suite: `test_merge_candidates_threshold_classification`,
  `test_plan_merge_is_preview_only_no_mutation`,
  `concurrent_apply_of_one_plan_applies_it_exactly_once`,
  `test_merge_state_rejects_...`, `test_merge_undo_conflict_...`,
  `test_merge_undo_concurrent_...`, `test_apply_then_undo_merge_...`,
  `test_supersede_invalidates_...`, `test_protect_blocks_merge_away`,
  all `test_auto_dedup_*` (6) + `set_retention`/`with_auto_merge_env` helpers,
  `pinning_auto_merge_in_one_test_...`,
  `test_apply_requires_confirm_...`, `apply_match_obeys_...`,
  `apply_match_can_use_...`, `test_merge_policy_roundtrip_persists`.
- Validity-window suite (fixture needs active embedding profile via
  `storage_with_marker_gate_runtime` + `MarkerEmbedder`):
  `inferred_as_of_validity_...`, `inferred_as_of_must_not_resurrect_...`,
  `explicit_valid_from_on_reinforce_...`, `create_path_still_stamps_...`,
  `older_dated_claim_after_newer_fact_...`.
- Reconsolidation suite (same fixture): `contradiction_during_live_window_...`,
  `approve_verdict_applies_...`, `reject_verdict_leaves_...`,
  `quarantine_verdict_suppresses_...`, `expired_window_auto_closes_...`,
  `pending_reconsolidation_plans_...` + `reconsolidation_candidate` helper.
- Vector fixture helpers still used by the above: `seed_node`, `axis_vector`.
- `comparative_benchmarks.rs` `test_barcode_orthogonality` +
  `test_content_pointer_accuracy`: structural (hash barcodes, content
  pointers) but reference `neuroscience::hippocampal_index`. If w1a deletes
  that module, these go with it. NOTE: the vestige-e2e-tests crate defines no
  features, so a `#[cfg(feature = "embeddings")]` there would be silently
  always-off; they were left running instead.
- `e2e_real_binary.rs` `foreign_key_orphans_are_repaired_...`: uses
  `node_embeddings` as a generic CASCADE-child fixture. If the table is
  dropped, swap the fixture to another CASCADE child (e.g. fsrs_cards).
- `tests/e2e/tests/journeys/ingest_recall_review.rs`
  `test_recall_search_modes`: constructs `SearchMode::Semantic` (w1b scope).
- `tests/e2e/Cargo.toml` still requests `features = ["embeddings",
  "vector-search"]` from vestige-core — drop when the features die.

## Notes for integration

- No e2e scenario exercised `match_context`/`search` tool aliases at this
  base (grep-verified); nothing to flip to removal-error assertions. The
  aliases exist only in source (`crates/vestige-mcp/src/server.rs`). HOWEVER,
  `cargo test -p vestige-mcp --lib` at this base already fails
  `server::tests::test_recall_lookup_matches_search_shape`
  ("recall(mode=lookup) must equal search byte-for-byte") — that IS a
  search-alias-expects-success test living in src tests; w1d should flip it
  to expect the removal error. Second pre-existing failure:
  `tools::backfill::tests::live_backfill_surfaces_root_cause_through_storage`.
  Both verified pre-existing via `git stash` roundtrip at base 5fbe1df
  (lib build contains none of this branch's changes).
- `crates/vestige-core/src/storage/sqlite/embeddings.rs`:
  `embedding_model_matches_active` / `embedding_vector_for_active_model` are
  now unused (their only callers were deleted tests) — dead-code warning under
  default features; resolves itself when w1a deletes the embeddings module.
- `cargo check --tests --no-default-features` (workspace): FAILS in NON-TEST
  source, pre-existing at base: `crates/vestige-mcp/src/tools/search_unified.rs:1261`
  E0382 "use of moved value: `rerank_candidates`" (into_iter at 1257, reused at
  1261; the no-default-features branch of the reranker). Not fixed here per
  owner order — belongs to w1b/w1d. vestige-core's test build compiles clean
  under no-default-features apart from the dead-code warning above.

## Verification (this tree, default features)

- `cargo test -p vestige-core --lib`: 936 passed, 0 failed, 1 ignored.
- `cargo test -p vestige-e2e-tests --test comparative_benchmarks --test
  proof_of_superiority`: 23 + 4 passed, 0 failed.
- `cargo test -p vestige-mcp --lib`: 756 passed, 2 failed — both failures
  pre-existing at base (see Notes), unaffected by this branch.
- `cargo test -p vestige-mcp --test e2e_real_binary -- contradictions
  correction`: 2 passed (the two keyword-path conversions).
- `cargo check -p vestige-e2e-tests --tests`: clean.
