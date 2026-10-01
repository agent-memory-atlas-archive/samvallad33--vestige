#!/bin/sh
# Operator Lite -- OpenAI Agents SDK (Python) port end-to-end test.
# POSIX sh, stdlib-only.
#
# Hermetic: builds a throwaway HOME (under $HOME, NEVER /tmp or $TMPDIR --
# the gate classifies system temp as scratch, which would weaken OP-003),
# runs the real install.sh into it, then drives the module's gate-call path
# directly with payloads in the gate's stdin contract:
#   {"tool_name", "tool_input": {"command"}, "cwd", "session_id"}
#
# Nothing is ever executed: command strings are only JSON-encoded into the
# gate's stdin, and the gate is analyzer-only. The EXIT trap removes every
# trace. If the openai-agents package is genuinely installed, a real gated
# tool is built and its guardrail deny form asserted; otherwise the suite
# says UNTESTED for that part, honestly.
set -u

PORT_DIR=$(cd "$(dirname "$0")" && pwd)

fail=0
ok()  { printf 'PASS: %s\n' "$*"; }
bad() { printf 'FAIL: %s\n' "$*"; fail=1; }
section() { printf '\n== %s ==\n' "$*"; }

# ---- hermetic environment --------------------------------------------------- #
unset OPERATOR_AGENT_SESSION OPERATOR_GATE OPERATOR_GATE_MODE OPERATOR_HOME OPERATOR_PYTHON

TEST_BASE="$HOME/.opgate-oai-agents-test.$$"
rm -rf "$TEST_BASE"
TEST_HOME="$TEST_BASE/home"
TEST_PROJ="$TEST_BASE/project"           # payload cwd: NOT under the temp HOME,
LOGS="$TEST_BASE/logs"                   #  NOT system temp (see OP-003 breadth rule)
mkdir -p "$TEST_HOME" "$TEST_PROJ" "$LOGS" || exit 1
TMPDIR="$TEST_BASE/tmp"; export TMPDIR
HOME="$TEST_HOME"; export HOME
trap 'rm -rf "$TEST_BASE"' EXIT

MOD_DIR="$TEST_HOME/.operator/ports/openai-agents"
GATE_DST="$TEST_HOME/.operator/gate/operator-gate.py"
TODAY=$(date -u +%Y-%m-%d)
RECEIPTS="$TEST_HOME/.operator/receipts/$TODAY.jsonl"

# the driver: module gate-call path only, prints outcome line + reason line
DRIVER="$TEST_BASE/drive.py"
cat > "$DRIVER" <<'PYEOF'
import json, os, sys
sys.path.insert(0, os.environ["MOD_DIR"])
import operator_guard as g
tool = sys.argv[1]
command = sys.argv[2]
assert g.extract_command('{"command": "ls -la"}') == "ls -la"
d = g.gate_decision(tool_name=tool, command=command,
                    cwd=os.environ["PCWD"], session_id="test-session")
print("outcome=%s exit=%s" % (d.outcome, d.exit_code if d.exit_code is not None else "-"))
# line 2 is what the model would see as the tool result on a block
print((g.deny_message(d.reason) if d.blocked else d.reason).replace("\n", " | "))
PYEOF

drive() { # drive <tool> <command> [tag]
  PCWD="$TEST_PROJ" MOD_DIR="$MOD_DIR" python3 "$DRIVER" "$1" "$2" > "$LOGS/$3.out" 2> "$LOGS/$3.err"
}
drv_outcome() { sed -n '1s/outcome=\([a-z]*\) .*/\1/p' "$LOGS/$1.out"; }
drv_exit()    { sed -n '1s/.*exit=\([0-9-]*\)/\1/p' "$LOGS/$1.out"; }
drv_reason()  { sed -n '2p' "$LOGS/$1.out"; }

receipt_field() { # receipt_field <jsonl-path> <key> -> value of the key in the last receipt
  python3 - "$1" "$2" <<'PYEOF'
import json, os, sys
path, key = sys.argv[1], sys.argv[2]
if not os.path.exists(path):
    print("(none)")
else:
    lines = [l for l in open(path).read().splitlines() if l.strip()]
    if not lines:
        print("(none)")
    else:
        v = json.loads(lines[-1]).get(key, "")
        print(",".join(v) if isinstance(v, list) else v)
PYEOF
}

