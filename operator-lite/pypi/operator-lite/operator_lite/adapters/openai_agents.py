"""OpenAI Agents SDK adapter (port of operator-lite/ports/openai-agents/
operator_guard.py onto operator_lite._core).

Tool input guardrails run BEFORE a function tool is invoked and are attached
at tool creation (verified against the SDK docs in the port, 2026-09):

    @function_tool(tool_input_guardrails=[operator_lite.input_guardrail()])

The deny form that leaves the run alive and shows the model the reason is
ToolGuardrailFunctionOutput.reject_content(message) -- "Operator Lite
blocked: <gate stderr>" replaces the tool result, so the model must change
course. Human approvals stay the separate, orthogonal SDK layer
(needs_approval=True).

The `agents` SDK is imported lazily inside the integration helpers; the
decision path (gate_decision) runs with the SDK not installed.
"""
import json
import os
import shlex
from typing import Any, NamedTuple, Optional

from .. import _core

SOURCE = "openai-agents"
DENY_PREFIX = "Operator Lite blocked: "

# Max raw argument string handed to the gate; the gate's preview/redaction
# paths cap far below this.
MAX_ARGUMENTS_CHARS = 65536


def extract_command(tool_arguments: Any) -> Optional[str]:
    """Pull the shell command out of a tool's arguments.

    Accepts the SDK's raw JSON-arguments string (ToolContext.tool_arguments),
    an already-parsed dict, or a bare string. Recognises the gate's keys
    ("command", "cmd"). Returns None when there is nothing shell-like to
    judge -- that call is not gated.
    """
    args: Any = tool_arguments
    if isinstance(args, str):
        stripped = args.strip()
        if not stripped:
            return None
        try:
            args = json.loads(stripped)
        except Exception:
            return stripped[:MAX_ARGUMENTS_CHARS]   # raw text: let the gate judge
    if isinstance(args, str):
        stripped = args.strip()
        return stripped[:MAX_ARGUMENTS_CHARS] if stripped else None
    if isinstance(args, dict):
        for key in ("command", "cmd"):
            value = args.get(key)
            if isinstance(value, str) and value.strip():
                return value[:MAX_ARGUMENTS_CHARS]
            if isinstance(value, list):             # mirror the gate's list handling
                joined = " ".join(shlex.quote(str(c)) for c in value)
                return joined[:MAX_ARGUMENTS_CHARS] if joined.strip() else None
        return None
    if isinstance(args, list):
        joined = " ".join(shlex.quote(str(c)) for c in args)
        return joined[:MAX_ARGUMENTS_CHARS] if joined.strip() else None
    return None


class GateDecision(NamedTuple):
    """The gate's verdict on one command: "allow" (exit 0), "block" (exit 2,
    reason carries the gate's stderr), or "unreachable" (spawn error /
    timeout / non-0-2 exit). An "unreachable" verdict is already "block" when
    the command matched the destructive safety net, fail-open otherwise."""

    outcome: str
    reason: str
    exit_code: Optional[int] = None
    error: Optional[str] = None
    gate: Optional[str] = None

    @property
    def allowed(self) -> bool:
        return self.outcome == "allow"

    @property
    def blocked(self) -> bool:
        return self.outcome == "block"

    @property
    def reachable(self) -> bool:
        return self.outcome != "unreachable"


def deny_message(reason: str) -> str:
    """The exact string the model sees when the gate says no."""
    return DENY_PREFIX + (reason or "OPERATOR LITE: STOPPED").strip()


def gate_decision(tool_name: str,
                  command: str,
                  cwd: Optional[str] = None,
                  session_id: Optional[str] = None,
                  gate: Optional[str] = None) -> GateDecision:
    """Judge one command through the gate. Payload is the gate's stdin
    contract verbatim. Maps exit 0 -> allow, 2 -> block, anything else ->
    unreachable + the ports' fail-open/fail-closed safety net."""
    if not command or not command.strip():
        return GateDecision("allow", "no command to judge",
                            gate=gate or _core.resolve_gate())
    code, reason, error = _core.spawn_gate(command, tool_name, cwd,
                                           session_id, SOURCE, gate=gate)
    resolved = gate or _core.resolve_gate()
    if error is None and code == 0:
        return GateDecision("allow", "", exit_code=0, gate=resolved)
    if error is None and code == 2:
        return GateDecision("block", reason, exit_code=2, gate=resolved)
    # unreachable: spawn error, timeout, or an exit the gate never emits
    if _core.DESTRUCTIVE_LIKE.search(command):
        return GateDecision(
            "block",
            ("gate unreachable (%s) and the command matches the destructive "
             "safety net; ask the owner." % (error or "gate exit %s" % code)),
            exit_code=code, error=error, gate=resolved)
    return GateDecision("unreachable", "", exit_code=code, error=error,
                        gate=resolved)


