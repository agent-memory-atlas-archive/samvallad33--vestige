#!/usr/bin/env python3
"""OpenHands -> operator-gate adapter.

Reshapes OpenHands' PreToolUse stdin payload into the gate's stdin contract,
then runs the gate and passes its exit code and stderr through untouched.
This file contains no rule logic and no analysis -- it is payload shaping only.

OpenHands payload (verified against docs.openhands.dev/openhands/usage/customization/hooks):
    {"event_type": "PreToolUse",
     "tool_name": "terminal",
     "tool_input": {"command": "rm -rf /tmp/data"},
     "session_id": "abc-123",
     "working_dir": "/workspace"}

Gate payload (operator-gate.py hook):
    {"tool_name": ..., "tool_input": {"command": ...}, "cwd": ..., "session_id": ...}

Two reshapes are needed:
  1. OpenHands names the working directory `working_dir`; the gate reads `cwd`.
     Without this, relative delete targets cannot be resolved and the gate
     would downgrade them to unresolved (OP-S05) instead of judging them.
  2. OpenHands' mutating file tool is `file_editor`; the gate's write-tool
     registry knows it as `str_replace_editor`. Renaming mutating calls
     (create / str_replace / insert) gives file writes the full OP-000 / OP-002 /
     OP-008 / OP-S14 write-path coverage. Read-only `view` calls are left alone.

Exit codes (OpenHands semantics, same doc): 0 = allow, 2 = deny, any other
code = error (OpenHands proceeds and logs). The gate only ever emits 0 or 2.

Stdlib only, Python 3.9 compatible.
"""
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
GATE = os.path.join(HERE, "operator-gate.py")
if not os.path.exists(GATE):
    GATE = os.path.join(os.path.expanduser("~"), ".operator", "gate", "operator-gate.py")

# Mutating file_editor ops. "view" is a read and must not be renamed.
FILE_EDITOR_MUTATIONS = {"create", "str_replace", "insert"}


def reshape(payload):
    """OpenHands payload -> gate payload. Extra keys pass through (gate ignores them)."""
    if not isinstance(payload, dict):
        return payload
    out = dict(payload)
    wd = out.get("working_dir") or out.get("workingDir")
    if wd and not out.get("cwd"):
        out["cwd"] = wd
    tool = str(out.get("tool_name") or out.get("toolName") or "")
    ti = out.get("tool_input") or out.get("toolInput") or out.get("input")
    if tool == "file_editor" and isinstance(ti, dict) \
            and ti.get("command") in FILE_EDITOR_MUTATIONS:
        out["tool_name"] = "str_replace_editor"   # a write-tool name the gate already knows
    return out


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
        # Broken install: fail open, loudly. OpenHands would treat any non-0/2
        # exit as an error and proceed anyway, so exit 0 with a stderr note.
        sys.stderr.write("operator-gate: wrapper present but gate missing at %s "
                         "-- rerun install.sh\n" % GATE)
        sys.exit(0)

    # OPERATOR_AGENT_SESSION mirrors the OpenClaw adapter: approvals/installs
    # (gate owner commands) refuse inside an agent session.
    env = dict(os.environ, OPERATOR_AGENT_SESSION="1")
    r = subprocess.run([sys.executable, GATE, "hook", "--source", "openhands"],
                       input=body, env=env)
    sys.exit(r.returncode)


if __name__ == "__main__":
    main()
