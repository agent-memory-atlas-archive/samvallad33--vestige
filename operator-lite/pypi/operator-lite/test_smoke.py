#!/usr/bin/env python3
"""Smoke test for the operator-lite PyPI package.

Verifies the gate CONTRACT against an installed gate (skips cleanly when
there is none): stdin payload shape, exit 0 = allow, exit 2 = block with the
reason on stderr, shadow = log-only with receipts, and the package's own
decide() path. Nothing is ever executed: command strings are only piped to
the gate as JSON, and the gate is analyzer-only.

Hermetic, like the ports' test.sh: a throwaway HOME is built under the REAL
$HOME (never /tmp or $TMPDIR -- the gate classifies system temp as scratch,
which would weaken OP-003), its mode file says `shadow` (so the test passes
regardless of the host's real mode; the real ~/.operator is never touched),
and every trace is removed on exit. Run standalone:

    python3 test_smoke.py

or under pytest:

    pytest test_smoke.py
"""
import contextlib
import glob
import json
import os
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)  # allow runs from any cwd before `pip install -e .`

import operator_lite
from operator_lite import _core

try:
    import pytest
    _HAS_PYTEST = True
except ImportError:
    pytest = None
    _HAS_PYTEST = False

GATE = _core.resolve_gate()
HAS_GATE = bool(GATE and os.path.exists(GATE))
RECEIPTS_GLOB = "receipts/%s.jsonl" % time.strftime("%Y-%m-%d", time.gmtime())

BENIGN = "cargo test -p vestige-mcp"
DESTRUCTIVE = "rm -rf ~/Developer/vestige"

_results = []


def _skipif_no_gate(fn):
    if _HAS_PYTEST:
        return pytest.mark.skipif(
            not HAS_GATE,
            reason="no operator-gate installed (looked in $OPERATOR_GATE, "
                   "~/.operator/gate/operator-gate.py)")(fn)
    return fn


@contextlib.contextmanager
def hermetic_shadow_home(enforce=False):
    """A throwaway operator home under the real $HOME, mode=shadow (or
    enforce), with HOME/OPERATOR_HOME/OPERATOR_GATE pointed at it for the
    duration. Restores the environment and deletes every trace."""
    real_home = os.path.expanduser("~")
    base = os.path.join(real_home, ".operator-lite-pypi-smoke.%d" % os.getpid())
    home = os.path.join(base, "home")
    op_home = os.path.join(home, ".operator")
    os.makedirs(os.path.join(op_home, "gate"), exist_ok=True)
    with open(os.path.join(op_home, "mode"), "w", encoding="utf-8") as f:
        f.write("enforce\n" if enforce else "shadow\n")
    keys = ("HOME", "OPERATOR_HOME", "OPERATOR_GATE", "OPERATOR_GATE_MODE",
            "OPERATOR_AGENT_SESSION", "OPERATOR_PYTHON")
    old = {k: os.environ.get(k) for k in keys}
    os.environ["HOME"] = home
    os.environ["OPERATOR_HOME"] = op_home
    os.environ["OPERATOR_GATE"] = GATE
    for k in ("OPERATOR_GATE_MODE", "OPERATOR_AGENT_SESSION"):
        os.environ.pop(k, None)
    try:
        yield {"base": base, "home": home, "op_home": op_home}
    finally:
        for key, value in old.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value
        shutil.rmtree(base, ignore_errors=True)


def run_hook(payload, hermetic):
    """Spawn the gate with its exact hook contract; env is already hermetic."""
    return subprocess.run(
        [sys.executable, GATE, "hook", "--source", "pypi-smoke"],
        input=json.dumps(payload).encode("utf-8"),
        capture_output=True, timeout=15, env=dict(os.environ))


def receipts(hermetic):
    return glob.glob(os.path.join(hermetic["op_home"], RECEIPTS_GLOB))


def receipt_decisions(hermetic):
    out = []
    for path in receipts(hermetic):
        with open(path, encoding="utf-8") as f:
            for line in f:
                try:
                    out.append(json.loads(line).get("decision"))
                except Exception:
                    pass
    return out


# --------------------------------------------------------------------------- #
# tests (pytest collects these; __main__ below runs the same functions)
# --------------------------------------------------------------------------- #
@_skipif_no_gate
def test_gate_parses_and_declares_version():
    assert _core.gate_version(GATE), "gate does not parse or has no VERSION"


@_skipif_no_gate
def test_import_stays_framework_free():
    """import operator_lite must not pull langchain/crewai/agents; lazy
    attribute access resolves the adapter module but still not the SDK."""
    for mod in ("langchain", "crewai", "agents"):
        assert mod not in sys.modules, \
            "importing operator_lite eagerly imported %r" % mod
    assert callable(operator_lite.gate_decision)   # lazy resolution works
    assert "agents" not in sys.modules             # SDK still untouched


@_skipif_no_gate
def test_hook_contract_allow_in_shadow():
    """Benign command -> exit 0, silent, and NO stop receipt (shadow logs
    only what it would stop)."""
    with hermetic_shadow_home() as hermetic:
        proc = run_hook({"tool_name": "Bash",
                         "tool_input": {"command": BENIGN},
                         "cwd": hermetic["home"]}, hermetic)
        assert proc.returncode == 0, "benign command did not allow: %s" % \
            proc.stderr.decode("utf-8", "replace")
        assert proc.stdout.strip() == b""
        hard_stops = [d for d in (receipt_decisions(hermetic) or [])
                      if d != "SHADOW_STOP"]
        assert not hard_stops, "benign command drew hard stops: %s" % hard_stops


