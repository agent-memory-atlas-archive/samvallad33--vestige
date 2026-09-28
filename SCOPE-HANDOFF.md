# SCOPE-HANDOFF — w3d: selftest + forgotten_lesson

Branch `build/w3d-selftest-forgotten` (base `main` @ `5fbe1df`). All additions
are append-only except three marked test-constant updates forced by parity
guards; nothing outside the listed files was touched.

## New exposed APIs

### `vestige-mcp` MCP tools

| Tool | Module | Signature |
|---|---|---|
| `selftest` | `crates/vestige-mcp/src/tools/selftest.rs` | `pub async fn execute(storage: &Arc<Storage>, args: Option<Value>) -> Result<Value, String>` — no arguments |
| `forgotten_lesson` | `crates/vestige-mcp/src/tools/forgotten_lesson.rs` | same shape; args `{failure_id (required), scope?, scan_limit?}` |

`tools::selftest::schema()`, `tools::forgotten_lesson::schema()` — full JSON
schemas; both registered in `tools::compact::full_schema` (so
`memory_status view='tools'` unfolds them).

Pure helpers other agents can reuse:

- `tools::forgotten_lesson::detect_lesson(&KnowledgeNode, &HashSet<String> failure_anchors, DateTime<Utc> failure_at) -> Option<ForgottenLesson>` — one shared detection rule (lesson tag / exact past-tense fix marker + >=1 exact shared anchor + FSRS R < 0.5 at failure time).
- `tools::forgotten_lesson::{lesson_tagged, has_fix_marker}`, `pub const FORGOTTEN_THRESHOLD: f64 = 0.5`.
- `tools::selftest` internals are private; the calibration flow is only the `execute` tool.

### CLI subcommands (`crates/vestige-mcp/src/bin/cli.rs`)

- `vestige selftest` — prints the same payload as the MCP tool.
- `vestige forgotten-lesson <FAILURE_ID> [--scope S] [--json]`.

## Calibration definition (selftest)

`gap_calibration = true` iff the 6th round (cause and failure share NO anchor)
returns `triggered=true` with an EMPTY causes list AND a non-null `gap` whose
`missing_entities` contains the withheld env-shaped anchor `planted_cause_6`.
Rounds 1-5 each isolate one planted cause (5d backdated, own scope) with a
failure sharing exactly one anchor; hit@1 = cause ranked first, hit@3 = top 3.
The live store is only read (`Storage::backup_to` / `VACUUM INTO` snapshot to a
tempdir); all mutation happens on the copy; the temp store is deleted.

## Non-append-only edits (forced by existing parity guards)

`crates/vestige-mcp/src/server.rs` tests only: advertised-tool count 16 -> 18
(two places), read-only hint list now
`["forgotten_lesson", "memory_status", "selftest", "session_start"]`. Catalog
and dispatch arms themselves are append-only additions.

## Test status

- `cargo test -p vestige-mcp --lib selftest` → 2/2 ok.
- `cargo test -p vestige-mcp --lib forgotten` → 5/5 ok.
- Full `--lib` suite: 763 passed, 2 failed — BOTH failures
  (`server::tests::test_recall_lookup_matches_search_shape`,
  `tools::backfill::tests::live_backfill_surfaces_root_cause_through_storage`)
  reproduce on the clean base commit (verified via stash) and are pre-existing.
- `cargo clippy -p vestige-mcp --lib --bins`: clean.