# ---- 1. install.sh into the throwaway HOME ----------------------------------- #
section "install (fresh)"
if sh "$PORT_DIR/install.sh" > "$LOGS/install.log" 2>&1; then
  ok "install.sh exited 0"
else
  bad "install.sh failed"; cat "$LOGS/install.log"; exit 1
fi
cat "$LOGS/install.log"

[ -f "$GATE_DST" ] && ok "gate installed" || bad "gate missing"
GATE_VER=$(sed -n 's/^VERSION = *"\([^"]*\)".*/\1/p' "$GATE_DST" 2>/dev/null | head -n 1)
[ "$GATE_VER" != "" ] && ok "gate carries a VERSION ($GATE_VER)" || bad "gate VERSION unreadable"
[ -f "$MOD_DIR/operator_guard.py" ] && ok "guardrail module installed to $MOD_DIR" || bad "module not installed"
[ "$(cat "$TEST_HOME/.operator/mode" 2>/dev/null)" = "shadow" ] && ok "mode file = shadow" || bad "mode file not shadow"

# ---- 2. stdlib-only surface of the INSTALLED module --------------------------- #
section "module surface, stdlib-only"
python3 - "$MOD_DIR" <<'PYEOF' && ok "installed module imports; extraction + safety-net regex behave" || bad "module surface broken"
import sys
sys.path.insert(0, sys.argv[1])
import operator_guard as g
assert g.extract_command('{"command": "rm -rf x"}') == "rm -rf x"
assert g.extract_command('{"cmd": "ls"}') == "ls"
assert g.extract_command('{"other": 1}') is None
assert g.extract_command("not json but shellish") == "not json but shellish"
assert g.extract_command('{"command": ["git", "status; rm x"]}') == "git 'status; rm x'"
assert g.looks_destructive("rm -rf ~/demo-factory/canary")
assert g.looks_destructive("echo hi | tee ~/.zshrc")
assert g.looks_destructive("echo x > ~/.bashrc")
assert g.looks_destructive("git push --force origin main")
assert not g.looks_destructive("ls -la")
assert g.deny_message("OPERATOR: STOPPED (OP-003 x)") == "Operator Lite blocked: OPERATOR: STOPPED (OP-003 x)"
d = g.describe()
assert d["gate"] and isinstance(d["gate_present"], bool)
PYEOF

# ---- 3. shadow mode (default): receipts written, nothing blocked -------------- #
section "shadow mode (default)"
drive run_shell "ls -la" s-ls
[ "$(drv_outcome s-ls)" = "allow" ] && ok "benign ls -> allow" || bad "benign ls: $(drv_outcome s-ls)"

drive run_shell "rm -rf ~/demo-factory/canary" s-rm
[ "$(drv_outcome s-rm)" = "allow" ] && ok "rm -rf in shadow -> allow (logged, not blocked)" || bad "rm -rf in shadow: $(drv_outcome s-rm)"
[ -f "$RECEIPTS" ] && ok "receipts file exists" || bad "no receipts file"
[ "$(receipt_field "$RECEIPTS" decision)" = "SHADOW_STOP" ] && ok "destructive call receipted as SHADOW_STOP" || bad "receipt decision: $(receipt_field "$RECEIPTS" decision)"
[ "$(receipt_field "$RECEIPTS" commitments)" = "OP-003" ] && ok "shadow receipt carries OP-003" || bad "commitments: $(receipt_field "$RECEIPTS" commitments)"
[ "$(receipt_field "$RECEIPTS" source)" = "openai-agents" ] && ok "receipt source = openai-agents" || bad "receipt source: $(receipt_field "$RECEIPTS" source)"
[ "$(receipt_field "$RECEIPTS" tool)" = "run_shell" ] && ok "receipt tool = run_shell (real tool name plumbed)" || bad "receipt tool: $(receipt_field "$RECEIPTS" tool)"

