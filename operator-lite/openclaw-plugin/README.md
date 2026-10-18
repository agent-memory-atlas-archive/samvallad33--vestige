# Vestige Operator Lite for OpenClaw

A deterministic pre-tool gate. Every shell-like tool call your OpenClaw agent
makes is classified against a closed rule set **before it runs**. The gate is
bundled in this package — no clone, no copy step. Intelligence does not equal
authority.

## What it does

Subscribes to `before_tool_call` and pipes each command through the Operator
Lite gate (one stdlib-only Python file, analyzer-only — it never executes what
it inspects):

- **Deterministic verdicts** — 15 STOP rules + 15 SHADOW rules evaluated by a
  shell walker that follows `cd`, expands shell variables, reads heredocs,
  and simulates subshells before matching.
- **GuardFall-proof** — sees through the obfuscation classes from CSA's
  GuardFall research: `$IFS` splitting, ANSI-C quoting, quote reassembly
  (`r''m -rf`), command substitution as the program name, `base64 -d | sh`,
  brace and glob expansion against the live filesystem, subshell time-bombs,
  fork bombs. 46/46 on the adapted bypass corpus.
- **Hash-chained receipts** — every verdict (allow, block, shadow) is
  appended to `~/.operator/receipts/<date>.jsonl` as a tamper-evident chain.
  `operator-gate verify` walks the chain.

## Enforce mode from the first call

The plugin writes no mode file, and the gate reads a missing `~/.operator/mode`
as enforce: every command a STOP rule matches is blocked before it runs, and
every call gets a receipt. To record without blocking:

```sh
echo shadow > ~/.operator/mode
```

Back to blocking: `echo enforce > ~/.operator/mode`.

## What the STOP rules catch

| Rule | Catches |
|---|---|
| OP-000 | edits to the gate itself, its mode, permits and rules, hook registrations, settings files |
| OP-001 | delete/move of a workspace root or a parent of one, including `/` and home |
| OP-002 | wiping the Vestige memory store |
| OP-003 | recursive deletes outside the project and outside scratch/build folders, sweeps from `/` |
| OP-004 | force-push to shared branches, remote-main deletion, deleting `.git` or a whole repository |
| OP-005 | unreviewed publishes (npm/pnpm/yarn/cargo/twine/docker push/gh release), deleting a public repo |
| OP-006 | production deploys, infrastructure apply and teardown (terraform, pulumi, cdk, kubectl delete, helm uninstall, cloud delete commands), live billing |
| OP-007 | destructive SQL (DROP/TRUNCATE/unscoped DELETE, FLUSHALL), framework database resets |
| OP-008 | shell-init writes (`>> ~/.zshrc` = code execution by install) |
| OP-009 | reverse shells (`/dev/tcp`, `nc -e`, DNS-tunnel tools, inline code that hands a socket a shell) |
| OP-010 | cloud metadata endpoints (169.254.169.254, metadata.google.internal) |

| OP-011 | loss nothing can undo: `git reset --hard`, `git clean -f`, `git checkout -- <path>`, `git stash drop` when they would discard real work, `shred`, emptying a file outside the project, `crontab -r` |
| OP-014 | machine wreck: `dd` onto a device, `mkfs`/`fdisk`-class tools, `diskutil erase*`, recursive `chmod`/`chown` on `/`, a system folder or home, fork bombs |
| OP-018 | a download run unread: `curl X \| sh`, `bash <(curl X)`, `sh -c "$(curl X)"` |
| OP-CANARY | the verification canary string, which proves the plugin is wired |

SHADOW rules log git commands that can lose work but did not here, comms,
public mutation, secret writes, other pipe-to-interpreter, persistence, env
hijacks, exfil shapes, invisible characters, and sandbox-escape primitives —
without blocking.

## Exit contract

| gate exit | plugin behavior |
|-----------|-----------------|
| 0 | allow — normal flow continues |
| 2 | block — the gate's reason is surfaced to the model, which must change course |
| spawn failure | fail-open, unless the raw text plainly looks destructive |

## Also works without OpenClaw

The same file is the PreToolUse hook for Claude Code and for every host in
[ports/](https://github.com/samvallad33/vestige/tree/main/operator-lite/ports) — see the
[Operator Lite README](https://github.com/samvallad33/vestige/tree/main/operator-lite).

Local-first. No telemetry. No dependencies beyond python3.
