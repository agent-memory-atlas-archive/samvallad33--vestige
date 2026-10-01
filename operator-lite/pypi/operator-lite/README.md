# operator-lite (PyPI)

One pip install that puts the **Operator Lite pre-tool gate** in front of the
shell tool calls of your Python agents:

```python
import operator_lite

# LangChain v1 / LangGraph (lazy import; needs `pip install operator-lite[langchain]`)
agent = create_agent(model, tools=[...],
                     middleware=[operator_lite.operator_middleware()])

# CrewAI (needs `pip install operator-lite[crewai]`)
operator_lite.crewai_install()          # registers a global PRE_TOOL_CALL hook

# OpenAI Agents SDK (needs `pip install operator-lite[openai-agents]`)
@function_tool(tool_input_guardrails=[operator_lite.input_guardrail()])
async def run_shell(command: str) -> str: ...
```

Every verdict is **deterministic**: a single stdlib-only gate file classifies
the command before it runs — exit 0 allow, exit 2 block with a reason the
model sees — and appends a hash-chained receipt. Shadow mode (the default
after install) logs every would-block without blocking; flip with
`echo enforce > ~/.operator/mode`.

**The gate is not in this wheel.** It is one auditable file, installed to
`~/.operator/gate/operator-gate.py`:

```python
import operator_lite
operator_lite.ensure_gate()   # downloads from upstream, never overwrites a newer copy
```

or via the repo: `sh operator-lite/ports/<host>/install.sh` (also wires
non-Python hosts). Resolution order at call time: `$OPERATOR_GATE` →
`$OPERATOR_HOME/gate` → `~/.operator/gate/operator-gate.py`. No rule logic
lives in this package — the gate owns every decision; these adapters are
payload shaping and decision plumbing only, and fail open when the gate is
unreachable (except plainly destructive commands, which fail closed).

Part of [Vestige](https://github.com/samvallad33/vestige). AGPL-3.0.

Smoke test (skips cleanly when no gate is installed):

```sh
python3 test_smoke.py        # or: pytest test_smoke.py
```