drive run_shell "r''m -rf ~/demo-factory/canary" s-obs
[ "$(receipt_field "$RECEIPTS" decision)" = "SHADOW_STOP" ] && ok "obfuscated r''m receipted in shadow too (gate reassembles quotes)" || bad "obfuscated shadow receipt: $(receipt_field "$RECEIPTS" decision)"

# ---- 4. enforce mode: the documented flip, then real blocks ------------------- #
section "enforce mode"
echo enforce > "$TEST_HOME/.operator/mode"

drive run_shell "rm -rf ~/demo-factory/canary" e-rm
[ "$(drv_outcome e-rm)" = "block" ] && ok "plain rm -rf -> BLOCKED" || bad "rm -rf: $(drv_outcome e-rm)"
[ "$(drv_exit e-rm)" = "2" ] && ok "gate exit code 2 carried through" || bad "exit: $(drv_exit e-rm)"
case "$(drv_reason e-rm)" in
  "Operator Lite blocked: "*) ok "deny form exact: 'Operator Lite blocked: <gate stderr>'" ;;
  *) bad "deny prefix wrong: $(drv_reason e-rm)" ;;
esac
case "$(drv_reason e-rm)" in
  *"OPERATOR: STOPPED"*"OP-003"*) ok "reason carries the gate's verdict (OPERATOR: STOPPED, OP-003)" ;;
  *) bad "reason missing gate verdict: $(drv_reason e-rm)" ;;
esac

drive run_shell "r''m -rf ~/demo-factory/canary" e-obs
[ "$(drv_outcome e-obs)" = "block" ] && ok "obfuscated r''m -rf -> BLOCKED (gate sees through quote reassembly)" || bad "obfuscated: $(drv_outcome e-obs)"

drive run_shell "echo OPERATOR_CANARY_STOP_7f3a" e-canary
[ "$(drv_outcome e-canary)" = "block" ] && ok "canary -> BLOCKED (whole chain is wired)" || bad "canary: $(drv_outcome e-canary)"

drive run_shell "ls -la" e-ls
[ "$(drv_outcome e-ls)" = "allow" ] && ok "benign ls still allowed in enforce" || bad "benign ls in enforce: $(drv_outcome e-ls)"

drive run_shell "rm -rf ./src" e-src
[ "$(drv_outcome e-src)" = "allow" ] && ok "recursive delete INSIDE cwd allowed (gate is precise, not trigger-happy)" || bad "in-cwd rm: $(drv_outcome e-src)"

grep -q '"decision": "STOP"' "$RECEIPTS" 2>/dev/null && ok "STOP receipt recorded for the denial" || bad "no STOP receipt anywhere in $RECEIPTS"

# ---- 5. unreachable gate: fail-open, except the destructive safety net --------- #
section "unreachable gate"
OPERATOR_GATE="$TEST_BASE/no-such-gate.py"
export OPERATOR_GATE

drive run_shell "ls -la" u-ls
[ "$(drv_outcome u-ls)" != "block" ] && ok "unreachable + benign -> fail-open (honest degrade; outcome=$(drv_outcome u-ls))" || bad "unreachable benign: $(drv_outcome u-ls)"

drive run_shell "echo hi | tee ~/.zshrc" u-tee
[ "$(drv_outcome u-tee)" = "block" ] && ok "unreachable + tee ~/.zshrc -> fail CLOSED (shell-rc extension)" || bad "unreachable tee: $(drv_outcome u-tee)"
case "$(drv_reason u-tee)" in
  *"Operator Lite blocked: "*"unreachable"*) ok "fail-closed reason says why (gate unreachable + safety net)" ;;
  *) bad "fail-closed reason: $(drv_reason u-tee)" ;;
esac

drive run_shell "rm -rf ~/demo-factory/canary" u-rm
[ "$(drv_outcome u-rm)" = "block" ] && ok "unreachable + rm -rf -> fail CLOSED (mirrored OpenClaw core)" || bad "unreachable rm: $(drv_outcome u-rm)"
unset OPERATOR_GATE

