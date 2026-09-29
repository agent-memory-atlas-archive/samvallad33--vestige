# SCOPE-HANDOFF — T1: Storage Trait Wall (branch `build/t1-trait-wall`)

(Replaces the w1d-absorb handoff that was already merged at 18b13c7.)

Base: `main @ 82f0cf5`. Status: **complete** — workspace green, suites green,
wall proven by mock test. This is the seam STRATA (T2) implements.

## What changed

- `crates/vestige-core/src/storage/memory_store.rs`
  - `LocalMemoryStore` now carries the full product seam: **26 async methods**
    (phase-1 cognitive surface, unchanged semantics) **+ 183 sync methods**
    (the product surface every tool/server/CLI uses). The sync methods have
    loud default bodies (`Err(StorageError::Init("<name> is not implemented
    by this backend"))`, or `unimplemented!` for non-Result returns), so a
    partial backend compiles and fails loudly rather than silently
    succeeding. NOTE: default bodies on the **async** methods are NOT
    possible with `trait-variant 0.1.3` (its desugaring rejects them);
    implement the async seam explicitly (see the mock for a copy-paste
    Err-stub block).
  - The dyn-compatible `MemoryStore` trait mirrors all 209 methods
    (async boxed-future + sync passthrough); the blanket
    `impl<T: MemoryStoreSend> MemoryStore for T` forwards everything.
  - Two phase-1 async methods were renamed to free their names for the
    product-shaped sync methods: `search` -> `search_records`,
    `get_stats` -> `get_store_stats`.
- `crates/vestige-core/src/storage/sqlite/mod.rs`
  - `impl MemoryStoreSend for SqliteMemoryStore` now forwards all 183 sync
    methods to the inherent SQLite implementations (explicit
    `SqliteMemoryStore::method(self, ..)` form). SQLite is an implementation
    behind the wall, nothing more.
- `crates/vestige-core/src/storage/mod.rs`
  - `pub type Storage = dyn MemoryStore;` (was `= SqliteMemoryStore`). Every
    `Arc<Storage>` / `&Storage` site is now engine-agnostic.
  - `open_storage(path: Option<PathBuf>) -> Result<Arc<dyn MemoryStore>>`
    is the only constructor the MCP layer may call. Path helpers
    `default_db_path()` / `db_path_for_data_dir()` are free functions now.
  - New re-exports: `PurgeReport`.
- `crates/vestige-mcp` — zero `SqliteMemoryStore` references (grep-verified,
  including comments). All constructors go through `vestige_core::open_storage`.
  Core product fns that took `&SqliteMemoryStore` (`run_sync`,
  `run_dream_compile`, `walk_storage`, `persist_evidence_edges`) now sit on
  the trait surface.
- `crates/vestige-mcp/tests/backfill_trait_wall.rs` — **the wall proof**: a
  pure in-memory mock (overrides only the 7 methods backfill touches; async
  seam Err-stubbed) serves `tools::backfill::execute` end-to-end. Passing
  means no SQLite reachability from the tools layer.

## Rust coercion notes for callers (bit us repeatedly)

- `&Arc<dyn Storage>` does NOT coerce to `&dyn Storage` — pass `&*storage`.
- `&Arc<Concrete>` does NOT coerce to `&Arc<dyn Storage>` — coerce the Arc
  first, or pass `&*storage` into `&Storage` params.
- `&Concrete` DOES coerce to `&dyn Storage` (single unsize step).

## The trait surface T2 must implement

Implement `MemoryStoreSend` (you get `LocalMemoryStore` + `MemoryStore` for
free via the blanket impl). Error type: `StorageError` (`storage::Result`).
Signatures mirror `SqliteMemoryStore`'s inherent methods exactly — check
`crates/vestige-core/src/storage/memory_store.rs` (source of truth) for
exact args/returns.

### Sync product seam (183 methods, defaults provided)

