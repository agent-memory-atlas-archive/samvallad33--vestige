#!/bin/sh
# Operator Lite -- Goose port end-to-end test. POSIX sh, stdlib-only.
#
# Hermetic: builds a throwaway HOME (NOT under /tmp: the gate classifies system
# temp as scratch, which would never exercise OP-003), runs the real install.sh
# into it, then drives the REAL installed hook command the way goose runs hook
# commands (`sh -c` with PLUGIN_ROOT set) with payloads in the EXACT stdin
# shape goose documents for PreToolUse:
#   {"event","session_id","tool_name","tool_input":{"command"},"working_dir"}
#
# Nothing is ever executed: payloads are only piped into the hook as JSON, and
# the gate is analyzer-only. The EXIT trap removes every trace.
set -u

PORT_DIR=$(cd "$(dirname "$0")" && pwd)

fail=0
ok()  { printf 'PASS: %s\n' "$*"; }
bad() { printf 'FAIL: %s\n' "$*"; fail=1; }
section() { printf '\n== %s ==\n' "$*"; }

# ---- hermetic environment --------------------------------------------------- #
unset OPERATOR_AGENT_SESSION OPERATOR_GATE OPERATOR_HOME OPERATOR_PYTHON

TEST_BASE="$HOME/.opgate-goose-test.$$"
rm -rf "$TEST_BASE"
TEST_HOME="$TEST_BASE/home"; TEST_PROJ="$TEST_BASE/project"
LOGS="$TEST_BASE/logs"
mkdir -p "$TEST_HOME" "$TEST_PROJ" "$LOGS" || exit 1
TMPDIR="$TEST_BASE/tmp"; export TMPDIR
HOME="$TEST_HOME"; export HOME
trap 'rm -rf "$TEST_BASE"' EXIT

PLUGIN_DIR="$TEST_HOME/.agents/plugins/operator-lite"
HOOK_CMD='python3 "${PLUGIN_ROOT}/scripts/operator-hook.py"'
TODAY=$(date -u +%Y-%m-%d)
RECEIPTS="$TEST_HOME/.operator/receipts/$TODAY.jsonl"

gate_hook() { # $1 = payload json ; env: PLUGIN_ROOT ; echoes exit code
  printf '%s' "$1" | PLUGIN_ROOT="$PLUGIN_DIR" sh -c "$HOOK_CMD"
}

receipt_field() { # $1 = python expression over receipt dict r (last receipt)
  python3 - "$RECEIPTS" "$1" <<'PYEOF'
import json, sys
lines = open(sys.argv[1]).read().splitlines()
r = json.loads(lines[-1])
print(eval(sys.argv[2], {"r": r}))
PYEOF
}

# ---- 1. install.sh into the throwaway HOME ---------------------------------- #
section "install"
if sh "$PORT_DIR/install.sh" > "$LOGS/install.log" 2>&1; then
  ok "install.sh exited 0"
else
  bad "install.sh failed"; cat "$LOGS/install.log"; exit 1
fi
cat "$LOGS/install.log"

[ -f "$TEST_HOME/.operator/gate/operator-gate.py" ] && ok "gate installed" || bad "gate missing"
[ -x "$TEST_HOME/.operator/gate/operator-gate.py" ] && ok "gate is executable" || bad "gate not executable"
[ "$(cat "$TEST_HOME/.operator/mode" 2>/dev/null)" = "shadow" ] && ok "mode file = shadow" || bad "mode file not shadow"
[ -x "$PLUGIN_DIR/scripts/operator-hook.py" ] && ok "hook shim installed + executable" || bad "hook shim missing"

python3 - "$PLUGIN_DIR/hooks/hooks.json" <<'PYEOF' && ok "hooks.json valid + wired (\${PLUGIN_ROOT}, PreToolUse, timeout)" || bad "hooks.json wrong"
import json, sys
d = json.load(open(sys.argv[1]))
e = d["hooks"]["PreToolUse"][0]
h = e["hooks"][0]
assert h["type"] == "command", h
assert h["command"] == "${PLUGIN_ROOT}/scripts/operator-hook.py", h
assert h["timeout"] == 10, h
PYEOF

