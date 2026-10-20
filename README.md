<p align="center">
  <img src="https://raw.githubusercontent.com/samvallad33/vestige/media/vestige-logo.png" alt="Vestige" width="620">
</p>

# Vestige

Vestige gives AI agents a memory you can check and a guard that stops them before they do damage. Memory lives in Strata, an append-only log where every write passes a gate and leaves a receipt, and a memory is found by its exact id or tag, never by vector similarity. The guard runs outside the model on every tool call, stops destructive commands before they run, keeps a receipt of every decision, and lets a stopped command through only after the owner approves that exact command from their own terminal, so an old yes is never treated as permission to act now. When something breaks, `causal_walk` follows the recorded history back to the real cause.

[![Release](https://img.shields.io/github/v/release/samvallad33/vestige?color=06b6d4)](https://github.com/samvallad33/vestige/releases/latest)
[![Tests](https://img.shields.io/github/actions/workflow/status/samvallad33/vestige/ci.yml?branch=main&label=CI)](https://github.com/samvallad33/vestige/actions)
[![Binary](https://img.shields.io/badge/platforms-5_release_targets-informational)](https://github.com/samvallad33/vestige/releases/latest)
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

## Quick start

One command installs the whole system. Run it in your own terminal:

```bash
npx @vestige/init
```

In order, it:

1. installs the `vestige-mcp`, `vestige`, `vestige-restore` and `vestige-upgrade` binaries for your machine from npm, unless the same or a newer version is already installed;
2. starts `vestige-mcp` once, which creates your Strata store, and checks that it answers with its 18 tools;
3. connects the MCP clients it finds (Claude Code, Claude Desktop, Cursor, VS Code, OpenCode, Xcode, JetBrains, Windsurf), backing up each file before changing it;
4. adds the Claude Code rules hook, so the decisions and corrections you tag `pinned` come back at every session start and after every compaction;
5. installs Operator Lite, the free gate, in enforce mode, and wires every agent host it finds;
6. runs the gate's own bypass test, then lists what it configured and what to verify after a restart.

It never downgrades: a newer binary, or a newer or paid gate, stays where it is. `--dry-run` shows every change and writes nothing, `--yes` connects every detected client without a prompt, and `--no-gate` and `--no-hooks` skip those steps. Operator Lite needs Python 3.9 or newer, and it refuses to install from inside an AI agent's shell. Tested end to end on macOS and Linux; the Windows x64 binaries ship in the same package but that path has not been tested end to end yet.

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
