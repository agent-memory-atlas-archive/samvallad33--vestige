#!/usr/bin/env python3
"""Operator Lite middleware for LangChain agents and LangGraph tool nodes.

The Operator gate (one stdlib-only Python file, analyzer-only) is spawned as a
subprocess for every shell-ish tool call; this module is payload shaping and
decision plumbing only -- it contains no rule logic and no analysis.

    gate exit 0 -> allow        (the wrapped handler runs normally)
    gate exit 2 -> block        (a ToolMessage is returned instead; the handler
                                 is never called, and the LLM sees
                                 "Operator Lite blocked: <gate stderr>")
    spawn error -> fail-open, UNLESS the command matches DESTRUCTIVE_LIKE,
                   in which case fail-closed (same policy as the OpenClaw
                   adapter, extended with shell-rc writes)

Verified against the current LangChain v1 middleware API (checked 2026-09):

  * `wrap_tool_call` / class hook on `AgentMiddleware`
    https://reference.langchain.com/python/langchain/agents/middleware/types/wrap_tool_call
    Signature: (request: ToolCallRequest,
                handler: Callable[[ToolCallRequest], ToolMessage | Command])
             -> ToolMessage | Command
  * `ToolCallRequest.tool_call` is the model's call dict {"name", "args", "id"}
    https://docs.langchain.com/oss/python/langchain/middleware/custom
  * Blocking = return a ToolMessage and call the handler zero times
    (the docs' own short-circuit example). Raising is NOT the block form:
    "Exceptions propagate unless `handle_tool_errors` is configured on
    ToolNode" -- a raise can be swallowed into a generic error message, while
    a returned ToolMessage is deterministic.
  * Attached via `create_agent(model, tools, middleware=[...])`
    https://docs.langchain.com/oss/python/langchain/middleware/built-in
  * LangGraph note: `langgraph.prebuilt.ToolNode` takes no middleware. For a
    hand-rolled StateGraph, guard the tool function itself with `decide()` and
    raise `ToolException` (see README). `HumanInTheLoopMiddleware` (per-tool
    `interrupt_on` with `RejectDecision`) is the interactive complement for
    calls this deterministic gate should pause rather than decide.

Gate resolution order (same as every port):
    1. $OPERATOR_GATE (explicit override, re-read on every call)
    2. ~/.operator/gate/operator-gate.py (installed copy)

The gate contract on its stdin:
    {"tool_name": ..., "tool_input": {"command": ...}, "cwd": ...,
     "session_id": ...?}
spawned as: python3 <gate> hook --source langchain
Shadow vs enforce is a file: ~/.operator/mode (shadow default).

This module is stdlib-only and importable WITHOUT langchain installed; the
langchain-facing pieces (`OperatorLiteMiddleware`, `operator_middleware()`)
import langchain lazily on first use. Python 3.9 compatible.
"""
import json
import os
import re
import subprocess
import sys

VERSION = "0.1.0"
SOURCE = "langchain"
GATE_TIMEOUT_S = float(os.environ.get("OPERATOR_GATE_TIMEOUT_S", "8"))

# Verdicts returned by decide() / gate_verdict().
PASS = "pass"          # not shell-ish -> no gating, call the handler
ALLOW = "allow"        # gate allowed the call (receipt already written by the gate)
BLOCK = "block"        # gate stopped it, or unreachable + destructive-like
UNREACHABLE = "unreachable"

# Keys recognised as "carries a shell command" in a tool's args. `command` is
# the LangChain ShellTool / ShellToolMiddleware argument; `cmd` and `script`
# are the OpenClaw adapter's fallbacks, kept for parity.
COMMAND_KEYS = ("command", "cmd", "script")

# OpenClaw adapter's destructive-like regex, extended with shell-rc writes
# (used ONLY for the gate-unreachable decision, never as a rule engine).
DESTRUCTIVE_LIKE = re.compile(
    r"\brm\s+-[a-zA-Z]*[rR]"                       # rm -r / -rf / -R
    r"|\bpush\s+.*--force"                          # force push
    r"|\bDROP\s+TABLE"
    r"|vestige\.db"
    r"|fly\s+deploy"
    r"|mkfs"
    r"|\bdd\s+if="
    # shell-rc / init writes (>> file, > file, tee [-a] file):
    r"|(?:>>|>|\btee\s+(?:-a\s+)?)[^\n|;&]*"
    r"\.(?:zshrc|zshenv|zprofile|zlogin|bashrc|bash_profile|profile|ssh/rc|config\.fish)\b",
    re.I)


def resolve_gate():
    """Locate the gate. $OPERATOR_GATE wins; re-read every call so tests and
    embedding processes can redirect it without reloading this module."""
    gate = os.environ.get("OPERATOR_GATE")
    if gate:
        return gate
    return os.path.join(os.path.expanduser("~"), ".operator", "gate", "operator-gate.py")


def extract_command(args):
    """The shell command string carried by a tool's args, or None."""
    if not isinstance(args, dict):
        return None
    for key in COMMAND_KEYS:
        value = args.get(key)
        if value is not None and value != "":
            return value
    return None


def is_shellish(tool_name, args):
    """Shell-ish by name ("shell" anywhere in the tool name) or by shape
    (args carry a command key). The gate, not this list, decides relevance."""
    name = str(tool_name or "").lower()
    if "shell" in name:
        return True
    return isinstance(args, dict) and any(k in args for k in COMMAND_KEYS)


def _command_text(command):
    return command if isinstance(command, str) else " ".join(str(c) for c in command)


