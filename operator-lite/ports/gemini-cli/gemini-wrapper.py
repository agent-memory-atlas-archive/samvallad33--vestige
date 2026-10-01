#!/usr/bin/env python3
"""Gemini CLI -> operator-gate adapter.

Reshapes Gemini CLI's BeforeTool stdin payload into the gate's stdin contract,
then runs the gate and passes its exit code and stderr through untouched.
This file contains no rule logic and no analysis -- it is payload shaping only.

Gemini CLI payload (verified against geminicli.com/docs/hooks/reference and
packages/core/src/hooks/types.ts, BeforeToolInput):
    {"session_id": "...", "transcript_path": "...", "cwd": "/workspace",
     "hook_event_name": "BeforeTool", "timestamp": "ISO-8601",
     "tool_name": "run_shell_command",
     "tool_input": {"command": "rm -rf /tmp/data"}}

Gate payload (operator-gate.py hook):
    {"tool_name": ..., "tool_input": {"command": ...}, "cwd": ..., "session_id": ...}

The payload fields already match by name (tool_name / tool_input.command /
cwd / session_id all line up). One reshape is needed:

  Gemini's editing tool is `replace` (file_path, old_string, new_string);
  the gate's write-tool registry knows that shape as `edit`. Renaming it gives
  file edits the full write-path coverage (OP-000 / OP-002 / OP-008 / OP-S14).
  Gemini's other mutating tool, `write_file`, is already a name the gate
  knows, so it is left alone. Read-only tools pass through untouched.

Exit codes (Gemini CLI semantics, verified in packages/core/src/hooks/
hookRunner.ts): 0 = allow; 1 = non-fatal warning, the CLI proceeds and shows
stderr as a warning; any other code, including 2, denies the call and uses
the stderr text as the reason sent to the agent. The gate only ever emits
0 or 2, so the mapping is exact.

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

# Gemini's mutating edit tool -> a write-tool name the gate already knows.
EDIT_TOOLS = {"replace"}


def reshape(payload):
    """Gemini payload -> gate payload. Extra keys pass through (gate ignores them)."""
    if not isinstance(payload, dict):
        return payload
    out = dict(payload)
    tool = str(out.get("tool_name") or out.get("toolName") or "")
    if tool in EDIT_TOOLS:
        out["tool_name"] = "edit"          # a write-tool name the gate already knows
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
        # Broken install: fail open, loudly. Gemini CLI treats exit 1 as a
        # non-fatal warning (proceeds, shows the stderr text), which is the
        # honest direction: every call is allowed and every warning is seen.
        sys.stderr.write("operator-gate: wrapper present but gate missing at %s "
                         "-- rerun install.sh\n" % GATE)
        sys.exit(1)

    # OPERATOR_AGENT_SESSION mirrors the OpenHands/Goose adapters: approvals and
    # installs (gate owner commands) refuse inside an agent session.
    env = dict(os.environ, OPERATOR_AGENT_SESSION="1")
    r = subprocess.run([sys.executable, GATE, "hook", "--source", "gemini"],
                       input=body, env=env)
    sys.exit(r.returncode)


if __name__ == "__main__":
    main()
