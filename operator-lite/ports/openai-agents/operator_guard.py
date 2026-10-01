#!/usr/bin/env python3
"""Operator Lite gate for the OpenAI Agents SDK (Python) -- tool guardrail.

One job: before a command-carrying function tool runs, spawn the host-agnostic
gate and let its closed rule set judge the command string.

    gate exit 0 -> allow  (the tool runs; shadow receipts written by the gate)
    gate exit 2 -> deny   ("Operator Lite blocked: <gate stderr>" replaces the
                           tool result, so the model sees the reason and must
                           change course)
    gate unreachable -> fail-open, EXCEPT when the command matches the
    destructive safety net (recursive deletes, force-pushes, DROP TABLE,
    mkfs, dd, and -- extended past the OpenClaw adapter -- shell-rc writes,
    which fail closed: planting a shell init file is code execution by
    install).

Verified against the OpenAI Agents SDK docs (fetched 2026-09-29):

  * Tool input guardrails run BEFORE a function tool is invoked and are
    attached at tool creation:
        @function_tool(tool_input_guardrails=[...])          (ref/decorators/)
    The guardrail function is sync-or-async, receives
    ToolInputGuardrailData, and reads the raw arguments off
    data.context.tool_arguments (a JSON string) and the tool name off
    data.context.tool_name (ToolContext fields, src/agents/tool_context.py).

  * The deny form that leaves the run alive and shows the model the reason:
        ToolGuardrailFunctionOutput.reject_content(message)
    "Message to send to the model instead of the tool result."
    (ref/tool_guardrails/). reject_content is used instead of
    .raise_exception() on purpose: it mirrors the gate's own contract -- the
    agent must change course -- and the OpenClaw adapter's {block, reason},
    rather than killing the whole run.

  * Human approvals are the separate, orthogonal layer:
        @function_tool(needs_approval=True)  -> the run pauses,
        result.interruptions (ToolApprovalItem),
        state = result.to_state(),
        state.approve(item) / state.reject(item, rejection_message=...),
        resume with Runner.run(agent, state)   (human_in_the_loop/).
    gated_shell_tool(..., needs_approval=True) passes that straight through.

This file contains no rule logic and no analysis -- it is payload shaping and
process plumbing only. The gate (operator-gate.py) owns every decision.

Gate resolution order (mirrors the OpenClaw adapter):
    1. $OPERATOR_GATE (explicit override)
    2. an operator-gate.py sibling of this file (repo checkout / bundled)
    3. ~/.operator/gate/operator-gate.py  (a previously installed copy)

Stdlib only at module level; `agents` is imported lazily inside the
integration helpers, so this module imports (and gate_decision() runs) with
the SDK not installed. Python 3.9 compatible.
"""
import json
import os
import re
import shlex
import subprocess
import sys
from typing import Any, NamedTuple, Optional

__version__ = "0.1.0"

DENY_PREFIX = "Operator Lite blocked: "

# Gate wall-clock budget (the OpenClaw adapter uses the same 8s).
GATE_TIMEOUT_S = float(os.environ.get("OPERATOR_GATE_TIMEOUT_S", "8"))

# Max raw argument string handed to the gate. Model output can be long; the
# gate's preview/redaction paths cap far below this.
MAX_ARGUMENTS_CHARS = 65536

# ---------------------------------------------------------------------------
# destructive safety net (gate unreachable -> fail closed on these)
# ---------------------------------------------------------------------------
# Core mirrored verbatim from the OpenClaw adapter's DESTRUCTIVE_LIKE
# (openclaw-plugin/index.js); comment tags mark the seam.
DESTRUCTIVE_LIKE = re.compile(
    # --- mirrored core (OpenClaw) ---
    r"\brm\s+-[a-zA-Z]*[rR]"      # recursive delete
    r"|\bpush\s+.*--force"        # history rewrite
    r"|\bDROP\s+TABLE"            # destructive SQL
    r"|vestige\.db"               # the memory store
    r"|fly\s+deploy"              # paid control-plane deploy
    r"|\bmkfs\b"
    r"|\bdd\s+if="
    # --- extension: shell-rc / init writes fail closed (OP-008's territory) ---
    r"|\b(?:tee|cp|mv|install|truncate|dd)\b[^|;&\n]*[/ ]\."
    r"(?:zshrc|zshenv|zprofile|bashrc|bash_profile|profile|ssh/rc)\b"
    r"|[;&|\s]>\s*~?[^|;&\n]*[/ ]?\."
    r"(?:zshrc|zshenv|zprofile|bashrc|bash_profile|profile|ssh/rc)\b",
    re.I,
)