# ---- 2. shadow mode: receipts written, nothing blocked ----------------------- #
section "shadow mode (default)"
rc=$(gate_hook '{"event":"PreToolUse","session_id":"s1","tool_name":"shell","tool_input":{"command":"ls -la"},"working_dir":"'"$TEST_PROJ"'"}' >/dev/null 2>&1; echo $?)
[ "$rc" = "0" ] && ok "benign ls -> exit 0 (allowed)" || bad "benign ls exited $rc, want 0"

# ~/demo-factory under the throwaway HOME: outside the session cwd, so the
# gate's OP-003 applies (a recursive delete INSIDE cwd is the agent's normal
# job -- asserted further down). The path never exists; nothing is executed.
rc=$(gate_hook '{"event":"PreToolUse","session_id":"s1","tool_name":"shell","tool_input":{"command":"rm -rf ~/demo-factory/canary"},"working_dir":"'"$TEST_PROJ"'"}' >/dev/null 2>&1; echo $?)
[ "$rc" = "0" ] && ok "rm -rf in shadow -> exit 0 (logged, not blocked)" || bad "rm -rf in shadow exited $rc, want 0"
[ -f "$RECEIPTS" ] && ok "receipts file exists" || bad "no receipts file"
[ "$(receipt_field "r['decision']")" = "SHADOW_STOP" ] && ok "destructive call receipted as SHADOW_STOP" || bad "receipt decision: $(receipt_field "r['decision']")"
[ "$(receipt_field "','.join(r['commitments'])")" = "OP-003" ] && ok "shadow receipt carries OP-003" || bad "commitments: $(receipt_field "','.join(r['commitments'])")"

# ---- 3. enforce mode: the gate blocks, goose sees exit 2 + stderr ------------- #
section "enforce mode"
echo enforce > "$TEST_HOME/.operator/mode"

rc=$(gate_hook '{"event":"PreToolUse","session_id":"s2","tool_name":"shell","tool_input":{"command":"ls -la"},"working_dir":"'"$TEST_PROJ"'"}' >"$LOGS/benign.out" 2>"$LOGS/benign.err"; echo $?)
[ "$rc" = "0" ] && ok "benign ls -> exit 0 (allowed)" || bad "benign ls exited $rc, want 0"
[ ! -s "$LOGS/benign.out" ] && ok "benign allow keeps stdout empty (goose contract)" || bad "stdout not empty on allow"
[ ! -s "$LOGS/benign.err" ] && ok "benign allow writes no stderr" || bad "stderr noise on allow"

rc=$(gate_hook '{"event":"PreToolUse","session_id":"s3","tool_name":"shell","tool_input":{"command":"rm -rf ~/demo-factory/canary"},"working_dir":"'"$TEST_PROJ"'"}' >"$LOGS/rm.out" 2>"$LOGS/rm.err"; echo $?)
[ "$rc" = "2" ] && ok "plain rm -rf -> exit 2 (BLOCKED)" || bad "rm -rf exited $rc, want 2"
grep -q "OPERATOR: STOPPED" "$LOGS/rm.err" && ok "stderr carries OPERATOR: STOPPED reason (goose shows it)" || bad "no OPERATOR reason on stderr"
[ ! -s "$LOGS/rm.out" ] && ok "block keeps stdout empty (goose reads decisions from stderr)" || bad "stdout not empty on block"

rc=$(gate_hook '{"event":"PreToolUse","session_id":"s3b","tool_name":"shell","tool_input":{"command":"rm -rf ./src"},"working_dir":"'"$TEST_PROJ"'"}' >/dev/null 2>/dev/null; echo $?)
[ "$rc" = "0" ] && ok "recursive delete INSIDE cwd -> exit 0 (gate is precise, not trigger-happy)" || bad "in-cwd rm -rf exited $rc, want 0"

rc=$(gate_hook '{"event":"PreToolUse","session_id":"s4","tool_name":"shell","tool_input":{"command":"r'"''"'m -rf ~/demo-factory/canary"},"working_dir":"'"$TEST_PROJ"'"}' >/dev/null 2>"$LOGS/obs.err"; echo $?)
[ "$rc" = "2" ] && ok "obfuscated r''m -rf -> exit 2 (gate sees through quote reassembly)" || bad "obfuscated rm exited $rc, want 2"

