#!/bin/sh
# Operator Lite -- Crush port end-to-end test. POSIX sh, stdlib-only.
#
# Hermetic: builds a throwaway HOME and projects (NOT under /tmp or $TMPDIR --
# the gate classifies system temp as scratch, which would weaken OP-003), runs
# the real install.sh into them, then drives the EXACT command string read back
# out of the installed crush.json, with payloads in the EXACT stdin shape
# crush sends to PreToolUse hooks (verified in internal/hooks/input.go):
#   {"event","session_id","cwd","tool_name","tool_input":{"command"}}
#
# Nothing is ever executed: payloads are only piped into the hook command as
# JSON, and the gate is analyzer-only. The EXIT trap removes every trace.
set -u

PORT_DIR=$(cd "$(dirname "$0")" && pwd)

fail=0
ok()  { printf 'PASS: %s\n' "$*"; }
bad() { printf 'FAIL: %s\n' "$*"; fail=1; }
section() { printf '\n== %s ==\n' "$*"; }

# ---- hermetic environment --------------------------------------------------- #
unset OPERATOR_AGENT_SESSION OPERATOR_GATE OPERATOR_HOME OPERATOR_GATE_MODE OPERATOR_PYTHON

TEST_BASE="$HOME/.opgate-crush-test.$$"
rm -rf "$TEST_BASE"
TEST_HOME="$TEST_BASE/home"
TEST_PROJ="$TEST_BASE/project"           # fresh install (no existing crush.json)
TEST_PROJ2="$TEST_BASE/project-merge"    # pre-existing crush.json -> merge path
TEST_PROJ3="$TEST_BASE/project-broken"   # invalid crush.json -> refuse path
LOGS="$TEST_BASE/logs"
mkdir -p "$TEST_HOME" "$TEST_PROJ" "$TEST_PROJ2" "$TEST_PROJ3" "$LOGS" || exit 1
TMPDIR="$TEST_BASE/tmp"; export TMPDIR
HOME="$TEST_HOME"; export HOME
trap 'rm -rf "$TEST_BASE"' EXIT

CRUSH_JSON="$TEST_PROJ/crush.json"
TODAY=$(date -u +%Y-%m-%d)
RECEIPTS="$TEST_HOME/.operator/receipts/$TODAY.jsonl"

hook_cmd_of() { # extract the command install.sh wrote into a crush.json
  python3 - "$1" <<'PYEOF'
import json, sys
print(json.load(open(sys.argv[1]))["hooks"]["PreToolUse"][0]["command"])
PYEOF
}

run_hook() { # run_hook <hook-cmd> <tool> <tool-input-json> <cwd> <session> <tag>
  _hcmd="$1"; _tool="$2"; _pti="$3"; _cwd="$4"; _sid="$5"; _tag="$6"
  PTOOL="$_tool" PTI="$_pti" PCWD="$_cwd" PSID="$_sid" python3 - > "$LOGS/$_tag.payload" <<'PYEOF'
import json, os
print(json.dumps({
    "event": "PreToolUse",
    "session_id": os.environ["PSID"],
    "cwd": os.environ["PCWD"],
    "tool_name": os.environ["PTOOL"],
    "tool_input": json.loads(os.environ["PTI"]),
}))
PYEOF
  sh -c "$_hcmd" < "$LOGS/$_tag.payload" > "$LOGS/$_tag.stdout" 2> "$LOGS/$_tag.stderr"
}