def describe() -> dict:
    """Install/ops snapshot: gate path, mode, SDK presence."""
    gate = _core.resolve_gate()
    mode = ""
    try:
        with open(os.path.join(_core.operator_home(), "mode"),
                  encoding="utf-8") as f:
            mode = f.read().strip()
    except OSError:
        pass
    return {
        "gate": gate,
        "gate_present": os.path.exists(gate),
        "mode": mode or "(no mode file -- the gate treats this as enforce; "
                        "ensure_gate/install writes 'shadow')",
        "agents_sdk": agents_sdk_available(),
    }


# ---------------------------------------------------------------------------
# OpenAI Agents SDK integration (lazy: `agents` is imported only here)
# ---------------------------------------------------------------------------
def agents_sdk_available() -> bool:
    try:
        from agents import (function_tool, tool_input_guardrail,  # noqa: F401
                            ToolGuardrailFunctionOutput)  # noqa: F401
        return True
    except Exception:
        return False


def input_guardrail():
    """A ToolInputGuardrail wrapping the gate (requires the SDK).

    Attach it to any function tool:
        @function_tool(tool_input_guardrails=[operator_lite.input_guardrail()])
    """
    from agents import tool_input_guardrail
    return tool_input_guardrail(operator_tool_input_guardrail,
                                name="operator_lite")


def operator_tool_input_guardrail(data: Any) -> Any:
    """Tool-input-guardrail function: gate the arguments before the tool runs.
    Sync, so it works under both the sync and async run paths."""
    from agents import ToolGuardrailFunctionOutput
    ctx = getattr(data, "context", None)
    command = extract_command(getattr(ctx, "tool_arguments", None))
    if command is None:
        return ToolGuardrailFunctionOutput.allow()
    decision = gate_decision(
        tool_name=getattr(ctx, "tool_name", "") or "",
        command=command,
        cwd=os.getcwd(),
        # the per-call id lands in the receipt for traceability
        session_id=getattr(ctx, "tool_call_id", None),
    )
    if decision.blocked:
        # reject_content: the message replaces the tool result, the model
        # sees the reason, the run continues (the verified deny form).
        return ToolGuardrailFunctionOutput.reject_content(
            deny_message(decision.reason))
    return ToolGuardrailFunctionOutput.allow()


def gated_shell_tool(func=None, *,
                     name: Optional[str] = None,
                     description: Optional[str] = None,
                     needs_approval: Any = False,
                     extra_input_guardrails: Any = None):
    """Decorator/factory: a function tool whose every call passes the gate.

        @gated_shell_tool(name="run_shell")
        async def run_shell(command: str) -> str: ...

    needs_approval passes through to function_tool (True always pauses the
    run for a human decision). The gate is deterministic and runs first.
    """
    from agents import function_tool
    guard = input_guardrail()

    def deco(f):
        kwargs: dict = {"tool_input_guardrails":
                        [guard] + list(extra_input_guardrails or [])}
        if name:
            kwargs["name_override"] = name
        if description:
            kwargs["description_override"] = description
        if needs_approval is not False:
            kwargs["needs_approval"] = needs_approval
        return function_tool(f, **kwargs)

    return deco(func) if func is not None else deco


def wrap_tool(tool: Any) -> Any:
    """Gate ANY prebuilt FunctionTool in place, without rebuilding it: the
    gate sees the raw JSON arguments first; on block, the deny string
    becomes the tool result."""
    import dataclasses
    from agents import FunctionTool
    if not isinstance(tool, FunctionTool):
        raise TypeError("wrap_tool expects an agents.FunctionTool, got %r"
                        % type(tool).__name__)
    original = tool.on_invoke_tool

    async def gated_on_invoke(ctx, arguments: str):
        command = extract_command(arguments)
        if command is not None:
            decision = gate_decision(
                tool_name=getattr(ctx, "tool_name", None) or tool.name,
                command=command,
                cwd=os.getcwd(),
                session_id=getattr(ctx, "tool_call_id", None),
            )
            if decision.blocked:
                return deny_message(decision.reason)
        return await original(ctx, arguments)

    return dataclasses.replace(tool, on_invoke_tool=gated_on_invoke)