@_skipif_no_gate
def test_hook_contract_shadow_is_log_only():
    """Destructive command under shadow mode -> still exit 0 (the model is
    never blocked), with a SHADOW_STOP receipt on the chain."""
    with hermetic_shadow_home() as hermetic:
        proc = run_hook({"tool_name": "Bash",
                         "tool_input": {"command": DESTRUCTIVE},
                         "cwd": hermetic["home"]}, hermetic)
        assert proc.returncode == 0, \
            "shadow mode blocked; it must only log: %s" % \
            proc.stderr.decode("utf-8", "replace")
        assert "SHADOW_STOP" in (receipt_decisions(hermetic) or ["<none>"]), \
            "shadow run wrote no SHADOW_STOP receipt under %s" % hermetic["op_home"]


@_skipif_no_gate
def test_hook_contract_block_is_exit_2():
    """The same command with the hermetic mode flipped to enforce -> exit 2,
    reason on stderr: the block form every host surfaces to the model."""
    with hermetic_shadow_home(enforce=True) as hermetic:
        proc = run_hook({"tool_name": "Bash",
                         "tool_input": {"command": DESTRUCTIVE},
                         "cwd": hermetic["home"]}, hermetic)
        assert proc.returncode == 2, \
            "enforce mode did not block (want exit 2, got %s)" % proc.returncode
        assert b"OPERATOR: STOPPED" in proc.stderr


@_skipif_no_gate
def test_package_decide_path():
    """The package's full decision path (resolve -> spawn -> verdict) with
    tool args in the exact shape the adapters receive them."""
    with hermetic_shadow_home() as hermetic:
        verdict, message = operator_lite.decide(
            "shell", {"command": BENIGN}, cwd=hermetic["home"])
        assert verdict == operator_lite.ALLOW, (verdict, message)
        verdict, message = operator_lite.decide(
            "shell", {"command": DESTRUCTIVE}, cwd=hermetic["home"])
        assert verdict == operator_lite.ALLOW, \
            "shadow mode must log, not block (got %s: %s)" % (verdict, message)
        assert "SHADOW_STOP" in (receipt_decisions(hermetic) or ["<none>"])


@_skipif_no_gate
def test_unreachable_gate_fails_closed_on_destructive():
    """No gate reachable + plainly destructive text -> the package blocks
    (the ports' shared fail-closed safety net)."""
    keys = ("OPERATOR_GATE", "OPERATOR_HOME")
    old = {k: os.environ.get(k) for k in keys}
    os.environ["OPERATOR_GATE"] = os.path.join(
        os.path.expanduser("~"), ".operator-lite-no-such-gate.py")
    os.environ["OPERATOR_HOME"] = os.path.join(
        os.path.expanduser("~"), ".operator-lite-no-such-home")
    try:
        verdict, message = operator_lite.decide(
            "shell", {"command": DESTRUCTIVE})
        assert verdict == operator_lite.BLOCK, (verdict, message)
        verdict, _ = operator_lite.decide("shell", {"command": BENIGN})
        assert verdict == operator_lite.ALLOW  # fail-open for the benign case
    finally:
        for key, value in old.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value


@_skipif_no_gate
def test_corpus_replay_43_of_43():
    """When the gate is a repo checkout (corpora/ next to it), the gate's own
    GuardFall corpus must pass 43/43 through classify()."""
    corpus = os.path.join(os.path.dirname(GATE), "corpora", "guardfall.json")
    if not os.path.exists(corpus):
        # Installed-copy case: ~/.operator/gate/ ships without corpora/.
        if os.environ.get("PYTEST_CURRENT_TEST") and _HAS_PYTEST:
            pytest.skip("no corpora/ next to %s (installed-copy case)" % GATE)
        print("SKIP: no corpora/ next to the gate; corpus replay not applicable")
        return
    with hermetic_shadow_home() as hermetic:
        proc = subprocess.run([sys.executable, GATE, "corpus", "guardfall"],
                              capture_output=True, timeout=60,
                              env=dict(os.environ))
        assert proc.returncode == 0, proc.stderr.decode("utf-8", "replace")
        assert b"passed=43" in proc.stdout and b"failed=0" in proc.stdout, \
            proc.stdout.decode("utf-8", "replace")


# --------------------------------------------------------------------------- #
# standalone runner
# --------------------------------------------------------------------------- #
def main():
    tests = [(name, fn) for name, fn in sorted(globals().items())
             if name.startswith("test_") and callable(fn)]
    if not HAS_GATE:
        print("SKIP: no operator-gate installed (looked in $OPERATOR_GATE, "
              "~/.operator/gate/operator-gate.py).")
        print("      Install it with: operator_lite.ensure_gate() -- or the "
              "repo's ports/<host>/install.sh")
        return 0
    failures = 0
    for name, fn in tests:
        try:
            fn()
            print("PASS: %s" % name)
        except AssertionError as exc:
            failures += 1
            print("FAIL: %s\n      %s" % (name, exc))
        except Exception as exc:  # noqa: BLE001 - report, don't crash the sweep
            failures += 1
            print("FAIL: %s (unexpected %s)\n      %s"
                  % (name, type(exc).__name__, exc))
    print("\noperator-lite smoke: %d/%d passed (gate: %s)"
          % (len(tests) - failures, len(tests), GATE))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