receipt_decision()    { python3 -c 'import json,sys; print(json.loads(open(sys.argv[1]).read().splitlines()[-1])["decision"])' "$RECEIPTS" 2>/dev/null; }
receipt_commitments() { python3 -c 'import json,sys; print(",".join(json.loads(open(sys.argv[1]).read().splitlines()[-1]).get("commitments") or []))' "$RECEIPTS" 2>/dev/null; }
receipt_source()      { python3 -c 'import json,sys; print(json.loads(open(sys.argv[1]).read().splitlines()[-1]).get("source",""))' "$RECEIPTS" 2>/dev/null; }
receipt_tool()        { python3 -c 'import json,sys; print(json.loads(open(sys.argv[1]).read().splitlines()[-1]).get("tool",""))' "$RECEIPTS" 2>/dev/null; }
receipt_cwd()         { python3 -c 'import json,sys; print(json.loads(open(sys.argv[1]).read().splitlines()[-1]).get("cwd",""))' "$RECEIPTS" 2>/dev/null; }
entry_count() { # operator-lite entries in a crush.json
  python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(len([h for h in d.get("hooks",{}).get("PreToolUse",[]) if isinstance(h,dict) and (h.get("name")=="operator-lite" or "operator-gate" in json.dumps(h))]))' "$1" 2>/dev/null
}

# ---- 1. install.sh into the throwaway HOME ----------------------------------- #
section "install (fresh project)"
if sh "$PORT_DIR/install.sh" "$TEST_PROJ" > "$LOGS/install.log" 2>&1; then
  ok "install.sh exited 0"
else
  bad "install.sh failed"; cat "$LOGS/install.log"; exit 1
fi
cat "$LOGS/install.log"

[ -f "$TEST_HOME/.operator/gate/operator-gate.py" ] && ok "gate installed" || bad "gate missing"
[ -x "$TEST_HOME/.operator/gate/operator-gate.py" ] && ok "gate is executable" || bad "gate not executable"
[ "$(cat "$TEST_HOME/.operator/mode" 2>/dev/null)" = "shadow" ] && ok "mode file = shadow" || bad "mode file not shadow"

python3 - "$CRUSH_JSON" <<'PYEOF' && ok "crush.json valid + wired (name, \$HOME command, --source crush, timeout)" || bad "crush.json wrong"
import json, sys
d = json.load(open(sys.argv[1]))
e = d["hooks"]["PreToolUse"][0]
assert e["name"] == "operator-lite", e
assert e["timeout"] == 10, e
assert 'operator-gate.py" hook --source crush' in e["command"], e
assert e["command"].startswith('python3 "$HOME/'), e
PYEOF

# ---- 2. shadow mode: receipts written, nothing blocked ------------------------ #
section "shadow mode (default)"
HOOK_CMD=$(hook_cmd_of "$CRUSH_JSON")

rc=$(run_hook "$HOOK_CMD" bash '{"command":"ls -la"}' "$TEST_PROJ" s1 p-ls; echo $?)
[ "$rc" = "0" ] && ok "benign ls -> exit 0 (allowed)" || bad "benign ls exited $rc, want 0"

rc=$(run_hook "$HOOK_CMD" bash '{"command":"rm -rf ~/demo-factory/canary"}' "$TEST_PROJ" s1 p-rm; echo $?)
[ "$rc" = "0" ] && ok "rm -rf in shadow -> exit 0 (logged, not blocked)" || bad "rm -rf in shadow exited $rc, want 0"
[ -f "$RECEIPTS" ] && ok "receipts file exists" || bad "no receipts file"
[ "$(receipt_decision)" = "SHADOW_STOP" ] && ok "destructive call receipted as SHADOW_STOP" || bad "receipt decision: $(receipt_decision)"
[ "$(receipt_commitments)" = "OP-003" ] && ok "shadow receipt carries OP-003" || bad "commitments: $(receipt_commitments)"
[ "$(receipt_source)" = "crush" ] && ok "receipt source = crush (--source crush plumbed)" || bad "receipt source: $(receipt_source)"
[ "$(receipt_tool)" = "bash" ] && ok "receipt tool = bash (crush tool name)" || bad "receipt tool: $(receipt_tool)"
[ "$(receipt_cwd)" = "$TEST_PROJ" ] && ok "payload cwd reached the gate untouched" || bad "receipt cwd: $(receipt_cwd)"

rc=$(run_hook "$HOOK_CMD" edit "{\"file_path\":\"$TEST_HOME/.zshrc\",\"old_string\":\"a\",\"new_string\":\"b\"}" "$TEST_PROJ" s1 p-edit; echo $?)
[ "$rc" = "0" ] && ok "edit -> ~/.zshrc in shadow -> exit 0 (logged)" || bad "edit in shadow exited $rc, want 0"
[ "$(receipt_commitments)" = "OP-008" ] && ok "edit receipt carries OP-008 (shell-init write)" || bad "edit commitments: $(receipt_commitments)"

