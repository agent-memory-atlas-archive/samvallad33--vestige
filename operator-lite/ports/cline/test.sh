#!/bin/sh
# Gate-contract test for the Cline port of Operator Lite.
#
# Feeds three Cline PreToolUse-shaped payloads through the real adapter
# (./PreToolUse) and the real gate, in an isolated OPERATOR_HOME so your
# ~/.operator receipts and mode are never touched. The test command strings
# are only ever serialized as JSON and *classified* -- nothing is executed,
# by the test or by the gate.
#
#   case 1: plain rm -rf of a workspace root   -> block (gate exit 2)
#   case 2: quote-obfuscated r''m -rf          -> block (gate exit 2)
#   case 3: benign ls -la                      -> allow  (gate exit 0)
#
# What this script CANNOT test locally (marked UNTESTED below): the hook
# firing inside a live Cline session, and `cline plugin install`.

set -u

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
HOOK="$SCRIPT_DIR/PreToolUse"
REPO_GATE="$SCRIPT_DIR/../../operator-gate.py"

command -v python3 >/dev/null 2>&1 || { echo "FATAL: python3 required"; exit 1; }
command -v node   >/dev/null 2>&1 || { echo "FATAL: node required"; exit 1; }
[ -f "$HOOK" ] || { echo "FATAL: $HOOK missing"; exit 1; }
[ -f "$REPO_GATE" ] || { echo "FATAL: $REPO_GATE missing"; exit 1; }

T=$(mktemp -d "${TMPDIR:-/tmp}/opgate-cline-test.XXXXXX") || exit 1
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/gate"
cp "$REPO_GATE" "$T/gate/operator-gate.py"
chmod 755 "$T/gate/operator-gate.py"
echo enforce > "$T/mode"          # in the TEMP home only, so blocks actually block
export OPERATOR_HOME="$T"
export OPERATOR_GATE_PY="$T/gate/operator-gate.py"

failures=0

# run_case <n> <command-string> <expect: block|allow>
run_case() {
    n="$1"; cmd="$2"; expect="$3"
    payload=$(printf '{"hookName":"tool_call","taskId":"test-task","clineVersion":"test","userId":"test","agent_id":"test","parent_agent_id":null,"iteration":1,"workspaceRoots":["/tmp"],"tool_call":{"id":"t%s","name":"run_commands","input":{"command":"%s"}},"preToolUse":{"toolName":"run_commands","parameters":{"command":"%s"}}}' "$n" "$cmd" "$cmd")

    out=$(printf '%s' "$payload" | "$HOOK" 2>/dev/null)
    hook_exit=$?

    cancel=$(printf '%s' "$out" | OPERATOR_GATE_PY="$T/gate/operator-gate.py" python3 -c 'import json,sys
try:
    print(str(json.load(sys.stdin).get("cancel", False)).lower())
except Exception:
    print("parse-error")')

    # The raw gate contract for the same call (exit code 0/2).
    gate_payload=$(printf '{"tool_name":"run_commands","tool_input":{"command":"%s"},"cwd":"/tmp","session_id":"test-task"}' "$cmd")
    printf '%s' "$gate_payload" | python3 "$T/gate/operator-gate.py" hook --source cline >/dev/null 2>"$T/err.txt"
    gate_exit=$?
    reason=$(head -n 1 "$T/err.txt")

    if [ "$expect" = "block" ]; then
        ok_cancel=$([ "$cancel" = "true" ] && echo yes || echo no)
        ok_gate=$([ "$gate_exit" = "2" ] && echo yes || echo no)
    else
        ok_cancel=$([ "$cancel" = "false" ] && echo yes || echo no)
        ok_gate=$([ "$gate_exit" = "0" ] && echo yes || echo no)
    fi

    printf 'case %s: %-22s hook_exit=%s cancel=%s gate_exit=%s\n' "$n" "$expect" "$hook_exit" "$cancel" "$gate_exit"
    printf '         gate verdict: %s\n' "${reason:-<none>}"
    if [ "$ok_cancel" = "yes" ] && [ "$ok_gate" = "yes" ] && [ "$hook_exit" = "0" ]; then
        printf '         PASS\n'
    else
        printf '         FAIL (expected %s: cancel=%s gate_exit=%s)\n' "$expect" \
            "$([ "$expect" = block ] && echo true || echo false)" \
            "$([ "$expect" = block ] && echo 2 || echo 0)"
        failures=$((failures + 1))
    fi
    printf '\n'
}

echo "operator-lite / cline port -- gate contract test"
echo "adapter: $HOOK"
echo "gate:    $T/gate/operator-gate.py (isolated OPERATOR_HOME, mode=enforce)"
echo "note:    the adapter always exits 0 -- blocking is signaled via the"
echo "         stdout control JSON ({\"cancel\": true, ...}), per Cline's"
echo "         verified hook protocol. Gate exit codes show the raw contract."
echo ""

# $HOME expanded at payload-build time; strings are data, never executed.
run_case 1 "rm -rf $HOME/Developer/vestige" block
run_case 2 "r''m -rf $HOME/Developer/vestige" block
run_case 3 "ls -la" allow

echo "receipt chain in isolated home:"
python3 "$T/gate/operator-gate.py" verify | head -n 1

echo ""
echo "--- cline CLI probe ---"
if command -v cline >/dev/null 2>&1; then
    cline --version 2>/dev/null || echo "cline present but --version failed"
    if cline plugin install --help >/dev/null 2>&1 || cline plugin --help >/dev/null 2>&1; then
        echo "cline plugin subcommand: present (actual install left to install.sh / you)"
    else
        echo "cline present but no 'plugin' subcommand: UNTESTED route on this Cline build"
    fi
else
    echo "cline CLI: not found locally -> 'cline plugin install' route is UNTESTED here."
    echo "           In-session hook firing (VS Code extension) is also UNTESTED here;"
    echo "           the adapter and gate contract above are what this machine can prove."
fi

if [ "$failures" -gt 0 ]; then
    echo "RESULT: $failures case(s) FAILED"
    exit 1
fi
echo "RESULT: all 3 cases PASS"
