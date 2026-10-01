"""CrewAI adapter (port of operator-lite/ports/crewai/operator_hooks.py onto
operator_lite._core).

    import operator_lite
    operator_lite.crewai_install()   # registers a global PRE_TOOL_CALL hook

Per tool call the gate runs as a subprocess; blocking raises the dialect's
block form. Verified against crewAI source in the port (lib/crewai hooks/):
`HookAborted` is special-cased in dispatch._invoke_hook and propagates by
design; any OTHER exception a hook raises is swallowed (fail-open) -- which is
exactly why this adapter never relies on an ordinary exception to block.

    exit 0      -> allow  (hook returns None)
    exit 2      -> block  (HookAborted("Operator Lite blocked: <gate stderr>"))
    unreachable -> fail-open, unless the command matches the destructive
                   safety net: fail closed instead

crewai is imported at module level (dialect detection), which is safe because
operator_lite only imports this module lazily, on first use of the crewai
surface.
"""
import os
import re
import sys

from .. import _core

SOURCE = "crewai"

# crewai sanitizes tool names (lowercase a-z0-9_), so match lowercase.
SHELLISH_NAME_RE = re.compile(r"shell|command|terminal")

# Dispatcher (current crewai) vs legacy registries; detected at import.
DIALECT = None
_HOOKS_ERROR = None
try:
    from crewai.hooks import (HookAborted, InterceptionPoint, get_hooks,
                              register_hook, unregister_hook)
    DIALECT = "dispatcher"
except ImportError as exc:
    _HOOKS_ERROR = exc
    try:
        from crewai.hooks import (get_before_tool_call_hooks,
                                  register_before_tool_call_hook,
                                  unregister_before_tool_call_hook)
        DIALECT = "legacy"
    except ImportError as exc2:
        _HOOKS_ERROR = exc2


class _LegacyBlock(Exception):
    """Internal marker; only ever raised on the legacy dialect."""


class _BlockedByOperator(Exception):
    """Block carrier when crewai is absent (CLI/test mode only)."""


def is_gated(tool_name, tool_input):
    """Shell-ish by name (...shell/command/terminal...) or by payload."""
    if SHELLISH_NAME_RE.search(str(tool_name or "").lower()):
        return True
    return _core.extract_command(tool_input) is not None


def looks_destructive(command):
    """Gate-unreachable policy: fail closed only for plainly destructive text
    (the shared safety net in _core, which includes shell-rc writes)."""
    return bool(_core.DESTRUCTIVE_LIKE.search(_core._command_text(command)))


def _allow():
    """The dialect's allow value (None proceeds; legacy False would block)."""
    return False if DIALECT == "legacy" else None


def _block(reason):
    if DIALECT == "dispatcher":
        raise HookAborted(reason=reason, source="operator-lite")
    if DIALECT == "legacy":
        raise _LegacyBlock(reason)
    raise _BlockedByOperator(reason)


def hook(ctx):
    """The PRE_TOOL_CALL hook crewai calls. Returns None to allow; blocks per
    dialect. crewai passes ToolCallHookContext: .tool_name, .tool_input
    (mutable dict), .tool, .agent, .task, .crew."""
    tool_name = getattr(ctx, "tool_name", "") or ""
    tool_input = getattr(ctx, "tool_input", None)
    if not is_gated(tool_name, tool_input):
        return _allow()
    command = _core.extract_command(tool_input)
    if command is None:
        return _allow()
    session = getattr(getattr(ctx, "crew", None), "id", None)
    verdict, reason = _core.gate_verdict(command, tool_name=tool_name,
                                         session_id=session, source=SOURCE)
    if verdict == _core.ALLOW:
        return _allow()
    if verdict == _core.BLOCK:
        return _block("Operator Lite blocked: %s" % reason)
    if looks_destructive(command):      # gate unreachable -> fail closed here
        return _block("Operator Lite blocked (gate unreachable and the command "
                      "looks destructive; ask the owner): %s" % reason)
    sys.stderr.write("operator-lite: gate unreachable, failing open (%s)\n"
                     % reason)
    return _allow()


# legacy dialect's contract is return False, not raise
def _hook(ctx):
    try:
        return hook(ctx)
    except _LegacyBlock:
        if _CLI_MODE:
            raise
        return False


_CLI_MODE = False       # set by hookcheck (blocks must surface)
_installed = False


def install():
    """Register the PRE_TOOL_CALL hook globally. Idempotent. True when wired."""
    global _installed
    if DIALECT is None:
        sys.stderr.write("operator-lite: crewai not importable (%s); "
                         "nothing registered\n" % _HOOKS_ERROR)
        return False
    if _installed:
        return True
    if DIALECT == "dispatcher":
        if hook in get_hooks(InterceptionPoint.PRE_TOOL_CALL):
            _installed = True
            return True
        register_hook(InterceptionPoint.PRE_TOOL_CALL, hook)
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
    removed = False
    if DIALECT == "dispatcher":
        removed = unregister_hook(InterceptionPoint.PRE_TOOL_CALL, hook)
    else:
        removed = unregister_before_tool_call_hook(_hook)
    _installed = False
    return removed


def status():
    """One-line wiring report: dialect, wired, gate path, mode."""
    gate = _core.resolve_gate()
    present = os.path.exists(gate)
    mode = ""
    try:
        with open(os.path.join(_core.operator_home(), "mode"),
                  encoding="utf-8") as f:
            mode = f.read().strip()
    except OSError:
        pass
    return ("operator-lite: dialect=%s wired=%s gate=%s mode=%s"
            % (DIALECT or "none", _installed,
               gate if present else "MISSING", mode or "unset(enforce)"))


class _FakeCtx:
    """The slice of ToolCallHookContext the hook reads (tests/CLI only)."""

    def __init__(self, tool_name, command):
        self.tool_name = tool_name
        self.tool_input = {"command": command}
        self.crew = None


def _main(argv):
    """CLI driving the exact hook decision path without crewai:
    hookcheck '<command>' [tool] -> exit 0 allow / 2 block."""
    if argv[:1] == ["status"]:
        print(status())
        return 0
    if argv[:1] == ["hookcheck"] and len(argv) >= 2:
        global _CLI_MODE
        _CLI_MODE = True
        try:
            hook(_FakeCtx(argv[2] if len(argv) > 2 else "bash", argv[1]))
        except _BlockedByOperator as exc:
            sys.stderr.write(str(exc) + "\n")
            return 2
        except Exception as exc:    # HookAborted on the dispatcher dialect
            reason = getattr(exc, "reason", None) or str(exc)
            if "Operator Lite blocked" not in reason:
                raise
            sys.stderr.write(reason + "\n")
            return 2
        return 0
    print("usage: python -m operator_lite.adapters.crewai hookcheck "
          "'<command>' [tool_name] | status", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(_main(sys.argv[1:]))