rc=$(run_hook "$HOOK_CMD" bash '{"command":"rm -rf ./src"}' "$TEST_PROJ" s1 p-src; echo $?)
[ "$rc" = "0" ] && ok "recursive delete INSIDE cwd -> exit 0 (gate is precise, not trigger-happy)" || bad "in-cwd rm -rf exited $rc, want 0"

# ---- 3. merge into an existing crush.json (+ backup, idempotency) -------------- #
section "merge into existing crush.json"
printf '{\n  "$schema": "https://charm.land/crush.json",\n  "permissions": {"allow": ["view", "ls", "grep", "edit", "bash"]}\n}\n' > "$TEST_PROJ2/crush.json"
if sh "$PORT_DIR/install.sh" "$TEST_PROJ2" > "$LOGS/install2.log" 2>&1; then
  ok "install.sh into project with existing crush.json exited 0"
else
  bad "install.sh merge failed"; cat "$LOGS/install2.log"
fi
python3 - "$TEST_PROJ2/crush.json" <<'PYEOF' && ok "merge preserved existing keys (permissions) + appended hook" || bad "merge lost data"
import json, sys
d = json.load(open(sys.argv[1]))
assert d["permissions"]["allow"] == ["view", "ls", "grep", "edit", "bash"], d
e = d["hooks"]["PreToolUse"][0]
assert e["name"] == "operator-lite" and e["timeout"] == 10, e
PYEOF
ls "$TEST_PROJ2"/crush.json.bak-* >/dev/null 2>&1 && ok "original backed up (crush.json.bak-*)" || bad "no backup of pre-existing crush.json"

sh "$PORT_DIR/install.sh" "$TEST_PROJ2" > "$LOGS/install3.log" 2>&1 && ok "rerun exited 0" || bad "rerun failed"
[ "$(entry_count "$TEST_PROJ2/crush.json")" = "1" ] && ok "rerun is idempotent (still 1 operator-lite entry)" || bad "duplicate entries: $(entry_count "$TEST_PROJ2/crush.json")"
set -- "$TEST_PROJ2"/crush.json.bak-*
[ "$#" -eq 1 ] && ok "no extra backup on the no-op rerun" || bad "backup count changed on rerun (got $#)"

# ---- 4. invalid crush.json: refused, untouched, manual instructions ------------ #
section "invalid crush.json refused"
printf '{ this is not json' > "$TEST_PROJ3/crush.json"
cp "$TEST_PROJ3/crush.json" "$LOGS/broken-before"
if sh "$PORT_DIR/install.sh" "$TEST_PROJ3" > "$LOGS/install4.log" 2>&1; then
  bad "install.sh should fail on invalid crush.json"
else
  ok "install.sh refused invalid crush.json (nonzero exit)"
fi
cmp -s "$LOGS/broken-before" "$TEST_PROJ3/crush.json" && ok "invalid file left byte-identical" || bad "invalid file was modified"
grep -q "not valid JSON" "$LOGS/install4.log" && ok "manual merge instructions printed" || bad "no guidance on refusal"
ls "$TEST_PROJ3"/crush.json.bak-* >/dev/null 2>&1 && bad "backup written for untouched file" || ok "no backup written (nothing was modified)"

# ---- 5. enforce mode: the documented flip, then real blocks --------------------- #
section "enforce mode"
echo enforce > "$TEST_HOME/.operator/mode"

rc=$(run_hook "$HOOK_CMD" bash '{"command":"ls -la"}' "$TEST_PROJ" s2 e-ls; echo $?)
[ "$rc" = "0" ] && ok "benign ls -> exit 0 (allowed)" || bad "benign ls exited $rc, want 0"
[ ! -s "$LOGS/e-ls.stdout" ] && ok "benign allow keeps stdout empty (crush parses stdout as decision JSON)" || bad "stdout not empty on allow"
[ ! -s "$LOGS/e-ls.stderr" ] && ok "benign allow writes no stderr" || bad "stderr noise on allow"

