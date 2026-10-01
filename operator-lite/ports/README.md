# Operator Lite ports — one gate, every host

The gate is one stdlib-only Python file: `../operator-gate.py`. It is
host-agnostic by contract:

- reads a JSON payload on stdin: `{"tool_name", "tool_input": {"command"}, "cwd", "session_id"?}`
- exit 0 = allow (silent), exit 2 = block (the reason is on stderr, and the
  host surfaces it to the model, which must change course)
- writes its own hash-chained receipts under `~/.operator/receipts/`
- shadow vs enforce is a file: `~/.operator/mode` (`shadow` default)

Every port in this directory is a thin adapter that carries that contract to
a different agent host. No port contains rule logic. No port re-implements
analysis. One gate, verified once against the 43-case GuardFall bypass
corpus, everywhere.

## The matrix

| Host | Mechanism | Port | Status |
|---|---|---|---|
| OpenClaw | `before_tool_call` plugin | [../../openclaw-plugin/](../../openclaw-plugin/) (ClawHub: `clawhub install vestige-operator-lite`) | published, inspector PASS |
| Claude Code | `PreToolUse` hook | [../README.md](../README.md#install-3-hosts-one-script) | shipped |
| Codex | hooks.json | [../README.md](../README.md#install-3-hosts-one-script) | shipped |
| OpenHands | `.openhands/hooks.json` PreToolUse | [openhands/](openhands/) | gate-tested, suite PASS |
| OpenCode | `.opencode/plugins/` `tool.execute.before` | [opencode/](opencode/) | gate-tested, corpus 43/43 via adapter, boots in opencode CLI |
| Cline | file hooks `~/.cline/hooks/PreToolUse` (fires before approval policies — covers YOLO) + SDK plugin | [cline/](cline/) | gate-tested, suite PASS, receipt chain OK |
| Goose | lifecycle hooks `PreToolUse` (exit-2 block, goose v1.41.0+) | [goose/](goose/) | gate-tested, 29/29 |
| Crush | `hooks.PreToolUse[]` in crush.json — flat `{name, matcher?, command, timeout}` entries; payload is the gate's contract verbatim (exit-2 block, stderr reason) | [crush/](crush/) | gate-tested, suite 40/40 |
| Gemini CLI | `BeforeTool` hook in `~/.gemini/settings.json` (exit-2 deny) | [gemini-cli/](gemini-cli/) | gate-tested, suite PASS, corpus 43/43 |
| Aider | no hook API — terminal wrapper | [aider/](aider/) | see port README |

Status values: `shipped` (tested end-to-end) · `gate-tested` (gate contract
verified locally; host wiring follows documented, unmodified upstream
behavior) · `experimental`.

## Install any port

```
sh ports/<host>/install.sh
```

Every install.sh: installs the gate to `~/.operator/gate/` (never
overwrites a newer copy), wires the host hook, sets shadow mode, prints the
flip command: `echo enforce > ~/.operator/mode`.

## The rule

Forks die; adapters compound. The platforms upstream control their release
cycles — we ride the hooks they already shipped. If a platform adds a
better interception point later, the port shrinks; the gate stays.