# ---- 6. receipt chain integrity ------------------------------------------------ #
section "receipt chain"
chain=$(python3 "$GATE_DST" verify 2>/dev/null | head -1)
case "$chain" in
  *"chain=OK"*) ok "receipt hash chain verifies ($chain)" ;;
  *) bad "receipt chain: $chain" ;;
esac

# ---- 7. nothing was ever executed ----------------------------------------------- #
[ ! -e "$TEST_HOME/demo-factory" ] && ok "no ~/demo-factory was created (payloads never execute)" \
  || bad "demo-factory exists -- payloads must never execute"
[ ! -e "$TEST_PROJ/src" ] && ok "no ./src was created" || bad "./src exists -- payloads must never execute"

# ---- 8. real SDK wrap test (only when the SDK is genuinely importable) ---------- #
section "openai-agents SDK wrap test"
if python3 -c 'from agents import function_tool, tool_input_guardrail, ToolGuardrailFunctionOutput, ToolInputGuardrailData' 2>/dev/null; then
  PCWD="$TEST_PROJ" MOD_DIR="$MOD_DIR" CANARY_CMD='rm -rf ~/demo-factory/canary' python3 - > "$LOGS/sdk.out" 2>&1 <<'PYEOF' \
    && ok "real SDK: gated tool built, guardrail attached, reject_content deny form verified" \
    || { bad "SDK wrap test failed"; cat "$LOGS/sdk.out"; }
import asyncio, json, os, sys
sys.path.insert(0, os.environ["MOD_DIR"])
import operator_guard as g
from agents import Agent, FunctionTool, ToolInputGuardrailData

@g.gated_shell_tool(name="run_shell")
async def run_shell(command: str) -> str:
    """Run a shell command locally."""
    return "SHOULD NEVER RUN"

assert isinstance(run_shell, FunctionTool), type(run_shell)
grs = getattr(run_shell, "tool_input_guardrails", None)
assert grs, "no tool_input_guardrails attached"

def make_ctx(command):
    try:
        from agents.tool_context import ToolContext
        return ToolContext(context=None, tool_name="run_shell", tool_call_id="c1",
                           tool_arguments=json.dumps({"command": command}))
    except ImportError:
        from types import SimpleNamespace
        return SimpleNamespace(tool_name="run_shell", tool_call_id="c1",
                               tool_arguments=json.dumps({"command": command}))

def message_of(out):
    b = getattr(out, "behavior", None)
    for src in (b, out):
        if isinstance(src, dict) and src.get("message"):
            return src["message"]
        m = getattr(src, "message", None)
        if isinstance(m, str) and m:
            return m
    return json.dumps(out, default=str)

def wrap(command):
    ctx = make_ctx(command)
    for kwargs in ({"context": ctx}, {"context": ctx, "agent": None}):
        try:
            return g.operator_tool_input_guardrail(ToolInputGuardrailData(**kwargs))
        except TypeError:
            continue
    raise AssertionError("cannot construct ToolInputGuardrailData")

out = wrap(os.environ["CANARY_CMD"])
blob = json.dumps(getattr(out, "behavior", out), default=str)
assert "reject_content" in blob, blob
msg = message_of(out)
assert msg.startswith("Operator Lite blocked: "), msg
assert "OPERATOR: STOPPED" in msg and "OP-003" in msg, msg

allow_out = wrap("ls -la")
assert "allow" in json.dumps(getattr(allow_out, "behavior", allow_out), default=str)

guarded = Agent(name="t", tools=[run_shell])
assert any(t.name == "run_shell" for t in guarded.tools)
print("SDK wrap test: gated tool ok, deny form ok")
PYEOF
else
  echo "UNTESTED: real SDK wrap test -- the openai-agents package is not" \
       "importable in this environment (a bare 'import agents' succeeding is" \
       "NOT sufficient; a real 'from agents import function_tool' marker is" \
       "required). Everything above exercises the stdlib-only gate-call path," \
       "which is the same code the guardrail function calls."
fi

# ---- summary --------------------------------------------------------------------- #
printf '\n'
if [ "$fail" = 0 ]; then
  echo "ALL CHECKS PASSED (hermetic sandbox removed; nothing outside it touched)."
else
  echo "SOME TESTS FAILED."
fi
exit "$fail"