rc=$(run_hook "$HOOK_CMD" bash '{"command":"rm -rf ~/demo-factory/canary"}' "$TEST_PROJ" s3 e-rm; echo $?)
[ "$rc" = "2" ] && ok "plain rm -rf -> exit 2 (BLOCKED)" || bad "rm -rf exited $rc, want 2"
grep -q "OPERATOR: STOPPED" "$LOGS/e-rm.stderr" && ok "stderr carries OPERATOR: STOPPED reason (crush shows it as the deny reason)" || bad "no OPERATOR reason on stderr"
grep -q "OP-003" "$LOGS/e-rm.stderr" && ok "stderr names the rule (OP-003)" || bad "stderr missing rule id"
[ ! -s "$LOGS/e-rm.stdout" ] && ok "block keeps stdout empty (exit 2 + stderr is the contract)" || bad "stdout not empty on block"

rc=$(run_hook "$HOOK_CMD" bash '{"command":"r'"''"'m -rf ~/demo-factory/canary"}' "$TEST_PROJ" s3 e-obs; echo $?)
[ "$rc" = "2" ] && ok "obfuscated r''m -rf -> exit 2 (gate sees through quote reassembly)" || bad "obfuscated rm exited $rc, want 2"

rc=$(run_hook "$HOOK_CMD" bash '{"command":"echo OPERATOR_CANARY_STOP_7f3a"}' "$TEST_PROJ" s4 e-canary; echo $?)
[ "$rc" = "2" ] && ok "canary payload -> exit 2 (proves the whole chain is wired)" || bad "canary exited $rc, want 2"

rc=$(run_hook "$HOOK_CMD" edit "{\"file_path\":\"$TEST_HOME/.zshrc\",\"old_string\":\"a\",\"new_string\":\"b\"}" "$TEST_PROJ" s5 e-edit; echo $?)
[ "$rc" = "2" ] && ok "edit -> ~/.zshrc in enforce -> exit 2 (OP-008)" || bad "edit in enforce exited $rc, want 2"
grep -q "OP-008" "$LOGS/e-edit.stderr" && ok "edit stderr names OP-008" || bad "edit stderr missing rule id"

rc=$(run_hook "$HOOK_CMD" bash '{"command":"rm -rf ./src"}' "$TEST_PROJ" s6 e-src; echo $?)
[ "$rc" = "0" ] && ok "in-cwd rm -rf still allowed in enforce" || bad "in-cwd rm -rf exited $rc, want 0"

grep -q '"decision": "STOP"' "$RECEIPTS" 2>/dev/null && ok "STOP receipt recorded for the denial" || bad "no STOP receipt"

# ---- 6. receipt chain integrity -------------------------------------------------- #
section "receipt chain"
chain=$(python3 "$TEST_HOME/.operator/gate/operator-gate.py" verify 2>/dev/null | head -1)
case "$chain" in
  *"chain=OK"*) ok "receipt hash chain verifies ($chain)" ;;
  *) bad "receipt chain: $chain" ;;
esac

# ---- 7. nothing was ever executed ------------------------------------------------- #
[ ! -e "$TEST_HOME/demo-factory" ] && ok "no ~/demo-factory directory was created" \
  || bad "demo-factory exists -- payloads must never execute"
[ ! -e "$TEST_PROJ/src" ] && ok "no ./src directory was created" \
  || bad "./src exists -- payloads must never execute"

# ---- summary ----------------------------------------------------------------------- #
printf '\n'
if [ "$fail" = 0 ]; then
  echo "ALL CHECKS PASSED (hermetic sandbox removed; nothing outside it touched)."
  echo "UNTESTED: a live crush session (crush not installed on this machine) --"
  echo "hook discovery of crush.json at startup and crush-side display of the"
  echo "stderr deny reason should be confirmed by running install.sh and the"
  echo "canary step inside crush itself."
else
  echo "SOME TESTS FAILED."
fi
exit "$fail"
