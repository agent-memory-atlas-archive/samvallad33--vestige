# Operator Lite for Cline

A deterministic pre-tool gate for [Cline](https://cline.bot). Every tool call
Cline is about to make — shell commands, file writes, MCP calls — is classified
against a closed rule set **before it runs**, including calls that would be
auto-approved. Intelligence does not equal authority.

Part of [Vestige](https://github.com/samvallad33/vestige) (operator-lite):
one stdlib-only Python file (`operator-gate.py`), analyzer-only — it never
executes what it inspects. This port adds the Cline adapter: no gate changes,
no npm dependencies, node builtins only.

## How it works (verified against Cline source, not just docs)

Cline v3.36+ ships **file-based hooks**. A script named `PreToolUse`, placed in
`~/.cline/hooks/` (or `~/Documents/Cline/Hooks/`, or `<workspace>/.cline/hooks/`),
is run as a subprocess before every tool call with JSON on stdin
(`tool_call: {name, input}`). The script replies on stdout with a control JSON:
`{}` allows; `{"cancel": true, "errorMessage": "..."}` cancels the call and
surfaces the message as the stop reason.

Two verified properties make this a real gate rather than a suggestion:

1. **Hooks run before tool policies and before user approval**
   (`sdk/packages/agents/src/agent-runtime.ts`: the `beforeTool` hook loop
   precedes `requestToolApproval`). Auto-approve / YOLO mode does not bypass
   the gate — it is exactly the configuration this port protects.
2. **A cancel stops the run with the reason**
   (`cancel: true` → `{stop: true, reason}` → `ControlledStopError`): the
   command never executes, and the gate's explanation ("Why", "Compliant
   path", receipt id) is what surfaces.

Sources: [`storage/paths.ts`](https://github.com/cline/cline/blob/main/sdk/packages/shared/src/storage/paths.ts),
[`hook-file-config.ts`](https://github.com/cline/cline/blob/main/sdk/packages/core/src/hooks/hook-file-config.ts),
[`hook-file-hooks.ts`](https://github.com/cline/cline/blob/main/sdk/packages/core/src/hooks/hook-file-hooks.ts),
[`subprocess.ts`](https://github.com/cline/cline/blob/main/sdk/packages/core/src/hooks/subprocess.ts),
[`hooks-adapter.ts`](https://github.com/cline/cline/blob/main/apps/vscode/src/sdk/hooks-adapter.ts),
plus the [v3.36 hooks announcement](https://cline.bot/blog/cline-v3-36-hooks) and
[writing-plugins guide](https://docs.cline.bot/sdk/guides/writing-plugins).

## What it stops

15 STOP rules (block) + 15 SHADOW rules (log-only until promoted), evaluated by
a shell walker that follows `cd`, expands variables, reads heredocs, and
simulates subshells before matching. It sees through the GuardFall obfuscation
classes: `$IFS` splitting, ANSI-C quoting, quote reassembly (`r''m -rf`),
command substitution as the program name, `base64 -d | sh`, brace/glob
expansion, fork bombs.

| Rule | Catches |
|---|---|
| OP-000 | edits to the gate itself, hook registrations, agent settings files |
| OP-001 | delete/move of a workspace root (or a parent of one) |
| OP-002 | wiping the Vestige store |
| OP-003 | recursive deletes outside the project and outside scratch/build folders, sweeps from `/` |
| OP-004 | force-push to shared branches, remote-main deletion, deleting `.git` or a whole repository |
| OP-005 | unreviewed publishes (npm/pnpm/yarn/cargo/twine/docker push/gh release), deleting a public repo |
| OP-006 | production deploys, infrastructure apply and teardown, live billing |
| OP-007 | destructive SQL (DROP/TRUNCATE/unscoped DELETE), framework database resets |
| OP-008 | shell-init writes (`>> ~/.zshrc` = code execution by install) |
| OP-009 | reverse shells (`/dev/tcp`, `nc -e`, DNS tunnels) |
| OP-010 | cloud metadata endpoints (169.254.169.254 et al.) |

| OP-011 | discarding uncommitted git work, shredding or emptying files outside the project, `crontab -r` |
| OP-014 | raw-disk writes, formatting or erasing a volume, fork bombs, recursive `chmod`/`chown` on `/` or home |
| OP-018 | a download piped or handed straight to a shell or interpreter (`curl X \| sh`, `bash <(curl X)`) |
| OP-CANARY | the verification canary string |

SHADOW rules log work-loss git patterns, outbound comms, public mutations,
secret-shaped writes, pipe-to-interpreter, persistence (cron/launchd),
env hijacks, exfil shapes, invisible characters, agent-config writes, and
sandbox-escape primitives — without blocking. Every verdict (allow, block,
shadow) gets a hash-chained receipt in `~/.operator/receipts/<date>.jsonl`;
`operator-gate verify` walks the chain. Receipts are digest chains, not
signatures — they detect accidental edits, nothing more.

## Install

From a checkout of the repo:

```sh
sh operator-lite/ports/cline/install.sh
```

Or without a checkout (downloads the gate and the hook itself):

```sh
curl -fsSL https://raw.githubusercontent.com/samvallad33/vestige/main/operator-lite/ports/cline/install.sh | sh
```

Then one manual step the installer cannot do for you: in the Cline VS Code
extension, open **Settings → Features** and enable **Hooks**, and start a new
task. That's it — 2 commands + 1 toggle.

The gate is in place before the hook is: install.sh puts the gate down first,
then the hook, so the hook never runs without a gate behind it.

## Enforce and shadow

The gate starts in **enforce** mode: it stops what a STOP rule matches and
receipts every call. An existing `~/.operator/mode` file is kept, except the `shadow` every install before 0.3.9 wrote, which becomes `enforce` once; a shadow you choose with `operator-gate mode shadow` stays.

```sh
echo shadow  > ~/.operator/mode     # log-only: nothing is blocked
echo enforce > ~/.operator/mode     # blocking again
```

The canary proves wiring end-to-end: with hooks enabled, ask Cline to run
`echo OPERATOR_CANARY_STOP_7f3a` — in enforce mode the call is cancelled
(OP-CANARY) with a receipt; in shadow mode it is receipted only.

## Files

| File | Role |
|---|---|
| `PreToolUse` | the hook script (node builtins only); reads Cline's stdin payload, normalizes Cline tool names, runs the gate, translates exit 2 into `cancel: true` |
| `operator-gate.plugin.mjs` | the same gate as an SDK/CLI plugin (`cline plugin install <file>`) using `beforeTool` → `{skip: true, reason}`; for headless Cline users |
| `install.sh` | gate install from the checkout, else a download (never overwrites), enforce mode when no mode file exists, hook install (backs up existing), optional CLI plugin install |
| `test.sh` | gate-contract test with Cline-shaped payloads; prints exit codes and the cancel verdicts |

Cline tool names are normalized adapter-side, so the gate file stays untouched:
`write_to_file → write_file`, `apply_diff` / `replace_in_file` /
`search_and_replace` / `insert_content → edit`, `use_mcp_tool →
mcp__use_mcp_tool` (unlocks the gate's MCP-argument secret checks). Shell calls
(`run_commands`, input shapes: string, array, `{command|commands|cmd}`) pass
through with the command intact. Tools with no gate rules (reads, listings,
questions) are allowed without spawning Python.

## Honest limitations

- **In-session behavior is untested on the build machine.** The adapter and
  the gate contract are tested (`sh test.sh`, all cases pass: plain
  `rm -rf`, quote-obfuscated `r''m -rf`, benign `ls`); firing inside a live
  Cline session is not — no Cline build was available. The protocol above is
  verified at source level, but expect to be the first to run it live.
- **`cline plugin install` route is documented-but-untested here** (no `cline`
  CLI locally). The command syntax is from the official docs and the
  `env-blocker` example; the file-based hook is the primary, verified route.
- **Hooks are a Cline feature you can switch off** (Settings → Features).
  That switch is the owner's to flip, like uninstalling the gate — but it is a
  single toggle, not a protected path. The gate's own OP-000 rule blocks an
  *agent* from editing its registration files.
- **macOS/Linux only** — Cline's hook feature does not support Windows yet.
- **Closed rule set.** The gate blocks what its rules recognize, routed
  through hooked tools. Shell obfuscation it cannot parse is a residual gap;
  actions outside hooked tools (you running commands yourself) are out of
  scope by design.
- **Node + python3 required** on the host. No other dependencies.
- Both routes at once (file hook + installed plugin) would run the gate twice
  per call in hosts that load both; pick one per host.

## Gate home

Everything lives under `~/.operator/`: `gate/operator-gate.py`, `mode`,
`receipts/`, `permits/`. One gate, shared across hosts (Claude Code, OpenClaw,
Cline and every other port) — one receipt chain, one set of owner rules in
`~/.operator/commitments.json`: `protected_roots` and `protected_files` add
paths, `mode_overrides` sets a rule to `enforce` or `shadow`, and
`disabled_rules` turns a rule off. Only the owner edits that file; OP-000
stops an agent from writing anything under `~/.operator/`.