SHELL_RC_NOTE = ("writing a shell init file plants commands that fire on "
                 "every future shell")


def looks_destructive(command: str) -> bool:
    """True when the safety net must fail closed even without a reachable gate."""
    return bool(DESTRUCTIVE_LIKE.search(command or ""))


# ---------------------------------------------------------------------------
# gate plumbing
# ---------------------------------------------------------------------------
def resolve_gate() -> str:
    """$OPERATOR_GATE -> sibling operator-gate.py -> ~/.operator/gate/..."""
    env = os.environ.get("OPERATOR_GATE")
    if env:
        return env
    sibling = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                           "operator-gate.py")
    if os.path.exists(sibling):
        return sibling
    return os.path.join(os.path.expanduser("~"), ".operator", "gate",
                        "operator-gate.py")


def extract_command(tool_arguments: Any) -> Optional[str]:
    """Pull the shell command out of a tool's arguments.

    Accepts the SDK's raw JSON-arguments string (ToolContext.tool_arguments),
    an already-parsed dict, or a bare string. Recognises the same keys as the
    gate itself ("command", "cmd"), plus string/list arguments. Returns None
    when there is nothing shell-like to judge -- that call is not gated.
    """
    args: Any = tool_arguments
    if isinstance(args, str):
        s = args.strip()
        if not s:
            return None
        try:
            args = json.loads(s)
        except Exception:
            return s[:MAX_ARGUMENTS_CHARS]      # raw model text: let the gate judge it
    if isinstance(args, str):
        s = args.strip()
        return s[:MAX_ARGUMENTS_CHARS] if s else None
    if isinstance(args, dict):
        for key in ("command", "cmd"):
            v = args.get(key)
            if isinstance(v, str) and v.strip():
                return v[:MAX_ARGUMENTS_CHARS]
            if isinstance(v, list):             # mirror the gate's own list handling
                joined = " ".join(shlex.quote(str(c)) for c in v)
                return joined[:MAX_ARGUMENTS_CHARS] if joined.strip() else None
        return None
    if isinstance(args, list):
        joined = " ".join(shlex.quote(str(c)) for c in args)
        return joined[:MAX_ARGUMENTS_CHARS] if joined.strip() else None
    return None


class GateDecision(NamedTuple):
    """The gate's verdict on one command.

    outcome is "allow" (exit 0), "block" (exit 2, reason carries the gate's
    stderr), or "unreachable" (spawn error / timeout / non-0-2 exit). The
    destructive safety net is applied here, not by the caller: an
    "unreachable" verdict is already "block" when the command matched it,
    and means fail-open otherwise.
    """
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


def _spawn_gate(gate: str, payload: dict) -> GateDecision:
    if not os.path.exists(gate):
        return GateDecision("unreachable", "", error="gate missing at %s" % gate,
                            gate=gate)
    python = os.environ.get("OPERATOR_PYTHON") or sys.executable or "python3"
    cmd = [python, gate, "hook", "--source", "openai-agents"]
    # Mirrors the OpenHands/OpenClaw adapters: the agent session must never
    # run the gate's owner commands (approve/mode/install).
    env = dict(os.environ, OPERATOR_AGENT_SESSION="1")
    try:
        proc = subprocess.run(cmd, input=json.dumps(payload), env=env,
                              capture_output=True, text=True,
                              timeout=GATE_TIMEOUT_S)
    except subprocess.TimeoutExpired:
        return GateDecision("unreachable", "", error="gate timeout after %ss"
                            % GATE_TIMEOUT_S, gate=gate)
    except Exception as exc:
        return GateDecision("unreachable", "", error=str(exc), gate=gate)
    if proc.returncode == 0:
        return GateDecision("allow", "", exit_code=0, gate=gate)
    if proc.returncode == 2:
        return GateDecision("block", proc.stderr.strip(), exit_code=2, gate=gate)
    # The gate only ever emits 0/2; anything else means we could not really
    # run it (bad interpreter, crash before its own degrade handler, ...).
    return GateDecision("unreachable", "", exit_code=proc.returncode,
                        error=(proc.stderr.strip() or "gate exit %d" % proc.returncode),
                        gate=gate)


