<p align="center">
  <img src="https://raw.githubusercontent.com/samvallad33/vestige/media/vestige-logo.png" alt="Vestige" width="620">
</p>

# Vestige

Vestige gives AI agents a memory you can check and a guard that checks connected tool calls before execution. Memory lives in Strata, an append-only log where writes pass a gate and leave a receipt, and a memory is found by its exact id or tag, never by vector similarity. Operator Lite stops recognized destructive actions routed through its hooks, records its decisions, and supports one-use owner permits for eligible stopped actions. A remembered approval is history, never current permission. `causal_walk` follows recorded relationships backward to investigate a failure; Retroactive Salience Backfill updates how much earlier connected memories matter when new information arrives.

[![Release](https://img.shields.io/github/v/release/samvallad33/vestige?color=06b6d4)](https://github.com/samvallad33/vestige/releases/latest)
[![Tests](https://img.shields.io/github/actions/workflow/status/samvallad33/vestige/ci.yml?branch=main&label=CI)](https://github.com/samvallad33/vestige/actions)
[![Binary](https://img.shields.io/badge/platforms-6_release_targets-informational)](https://github.com/samvallad33/vestige/releases/latest)
[![License](https://img.shields.io/badge/license-AGPL--3.0-3b82f6)](LICENSE)

## The problem

Agents run `rm -rf`, force-push, drop tables, and overwrite `.env` on their own, and nothing stops them. When something breaks, similarity search finds lookalikes, not causes.

## What's new in v5

[5.0.0 PRECEDENT](https://github.com/samvallad33/vestige/releases/tag/v5.0.0) shipped on October 18, 2026. Vestige now reads its own enforcement record.

- **`precedent`.** Any connected agent can ask what the gate stopped or allowed, under which rule, and which permit answered each stop. It reads the gate's own receipts, writes nothing and cannot issue a permit, so a stopped agent can see why instead of retrying.
- **18 tools and no vectors.** The memory server advertises 18 MCP tools, adds `precedent` and `gather`, and ships no embedding runtime. Free-text recall returns `similarity_disabled`; a memory is found by its exact id or tag.
- **`gather`.** Point it at a GitHub issue and it records the issue, its comments, the linked pull requests and the commit window as receipts, then walks back toward the cause.
- **Rules that survive compaction.** `vestige hook session-start` brings your pinned decisions and corrections back after every compaction. `npx @vestige/init` registers it for Claude Code; see the [rules hook guide](docs/integrations/claude-code-rules-hook.md).
- **The PRECEDENT journey.** `npx @vestige/init precedent` follows one real stop from your machine around the loop: the stop, its receipt re-derived from its own bytes, the precedent and the verdict.
- **A rebuilt dashboard** on WebGPU, with the gate's receipts in its Guard view.
- **Operator Lite 0.3.10.** It blocks from install with 30 rules, reads the shell commands inside `python -c` and `node -e` code, sees through wrappers such as `sudo -u root`, `env -S` and `busybox`, and stops an agent from throwing away uncommitted git work.
- **Tested end to end.** [`full_flow_stdio.rs`](crates/vestige-mcp/tests/full_flow_stdio.rs) runs the whole loop on the shipped binaries in CI on Linux and macOS: the gate stops a call, the stop is recorded, a second agent recalls it and walks it back to its rule, and an ALLOW ONCE runs exactly once.

## Vestige Operator

[![Watch Vestige Operator stop what Operator Lite lets through](operator-lite/media/from-lite-to-operator.gif)](operator-lite/media/from-lite-to-operator.mp4)

Click the picture to watch the full film.

Operator Lite is free and stays free. Vestige Operator is the owner's version of the same gate: $149 once, and every later version is yours at no charge, with laws you write, a Board of today's stops, and a weekly Letter of what your agents tried and what stopped them. Operator also judges the chain of steps in a session, so an agent that reads `.env` and later posts a file to an outside host is stopped with the earlier step's time and receipt, and it carries six more stop rules than Lite. You download a small archive the moment you pay; from an installed Operator Lite, `upgrade --install <the archive you downloaded>` unpacks it and starts the wizard.

[Buy Vestige Operator](https://payhip.com/b/d4xvu)

## Retroactive Salience Backfill returns in v5.0.1

New information can change the importance of an older memory. Backfill walks
backward through Strata's recorded relationships and exact shared entities,
then raises or lowers earlier memories' salience. It does not rank by text or
embedding similarity, invent a relationship, or update a memory newer than the
trigger. `causal_walk` remains the separate investigation tool: "what caused
this?"

For a personal-memory example, save **"Maya felt off after dinner at Bangkok
Garden"** with entities `person:Maya` and `place:Bangkok Garden`. Later, save
**"Maya has a peanut allergy"** with entity `person:Maya` and attribute
`allergy:peanut`. The shared entity is Maya. Backfill raises the earlier reaction
because the new health fact makes it relevant. It makes no claim that Bangkok
Garden serves peanuts, and it leaves a different person's experience at the
same restaurant unchanged.

When an agent plans Maya's birthday dinner, it resolves Maya to the exact handle
`person:Maya` and calls `recall` with that handle. Recall orders the connected
memories using the updated salience, so Maya's reaction can be considered before
choosing a venue. Natural-language text is not silently turned into a fuzzy
memory query.

Backfill runs automatically when a new memory is saved. Set
`VESTIGE_BACKFILL_AUTOFIRE=0` (`false`, `off`, and `no` also work) to disable
automatic runs. Manual review and exact undo remain available:

```bash
vestige backfill <trigger-memory-id> --dry-run
vestige backfill <trigger-memory-id>
vestige backfill undo <receipt-id>
```

The MCP tool accepts `backfill` with `trigger_id`, `dry_run`, and an `action`
(`run`, `receipts`, or `undo`). Each applied change records the earlier memory,
old and new scores, triggering memory, exact shared entity, reason, and time.
Dry runs write nothing. A repeated cause/trigger pair does not compound; undo
restores the score recorded before that change. Raises retain the original
bounded stability step, `min(stability * 1.5, stability + 365)`, with a matching
bounded decrease and floor for lowering.

This section describes the v5.0.1 source candidate. The release remains
unpublished until its checks, target builds, and owner review are complete.
See the [release preparation guide](docs/releases/v5.0.1.md).

## Quick start

1. Install the **v5.0.1 binaries** for your platform from [GitHub Releases](https://github.com/samvallad33/vestige/releases) and put `vestige-mcp` on your PATH. The initializer checks the existing binary and gives these steps without changing anything when it is missing or too old.
2. Run `npx @vestige/init`. Choose **Memory only** (default) or **Memory plus guard**, inspect the file preview, then confirm once. Existing configurations are backed up and hooks are merged alongside your own. `--yes` selects memory only without prompts; `--guard --yes` requests the guard as well.
3. Restart your clients and trust the new hook definitions where the client requires it. Run `npx @vestige/init doctor`, then try: **Tell me something about a friend, then ask me about them later.**

The v5.0.1 initializer installs session, prompt, turn-save, pre-compaction and
session-end memory hooks for Claude Code, Codex and Gemini CLI. Cursor has
session context and turn capture; its prompt hook cannot inject context, so
recall context arrives through its supported post-tool hook. Other detected
MCP clients receive a connection and a clear coverage note. Configured hooks
still require the client's trust/reload step; a file on disk does not prove
that a client has run it.

On prompt submission, editable literal trigger phrases and exact recorded
entity surfaces select a recall packet; its ordering uses current salience,
never text similarity. With no trigger the packet contains the top three
memories. Hooks save supported explicit user declarations verbatim through
`smart_ingest`; richer facts are extracted by the connected agent using the
same tool and explicit metadata. Backfill therefore runs on each admitted
save. Memory hooks have a bounded deadline, fail open and log errors locally.
Optional guard hooks fail closed. See the [installer guide](packages/vestige-init/README.md)
for the full client coverage, receipt paths, doctor, status and uninstall.

This is the unpublished v5.0.1 candidate. Publication and target builds remain
subject to the [release gates](docs/releases/v5.0.1.md).

### Operator Lite on its own

Operator Lite is the free gate in [`operator-lite/`](operator-lite/). It is one file, stdlib only, and it sits on a PreToolUse hook (Claude Code, OpenClaw, or any host with a documented command-hook port).

macOS and Linux:

```bash
curl -fsSL https://raw.githubusercontent.com/samvallad33/vestige/main/operator-lite/operator-gate.py -o /tmp/operator-gate.py && python3 /tmp/operator-gate.py install
```

Windows, in PowerShell, with Python 3.9 or newer:

```powershell
curl.exe -fsSL https://raw.githubusercontent.com/samvallad33/vestige/main/operator-lite/operator-gate.py -o "$env:TEMP\operator-gate.py"; python "$env:TEMP\operator-gate.py" install
```

OpenClaw:

```bash
clawhub install vestige-operator-lite
```

Install copies the gate to `~/.operator/gate`, registers the Claude Code hook, and starts in enforce mode: from your agent's next tool call, every command a STOP rule matches is stopped before it runs. It then replays your last 30 days of Claude Code history through the same rules. Nothing in that history is executed. An existing `~/.operator/mode` file is kept, except the `shadow` every install before 0.3.9 wrote, which becomes `enforce` once; `mode shadow` records without blocking, and `mode enforce` blocks again.

## Proof

`replay` prints a scoreboard. From a made-up history:

```
operator-gate replay: the last 30 days on this machine. Nothing was executed.

       22  tool calls your agents made (2 Claude Code sessions, 2 projects)
        4  a built-in rule would have stopped
        1  flagged in shadow: recorded, not stopped
       13  no built-in rule decides: only you can

Would have been stopped (all of them):
  Oct 16  shop-api           OP-004 force push to a shared branch
                             git push --force origin main
  Oct 15  shop-api           OP-007 destructive SQL
                             psql $DATABASE_URL -c 'DROP TABLE sessions'
  Oct 10  infra              OP-003 recursive delete of ~/Documents/old-terraform-state
                             rm -rf ~/Documents/old-terraform-state
  Oct 10  infra              OP-006 terraform apply
                             terraform apply -auto-approve

Flagged in shadow, recorded and not stopped:
      1  OP-S01 work-loss                   git branch -D spike-cache
```

- 30 deterministic rules. 15 STOP rules block: workspace armor, memory-store protection, recursive deletes outside the project, force-push and `.git` removal, unreviewed publishes, production deploys and infrastructure teardown, destructive SQL and framework database resets, shell-init poisoning, reverse shells, cloud-metadata endpoints, discarded uncommitted git work, shredded or emptied files, raw-disk writes and recursive permission changes on `/` or home, and a download piped into a shell. 15 SHADOW rules record without blocking: outbound messages, public mutations, secret-shaped writes, persistence, env hijacks, exfil shapes and MCP argument exfil.
- Shell obfuscation: quote reassembly (`r''m`), `$IFS`, `$(echo rm)` as the program, ANSI-C `$'\x72m'`, base64-decoded pipelines, brace and glob expansion against the live filesystem, subshell time-bombs, session variables, `cd` tracking, heredocs, and fork bombs.

Operator Lite receipts are hash-chained digests, not signatures. It only blocks what is routed through hooked tools.

Every claim above has a command that checks it on your own machine, with what it prints: [check every claim yourself](operator-lite/README.md#check-every-claim-yourself). `npx operator-lite test` replays the bypass corpus with the gate the package carries.

Any agent connected to the memory server can read that record with `precedent`: what the gate stopped or allowed, under which rule, and which permit answered each stop. It writes nothing and cannot issue a permit.

## Causal root cause

`causal_walk` walks backward only over recorded edges: commits, tool calls, and memory writes. It does not use embeddings or keyword matching. With no start point it returns `needs_report` and names what is missing; a walk that finds no cause says why in `emptyBecause`, from the edges the log holds.

`gather` starts that walk from a GitHub issue. It records the issue, its comments, the pull requests that cross-reference it and, given a local checkout, its commit window as receipts in one scope, then runs `causal_walk` from the issue. It reads GitHub through the `gh` CLI. When something was missing, `missingReceipts` names it instead of guessing a cause.

<a id="install"></a>
## Memory server

The memory server keeps everything in Strata, an append-only log. Each frame carries the BLAKE3 hash of the frame before it, sealed segments carry an Ed25519 signature (a segment is sealed at backup, nightly by default), and `vestige strata-verify` checks all of it while the server keeps running. Every write passes a gate and returns a receipt. Install the latest release with one command:

```bash
curl -fsSL https://raw.githubusercontent.com/samvallad33/vestige/main/scripts/install.sh | sh
```

The installer selects the right macOS or Linux archive, verifies its SHA-256 checksum, and installs all four Vestige binaries in `~/.local/bin`. Archive names, PATH, flags, and `vestige.toml` are in the [reference](docs/REFERENCE.md#install).

### Guided terminal setup

After the install, run this optional wizard to detect compatible local clients,
choose the ones you want, preserve a backup before it changes a configuration,
and configure the Vestige memory server:

```bash
curl -fsSL https://raw.githubusercontent.com/samvallad33/vestige/main/scripts/setup.sh | sh
```

The wizard is cached locally in `~/.local/share/vestige/setup/` (or your
`XDG_DATA_HOME` equivalent). Its **Proof Pulse** makes the boundary explicit: it shows what it
detected, what it backed up, and what it configured. `CONFIGURED` means a local
client configuration was written; restart the client and confirm its MCP tools
for live verification. It can optionally open Vestige's local Observatory after
setup—no cloud account, telemetry, or fake first-run data.

It configures memory-server connections only. Operator Lite, the execution
gate, is a separate install below.

To rehearse the interactive Proof Pulse without touching a real client config,
run `./scripts/proof-pulse-demo.sh` from a source checkout. The script itself
lives in this repository; its disposable fixture is removed when the rehearsal
ends.

For automation and terminal power users, preview without writes or choose an
explicit target:

```bash
# Preview detected clients and proposed paths; writes nothing.
curl -fsSL https://raw.githubusercontent.com/samvallad33/vestige/main/scripts/setup.sh | sh -s -- --dry-run

# Configure one detected client without a menu.
curl -fsSL https://raw.githubusercontent.com/samvallad33/vestige/main/scripts/setup.sh | sh -s -- --client opencode

# Configure all detected clients only with explicit confirmation.
curl -fsSL https://raw.githubusercontent.com/samvallad33/vestige/main/scripts/setup.sh | sh -s -- --all --yes
```

```bash
claude mcp add vestige vestige-mcp -s user
codex mcp add vestige -- vestige-mcp
```

## Choose your environment

The installer gives you the local binaries. `npx @vestige/init` detects and wires
Claude Code, Claude Desktop, Cursor, VS Code with Copilot, OpenCode, Xcode,
JetBrains and Windsurf for you, or choose one of the guides below to
connect the **Vestige memory server** to the client you use. MCP connection
makes Strata's durable records and receipts available; it does not itself add
an execution gate.

| Environment | Memory-server setup |
|---|---|
| Claude Code | Run `claude mcp add vestige vestige-mcp -s user` above. |
| Codex | [Codex guide](docs/integrations/codex.md) |
| Cursor | [Cursor guide](docs/integrations/cursor.md) |
| VS Code + GitHub Copilot | [VS Code guide](docs/integrations/vscode.md) |
| OpenCode | [OpenCode guide](docs/integrations/opencode.md) |
| Windsurf | [Windsurf guide](docs/integrations/windsurf.md) |
| Xcode | [Xcode guide](docs/integrations/xcode.md) |
| JetBrains / Junie | [JetBrains guide](docs/integrations/jetbrains.md) |
| Another stdio MCP client | Use the [MCP configuration pattern](docs/AGENT-MEMORY-PROTOCOL.md). |

### Add an execution boundary

Operator Lite is a separate, local pre-tool gate. Every supported port installs
in **enforce mode**: it stops what its STOP rules match and writes a hash-chained
receipt for every decision. `mode shadow` records without blocking.

| Environment | Operator Lite setup |
|---|---|
| Claude Code | [One-command install](operator-lite/README.md#install-one-command) |
| OpenClaw | `clawhub install vestige-operator-lite` |
| Cursor | [Cursor port](operator-lite/ports/cursor/README.md) |
| OpenCode | [OpenCode port](operator-lite/ports/opencode/README.md) |
| Gemini CLI | [Gemini CLI port](operator-lite/ports/gemini-cli/README.md) |
| Windsurf | [Windsurf port](operator-lite/ports/windsurf/README.md) |
| Cline | [Cline port](operator-lite/ports/cline/README.md) |
| Frameworks and additional hosts | [All documented ports](operator-lite/ports/README.md) |

The Codex guide above covers MCP memory. Its execution-hook wiring is not yet
listed here because it needs a separately verified local hook setup before it
can be described as ready.

## Docs

[Getting Started](docs/GETTING-STARTED.md) · [Tool contracts](docs/TOOL-CONTRACTS.md) · [Configuration](docs/CONFIGURATION.md) · [Storage](docs/STORAGE.md) · [Upgrading from v3](docs/REFERENCE.md#upgrading-from-v3) · [Changelog](CHANGELOG.md) · [operator-lite/README.md](operator-lite/README.md) · [Reference](docs/REFERENCE.md)

<a id="vestige-pro"></a>
## License

AGPL-3.0-only.
