# @vestige/init

**One command installs Vestige on your machine:** memory your AI agents can check, rules that come back after every compaction, and a gate that stops destructive commands before they run.

```bash
npx @vestige/init
```

Run it in your own terminal. In order, it:

1. **Installs the binaries.** `vestige-mcp`, `vestige`, `vestige-restore` and `vestige-upgrade` come from npm for your machine (`@vestige/mcp-<os>-<arch>`) and go into `~/.local/bin`. A version that is already installed and as new is kept.
2. **Opens your Strata memory.** It starts the memory server once, the way a client would, which creates the store, and checks that it answers with its 18 tools. Strata is an append-only, hash-chained log: every write passes a gate and leaves a receipt, and a memory is found by its exact id or tag, never by vector similarity.
3. **Connects your clients.** Claude Code, Claude Desktop, Cursor, VS Code, OpenCode, Xcode, JetBrains and Windsurf, whichever it finds. Each file is backed up before it changes.
4. **Adds the rules hook.** In Claude Code, `vestige hook session-start` hands every new session, and every session after a compaction, the decisions and corrections you tagged `pinned`, each with its memory id. It only reads, and it prints nothing when there is nothing to say.
5. **Installs Operator Lite,** the free gate, in enforce mode, and wires every agent host it finds. From your agent's next tool call, every command a STOP rule matches is stopped before it runs, with a hash-chained receipt for every decision.
6. **Checks the gate.** The gate replays the 46-case GuardFall bypass corpus it carries. It classifies every command and runs none.

Then it prints what it configured, and what to verify once you restart your clients.

## Follow one real stop around the loop

```bash
npx @vestige/init precedent
```

One stop from your own machine: what the agent tried and the gate stopping it, its receipt re-derived from its own bytes, the stop as a Vestige memory, and the precedent the next agent reads.

## Options

| Flag | What it does |
|---|---|
| `--dry-run` | Show every change; write nothing |
| `--yes` | Connect every detected client without a prompt |
| `--client <name>` | Connect one client (`claude-code`, `cursor`, `vscode`, `opencode`, `xcode`, `jetbrains`, `windsurf`) |
| `--no-gate` | Skip Operator Lite |
| `--no-hooks` | Skip the Claude Code rules hook |
| `--json` | Print a machine-readable result |

## What it never does

- It never downgrades. A newer binary stays where it is, and the gate's own installer never replaces a newer gate or the paid Vestige Operator.
- It never installs the gate from an AI agent's shell. Operator Lite's installer refuses there, so an agent cannot put a gate of its choosing in front of itself.
- It never sends anything anywhere: no cloud account, no telemetry. Your records stay on your machine.

Operator Lite needs Python 3.9 or newer. Tested end to end on macOS and Linux; the Windows x64 binaries ship in the same package, but that path has not been tested end to end yet.

Docs: https://github.com/samvallad33/vestige
