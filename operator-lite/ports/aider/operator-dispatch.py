#!/usr/bin/env python3
"""operator-dispatch -- the shim-side half of the Aider port of Operator Lite.

Aider has no hook or plugin API (verified against the options reference, the
config reference and HISTORY.md -- see README.md), but its agent runs real
shell commands through the user's PATH. install.sh therefore generates PATH
shims, and every shim calls this script instead of exec'ing the real binary
right away. It:

  1. rebuilds the shell command from argv (`bin` + shlex-quoted args), so the
     gate sees exactly the words the shell will act on,
  2. sends the standard gate payload to the real, unmodified gate as a
     subprocess:
         {"tool_name": "shim:<bin>", "tool_input": {"command": ...}, "cwd": $PWD}
  3. maps the gate's exit code onto the shim contract:
         exit 0  -> exit 0        (the shim execs the real binary)
         exit 2  -> print "Operator Lite blocked:" + the gate's reason to
                    stderr, exit 126
         other   -> gate unreachable: fail-open UNLESS the command plainly
                    looks destructive (DESTRUCTIVE_LIKE below), then block.

DESTRUCTIVE_LIKE is a verbatim mirror of the OpenClaw reference adapter's
fail-open filter (openclaw-plugin/index.js). The POSIX-sh copy baked into
every shim is the ERE translation of the same regex (POSIX ERE has no \\b).

This file transports; the gate judges. No rule logic lives here, the gate
file is never modified, and shadow vs enforce is the gate's own ~/.operator/mode.
"""
import json
import os
import re
import shlex
import subprocess
import sys

GATE_TIMEOUT_S = 8
SOURCE = os.environ.get("OPERATOR_SHIM_SOURCE", "aider")
OP_HOME = os.environ.get("OPERATOR_HOME", os.path.join(os.path.expanduser("~"), ".operator"))
GATE = os.path.join(OP_HOME, "gate", "operator-gate.py")

# Mirror of openclaw-plugin/index.js DESTRUCTIVE_LIKE, extended with
# shell-init writes (OP-008 class). Lesson from the 2026-09-30 incident: a
# truncated ~/.zshrc must never ride the fail-open path, even when the gate
# itself is unreachable. Shell rc files fail closed, always.
# Mirror of openclaw-plugin/index.js DESTRUCTIVE_LIKE, extended with
# shell-init writes (OP-008 class). Lesson from the 2026-09-30 incident: a
# truncated ~/.zshrc must never ride the fail-open path, even when the gate
# itself is unreachable. Shell rc files fail closed, always.
DESTRUCTIVE_LIKE = re.compile(
    r"\brm\s+-[a-zA-Z]*[rR]|\bpush\s+.*--force|\bDROP\s+TABLE|vestige\.db|fly\s+deploy|mkfs|\bdd\s+if="
    r"|(^|[\s;|&])(>>|\btee\s)[^|;&]*/\.(?:zshrc|zprofile|zshenv|zlogin|bashrc|bash_profile|profile)\b",
    re.I,
)

_COMMAND = ""  # set as soon as the command text is rebuilt; used by the degraded path


def unreachable(why):
    """Gate could not render a verdict: same policy as the reference adapter."""
    if _COMMAND and DESTRUCTIVE_LIKE.search(_COMMAND):
        sys.stderr.write(
            "Operator Lite blocked: gate unreachable (%s) and the command looks "
            "destructive; ask the owner.\n" % why
        )
        sys.exit(126)
    sys.stderr.write("operator-lite: gate unreachable (%s); failing open\n" % why)
    sys.exit(0)


def main():
    global _COMMAND
    if len(sys.argv) < 2:
        sys.stderr.write("operator-dispatch: usage: operator-dispatch.py <bin> [args...]\n")
        sys.exit(0)
    bin_name = sys.argv[1]
    args = sys.argv[2:]
    _COMMAND = " ".join([bin_name] + [shlex.quote(a) for a in args])

    cwd = os.environ.get("PWD", "")
    if not cwd.startswith("/"):
        try:
            cwd = os.getcwd()
        except OSError:
            cwd = "/"

    payload = {
        "tool_name": "shim:%s" % bin_name,
        "tool_input": {"command": _COMMAND},
        "cwd": cwd,
    }
    session = os.environ.get("OPERATOR_SESSION_ID")
    if session:
        payload["session_id"] = session[:64]

    if not os.path.isfile(GATE):
        unreachable("gate missing at %s" % GATE)

    env = dict(os.environ)
    env["OPERATOR_AGENT_SESSION"] = "1"
    try:
        proc = subprocess.run(
            [sys.executable, GATE, "hook", "--source", SOURCE],
            input=json.dumps(payload).encode("utf-8"),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=GATE_TIMEOUT_S,
            env=env,
        )
    except subprocess.TimeoutExpired:
        unreachable("gate timeout after %ss" % GATE_TIMEOUT_S)
    except OSError as exc:
        unreachable(str(exc))

    if proc.returncode == 0:
        sys.exit(0)
    if proc.returncode == 2:
        reason = proc.stderr.decode("utf-8", "replace").strip() or "OPERATOR: STOPPED"
        lines = reason.splitlines()
        # "Operator Lite blocked: <reason>" -- the gate's verdict on the same
        # line, the Why/Compliant-path/permit lines following verbatim.
        sys.stderr.write("Operator Lite blocked: %s\n" % lines[0])
        for ln in lines[1:]:
            sys.stderr.write("%s\n" % ln)
        sys.exit(126)
    unreachable("gate exited %s" % proc.returncode)


if __name__ == "__main__":
    try:
        main()
    except SystemExit:
        raise
    except Exception as exc:  # dispatcher defect: degrade like an unreachable gate
        if _COMMAND and DESTRUCTIVE_LIKE.search(_COMMAND):
            sys.stderr.write(
                "Operator Lite blocked: dispatcher error (%s) and the command looks "
                "destructive; ask the owner.\n" % exc
            )
            sys.exit(126)
        sys.stderr.write("operator-lite: dispatcher error (%s); failing open\n" % exc)
        sys.exit(0)
