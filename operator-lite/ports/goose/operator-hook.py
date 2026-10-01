#!/usr/bin/env python3
"""
Operator Lite -- Goose port hook adapter.

Goose (Block's open-source agent) supports lifecycle hooks since v1.41.0
(Open Plugins spec, `~/.agents/plugins/<name>/hooks/hooks.json`). A
`PreToolUse` hook BLOCKS the tool call when it exits with code 2, taking the
reason from stderr; exit 0 with clean stdout allows. That is byte-for-byte the
Operator gate's own contract, so this shim only has to translate field names:

    goose stdin : {"event", "session_id", "tool_name", "tool_input", "working_dir"}
    gate stdin  : {"tool_name", "tool_input", "cwd", "session_id"}

and then propagate the gate's exit code and stderr verbatim.

    gate exit 0 -> allow (receipts already written by the gate; stdout stays EMPTY)
    gate exit 2 -> block (gate's stderr becomes the hook's stderr, goose shows it)
    gate unreachable -> fail OPEN unless the text plainly looks destructive
                        (mirror of the gate's own degraded_hit and the reference
                        OpenClaw adapter), in which case fail CLOSED with exit 2.

Every PreToolUse call is forwarded -- file edits and MCP extension tools
included -- so the gate, not the shim, decides relevance. Goose namespaces
extension tools as `{extension}__{tool}` (the developer extension sends bare
`shell`), and the gate's shell routing catches all of these via its
`command`-key fallback.

stdlib only. No tool call is ever executed here: this process only pipes JSON.
"""

import json
import os
import re
import subprocess
import sys
from pathlib import Path

GATE_TIMEOUT_S = 8  # slightly below hooks.json's 10s so we can self-degrade first

DESTRUCTIVE_LIKE = re.compile(
    r"\brm\s+-[a-zA-Z]*[rR]|\bmv\s+\S*(vestige|Developer|\.zcode|\.claude)"
    r"|\bpush\s+.*--force|\bDROP\s+TABLE|\.vestige/vestige\.db|fly\s+deploy"
    r"|OPERATOR_CANARY_STOP_7f3a|mkfs|\bdd\s+if=",
    re.I,
)


def gate_path():
    env = os.environ.get("OPERATOR_GATE")
    if env:
        return env
    here = Path(__file__).resolve().parent
    candidates = [
        Path.home() / ".operator" / "gate" / "operator-gate.py",
        # repo checkouts: ports/goose/scripts -> operator-lite/operator-gate.py
        here / ".." / ".." / ".." / "operator-gate.py",
        here / ".." / ".." / "operator-gate.py",
    ]
    for c in candidates:
        if c.is_file():
            return str(c.resolve())
    return str(candidates[0])


def degraded(raw, reason):
    """No gate verdict available: fail open unless plainly destructive text."""
    sys.stderr.write(reason + "\n")
    if DESTRUCTIVE_LIKE.search(raw or ""):
        sys.stderr.write(
            "Operator Lite: gate unreachable and the command looks destructive "
            "-- blocked. Ask the owner.\n"
        )
        return 2
    return 0


def run_gate(gate_payload):
    cmd = [
        os.environ.get("OPERATOR_PYTHON") or sys.executable,
        gate_path(),
        "hook",
        "--source",
        "goose",
    ]
    env = dict(os.environ, OPERATOR_AGENT_SESSION="1")
    return subprocess.run(
        cmd,
        input=json.dumps(gate_payload),
        capture_output=True,
        text=True,
        timeout=GATE_TIMEOUT_S,
        env=env,
    )


def main(raw):
    try:
        p = json.loads(raw) if raw.strip() else {}
        if not isinstance(p, dict):
            p = {}
    except Exception:
        return 0  # malformed hook payload: no decision -> goose allows

    # goose -> gate contract translation. goose sends `working_dir`; the gate
    # reads `cwd`. tool_input/session_id/tool_name pass through as-is.
    gate_payload = {
        "tool_name": p.get("tool_name") or "shell",
        "tool_input": p.get("tool_input") if p.get("tool_input") is not None else {},
        "cwd": p.get("working_dir") or p.get("cwd") or os.getcwd(),
        "session_id": p.get("session_id"),
    }

    try:
        proc = run_gate(gate_payload)
    except subprocess.TimeoutExpired:
        return degraded(raw, "Operator Lite: gate timed out after %ss." % GATE_TIMEOUT_S)
    except Exception as exc:
        return degraded(raw, "Operator Lite: gate unreachable (%s)." % exc)

    code = proc.returncode
    if code == 0:
        return 0  # allowed; stdout stays empty (goose blocks only on exit 2 / decision JSON)
    if code == 2 and "OPERATOR:" in (proc.stderr or ""):
        sys.stderr.write(proc.stderr)
        return 2  # block: goose takes the reason from stderr
    # Any other exit (missing interpreter, unreadable file, internal crash that
    # did not self-degrade) is "gate unreachable", not a block verdict.
    return degraded(
        raw,
        "Operator Lite: gate exited %s (%s)."
        % (code, (proc.stderr or "").strip()[:200]),
    )


if __name__ == "__main__":
    try:
        _raw = sys.stdin.read()
        sys.exit(main(_raw))
    except BrokenPipeError:
        sys.exit(0)
