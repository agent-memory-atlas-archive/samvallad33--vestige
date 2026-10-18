# Operator Lite ports — one gate, every host

The gate is one stdlib-only Python file: `../operator-gate.py`. It is
host-agnostic by contract:

- reads a JSON payload on stdin: `{"tool_name", "tool_input": {"command"}, "cwd", "session_id"?}`
- exit 0 = allow (silent), exit 2 = block (the reason is on stderr, and the
  host surfaces it to the model, which must change course)
- writes its own hash-chained receipts under `~/.operator/receipts/`
- enforce vs shadow is a file: `~/.operator/mode`. A missing file means enforce, and
  every installer writes `enforce` when there is no mode file yet, and switches the
  `shadow` an install before 0.3.9 wrote to `enforce`, once; `shadow` records without
  blocking

Every port in this directory is a thin adapter that carries that contract to
a different agent host. No port contains rule logic. No port re-implements
analysis. One gate, verified against the 46-case GuardFall bypass
corpus, everywhere.

## The matrix

| Host | Mechanism | Port | Status |
|---|---|---|---|
| OpenClaw | `before_tool_call` plugin | [../openclaw-plugin/](../openclaw-plugin/) (ClawHub: `clawhub install vestige-operator-lite`) | published, inspector PASS |
| Claude Code | `PreToolUse` hook | [../README.md](../README.md#install-one-command) | shipped |
| OpenHands | `.openhands/hooks.json` PreToolUse | [openhands/](openhands/) | gate-tested, suite 23/23 |
| OpenCode | `.opencode/plugins/` `tool.execute.before` | [opencode/](opencode/) | gate-tested, suite 11/11 incl. corpus 46/46 via adapter |
| Cline | file hooks `~/.cline/hooks/PreToolUse` (fires before approval policies — covers YOLO) + SDK plugin | [cline/](cline/) | gate-tested, 3 hook cases + fresh-install check, receipt chain OK |
| Goose | lifecycle hooks `PreToolUse` (exit-2 block, goose v1.41.0+) | [goose/](goose/) | gate-tested, 29/29 |
| Crush | `hooks.PreToolUse[]` in crush.json — flat `{name, matcher?, command, timeout}` entries; payload is the gate's contract verbatim (exit-2 block, stderr reason) | [crush/](crush/) | gate-tested, suite 42/42 |
| Gemini CLI | `BeforeTool` hook in `~/.gemini/settings.json` (exit-2 deny) | [gemini-cli/](gemini-cli/) | gate-tested, suite 27/27 |
| Amazon Q CLI | `preToolUse` hook in `~/.aws/amazonq/cli-agents/q_cli_default.json` (exit-2 block, stderr to model) | [amazon-q/](amazon-q/) | gate-tested, suite 34/34 incl. corpus 46/46 via adapter |
| Cursor | agent hooks in `~/.cursor/hooks.json` — `beforeShellExecution` + `beforeMCPExecution` + `preToolUse` (disjoint matchers); deny is stdout JSON `{"permission":"deny"}` at exit 0 (invalid output blocks too); `failClosed: true` shipped | [cursor/](cursor/) | gate-tested, suite 47/47 |
| Windsurf (Cascade) | `pre_run_command` / `pre_write_code` / `pre_mcp_tool_use` hooks in `~/.codeium/windsurf/hooks.json` (workspace `.windsurf/hooks.json` / `.devin/hooks.json` optional) — payload reshaped by a shim (`tool_info.command_line` → `tool_input.command`; exit-2 block, stderr reason) | [windsurf/](windsurf/) | gate-tested, suite 57/57 incl. corpus 46/46 via adapter |
| CrewAI | execution hooks — global `register_hook(PRE_TOOL_CALL, fn)`; block is `HookAborted(reason, source)` (special-cased; other hook exceptions fail open), LLM sees CrewAI's generic blocked message | [crewai/](crewai/) | gate-tested, suite 43/43; the real crewai dispatch runs when `OPERATOR_CREWAI_PYTHON` names a Python with crewai |
| OpenAI Agents SDK | tool input guardrails (`reject_content`) | [openai-agents/](openai-agents/) | gate-tested, suite 30/30; the real SDK wrap runs when `openai-agents` is importable |
| LangChain / LangGraph (Python) | `@wrap_tool_call` / `AgentMiddleware` middleware for LangChain v1 `create_agent` (block = returned `ToolMessage`, handler never runs); hand-rolled LangGraph `ToolNode` via `decide()` + `ToolException` in the tool | [langchain/](langchain/) | gate-tested, suite 43/43 incl. corpus 46/46 via adapter; the real middleware and a `create_agent` loop (scripted model) run when `langchain` is importable |
| Aider | no hook API — terminal wrapper | [aider/](aider/) | gate-tested, 6 wrapper cases |

Status values: `shipped` (tested end-to-end) · `gate-tested` (gate contract
verified locally; host wiring follows documented, unmodified upstream
behavior) · `experimental`.

Codex is intentionally absent from the execution-port table. Its
[MCP memory integration](../../docs/integrations/codex.md) is documented, but
the current Codex execution-hook wiring needs a separate local verification
before it can be described as a supported Operator Lite port.

## Install any port

```
sh ports/<host>/install.sh
```

Every install.sh installs the gate to `~/.operator/gate/` (never
overwrites a newer copy), wires the host hook, and writes `enforce` to
`~/.operator/mode` when there is no mode file yet. An existing mode file is
kept, except the `shadow` every install before 0.3.9 wrote, which becomes
`enforce` once (the gate's `mode --settle`); a shadow chosen after that stays. The gate comes from GitHub main, or from this checkout when its copy
is newer. cline, goose and opencode install the copy that ships with them
(the checkout's, or opencode's own identical copy) and download only when
there is none. To record without blocking: `echo shadow > ~/.operator/mode`.

## The rule

Forks die; adapters compound. The platforms upstream control their release
cycles — we ride the hooks they already shipped. If a platform adds a
better interception point later, the port shrinks; the gate stays.