actor_policy_snapshot, append_mcp_call_outcome, append_trace_event, apply_failure_feedback, apply_intention_graph, apply_tag_mutation, backup_to, blast_radius, blast_radius_with_link_types, capture_synaptic_event, checkpoint_wal, clear_dream_page_tags, closed_issue_nodes, code_anchors_for_node, code_anchors_for_nodes, commit_intention_check, concrete_search_filtered, count_memories_below_retention, count_memories_since, count_pending_memory_prs, count_suppressed, coverage_snapshot, create_context_ablation_replay, current_code_context_nodes, data_dir, db_path, decide_memory_pr, decide_pending_memory_mutation, delete_node, demote_memory, demote_memory_as_actor, dream_compile_candidates, due_for_review_node_ids, expire_stale_reconsolidation_plans, export_portable_archive, export_portable_archive_to_path, get_active_intentions, get_active_intentions_in_scope, get_agent_run, get_all_connections, get_all_nodes, get_all_nodes_in_scope, get_avg_retention, get_composition_event, get_composition_members, get_composition_neighbors, get_composition_outcomes, get_compositions_for_memory, get_connections_for_memory, get_connector_cursor, get_consolidation_history, get_context_ablation_replay, get_dream_history, get_insights, get_intention, get_intentions_by_status, get_last_consolidation, get_last_dream, get_memory_pr, get_memory_subgraph, get_merge_operation, get_merge_policy, get_most_connected_memory, get_never_composed_candidates, get_never_composed_candidates_in_scope, get_node, get_overdue_intentions, get_receipt, get_receipt_attestation_envelope, get_recent_composition_events, get_recent_composition_events_page, get_recent_connections, get_recent_state_transitions, get_retention_distribution, get_retention_trend, get_retrieval_replay_capsule, get_review_queue, get_state_transitions, get_stats, get_trace, get_walk_receipt, git_commit_nodes, grant_actor_role, hybrid_search, hybrid_search_filtered, hygiene_snapshot, import_portable_archive, import_portable_archive_from_path, ingest, ingest_in_scope, ingest_in_scope_with_secret_policy, ingest_with_secret_policy, intention_memory_snapshot, last_backup_timestamp, last_session_failed_calls, latest_receipt_chain_entry, link_receipt_to_run, list_agent_runs, list_endorsement_events, list_memory_prs, list_merge_operations, list_receipts, list_receipts_for_run, list_reconsolidation_plans, list_tag_operations, load_active_synaptic_tags, lowest_retention_nodes, maintain_gc_batch, maintain_lifecycle_batch, maintain_log_batch, maintenance_memory_page, mark_reviewed, merge_candidates, merge_undo, node_is_in_scope, open_failures_touching, preview_tag_mutation, process_actor_did, process_synaptic_ingest, projection_candidates, promote_memory, promote_memory_as_actor, promote_memory_backfill, prune_agent_traces, purge_node, query_time_range, recall, receipt_attestation_status, reconcile_source_tombstones, record_anchor_verification, record_batch_retrieval, record_code_anchors, record_composition_outcome, record_memory_access, record_reinforce_endorsement, register_receipt_signing_key, registered_receipt_signing_key, release_quarantine, replace_code_anchors, replay_intention_graph, resolve_actor_role, resolve_commit_sha_root, resolve_handle, retire_affected, reverse_suppression, run_consolidation, run_rac1_cascade_sweep, save_composition, save_connection, save_connector_cursor, save_counterfactual_replay_receipt, save_dream_history, save_insight, save_intention, save_memory_pr, save_receipt, save_retrieval_receipt_with_replay_capsule, save_signed_retrieval_receipt_with_replay_capsule_atomic, save_synaptic_tag, save_walk_receipt, schema_introspection, search, set_created_at, set_merge_policy, set_process_actor, set_protected, sidecar_dir, smart_ingest_excluding_in_scope_with_secret_policy_and_labile, snooze_intention, state_distribution, strengthen_connection, superseded_node_ids, supersession_pairs, suppress_memory, sync_portable_archive_cloud, sync_portable_archive_file, tag_vocabulary, undo_tag_mutation, update_intention_status, update_memory_state, update_node_content, upsert_by_source, verify_stored_receipt_attestation

### Async cognitive seam (26 methods, NO defaults — implement explicitly)

add_edge, classify, count, delete, delete_domain, fts_search, get, get_domain, get_due_memories, get_edges, get_neighbors, get_scheduling, get_store_stats, health_check, init, insert, list_domains, register_model, registered_model, remove_edge, search_records, update, update_scheduling, upsert_domain, vacuum, vector_search

## Verification receipts

- `cargo check --workspace --all-targets` — 0 errors.
- `cargo test -p vestige-core -p vestige-mcp` — exit 0 (core green; mcp lib
  789 passed, protocol suites 46 + 35 passed, sized-store 2 passed,
  wall mock test 1 passed).
- `cargo test -p vestige-phase-1-tests` — 19 passed (dyn-trait consumers).
- `grep -rn SqliteMemoryStore crates/vestige-mcp/` — 0 hits.

## Known non-goals / leftovers

- The empty `embeddings` feature still guards a dead
  `storage.init_embeddings()` call in `bin/cli.rs` (dead at default features
  since the vectorless build; unchanged from base).
- `tests/e2e` harness holds `Arc<Storage>` via `open_storage`; its suites
  compile but were not run here (not in scope; they drive server binaries).