def gate_decision(tool_name: str,
                  command: str,
                  cwd: Optional[str] = None,
                  session_id: Optional[str] = None,
                  gate: Optional[str] = None) -> GateDecision:
    """Judge one command through the gate. The only decision path here.

    Payload is the gate's stdin contract verbatim:
        {"tool_name", "tool_input": {"command"}, "cwd", "session_id"?}
    """
    if not command or not command.strip():
        return GateDecision("allow", "no command to judge",
                            gate=gate or resolve_gate())
    gate = gate or resolve_gate()
    payload: dict = {
        "tool_name": tool_name or "shell",
        "tool_input": {"command": command},
        "cwd": cwd or os.getcwd(),
    }
    if session_id:
        payload["session_id"] = str(session_id)[:64]
    decision = _spawn_gate(gate, payload)
    if decision.outcome == "unreachable":
        if looks_destructive(command):
            return GateDecision(
                "block",
                ("gate unreachable (%s) and the command matches the "
                 "destructive safety net (%s); ask the owner."
                 % (decision.error or "unknown", SHELL_RC_NOTE
                    if re.search(r"(?:zshrc|zshenv|zprofile|bashrc|bash_profile"
                                 r"|profile|ssh/rc)", command, re.I)
                    else "destructive shape")),
                exit_code=decision.exit_code, error=decision.error, gate=gate)
        return decision
    return decision


def describe() -> dict:
    """Install/ops snapshot: where the gate is, mode, SDK presence."""
    gate = resolve_gate()
    mode = os.environ.get("OPERATOR_GATE_MODE", "")
    if not mode:
        try:
            with open(os.path.join(os.path.expanduser("~"), ".operator",
                                   "mode")) as f:
                mode = f.read().strip()
        except Exception:
            mode = ""
    return {
        "version": __version__,
        "gate": gate,
        "gate_present": os.path.exists(gate),
        "mode": mode or "(no mode file -- the gate treats this as enforce; "
                        "install.sh writes 'shadow')",
        "agents_sdk": agents_sdk_available(),
    }


# ---------------------------------------------------------------------------
# OpenAI Agents SDK integration (lazy: `agents` is imported only here)
# ---------------------------------------------------------------------------
def agents_sdk_available() -> bool:
    try:
        from agents import (ToolGuardrailFunctionOutput,  # noqa: F401
                            function_tool,  # noqa: F401
                            tool_input_guardrail)  # noqa: F401
        return True
    except Exception:
        return False


def input_guardrail():
    """A ToolInputGuardrail wrapping the gate (requires the SDK).

    Attach it to any function tool:
        @function_tool(tool_input_guardrails=[operator_guard.input_guardrail()])
    """
    from agents import tool_input_guardrail
    return tool_input_guardrail(operator_tool_input_guardrail,
                                name="operator_lite")


def operator_tool_input_guardrail(data: Any) -> Any:
    """Tool-input-guardrail function: gate the arguments before the tool runs.

    Signature per ref/tool_guardrails/: receives ToolInputGuardrailData,
    returns ToolGuardrailFunctionOutput. Sync, so it works under both the
    sync and async run paths.
    """
    from agents import ToolGuardrailFunctionOutput
    ctx = getattr(data, "context", None)
    command = extract_command(getattr(ctx, "tool_arguments", None))
    if command is None:
        return ToolGuardrailFunctionOutput.allow()
    decision = gate_decision(
        tool_name=getattr(ctx, "tool_name", "") or "",
        command=command,
        cwd=os.getcwd(),
        # the per-call id is the closest thing to a session label the
        # ToolContext carries; it lands in the receipt for traceability
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

    needs_approval passes through to function_tool: True always pauses the
    run for a human decision (state.approve / state.reject); an async
    callable (run_context, params: dict, call_id: str) -> bool decides per
    call. The gate is deterministic and runs first either way.
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
    """Gate ANY prebuilt FunctionTool in place, without rebuilding it.

    Composes on_invoke_tool (signature per ref/tool/:
    Callable[[ToolContext, str], Awaitable[Any]]): the gate sees the raw
    JSON arguments first; on block, the deny string becomes the tool result.
    """
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


__all__ = [
    "DENY_PREFIX", "GATE_TIMEOUT_S", "DESTRUCTIVE_LIKE", "GateDecision",
    "agents_sdk_available", "deny_message", "describe", "extract_command",
    "gated_shell_tool", "gate_decision", "input_guardrail", "looks_destructive",
    "operator_tool_input_guardrail", "resolve_gate", "wrap_tool",
]
