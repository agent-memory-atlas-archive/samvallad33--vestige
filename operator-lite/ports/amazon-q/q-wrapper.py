#!/usr/bin/env python3
"""Amazon Q CLI -> operator-gate adapter.

Reshapes Amazon Q Developer CLI's preToolUse stdin payload into the gate's
stdin contract, then runs the gate and passes its exit code and stderr through
untouched. This file contains no rule logic and no analysis -- payload
shaping only.

Amazon Q payload (verified in crates/chat-cli/src/cli/chat/cli/hooks.rs,
run_hook: `serde_json::json!({"hook_event_name", "cwd"})` plus tool context):
    {"hook_event_name": "preToolUse", "cwd": "/workspace",
     "tool_name": "execute_bash",
     "tool_input": {"command": "rm -rf /tmp/data", "summary": "..."}}

There is no session_id field; the gate reads it optionally and receipts
record an empty session. Extra keys pass through (the gate ignores them).

Gate payload (operator-gate.py hook):
    {"tool_name": ..., "tool_input": {"command": ...}, "cwd": ..., "session_id"?}

Two reshapes are needed:

  1. Q's shell tool is `execute_bash` (crates/chat-cli/src/cli/chat/tools/
     tool_index.json: input `{command, summary}`, required `command`); on
     Windows it is `execute_cmd`. The gate's shell registry knows the shape
     as `bash`, so the tool is renamed. (The gate's command-key fallback
     would in fact catch execute_bash with no wrapper at all -- `command`
     matches by name -- the explicit rename keeps the branch deterministic.)
  2. Q's file-write tool is `fs_write`, whose input is
     `{command: create|str_replace|insert|append, path, file_text,
       old_str?, new_str?, insert_line?}` (required `["command", "path"]`).
     Left alone it would be misjudged: its enum `command` field collides
     with the gate's command-key fallback. Renamed to `write` (a name in
     the gate's write-tool registry) it gets full write-path coverage
     (OP-000 / OP-002 / OP-008 / OP-S14) via its `path`; the body is
     re-exposed as `content` / `new_string`, the names the gate's
     credential-shaped-write check (OP-S04) reads.

Everything else (fs_read, use_aws, knowledge, thinking, todo_list,
gh_issue, introspect, delegate, @server/tool MCP calls) passes through
untouched.

Exit codes (verified in crates/chat-cli/src/cli/chat/mod.rs, PreToolUse
hook handling): 0 = allow; 2 = block, stderr returned to the model as
"PreToolHook blocked the tool execution: <stderr>"; any other code = the
CLI shows a warning and ALLOWS the tool. The gate only ever emits 0 and 2,
so the mapping is exact. A missing gate (broken install) exits 1 here:
fail-open with a loud warning, the honest direction.

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

# Q's shell tools -> a name in the gate's SHELL_TOOLS registry
# (`execute_cmd` is the cfg(windows) twin of execute_bash).
SHELL_TOOLS = {"execute_bash", "execute_cmd"}
# Q's file-write tool -> a write-tool name the gate already knows.
WRITE_TOOL = "fs_write"


def reshape(payload):
    """Q payload -> gate payload. Extra keys pass through (gate ignores them)."""
    if not isinstance(payload, dict):
        return payload
    out = dict(payload)
    tool = str(out.get("tool_name") or "")
    if tool in SHELL_TOOLS:
        out["tool_name"] = "bash"              # a shell-tool name the gate knows
    elif tool == WRITE_TOOL:
        out["tool_name"] = "write"             # a write-tool name the gate knows
        ti = out.get("tool_input")
        if isinstance(ti, dict):
            ti = dict(ti)
            # fs_write's `command` is an enum (create/str_replace/insert/append),
            # not a shell command; drop it so no fallback ever reads it as one.
            ti.pop("command", None)
            # expose the body under the names the gate's content checks read
            if "file_text" in ti and "content" not in ti:
                ti["content"] = ti["file_text"]
            if "new_str" in ti and "new_string" not in ti:
                ti["new_string"] = ti["new_str"]
            out["tool_input"] = ti
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
        # Broken install: fail open, loudly. Q CLI treats a nonzero exit other
        # than 2 as a warning and runs the tool anyway, which is the honest
        # direction: every call is allowed and every warning is seen.
        sys.stderr.write("operator-gate: wrapper present but gate missing at %s "
                         "-- rerun install.sh\n" % GATE)
        sys.exit(1)

    # OPERATOR_AGENT_SESSION mirrors the OpenHands/Goose/Gemini adapters:
    # approvals and installs (gate owner commands) refuse inside an agent session.
    env = dict(os.environ, OPERATOR_AGENT_SESSION="1")
    # Q spawns hook commands as `bash -c <command>` with the payload on stdin
    # (verified in hooks.rs); mirror that spawn exactly.
    r = subprocess.run([sys.executable, GATE, "hook", "--source", "amazon-q"],
                       input=body, env=env)
    sys.exit(r.returncode)


if __name__ == "__main__":
    main()
