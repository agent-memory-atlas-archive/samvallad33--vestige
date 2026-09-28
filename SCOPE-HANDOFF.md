# SCOPE-HANDOFF — Handle-Based Recall Resolver (build/w2c-resolver)

Branch: `build/w2c-resolver`, base = `main` @ `5fbe1df`. Not pushed.

## Files (the entire scope)

- `crates/vestige-core/src/storage/resolver.rs` — NEW. The resolver.
- `crates/vestige-core/src/storage/mod.rs` — wiring only (`mod resolver;` + re-exports).
- `crates/vestige-mcp/src/tools/recall.rs` — thin handle-mode extension at entry.
- `SCOPE-HANDOFF.md` — this file.

## Core API (vestige-core)

```rust
// crates/vestige-core/src/storage/resolver.rs
impl SqliteMemoryStore {
    pub fn resolve_handle(&self, query: &str) -> HandleResolution;
}

pub struct HandleResolution {
    pub kind: HandleKind,                     // what the query resolved as
    pub ids: Vec<String>,                     // node/trace ids resolved
    pub exact: bool,                          // true iff equality (not prefix)
    pub candidates: Vec<(String, HandleKind)>,// when prefix-ambiguous, capped at 20
    pub handle_required: Option<String>,      // guidance when nothing matched
}

pub enum HandleKind { Memory, Commit, File, Symbol, Test, Run, ToolCall, Tag, Unknown }

pub const MAX_CANDIDATES: usize = 20;
pub const HANDLE_REQUIRED_DETAIL: &str =
    "recall is handle-based: pass a memory id, commit sha, file, symbol, test, run, or tool-call id";
```

Import paths: `vestige_core::storage::{HandleKind, HandleResolution, MAX_CANDIDATES, HANDLE_REQUIRED_DETAIL}`. `resolve_handle` is a method on `SqliteMemoryStore` (`vestige_core::Storage`). Read-only: no FSRS/edge/graph writes.

## Resolution rules (EXACT or PREFIX only — no fuzzy, no lexical ranking, no FTS)

| # | Kind | Match | Prefix? | Source |
|---|------|-------|---------|--------|
| 1 | Memory | query parses as uuid, equals `knowledge_nodes.id` | no | `knowledge_nodes` |
| 2 | Commit | `commit <sha>` on content line 1 of `git-commit`-tagged nodes; case-insensitive | **yes, >= 7 hex chars**; 4–6 hex chars w/ a digit = ambiguity error; unique prefix resolves with `exact=false`; multiple = capped candidates | `knowledge_nodes` |
| 3 | File / Test | whole-token boundary match over content + tags (coarse LIKE prefilter, exact Rust verify). File if path-shaped (`/` or `.`); Test if test-shaped (`tests/`, `_test`, `test_` prefix) | no | `knowledge_nodes` |
| 4 | Symbol | query normalized camel→snake+lowercase (same `extract_entities` normalization); matched against Code-tier extracted entities only — Word/Path/Version tiers excluded so `pyvenv` can never prefix-match `pyvenv.cfg` | **yes (unique prefix)** | `knowledge_nodes` |
| 5 | Run | equals `agent_runs.run_id` or `agent_traces.run_id` | no | V18 tables |
| 6 | ToolCall | equals `agent_traces.id` (trace event id) | no | V18 tables |
| 7 | Tag | exact string equality against parsed tags JSON (case-sensitive) | no | `knowledge_nodes` |

Unresolved / empty → `kind=Unknown`, `handle_required=Some(HANDLE_REQUIRED_DETAIL)`.

## MCP surface (recall)

New schema property `handle` (string). Semantics in `recall::execute`:

- args WITHOUT a `handle` key → legacy mode dispatch, byte-identical behavior
  (hot-path invariant preserved; existing tests pass unchanged). **This gate
  (`handle_flow` returning None) is the single flip point for making recall
  handle-only by default.**
- args WITH `handle: "<handle>"` → resolve, then:
  - resolved: `{"handle", "kind", "exact", "nodes":[{id,type,content,tags}], "neighbors":[{from,to,link_type,strength,direction,node}]}` — neighbors are one hop over `memory_connections` (both directions, strength-desc, deduped, capped at 20 edges).
  - ambiguous: `{"error":"ambiguous", "detail", "handle", "kind", "candidates":[{id,kind}]}`.
  - nothing: `{"error":"handle_required", "detail", "candidates":[]}` (`detail` carries the resolver's specific too-short-sha message when applicable).
- args WITH `handle: ""` (or non-string) → free-text branch: same
  `handle_required` payload, with candidates mined by running the resolver on
  the whole `query` text and its first 8 identifier-shaped tokens (exact/prefix
  results only, deduped, capped at 20).

## Integrator notes

- Scope-agnostic by design: handles are globally unique, so resolution ignores
  `scope`. Filter client-side if a scoped surface is needed.
- Symbol prefix scans all nodes and runs `extract_entities` per row (no entity
  index yet). Correctness-first; add an entity index before high-QPS use.
- Commit resolution reads shas only from `git-commit`-tagged records in the
  canonical `git_records::record_content` shape.
- Known deviation from the letter of the design: File and Test share one
  boundary-exact step (file before symbol per the order); a test-shaped query
  resolves as kind `Test` even before the symbol step would have matched.
- English hex words without digits (`face`, `added`) are not treated as
  too-short shas; 4–6 char hex WITH a digit is the ambiguity error.

## Tests

- `cargo test -p vestige-core --lib resolver` → 8 passed (uuid exact/prefix
  paths, full/prefix/ambiguous/too-short shas, file exact + NO-fuzzy +
  NO-prefix receipts (`pyvenv` and `pyvenv.c` both fail against `pyvenv.cfg`),
  symbol camel/snake/env normalization + prefix, test names, tags exact,
  run/toolcall exact, free prose unresolved).
- `cargo test -p vestige-mcp --lib tools::recall` → 7 passed (schema, legacy
  default, contradictions, uuid + one-hop neighbors ordering, sha exact +
  ambiguous, handle_required with/without candidates, no-handle legacy path).
- `cargo test -p vestige-core --lib "storage::sqlite"` → 220 passed (no
  regressions in the neighboring suite).
- Pre-existing failure, NOT from this branch (fails on clean `main` @
  `5fbe1df` too): `vestige-mcp server::tests::test_recall_lookup_matches_search_shape`
  — it calls the removed `search` tool and compares against its error text.
  `server.rs` is outside this task's file scope.
