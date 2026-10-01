#!/usr/bin/env python3
"""operator_hooks -- Operator Lite port for CrewAI (execution-hooks dialect).

One import wires the Operator gate ([../../operator-gate.py](../../operator-gate.py))
in front of every tool call a Crew makes:

    import operator_hooks   # registers a global PRE_TOOL_CALL hook, shadow by default

This module is the thin adapter; all rule logic lives in the gate. Per tool call
it runs:

    python3 <gate> hook --source crewai
      stdin: {"tool_name": ..., "tool_input": {"command": ...}, "cwd": os.getcwd()}
             (+ "session_id" when the crew id is readable)
      exit 0        -> allow  (hook returns None; receipts already written)
      exit 2        -> block  (raises HookAborted("Operator Lite blocked: <gate stderr>",
                               source="operator-lite"))
      unreachable   -> fail-open, unless the command plainly looks destructive
                       (or writes a shell rc/init file): fail closed instead.

The verified blocking mechanism (crewAI source, lib/crewai/src/crewai/hooks/):
`HookAborted` is special-cased in `dispatch._invoke_hook` and propagates by
design; any OTHER exception a hook raises is swallowed (fail-open), which is
exactly why this adapter never relies on an ordinary exception to block.

Stdlib only. Importable (and testable) with crewai NOT installed; without
crewai the module loads, reports it, and registers nothing.
"""
import json
import os
import re
import subprocess
import sys

__version__ = "0.1.0"

SOURCE = "crewai"
GATE_TIMEOUT_S = 8.0          # mirrors the OpenClaw adapter
GATE_ENV_VAR = "OPERATOR_GATE"


# ---- gate resolution: $OPERATOR_GATE -> $OPERATOR_HOME/gate -> ~/.operator/gate --- #
def gate_candidates():
    home = os.path.expanduser("~")
    out = []
    env = os.environ.get(GATE_ENV_VAR)
    if env:
        out.append(env)
    op_home = os.environ.get("OPERATOR_HOME", os.path.join(home, ".operator"))
    out.append(os.path.join(op_home, "gate", "operator-gate.py"))
    out.append(os.path.join(home, ".operator", "gate", "operator-gate.py"))
    seen, uniq = set(), []
    for p in out:
        if p and p not in seen:
            seen.add(p)
            uniq.append(p)
    return uniq


def resolve_gate():
    for p in gate_candidates():
        if os.path.isfile(p):
            return p
    return None


# ---- what gets gated --------------------------------------------------------------- #
# crewai sanitizes tool names (lowercase a-z0-9_), so match lowercase.
SHELLISH_NAME_RE = re.compile(r"shell|command|terminal")
COMMAND_KEYS = ("command", "cmd", "script")


def extract_command(tool_input):
    """The shell-ish payload of a tool input, or None when there is nothing to judge."""
    if not isinstance(tool_input, dict):
        return None
    for key in COMMAND_KEYS:
        value = tool_input.get(key)
        if value not in (None, ""):
            return value
    return None


def is_gated(tool_name, tool_input):
    """Shell-ish by name (...shell/command/terminal...) or by payload (a command key)."""
    if SHELLISH_NAME_RE.search(str(tool_name or "").lower()):
        return True
    return extract_command(tool_input) is not None


# ---- fail-closed fallback when the gate is unreachable ------------------------------ #
# Mirrors the OpenClaw adapter's DESTRUCTIVE_LIKE, extended with the gate's
# degraded-hit canary and with shell rc/init writes (the gate's OP-008 set):
# if we cannot reach the gate, these still fail closed.
DESTRUCTIVE_LIKE = re.compile(
    r"\brm\s+-[a-zA-Z]*[rR]"
    r"|\bpush\s+.*--force"
    r"|\bDROP\s+TABLE"
    r"|vestige\.db"
    r"|fly\s+deploy"
    r"|mkfs"
    r"|\bdd\s+if="
    r"|OPERATOR_CANARY_STOP_7f3a",
    re.I,
)
SHELL_INIT_FILE_LIKE = re.compile(
    r"\.(?:zshrc|zshenv|zprofile|bashrc|bash_profile|profile|zlogin)\b"
    r"|\.ssh[/\\]rc\b"
    r"|config\.fish\b",
    re.I,
)
WRITE_OPERATOR_LIKE = re.compile(
    r">>?>|(^|\s)-{0,2}tee\b|(^|\s)cp\b|(^|\s)mv\b|(^|\s)install\b"
    r"|(^|\s)dd\b|\bsed\s+(-[a-zA-Z]*i|--in-place)",
    re.I,
)


