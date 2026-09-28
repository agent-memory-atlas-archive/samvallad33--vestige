# SCOPE-HANDOFF — build/w3c-session-sync

Base: main @ 5fbe1df. Scope held: `crates/vestige-mcp/src/tools/session_context.rs`,
`crates/vestige-mcp/src/tools/source_sync.rs`, new
`crates/vestige-core/src/storage/session_queries.rs` (+ the one-line module
wiring and re-export in `crates/vestige-core/src/storage/mod.rs`, required to
compile the new file). Nothing else touched.

## What was built

### 1. `session_queries.rs` (new, on SqliteMemoryStore)

- `open_failures_touching(changed_files: &[String]) -> Vec<OpenFailureTouching{id, content_preview, anchor}>`
  — failure-like nodes (`retroactive_backfill::looks_like_failure`, reuse) that
  are open (not superseded/suppressed, inside validity window) and whose
  recorded files intersect the changed set by EXACT path equality:
  `code_memory_anchors.file_path` first (anchor rendered `path` or
  `path:symbol`), then the `files:` line of a git-commit record (the
  ` (+N more)` suffix stripped). Prefix/suffix near-misses do not hit. Bounded
  chunked node scan (pages of 500, hard cap 10k nodes).
- `last_session_failed_calls(run_id: Option<&str>) -> Vec<FailedToolCall{run_id, tool, at, error_excerpt}>`
  — `agent_traces` rows of the given run, or the latest run by
  `agent_runs.last_at` when `None`, whose serialized `mcp.call` payload
  carries `success: false`. Last 20, chronological. Rows without a `success`
  field are not failed calls.
- `append_mcp_call_outcome(run_id, tool, success, error, at_ms)` — writes an
  `mcp.call`-shaped trace row extended with `success`/`error` (argsHash is the
  opaque constant `"outcome"`). Payload keeps the exact event shape so
  `get_trace` replay still parses it. NOTE: no production caller yet — the
  trace recorder (`trace_recorder.rs`) is out of scope and still writes
  plain `mcp.call` events, so the failed-calls section stays empty until the
  recorder adopts this method. That adoption is the natural follow-up.
- `closed_issue_nodes(source_system, scope)` / `git_commit_nodes(limit)` —
  exact-tag lookups (`state:closed`, `git-commit`) backing the closed_by link.

### 2. `session_context.rs` (session_start) — purely additive

- New optional arg `changed_files: Vec<String>` (schema + serde, camelCase
  alias). Absent = open-failures section skipped. Present but no exact match =
  no section (silence, never an empty header).
- Section `**Open failures touching changed files:**` — up to 8 lines
  `- [id] preview (anchor)`, budget-accounted like every other section.
- Section `**Last session failed calls (run):**` — automatic, latest run, up
  to 8 lines `- tool: error_excerpt`.
- Both sections degrade silently on query error; all existing sections and
  the budget-trim loop are untouched.

### 3. `source_sync.rs` — `closed_by` chain linking (github, local-only)

- Verified what the github connector fetches today: issues + comments only —
  no closing-PR sha, no timeline/cross-referenced events. Per the build
  instruction, remote fetch is SKIPPED; linking is local-only and
  deterministic.
- `commit_closes_issue(content, issue_number)` — GitHub closing keywords
  (close/closes/closed/fix/fixes/fixed/resolve/resolves/resolved), whole-word,
  case-insensitive, and `#<number>` with exact digit boundaries (`#420` never
  satisfies #42; `abc#42`/`##42` rejected), keyword + reference on the SAME
  line.
- `link_closed_by_from_local_commits(storage, scope)` — for each live closed
  github issue in scope, writes `memory_connections` edges
  `link_type="closed_by"`, `source_id` = issue node, `target_id` = commit
  node, via the existing `save_connection` (INSERT OR REPLACE ⇒ idempotent
  re-syncs). Runs after every successful github `run_sync`; count surfaced as
  `closedByLinks` in the tool result and appended to the summary when > 0.
  Redmine is untouched.

## Tests (all green)

- `cargo test -p vestige-core --lib session_queries` — 6/6.
- `cargo test -p vestige-core --lib storage::` — 372/372.
- `cargo test -p vestige-mcp --lib session` — 27/27 (4 new:
  open-failures populate from anchors + `files:` lines, negative no-noise
  near-miss/absent-arg, failed-calls latest-run, silent without outcome rows).
- `cargo test -p vestige-mcp --lib source_sync` — 4/4 (keyword forms,
  near-miss rejects incl. `discloses`/`#420`/cross-line, edge appears +
  idempotent + open-issue/commit negatives, silent without candidates).
- Full `cargo test -p vestige-mcp --lib`: 764 passed, 2 failed —
  `server::tests::test_recall_lookup_matches_search_shape` and
  `tools::backfill::tests::live_backfill_surfaces_root_cause_through_storage`
  both REPRODUCE ON PRISTINE main @ 5fbe1df (verified via stash); pre-existing,
  not from this branch.

## Follow-ups (out of scope here)

- Recorder adoption of `append_mcp_call_outcome` in `trace_recorder.rs` /
  `server.rs` dispatch so the failed-calls section has a production producer.
- Remote closing-PR lookup if ever wanted: needs a timeline-events fetch in
  the github connector (new API surface, deliberately not added).
