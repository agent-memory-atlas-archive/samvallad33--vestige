#!/usr/bin/env python3
"""Windsurf / Devin Desktop Cascade -> operator-gate adapter.

Reshapes Windsurf Cascade's hook stdin payload into the gate's stdin contract,
then runs the gate and passes its exit code and streams through untouched.
This file contains no rule logic and no analysis -- it is payload shaping only.

Windsurf payload (verified against https://docs.devin.ai/desktop/cascade/hooks):
    {"agent_action_name": "pre_run_command",
     "trajectory_id": "...", "execution_id": "...",
     "timestamp": "ISO 8601", "model_name": "...",
     "tool_info": {"command_line": "npm install package-name",
                   "cwd": "/Users/yourname/project"}}

Gate payload (operator-gate.py hook):
    {"tool_name": ..., "tool_input": {"command": ...}, "cwd": ..., "session_id": ...}

Nothing matches by name, so this shim performs exactly three reshapes (one per
wired event) and nothing else:

  pre_run_command    -> tool_name "run_terminal_cmd" (a name already in the
                        gate's SHELL_TOOLS registry), tool_input.command :=
                        tool_info.command_line, cwd := tool_info.cwd
  pre_write_code     -> tool_name "edit" (a write-tool name the gate knows),
                        tool_input := {file_path, old_string, new_string}
                        (edits[] joined), so file writes get full write-path
                        coverage (OP-000 / OP-002 / OP-008 / OP-S14)
  pre_mcp_tool_use   -> tool_name := "mcp__<server>__<tool>" (the gate's MCP
                        rules key on the mcp__ prefix), tool_input :=
                        mcp_tool_arguments (OP-S02 / OP-S13)

session_id := trajectory_id (the conversation id; execution_id is per-turn).
cwd for write/MCP events := the hook process's cwd (Windsurf runs hooks at the
workspace root by default and the payload carries no cwd for those events).

Exit codes (Windsurf semantics, verified in the docs above): 0 = allow; 2 =
pre-hook blocks the action and Cascade shows the stderr text to the agent;
ANY other code = error, the action proceeds normally. The gate only ever emits
0 or 2, so the mapping is exact. A missing gate fails open with a loud stderr
note (exit 1: proceeds, error visible).

Stdlib only, Python 3.9 compatible. No tool call is ever executed here: this
process only pipes JSON.
"""
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
GATE = os.path.join(HERE, "operator-gate.py")
if not os.path.exists(GATE):
    GATE = os.path.join(os.path.expanduser("~"), ".operator", "gate", "operator-gate.py")

# Windsurf fires the same script for every wired event; agent_action_name
# selects the reshape. Only pre-hooks can block, so only pre-hooks are wired.
RUN_COMMAND = "pre_run_command"
WRITE_CODE = "pre_write_code"
MCP_TOOL_USE = "pre_mcp_tool_use"


def _tool_info(payload):
    ti = payload.get("tool_info")
    return ti if isinstance(ti, dict) else {}


def reshape(payload):
    """Windsurf payload -> gate payload. Extra keys are dropped (gate ignores them)."""
    if not isinstance(payload, dict):
        return {"tool_name": "unknown", "tool_input": {}, "cwd": os.getcwd()}
    ti = _tool_info(payload)
    action = str(payload.get("agent_action_name") or "")
    sid = payload.get("trajectory_id") or payload.get("execution_id") or ""

    if action == RUN_COMMAND:
        return {"tool_name": "run_terminal_cmd",
                "tool_input": {"command": ti.get("command_line", "")},
                "cwd": ti.get("cwd") or os.getcwd(),
                "session_id": sid}

    if action == WRITE_CODE:
        edits = ti.get("edits")
        if not isinstance(edits, list):
            edits = []
        olds = "\n".join(str(e.get("old_string", "")) for e in edits if isinstance(e, dict))
        news = "\n".join(str(e.get("new_string", "")) for e in edits if isinstance(e, dict))
        return {"tool_name": "edit",
                "tool_input": {"file_path": ti.get("file_path", ""),
                               "old_string": olds, "new_string": news},
                "cwd": os.getcwd(),
                "session_id": sid}

    if action == MCP_TOOL_USE:
        server = str(ti.get("mcp_server_name") or "")
        tool = str(ti.get("mcp_tool_name") or "")
        name = "mcp__%s__%s" % (server, tool) if (server or tool) else "mcp__unknown__unknown"
        args = ti.get("mcp_tool_arguments")
        if not isinstance(args, dict):
            args = {} if args is None else {"input": args}
        return {"tool_name": name, "tool_input": args,
                "cwd": os.getcwd(), "session_id": sid}

    # Not one of the wired events: pass through unnamed. The gate classifies
    # unknown tools as no-hits, so this is an honest allow (still receipted).
    return {"tool_name": action or "unknown", "tool_input": ti,
            "cwd": os.getcwd(), "session_id": sid}


def main():
    raw = sys.stdin.buffer.read()
    try:
        payload = json.loads(raw.decode("utf-8", "replace")) if raw.strip() else {}
    except Exception:
        payload = raw                       # unparseable: let the gate degrade honestly
    if isinstance(payload, dict):
        body = json.dumps(reshape(payload)).encode("utf-8")
    else:
        body = raw

    if not os.path.exists(GATE):
        # Broken install: fail open, loudly. Windsurf treats any non-0/2 exit as
        # an error and proceeds, showing the stderr text -- every call is allowed
        # and every error is seen.
        sys.stderr.write("operator-gate: wrapper present but gate missing at %s "
                         "-- rerun install.sh\n" % GATE)
        sys.exit(1)

    # OPERATOR_AGENT_SESSION mirrors the Gemini/Goose/OpenHands adapters:
    # approvals and installs (gate owner commands) refuse inside an agent session.
    env = dict(os.environ, OPERATOR_AGENT_SESSION="1")
    r = subprocess.run([sys.executable, GATE, "hook", "--source", "windsurf"],
                       input=body, env=env)
    sys.exit(r.returncode)


if __name__ == "__main__":
    main()