def looks_destructive(command):
    """Gate-unreachable policy: fail closed only for plainly destructive text."""
    if isinstance(command, list):
        text = " ".join(str(c) for c in command)
    else:
        text = str(command)
    if DESTRUCTIVE_LIKE.search(text):
        return True
    return bool(SHELL_INIT_FILE_LIKE.search(text) and WRITE_OPERATOR_LIKE.search(text))


# ---- the gate call ------------------------------------------------------------------ #
def gate_verdict(command, tool_name, cwd=None, session_id=None):
    """Run the gate once. Returns ("allow"|"block"|"unreachable", reason).

    The payload is the gate's own hook contract; the gate classifies, receipts,
    and decides. This port contains no rule logic.
    """
    gate = resolve_gate()
    if not gate:
        return "unreachable", ("gate not found (looked in $%s, $OPERATOR_HOME/gate, "
                               "~/.operator/gate)" % GATE_ENV_VAR)
    payload = {
        "tool_name": str(tool_name or ""),
        "tool_input": {"command": command},
        "cwd": cwd or os.getcwd(),
    }
    if session_id:
        payload["session_id"] = str(session_id)[:64]
    env = dict(os.environ, OPERATOR_AGENT_SESSION="1")  # approvals are owner-tty only
    try:
        proc = subprocess.run(
            [sys.executable or "python3", gate, "hook", "--source", SOURCE],
            input=json.dumps(payload), capture_output=True, text=True,
            timeout=GATE_TIMEOUT_S, env=env,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return "unreachable", "gate spawn failed: %s" % exc
    if proc.returncode == 0:
        return "allow", ""
    if proc.returncode == 2:
        return "block", (proc.stderr or "").strip() or "OPERATOR LITE: STOPPED"
    return "unreachable", "gate exited %d (expected 0 or 2); stderr: %s" % (
        proc.returncode, (proc.stderr or "").strip()[:200])


# ---- crewai wiring ------------------------------------------------------------------- #
# Two dialects, verified from crewAI source (lib/crewai/src/crewai/hooks/):
#   dispatcher (current): crewai.hooks exposes register_hook/unregister_hook/
#     get_hooks + InterceptionPoint + HookAborted; the legacy register_* registries
#     are aliased onto the same queue. Blocking = raise HookAborted(reason, source).
#   legacy (older crewai): only register_before_tool_call_hook exists;
#     blocking = return False (no reason carried). We degrade to that.
DIALECT = None        # "dispatcher" | "legacy" | None
_HOOKS_ERROR = None
_installed = False

try:
    from crewai.hooks import (InterceptionPoint, HookAborted,
                              register_hook, unregister_hook, get_hooks)
    DIALECT = "dispatcher"
except ImportError as exc:
    _HOOKS_ERROR = exc
    try:
        from crewai.hooks import (register_before_tool_call_hook,
                                  unregister_before_tool_call_hook,
                                  get_before_tool_call_hooks)
        DIALECT = "legacy"
    except ImportError as exc2:
        _HOOKS_ERROR = exc2


class _LegacyBlock(Exception):
    """Internal marker; only ever raised on the legacy dialect."""


class _BlockedByOperator(Exception):
    """Block carrier when crewai is absent (CLI/test mode only); never seen in a crew."""


def _allow():
    """The dialect's allow value (None proceeds; legacy False would block)."""
    return False if DIALECT == "legacy" else None


def _block(reason):
    """The dialect's block action, with the gate's reason where crewai carries one."""
    if DIALECT == "dispatcher":
        raise HookAborted(reason=reason, source="operator-lite")
    if DIALECT == "legacy":
        raise _LegacyBlock(reason)
    raise _BlockedByOperator(reason)


def _hook(ctx):
    """The PRE_TOOL_CALL hook crewai calls. Returns None to allow; blocks per dialect.

    crewai passes a ToolCallHookContext: .tool_name, .tool_input (a mutable dict),
    .tool, .agent, .task, .crew. Nothing here can raise an ordinary exception to
    block -- crewai swallows those (fail-open) -- so every failure below degrades
    explicitly per the unreachable-gate policy instead of relying on the swallow.
    """
    try:
        tool_name = getattr(ctx, "tool_name", "") or ""
        tool_input = getattr(ctx, "tool_input", None)
        if not is_gated(tool_name, tool_input):
            return _allow()
        command = extract_command(tool_input)
        if command is None:
            return _allow()
        session = getattr(getattr(ctx, "crew", None), "id", None)
        verdict, reason = gate_verdict(command, tool_name, session_id=session)
        if verdict == "allow":
            return _allow()
        if verdict == "block":
            return _block("Operator Lite blocked: %s" % reason)
        if looks_destructive(command):     # gate unreachable -> fail closed here
            return _block("Operator Lite blocked (gate unreachable and the command "
                          "looks destructive; ask the owner): %s" % reason)
        sys.stderr.write("operator-lite: gate unreachable, failing open (%s)\n" % reason)
        return _allow()
    except _LegacyBlock:                   # legacy dialect: contract is return False
        if _CLI_MODE:
            raise                          # CLI reports the block, not the allow
        return False


_CLI_MODE = False                       # set by the hookcheck CLI (blocks must surface)


def install():
    """Register the PRE_TOOL_CALL hook globally. Idempotent. True when wired."""
    global _installed
    if DIALECT is None:
        sys.stderr.write("operator-lite: crewai not importable (%s); nothing registered\n"
                         % _HOOKS_ERROR)
        return False
    if _installed:
        return True
    if DIALECT == "dispatcher":
        if _hook in get_hooks(InterceptionPoint.PRE_TOOL_CALL):
            _installed = True
            return True
        register_hook(InterceptionPoint.PRE_TOOL_CALL, _hook)
    else:
        if _hook in get_before_tool_call_hooks():
            _installed = True
            return True
        register_before_tool_call_hook(_hook)
    _installed = True
    return True


def uninstall():
    """Remove the hook this module registered. True when it was removed."""
    global _installed
    if DIALECT is None or not _installed:
        return False
    if DIALECT == "dispatcher":
        removed = unregister_hook(InterceptionPoint.PRE_TOOL_CALL, _hook)
    else:
        removed = unregister_before_tool_call_hook(_hook)
    _installed = False
    return removed


def _gate_mode():
    env = os.environ.get("OPERATOR_GATE_MODE")
    if env:
        return env
    home = os.path.expanduser("~")
    op_home = os.environ.get("OPERATOR_HOME", os.path.join(home, ".operator"))
    try:
        with open(os.path.join(op_home, "mode")) as f:
            return f.read().strip() or "unknown"
    except Exception:
        return "unset (gate defaults to enforce)"


def status():
    """One-line wiring report: dialect, wired, gate path, mode."""
    return "operator-lite: dialect=%s wired=%s gate=%s mode=%s" % (
        DIALECT or "none", _installed, resolve_gate() or "MISSING", _gate_mode())


# Auto-wire on import (the whole point of `import operator_hooks`); opt out with
# OPERATOR_CREWAI_AUTOINSTALL=0 and call install() yourself.
if DIALECT is not None and os.environ.get("OPERATOR_CREWAI_AUTOINSTALL", "1") != "0":
    install()


# ---- CLI: drive the exact hook decision path without crewai (test.sh uses this) ---- #
class _FakeCtx:
    """The slice of ToolCallHookContext the hook reads (tool_name/tool_input/crew)."""

    def __init__(self, tool_name, command):
        self.tool_name = tool_name
        self.tool_input = {"command": command}
        self.crew = None


def _main(argv):
    """hookcheck '<command>' [tool] -> exit 0 allow / 2 block (full _hook path);
    check '<command>' [tool] -> raw gate_verdict (0/2, 3 unreachable); status."""
    if argv[:1] == ["status"]:
        print(status())
        return 0
    if argv[:1] in (["check"], ["hookcheck"]) and len(argv) >= 2:
        tool = argv[2] if len(argv) > 2 else "bash"
        if argv[0] == "hookcheck":
            global _CLI_MODE
            _CLI_MODE = True
            try:
                _hook(_FakeCtx(tool, argv[1]))
            except _BlockedByOperator as exc:
                sys.stderr.write(str(exc) + "\n")
                return 2
            except Exception as exc:          # HookAborted on the dispatcher dialect
                reason = getattr(exc, "reason", None) or str(exc)
                if "Operator Lite blocked" not in reason:
                    raise
                sys.stderr.write(reason + "\n")
                return 2
            return 0
        verdict, reason = gate_verdict(argv[1], tool)
        if verdict == "allow":
            return 0
        sys.stderr.write(reason + "\n")
        return 2 if verdict == "block" else 3
    print("usage: operator_hooks.py hookcheck '<command>' [tool_name] | "
          "check '<command>' [tool_name] | status", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(_main(sys.argv[1:]))
