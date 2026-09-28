<p align="center">
  <img src="https://raw.githubusercontent.com/samvallad33/vestige/media/vestige-logo.png" alt="Vestige" width="620">
</p>

# Vestige

**Local MCP memory for coding agents. Version 3.1.0.**

The server binary is `vestige-mcp`. The CLI is `vestige`. Memories live in a SQLite file on the machine. `smart_ingest` creates a memory, merges it into a similar one, or supersedes an outdated one. `recall` retrieves. A retrieval can persist a receipt. Review mode defaults to `fast` (writes auto-commit). Cloud sync stays off until you run it.

[![Release](https://img.shields.io/github/v/release/samvallad33/vestige?color=06b6d4)](https://github.com/samvallad33/vestige/releases/latest)
[![Tests](https://img.shields.io/github/actions/workflow/status/samvallad33/vestige/ci.yml?branch=main&label=CI)](https://github.com/samvallad33/vestige/actions)
[![Binary](https://img.shields.io/badge/platforms-5_release_targets-informational)](https://github.com/samvallad33/vestige/releases/latest)
[![License](https://img.shields.io/badge/license-AGPL--3.0-3b82f6)](LICENSE)

[Install](#install) · [Why not RAG](#why-not-just-rag) · [The Live Gate](#founding-operator) · [Continuity](#managed-continuity) · [Benchmark](#the-receipts-silent-rotation) · [Science](#the-science) · [Docs](#go-deeper)

<a id="getting-started"></a>
## The cause never looks like the bug

A new session does not automatically have the last session's decisions. Vestige keeps them in the local store. Redundant writes merge. `recall` with `mode` `contradictions` returns disagreement pairs for a topic. When a failure is already stored, `backfill` lists earlier memories that share entities with it. Those rows are hypotheses: `evidence_status` is `hypothesis` and `causality_verified` is false. `promote` defaults to false, so the preview writes no candidate edges and changes no strength.

<p align="center">
  <a href="https://raw.githubusercontent.com/samvallad33/vestige/media/vestige-black-box.mp4">
    <img src="https://raw.githubusercontent.com/samvallad33/vestige/media/black-box-cause.gif" alt="vestige backfill --contrast: a similarity ranking, then shared-entity candidates" width="100%">
  </a>
</p>

<p align="center"><sub><b>`vestige backfill --contrast`</b> prints a similarity ranking first (hybrid when embeddings are ready, otherwise keyword), labeled as resemblance, then the backward pass. Candidates that share entities are associations, not a proven cause. <a href="https://raw.githubusercontent.com/samvallad33/vestige/media/vestige-black-box.mp4">Watch the walk</a>.</sub></p>

## Install

The npm package needs Node.js. Homebrew and eget install a prebuilt binary. Release targets: macOS ARM and Intel, Linux x86_64 and arm64, Windows x86_64.

```bash
npm install -g vestige-mcp-server@latest
```

```bash
brew install samvallad33/tap/vestige
```

`eget samvallad33/vestige` pulls the matching prebuilt binary from Releases.

```json
{
  "mcpServers": {
    "vestige": { "command": "vestige-mcp" }
  }
}
```

| Client | Setup |
|---|---|
| Claude Code | `claude mcp add vestige vestige-mcp -s user` |
| Codex | `codex mcp add vestige -- vestige-mcp` |
| Cursor / VS Code / Windsurf | [docs/integrations/](docs/integrations/) |
| Claude Desktop | [docs/CONFIGURATION.md](docs/CONFIGURATION.md#claude-desktop-macos) |
| Cline / Continue / Zed / Goose | the JSON above, in that client's MCP settings |

`vestige dashboard` serves the UI at **http://localhost:3927/dashboard** (override with `--port` or `VESTIGE_DASHBOARD_PORT`). On a build with embeddings, the first run logs a ~130 MB download of `nomic-ai/nomic-embed-text-v1.5`. Keyword search and `smart_ingest` work before that download finishes; semantic ranking starts when the runtime is ready. Walkthrough: [docs/GETTING-STARTED.md](docs/GETTING-STARTED.md).

### CLI

`vestige` is the clap binary in `src/bin/cli.rs`. `vestige-mcp` is the stdio server. Subcommands:

`stats`, `health`, `consolidate`, `upgrade` (`--dry-run`), `update`, `sandwich`, `embeddings`, `restore`, `backup`, `export`, `portable-export`, `portable-import`, `sync`, `gc`, `dashboard`, `ingest`, `scan-secrets`, `backfill`, `recall`, `compose`, `project`, `serve`.

`backfill` takes `--failure-id`, `--manual`, `--lookback-days` (default 30), `--no-promote`, `--contrast`, and `--json`. `serve` listens on 3928 and can also start the dashboard on 3927. `sync --cloud` talks to a hosted endpoint; a plain `sync <archive>` is a file.

Global `--data-dir` overrides `VESTIGE_DATA_DIR`.

### Server config

`vestige-mcp` reads these (defaults from its `--help`):

| Variable | Default |
|---|---|
| `VESTIGE_DATA_DIR` | platform data dir, `vestige.db` inside it |
| `VESTIGE_HTTP_ENABLED` | off |
| `VESTIGE_HTTP_PORT` | 3928 |
| `VESTIGE_HTTP_BIND` | `127.0.0.1` |
| `VESTIGE_HTTP_ALLOWED_ORIGINS` | unset |
| `VESTIGE_AUTH_TOKEN` | override; otherwise `auth_token` in the data dir, created if missing |
| `VESTIGE_DASHBOARD_ENABLED` | off for the server process; `vestige dashboard` starts it |
| `VESTIGE_DASHBOARD_PORT` | 3927 |
| `VESTIGE_SYSTEM_PROMPT_MODE` | `minimal` (`full` injects the composition mandate) |
| `VESTIGE_CONSOLIDATION_INTERVAL_HOURS` | 6 |
| `VESTIGE_DREAM_COMPILE_AUTOFIRE` | off |
| `VESTIGE_ENCRYPTION_KEY` | used only when the `encryption` feature (SQLCipher) is compiled in |
| `VESTIGE_CLOUD_SYNC_KEY`, `VESTIGE_CLOUD_ENDPOINT` | required for `vestige sync --cloud` |
| `VESTIGE_CLOUD_ENCRYPTION_KEY` | passphrase for the client-side archive; never sent |

## Why not just RAG?

Similarity search ranks memories that resemble the query. `vestige backfill --contrast` shows that ranking, then a second list: earlier memories that share entities with a stored failure.

| | Vector search | Vestige |
|---|---|---|
| Retrieval basis | Similarity to the query | `recall` mode `lookup` is hybrid keyword and semantic search. `reason` adds trust, spreading activation, supersession, and contradictions |
| A stored failure | The lookalike, not a shared-entity candidate | `backfill` returns hypotheses. `promote=false` by default |
| Contradictions | Both stored, both returned | `recall` mode `contradictions` returns disagreement pairs |
| Redundant writes | Accumulate | `smart_ingest` merges into a similar memory |
| Unused memories | Stay at full weight | FSRS-6 decay, via consolidation |
| Where the file sits | Often a hosted index | SQLite on the machine. `vestige sync --cloud` is opt-in and encrypts the archive first |

The backward pass is the Retroactive Salience Backfill path (Cai 2024, *Nature*, named in the CLI and the `backfill` tool).

## 🛡️ Founding Operator

Review is `crates/vestige-core/src/trace/review.rs`.

- **`fast`** (default). Writes auto-commit. Purge runs directly.
- **`risk_gated`**. Ordinary writes auto-commit. Risky writes open a Memory PR.
- **`paranoid`**. Every write waits for approval.

Under `risk_gated` and `paranoid`, `purge`, `delete`, and `suppress` are pre-gated: the server opens a pending Memory PR and returns without applying the mutation. The dashboard lists those PRs and can promote, merge, supersede, quarantine, or forget them.

**See it stop a real agent: [▶ THE LIVE GATE (1:44)](https://github.com/samvallad33/vestige/releases/tag/launch-night-live-gate-20260914)**, recorded in one take.

<a id="managed-continuity"></a>
<a id="vestige-pro"></a>
## 🔄 Managed Continuity

`vestige backup <file>` copies the SQLite database. `vestige sync <archive>` two-way syncs a portable archive (Dropbox, iCloud, Syncthing, or git all work as the folder). `vestige sync --cloud` uses the hosted endpoint instead of a file.

Cloud bytes are encrypted on the client before upload: Argon2id derives a key from `VESTIGE_CLOUD_ENCRYPTION_KEY`, XChaCha20-Poly1305 seals the archive. The hosted service stores ciphertext. Losing the passphrase makes that archive unrecoverable.

```bash
vestige backup ~/vestige-backup.db
vestige sync ~/vestige-archive.vportable
vestige --data-dir ~/new-machine-store sync ~/vestige-archive.vportable
```

## The receipts: Silent Rotation

The pooled Silent Rotation numbers are being recounted on branch `benchmark/silent-rotation`. This README does not publish a score.

```bash
git clone -b benchmark/silent-rotation --depth 1 https://github.com/samvallad33/vestige.git
cd vestige/benchmarks/silent-rotation
python3 tests/bm25_baseline.py results/runA-trial-1/corpus-export.json --no-dense
```

## The science

Mechanisms that ship in this tree. Write-up: [docs/SCIENCE.md](docs/SCIENCE.md).

| Mechanism | What the code does | Where |
|---|---|---|
| Prediction-error gating | `smart_ingest` creates, merges, or supersedes | `tools/smart_ingest` |
| FSRS-6 | Unused memories decay; consolidation refreshes scores | `vestige-core/src/fsrs` |
| Retroactive salience backfill | Shared-entity hypotheses from a stored failure | `tools/backfill.rs` |
| Synaptic tagging | Tags memories for later consolidation | `tools/tagging.rs` |
| Spreading activation | `recall` mode `reason` activates related memories | `neuroscience/spreading_activation.rs` |
| Dual strength | Storage strength and retrieval strength are separate fields | `advanced/reconsolidation.rs` |
| Dream compile | `maintain` action `dream_compile` files Memory PRs | `tools/dream_compile.rs` |
| Active forgetting | `suppress` inhibits retrieval without deleting | `tools/suppress.rs` |

## The tools

`tools/list` advertises these sixteen. Folded names (`search`, `importance_score`, `context`, `deep_reference`, `session_context`) are not in that list.

| Tool | What it does |
|---|---|
| `recall` | `lookup` (default): hybrid keyword and semantic search. `reason`: trust, spreading activation, supersession, contradictions. `contradictions`: disagreement pairs |
| `receipt` | Read a persisted retrieval receipt, or replay its frozen evidence pack |
| `memory` | `get`, `get_batch`, `state`, `promote`, `demote`, `edit`, `purge` (`confirm=true`) |
| `purge` | Remove one memory's content and embeddings. Irreversible. `confirm=true`. Same path as `memory` action `purge` |
| `codebase` | `remember_pattern`, `remember_decision`, `get_context`, `verify`, `reanchor` |
| `project` | Preview or write a fenced region of `CLAUDE.md` or `MEMORY.md`. `write` needs `confirm=true` |
| `intention` | `set`, `check`, `update`, `list`; `graph` for evidence-aware plans |
| `smart_ingest` | Save through prediction-error gating. Batch via `items` (max 20) |
| `source_sync` | Index GitHub (`GITHUB_TOKEN`) or Redmine (`REDMINE_URL`, `REDMINE_API_KEY`) into local memories |
| `memory_status` | `health`, `retention`, `timeline`, `changelog`, `stats`, `tools` (full schema for one tool) |
| `maintain` | `consolidate`, `dream`, `dream_compile`, `gc` (`dry_run` default true), `importance_score`, `backup`, `export`, `restore` |
| `dedup` | `scan`, merge and supersede plans, `apply`, `undo`, `verdict` (approve, reject, or quarantine a reconsolidation plan), tag rename/merge, `protect`, `policy` |
| `graph` | `chain`, `associations`, `bridges`, `predict`, composition topology. `label` is the write |
| `session_start` | One call: memories, open intentions, status, predictions, codebase context, under a token budget |
| `suppress` | Inhibit retrieval and speed decay without deleting. `reverse=true` undoes it within 24 hours |
| `backfill` | Hypotheses from earlier memories that share entities with a failure. `promote` defaults to false |

Contracts: [docs/TOOL-CONTRACTS.md](docs/TOOL-CONTRACTS.md). Hygiene: [docs/MEMORY_HYGIENE.md](docs/MEMORY_HYGIENE.md).

## The dashboard

```bash
vestige dashboard
```

The observatory is at **http://localhost:3927/dashboard**. Its clock is a fixed 60fps loop, 720 frames (12 seconds). Loop export writes that loop to an mp4. Brain-print share links are structure only: a quantized shape, not memory text.

## Under the hood

| | |
|---|---|
| Engine | Rust 2024. Workspace, `vestige-mcp`, and `vestige-core` are version 3.1.0 |
| Retrieval | `nomic-ai/nomic-embed-text-v1.5` stored at 256 dimensions (Matryoshka truncation of the 768-d output), USearch HNSW, SQLite FTS5. Optional Qwen3 profiles behind `qwen3-embeddings` |
| Storage | SQLite. SQLCipher when built with `--features encryption` and `VESTIGE_ENCRYPTION_KEY` is set |
| Sync | Local by default. `vestige sync --cloud` encrypts first |

## Go deeper

[Getting Started](docs/GETTING-STARTED.md) · [FAQ](docs/FAQ.md) · [The Science](docs/SCIENCE.md) · [Configuration](docs/CONFIGURATION.md) · [Storage](docs/STORAGE.md) · [Tool contracts](docs/TOOL-CONTRACTS.md) · [Changelog](CHANGELOG.md)

## License

AGPL-3.0. See [LICENSE](LICENSE).