rc=$(gate_hook '{"event":"PreToolUse","session_id":"s5","tool_name":"shell","tool_input":{"command":"echo OPERATOR_CANARY_STOP_7f3a"},"working_dir":"'"$TEST_PROJ"'"}' >/dev/null 2>/dev/null; echo $?)
[ "$rc" = "2" ] && ok "canary payload -> exit 2 (proves the gate ran)" || bad "canary exited $rc, want 2"

# goose tool naming: developer extension sends bare `shell`; receipt must show
# source=goose, tool=shell, and cwd translated from working_dir.
[ "$(receipt_field "r['source']")" = "goose" ] && ok "receipt source = goose" || bad "receipt source: $(receipt_field "r['source']")"
[ "$(receipt_field "r['tool']")" = "shell" ] && ok "receipt tool = shell (goose tool name)" || bad "receipt tool: $(receipt_field "r['tool']")"
[ "$(receipt_field "r['cwd']")" = "$TEST_PROJ" ] && ok "working_dir mapped to gate cwd" || bad "receipt cwd: $(receipt_field "r['cwd']")"

# namespaced MCP-style tool (most goose extensions send `{ext}__{tool}`)
rc=$(gate_hook '{"event":"PreToolUse","session_id":"s6","tool_name":"proxy__exec","tool_input":{"command":"rm -rf ~/demo-factory/canary"},"working_dir":"'"$TEST_PROJ"'"}' >/dev/null 2>/dev/null; echo $?)
[ "$rc" = "2" ] && ok "namespaced tool proxy__exec with rm -rf -> exit 2" || bad "namespaced tool exited $rc, want 2"

# non-shell tool: forwarded, gate finds nothing shell-like, allows
rc=$(gate_hook '{"event":"PreToolUse","session_id":"s7","tool_name":"write","tool_input":{"path":"notes.md","content":"hi"},"working_dir":"'"$TEST_PROJ"'"}' >/dev/null 2>/dev/null; echo $?)
[ "$rc" = "0" ] && ok "write tool (no command) -> exit 0 (gate decides relevance)" || bad "write tool exited $rc, want 0"

# ---- 4. degraded gate: fail-open, fail-closed for plainly destructive --------- #
section "gate unreachable"
rc=$(printf '%s' '{"event":"PreToolUse","tool_name":"shell","tool_input":{"command":"rm -rf /"},"working_dir":"/"}' | PLUGIN_ROOT="$PLUGIN_DIR" OPERATOR_GATE="$TEST_BASE/missing-gate.py" sh -c "$HOOK_CMD" >/dev/null 2>/dev/null; echo $?)
[ "$rc" = "2" ] && ok "gate missing + plainly destructive text -> exit 2 (fail closed)" || bad "degraded rm exited $rc, want 2"

rc=$(printf '%s' '{"event":"PreToolUse","tool_name":"shell","tool_input":{"command":"ls -la"},"working_dir":"/"}' | PLUGIN_ROOT="$PLUGIN_DIR" OPERATOR_GATE="$TEST_BASE/missing-gate.py" sh -c "$HOOK_CMD" >/dev/null 2>/dev/null; echo $?)
[ "$rc" = "0" ] && ok "gate missing + benign text -> exit 0 (fail open)" || bad "degraded benign exited $rc, want 0"

rc=$(printf 'not json at all' | PLUGIN_ROOT="$PLUGIN_DIR" sh -c "$HOOK_CMD" >/dev/null 2>/dev/null; echo $?)
[ "$rc" = "0" ] && ok "malformed stdin -> exit 0 (no decision, goose allows)" || bad "malformed stdin exited $rc, want 0"

# ---- 5. real receipts verification (hash chain) ------------------------------- #
section "receipt chain"
chain=$(python3 "$TEST_HOME/.operator/gate/operator-gate.py" verify 2>/dev/null | head -1)
case "$chain" in
  *"chain=OK"*) ok "receipt hash chain verifies ($chain)" ;;
  *) bad "receipt chain: $chain" ;;
esac

printf '\n'
if [ "$fail" = 0 ]; then
  echo "ALL TESTS PASSED (hermetic sandbox removed; nothing outside it touched)."
  echo "UNTESTED: a live goose session (goose not installed on this machine) --"
  echo "hook discovery and goose-side stderr display should be confirmed by"
  echo "running install.sh and the canary step inside goose itself."
else
  echo "SOME TESTS FAILED."
fi
exit "$fail"
