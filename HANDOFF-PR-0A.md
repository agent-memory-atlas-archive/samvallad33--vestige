# HANDOFF: PR 0a (v3-guard) — Compilation Prerequisite Fix Needed

## TL;DR
The workspace does not compile. The root cause is a **type-scattering problem**: a 209-method storage trait (`LocalMemoryStore` in `memory_store.rs`) references ~50 type definitions (structs/enums) that live inside feature-gated SQLite modules. When `legacy-sqlite` is off, those types vanish and the trait breaks. Multiple fix attempts have left the tree in a conflicted state. **Start from `3270cf1` (proven green in both modes) and apply a clean fix.**

## Repo State
- **Repo**: `/Users/entity002/vestige-pr` (worktree of `~/vestige`, github.com/samvallad33/vestige)
- **Branch**: `strata/fix-00a-v3-guard`
- **Last good commit**: `3270cf1` — "fix(storage): remove orphaned doc comments from T1/T5 merge resolution"
- **Verify it's clean**: `git reset --hard 3270cf1` then:
  - `cargo check --workspace --all-targets` → 0 errors
  - `cargo check --workspace --no-default-features --all-targets` → 0 errors
  - `cargo test --workspace` → ~959 tests pass

## The Problem (exactly)

### Architecture
- `crates/vestige-core/src/storage/memory_store.rs` defines `trait LocalMemoryStore` (~209 methods, ~2100 lines).
- The trait's method signatures reference ~50 types: `StorageError`, `SmartIngestResult`, `ConnectionRecord`, `NeverComposedCandidate`, `BlastReport`, `HandleResolution`, `CoverageSnapshot`, `PortableArchive`, etc.
- These type definitions live in `crates/vestige-core/src/storage/sqlite/mod.rs` (a ~3000-line file behind `#[cfg(feature = "legacy-sqlite")]`), plus some in ungated modules like `blast.rs`, `resolver.rs`, `edges.rs`.
- The storage alias `pub type Storage = dyn MemoryStore` is in `storage/mod.rs`.

### What broke
- `legacy-sqlite` was removed from default features (commit `1c9ee5d`), meaning the default build has no SQLite module → no type definitions → the trait doesn't compile.
- Multiple agents + manual attempts to fix this by gating the trait or extracting types left duplicate imports, conflicting gates, and orphaned re-exports.

### What was tried (and why it failed)
1. **Gating the trait behind `legacy-sqlite`**: Works architecturally (wiped mode = no trait, MCP uses strata directly). Failed because `memory_store.rs` imports types from BOTH gated and ungated modules, and gating the module means `lib.rs` re-exports also need gating — cascading into every file that does `use crate::storage::{...}`.
2. **Extracting types to `types.rs`**: An agent created `storage/types.rs` with ~50 type definitions, but left broken `pub use crate::storage::types::{...}` re-exports in `attestation_store.rs`, `blast.rs`, `portable.rs`. Removing those re-exports removed the types entirely (the definitions had been MOVED, not copied).
3. **Regex-based bulk edits**: Caused more damage than they fixed (duplicate imports, broken syntax).

## The Fix (recommended approach)

**From `3270cf1`, do exactly this:**

### Step 1: Gate the trait module
In `storage/mod.rs`:
```rust
#[cfg(feature = "legacy-sqlite")]
mod memory_store;
```
And gate every `pub use memory_store::...` re-export with the same cfg.

### Step 2: Gate the Storage alias
```rust
#[cfg(feature = "legacy-sqlite")]
pub type Storage = dyn MemoryStore;
```

### Step 3: Gate the open_storage function and helpers
Gate `open_storage`, `default_db_path`, `db_path_for_data_dir` behind `#[cfg(feature = "legacy-sqlite")]`. Provide a feature-off twin:
```rust
#[cfg(not(feature = "legacy-sqlite"))]
pub fn open_storage(_p: Option<std::path::PathBuf>) -> Result<(), LegacySqliteDisabled> {
    Err(LegacySqliteDisabled)
}
```

### Step 4: Fix lib.rs re-exports
Currently at line ~198 of `crates/vestige-core/src/lib.rs`:
```rust
// CHANGE FROM (breaks: references gated items):
pub use storage::{
    ClassificationResult, ..., LocalMemoryStore, ..., LegacySqliteDisabled, open_storage,
};
// CHANGE TO:
pub use storage::LegacySqliteDisabled;
#[cfg(feature = "legacy-sqlite")]
pub use storage::{
    ClassificationResult, ..., LocalMemoryStore, ..., open_storage,
};
```

### Step 5: Fix MCP references (if any break)
Any `vestige-mcp` file that does `use vestige_core::Storage` or `use vestige_core::LocalMemoryStore` needs the same cfg gate or a trait-free alternative. The wire agent (branch `build/wire-strata`) already built `strata_boot` as the alternative path.

### Step 6: Verify BOTH modes
```bash
cargo check --workspace --all-targets          # 0 errors
cargo check --workspace --no-default-features --all-targets  # 0 errors
cargo test --workspace                          # all pass
```

## After the fix: PR 0a proper

Once compilation is green, the actual PR 0a work is:

1. **v3_guard.rs is already written** at `crates/vestige-core/src/storage/v3_guard.rs` — detect SQLite by magic bytes, open READ-ONLY via `file:<p>?mode=ro&immutable=1`, refuse with `V3StoreNeedsMigration`. Wire it into every storage entry point.

2. **strata-migrate crate** — already exists at `crates/strata-migrate/` (from T3 agent). Needs: `vestige migrate-to-strata` CLI command, BLAKE3 source hashing, GENESIS/PARAMS/replay through the gate, signed MIGRATION_RECEIPT frame (kind 46).

3. **9 named acceptance tests** (see the original PR spec in `~/Downloads/fix-prompts/01-0a-v3-guard-migrate-to-strata.md`).

4. **Full gate** (fmt, clippy `-D warnings`, workspace tests, strata crate tests, mcp check).

5. **5-line report** (format specified in the PR prompt).

## Other assets on other branches (DO NOT merge yet)
- `build/wire-strata` @ `f6018d9` — StrataStore boots as the MCP runtime (ingest/get/edge round-trip, 712/7cp tests green). Merges after PR 0a.
- `build/strata-log` — S1 log crate (merged to main already)
- All 12 W-build agents merged; all 6 STRATA crates exist

## The 15-PR series
The full fix series is at `~/Downloads/fix-prompts/` (15 files, README explains the order). PR 0a is first; each subsequent PR builds on the previous. Sam reviews and merges each before the next starts.
