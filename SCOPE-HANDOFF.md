# Scope handoff — build/w2a-hunk-lines

Base: main @ 5fbe1df. Scope: `crates/vestige-core/src/advanced/git_records.rs` + tests only.

## Out-of-scope file touched (minimal, compile-only)

`crates/vestige-mcp/src/tools/backfill.rs` — test helper `commit_record_content`
(line ~861) constructs `GitCommit` literally. Added the three new fields with
empty values so the crate compiles:

```rust
hunks: vec![],
extra_hunks: 0,
imports: vec![],
```

No logic changed; `tools::backfill` tests pass unchanged.

## What changed in git_records.rs

- `HunkSpan { file, start, len, symbol }` — new-side `@@` spans, `MAX_HUNKS = 200`,
  overflow counted in `GitCommit::extra_hunks`.
- `GitCommit::imports: Vec<(file_in_commit, target_path, resolved)>` —
  `MAX_IMPORTS = 40`, deduped, sorted by (file, target).
- `record_content` new lines:
  - `hunks: file:start+len` comma-joined, item list capped at 400 chars
    (`MAX_HUNK_LINE`), followed by ` (+N more)` when spans were dropped or
    overflowed; emitted only when spans exist.
  - `imports: file->target` comma-joined; unresolved edges render as
    `file->?target` (target kept as written, never guessed).
- Import capture: `use a::b::Item` / `import x.y.z` (multi-segment only, so
  `import os` noise is skipped) / `from x.y import z` / `#include "p"` and
  `#include <p>`. Resolution is exact module-segment matching against the same
  commit's file list and their module dirs (`crate` roots at `src/`; Rust
  `.rs`/`mod.rs`, Python `.py`/`__init__.py`, include verbatim). File hits beat
  module-dir hits; longest path first. No fuzzy matching.
- Hunks/imports for files beyond `MAX_FILES` are dropped (same
  non-attribution rule as symbols), not counted as span overflow.

## Pre-existing test failure (not from this branch)

`tools::backfill::tests::live_backfill_surfaces_root_cause_through_storage`
(crates/vestige-mcp) fails on pristine base @ 5fbe1df — verified via stash
with these changes removed: the billing-service distractor outranks the
env-var cause. The test never touches `GitCommit`/`record_content`; the two
commits below base reworked exactly that ranking code. All other
`tools::backfill` tests pass.

## Incident note: shared stash stack race

This repo's worktrees share one `git stash` stack. During verification, a
`git stash pop` in this worktree raced sibling worktrees and pulled in two
foreign stashes. Both were re-stashed onto the shared stack as labeled
entries — find them with `git stash list | grep RESCUE`:

- `RESCUE-A(65a5ea0)` — storage/session_context/source_sync changes, owner:
  build/w2c-resolver worktree.
- `RESCUE-B(ecfcddfb)` — dashboard cinema changes (MemoryCinema, sandbox,
  storm, temporal-emissive.ts), owner: feat/dashboard-live-max worktree.

Original dropped hashes are recoverable from the stash reflog if needed.
Use hash-based `git stash apply <hash>` (resolved by message) in this repo,
never positional `stash@{n}`.
