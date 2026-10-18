<p align="center">
  <img src="https://raw.githubusercontent.com/samvallad33/vestige/media/vestige-logo.png" alt="Vestige" width="620">
</p>

# Vestige

Vestige is a fail-closed runtime firewall for AI agents. The model proposes, a deterministic gate decides, and every action leaves a receipt. When something breaks, `causal_walk` finds the real cause.

[![Release](https://img.shields.io/github/v/release/samvallad33/vestige?color=06b6d4)](https://github.com/samvallad33/vestige/releases/latest)
[![Tests](https://img.shields.io/github/actions/workflow/status/samvallad33/vestige/ci.yml?branch=main&label=CI)](https://github.com/samvallad33/vestige/actions)
[![Binary](https://img.shields.io/badge/platforms-5_release_targets-informational)](https://github.com/samvallad33/vestige/releases/latest)
[![License](https://img.shields.io/badge/license-AGPL--3.0-3b82f6)](LICENSE)

## The problem

Agents run `rm -rf`, force-push, drop tables, and overwrite `.env` on their own, and nothing stops them. When something breaks, similarity search finds lookalikes, not causes.

## Vestige Operator

[![Watch Vestige Operator stop what Operator Lite lets through](operator-lite/media/from-lite-to-operator.gif)](operator-lite/media/from-lite-to-operator.mp4)

Click the picture to watch the full film.

Operator Lite is free and stays free. Vestige Operator is the owner's version of the same gate: $149 once, and every later version is yours at no charge, with laws you write, a Board of today's stops, and a weekly Letter of what your agents tried and what stopped them. You download a small archive the moment you pay; from an installed Operator Lite, `upgrade --install <the archive you downloaded>` unpacks it and starts the wizard.

[Buy Vestige Operator](https://payhip.com/b/d4xvu)

## Quick start

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

Install copies the gate to `~/.operator/gate`, registers the Claude Code hook, and starts in shadow mode, which records every verdict and blocks nothing. It then replays your last 30 days of Claude Code history through the same rules. Nothing in that history is executed. When that looks right, switch it on with `mode enforce`.

## Proof

`replay` prints a scoreboard. From a made-up history:

```
operator-gate replay: the last 30 days on this machine. Nothing was executed.

       23  tool calls your agents made (2 Claude Code sessions, 2 projects)
        3  a built-in rule would have stopped
        1  flagged in shadow: recorded, not stopped
       14  no built-in rule decides: only you can

Would have been stopped (all of them):
  Mar 21  shop-api           OP-004 force push to a shared branch
                             git push --force origin main
  Mar 19  shop-api           OP-007 destructive SQL
                             psql $DATABASE_URL -c 'DROP TABLE sessions'
  Mar 14  infra              OP-003 recursive delete of ~/Documents/old-terraform-state
                             rm -rf ~/Documents/old-terraform-state

Flagged in shadow, recorded and not stopped:
      1  OP-S01 work-loss                   git reset --hard HEAD~1
```

- 27 deterministic rules: workspace armor, memory-store protection, destructive SQL, force-push, unreviewed publishes, paid deploys, reverse shells, cloud-metadata endpoints, shell-init poisoning, and MCP argument exfil.
- Shell obfuscation: quote reassembly (`r''m`), `$IFS`, `$(echo rm)` as the program, ANSI-C `$'\x72m'`, base64-decoded pipelines, brace and glob expansion against the live filesystem, subshell time-bombs, session variables, `cd` tracking, heredocs, and fork bombs.

Operator Lite receipts are hash-chained digests, not signatures. It only blocks what is routed through hooked tools.

Any agent connected to the memory server can read that record with `precedent`: what the gate stopped or allowed, under which rule, and which permit answered each stop. It writes nothing and cannot issue a permit.

## Causal root cause

`causal_walk` walks backward only over recorded edges: commits, tool calls, and memory writes. It does not use embeddings or keyword matching. With no start point it returns `needs_report` and names what is missing; a walk that finds no cause says why in `emptyBecause`, from the edges the log holds.

`gather` starts that walk from a GitHub issue. It records the issue, its comments, the pull requests that cross-reference it and, given a local checkout, its commit window as receipts in one scope, then runs `causal_walk` from the issue. It reads GitHub through the `gh` CLI. When something was missing, `missingReceipts` names it instead of guessing a cause.

<a id="install"></a>
## Memory server

The memory server is a Strata signed append-only log. Every write is gated and returns a receipt. Install the latest signed release with one command:

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

It configures memory-server connections only. Operator Lite execution gates
remain a separate, shadow-first choice below.

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

The installer gives you the local binaries. Choose one of the guides below to
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

Operator Lite is a separate, local pre-tool gate. Every supported port begins
in **shadow mode**: it writes a receipt for each decision without blocking.
Review the receipt chain, then intentionally switch to enforce mode.

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