def gate_verdict(command, tool_name="shell", cwd=None, session_id=None):
    """Spawn the gate with its exact stdin contract. Returns
    (ALLOW, None) | (BLOCK, gate stderr) | (UNREACHABLE, error text).

    Nothing here executes the command: it is piped to the gate as JSON and
    the gate is analyzer-only.
    """
    gate = resolve_gate()
    if not os.path.exists(gate):
        # Missing gate -> unreachable (fail-open/fail-closed policy below).
        # This check is load-bearing: `python3 <missing>.py` itself exits 2,
        # which would otherwise collide with the gate's STOP code and turn a
        # broken install into bogus blocks with python's error as the "reason"
        # (the Windsurf wrapper guards the same collision with the same check).
        return (UNREACHABLE, "gate not found at %s" % gate)
    payload = {"tool_name": str(tool_name or "shell"),
               "tool_input": {"command": command},
               "cwd": cwd or os.getcwd()}
    if session_id:
        payload["session_id"] = str(session_id)[:64]
    # Mirrors the Windsurf/OpenClaw/Goose/OpenHands adapters: gate owner
    # commands (approve/mode/install) refuse inside an agent session.
    env = dict(os.environ, OPERATOR_AGENT_SESSION="1")
    python = os.environ.get("OPERATOR_PYTHON") or sys.executable or "python3"
    try:
        proc = subprocess.run(
            [python, gate, "hook", "--source", SOURCE],
            input=json.dumps(payload).encode("utf-8"),
            capture_output=True, timeout=GATE_TIMEOUT_S, env=env)
    except (OSError, subprocess.SubprocessError) as exc:
        return (UNREACHABLE, "%s" % exc)
    if proc.returncode == 2:
        reason = proc.stderr.decode("utf-8", "replace").strip()
        return (BLOCK, reason or "OPERATOR: STOPPED")
    return (ALLOW, None)          # 0 = allow (incl. shadow mode); other codes degrade to allow


def decide(tool_name, args, cwd=None, session_id=None):
    """The full decision the middleware applies to one tool call. Returns
    (verdict, block_message_or_None):

        (PASS,   None)  not shell-ish -> middleware just calls the handler
        (ALLOW,  None)  gate allowed it (a receipt was written by the gate)
        (BLOCK,  msg)   handler must NOT run; msg starts with
                        "Operator Lite blocked:" for the LLM
    """
    if not is_shellish(tool_name, args):
        return (PASS, None)
    command = extract_command(args)
    if command is None:
        return (PASS, None)

    verdict, detail = gate_verdict(command, tool_name=tool_name, cwd=cwd,
                                   session_id=session_id)
    if verdict == ALLOW:
        return (ALLOW, None)
    if verdict == BLOCK:
        return (BLOCK, "Operator Lite blocked: %s" % detail)

    # Gate unreachable: fail-open, unless the raw text plainly looks
    # destructive -- then fail closed (OpenClaw adapter policy, + rc writes).
    text = _command_text(command)
    if DESTRUCTIVE_LIKE.search(text):
        return (BLOCK, "Operator Lite blocked: gate unreachable at %s and the "
                       "command looks destructive; ask the owner." % resolve_gate())
    return (ALLOW, None)


# --------------------------------------------------------------------------- #
# langchain layer (lazy: the imports below happen on first use, so this module
# imports cleanly with no langchain installed)
# --------------------------------------------------------------------------- #
_LC_CACHE = {}


def _langchain_bits():
    if "bits" not in _LC_CACHE:
        from langchain.agents.middleware import AgentMiddleware
        from langchain_core.messages import ToolMessage
        _LC_CACHE["bits"] = (AgentMiddleware, ToolMessage)
    return _LC_CACHE["bits"]


def _build_middleware_class():
    AgentMiddleware, ToolMessage = _langchain_bits()

    class OperatorLiteMiddleware(AgentMiddleware):
        """Gate every shell-ish tool call through Operator Lite.

        wrap_tool_call signature and ToolMessage short-circuit per
        https://reference.langchain.com/python/langchain/agents/middleware/types/wrap_tool_call
        """

        def wrap_tool_call(self, request, handler):
            call = dict(getattr(request, "tool_call", None) or {})
            verdict, message = decide(call.get("name", ""), call.get("args") or {},
                                      cwd=os.getcwd())
            if verdict == BLOCK:
                return ToolMessage(content=message,
                                   tool_call_id=str(call.get("id") or ""))
            return handler(request)

        async def awrap_tool_call(self, request, handler):
            import asyncio
            call = dict(getattr(request, "tool_call", None) or {})
            loop = asyncio.get_running_loop()
            verdict, message = await loop.run_in_executor(
                None, lambda: decide(call.get("name", ""), call.get("args") or {},
                                     cwd=os.getcwd()))
            if verdict == BLOCK:
                return ToolMessage(content=message,
                                   tool_call_id=str(call.get("id") or ""))
            return await handler(request)

    OperatorLiteMiddleware.__module__ = __name__
    OperatorLiteMiddleware.__name__ = "OperatorLiteMiddleware"
    return OperatorLiteMiddleware


def __getattr__(name):
    """PEP 562: `OperatorLiteMiddleware` materialises on first attribute
    access; importing this module stays langchain-free."""
    if name == "OperatorLiteMiddleware":
        cls = _build_middleware_class()
        globals()["OperatorLiteMiddleware"] = cls
        return cls
    raise AttributeError("module %r has no attribute %r" % (__name__, name))


def operator_middleware():
    """Return an OperatorLiteMiddleware instance for create_agent's
    `middleware=[...]` list. First call imports langchain (lazily)."""
    cls = globals().get("OperatorLiteMiddleware") or __getattr__("OperatorLiteMiddleware")
    return cls()
