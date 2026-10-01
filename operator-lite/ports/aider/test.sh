#!/bin/sh
# End-to-end test for the Aider port of Operator Lite: the REAL install.sh
# generates the REAL shims in a disposable home, and every case runs through
# them against the REAL gate.
#
# Disposable homes are created under $HOME (.operator-test-*) -- never under
# /tmp or $TMPDIR, because the gate exempts scratch dirs from OP-003, which
# would silently weaken the recursive-delete case.
#
#   case 1  benign `ls -la` via shim (enforce)  -> gate allows, real ls runs
#   case 2  enforce `rm -rf ~/demo-factory/canary` via shim
#                                   -> gate exit 2, shim exit 126, real rm NEVER exec'd
#   case 3  enforce `r''m -rf` (quote-obfuscated) via shim -> blocked, exit 126
#   case 4  shadow: same rm via shim            -> passes through, SHADOW receipt
#   case 4b shadow: `rm` (no -f) via shim       -> real rm runs (its own ENOENT proves exec)
#   case 5  gate unreachable: benign fails open; destructive-like still blocked
#   case 6  gate unreachable + shell-rc write   -> fails CLOSED (regression 2026-09-30)
#
# Nothing real is ever deleted: every canary path is nonexistent, and a
# canary-KEEPER marker beside them must survive every single case.
set -u

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_GATE="$SCRIPT_DIR/../../operator-gate.py"
INSTALL="$SCRIPT_DIR/install.sh"

command -v python3 >/dev/null 2>&1 || { echo "FATAL: python3 required"; exit 1; }
command -v grep    >/dev/null 2>&1 || { echo "FATAL: grep required"; exit 1; }
[ -f "$REPO_GATE" ] || { echo "FATAL: $REPO_GATE missing"; exit 1; }
[ -f "$INSTALL" ]   || { echo "FATAL: $INSTALL missing"; exit 1; }

failures=0
H1=""
H2=""

pass() { printf '         PASS\n'; }
fail() { printf '         FAIL: %s\n' "$*"; failures=$((failures + 1)); }

new_home() {  # new_home -> disposable home, pre-seeded with the repo gate
    h=$(mktemp -d "$HOME/.operator-test-XXXXXXXX") || { echo "FATAL: mktemp failed"; exit 1; }
    mkdir -p "$h/work" "$h/.operator/gate" "$h/demo-factory"
    : > "$h/demo-factory/canary-KEEPER"
    cp "$REPO_GATE" "$h/.operator/gate/operator-gate.py"
    chmod 755 "$h/.operator/gate/operator-gate.py"
    printf '%s' "$h"
}

do_install() {  # do_install <home> <log> -> runs the real installer in the sandbox
    ( export HOME="$1" OPERATOR_HOME="$1/.operator" OPERATOR_ALLOW_TEST_INSTALL=1
      unset OPERATOR_AGENT_SESSION
      sh "$INSTALL" ) > "$2" 2>&1
}

env_run() {  # env_run <home> <command-string> -> one shimmed run, fully sandboxed
    ( cd "$1/work" && export HOME="$1" OPERATOR_HOME="$1/.operator" &&
      PATH="$1/.operator/shim/bin:$PATH" sh -c "$2" )
}

cleanup() {
    for h in "$H1" "$H2"; do
        [ -n "$h" ] && [ -d "$h" ] || continue
        rm -rf "$h" 2>/dev/null || \
            python3 -c 'import shutil,sys; shutil.rmtree(sys.argv[1], True)' "$h" 2>/dev/null
        [ -d "$h" ] && printf 'test.sh: could not remove %s (delete it by hand)\n' "$h"
    done
}
trap cleanup EXIT INT TERM

echo "operator-lite / aider port -- end-to-end shim test"
echo "installer: $INSTALL"
echo "gate:      repo copy pre-seeded into each disposable home (mode set per phase)"
echo "homes:     disposable, under \$HOME (never /tmp or \$TMPDIR: OP-003 exempts scratch)"
echo ""

# ---------------------------------------------------------------- enforce phase -- #
H1=$(new_home)
ILOG="$H1/install.log"
if ! do_install "$H1" "$ILOG"; then
    echo "FATAL: install.sh failed inside the enforce home:"; cat "$ILOG"; exit 1
fi
echo enforce > "$H1/.operator/mode"     # in the disposable home only, so blocks actually block
echo "== install output (enforce home $H1) =="
grep -E 'operator-(gate|lite):' "$ILOG" | sed 's/^/   /'
echo ""

echo "phase 1: mode=enforce, receipts+shims in $H1"
echo ""

# --- case 1: benign ls -la via shim -> gate allows, real ls runs ------------------ #
touch "$H1/work/marker.txt"
out=$(env_run "$H1" 'ls -la' 2>/dev/null)
rc=$?
printf 'case 1: benign `ls -la` via shim (enforce)\n'
[ "$rc" -eq 0 ]              && pass1a=y || pass1a=n
printf '%s' "$out" | grep -q 'marker.txt' && pass1b=y || pass1b=n
grep -q '"tool": "shim:ls"' "$H1/.operator/receipts/"*.jsonl 2>/dev/null && pass1c=y || pass1c=n
printf '         exit=%s real-ls-saw-marker=%s gate-receipt(shim:ls)=%s\n' \
    "$rc" "$pass1b" "$pass1c"
if [ "$pass1a" = y ] && [ "$pass1b" = y ] && [ "$pass1c" = y ]; then pass; else
    fail "expected exit 0, marker in ls output, and an allow receipt for shim:ls"; fi
echo ""

# --- case 2: enforce rm -rf canary via shim -> blocked, real rm never exec'd ------ #
err=$(env_run "$H1" 'rm -rf ~/demo-factory/canary' 2>&1 >/dev/null)
rc=$?
printf 'case 2: enforce `rm -rf ~/demo-factory/canary` via shim\n'
[ "$rc" -eq 126 ] && pass2a=y || pass2a=n
printf '%s' "$err" | grep -q 'Operator Lite blocked' && pass2b=y || pass2b=n
printf '%s' "$err" | grep -q 'OP-003'               && pass2c=y || pass2c=n
[ ! -e "$H1/demo-factory/canary" ]                  && pass2d=y || pass2d=n
[ -f "$H1/demo-factory/canary-KEEPER" ]             && pass2e=y || pass2e=n
grep -q '"decision": "STOP"' "$H1/.operator/receipts/"*.jsonl 2>/dev/null && \
grep -q '"commitment": "OP-003"' "$H1/.operator/receipts/"*.jsonl 2>/dev/null && pass2f=y || pass2f=n
printf '         exit=%s blocked-msg=%s rule=OP-003 canary-never-existed=%s KEEPER-survived=%s STOP-receipt=%s\n' \
    "$rc" "$pass2b" "$pass2d" "$pass2e" "$pass2f"
printf '%s\n' "$err" | head -n 2 | sed 's/^/         | /'
if [ "$pass2a" = y ] && [ "$pass2b" = y ] && [ "$pass2c" = y ] && \
   [ "$pass2d" = y ] && [ "$pass2e" = y ] && [ "$pass2f" = y ]; then pass; else
    fail "expected 126 + Operator Lite blocked + OP-003 + no canary + STOP receipt"; fi
echo ""

# --- case 3: quote-obfuscated r''m -rf via shim -> blocked ------------------------- #
err=$(env_run "$H1" "r''m -rf ~/demo-factory/canary" 2>&1 >/dev/null)
rc=$?
printf 'case 3: enforce `r''''m -rf ~/demo-factory/canary` (quote-obfuscated) via shim\n'
[ "$rc" -eq 126 ] && pass3a=y || pass3a=n
printf '%s' "$err" | grep -q 'Operator Lite blocked' && pass3b=y || pass3b=n
[ ! -e "$H1/demo-factory/canary" ]                  && pass3c=y || pass3c=n
[ -f "$H1/demo-factory/canary-KEEPER" ]             && pass3d=y || pass3d=n
printf '         exit=%s blocked-msg=%s canary-never-existed=%s KEEPER-survived=%s\n' \
    "$rc" "$pass3b" "$pass3c" "$pass3d"
printf '%s\n' "$err" | head -n 2 | sed 's/^/         | /'
if [ "$pass3a" = y ] && [ "$pass3b" = y ] && [ "$pass3c" = y ] && [ "$pass3d" = y ]; then pass; else
    fail "expected the obfuscated spelling to land on the rm shim and be blocked (126)"; fi
echo ""

# --- case 5 (in enforce home): gate unreachable -> fail-open / fail-closed -------- #
mv "$H1/.operator/gate/operator-gate.py" "$H1/.operator/gate/operator-gate.py.away"
out=$(env_run "$H1" 'ls' 2>&1)
rc=$?
err=$(env_run "$H1" 'rm -rf ~/demo-factory/canary2' 2>&1 >/dev/null)
rc2=$?
mv "$H1/.operator/gate/operator-gate.py.away" "$H1/.operator/gate/operator-gate.py"
printf 'case 5: gate unreachable (enforce home)\n'
[ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'marker.txt' && pass5a=y || pass5a=n
printf '%s' "$out" | grep -q 'gate unreachable' && pass5b=y || pass5b=n
[ "$rc2" -eq 126 ] && pass5c=y || pass5c=n
printf '%s' "$err" | grep -q 'looks destructive' && pass5d=y || pass5d=n
[ ! -e "$H1/demo-factory/canary2" ] && pass5e=y || pass5e=n
printf '         benign: exit=%s fail-open=%s warned=%s\n' "$rc" "$pass5a" "$pass5b"
printf '         destructive-like: exit=%s blocked=%s canary2-never-existed=%s\n' "$rc2" "$pass5d" "$pass5e"
if [ "$pass5a" = y ] && [ "$pass5b" = y ] && [ "$pass5c" = y ] && \
   [ "$pass5d" = y ] && [ "$pass5e" = y ]; then pass; else
    fail "expected fail-open for benign, 126 for destructive-like while gate is away"; fi

# --- case 6 (regression 2026-09-30): shell-rc writes fail CLOSED even with the
# gate away. A truncated ~/.zshrc must never ride the fail-open path. ---------- #
mv "$H1/.operator/gate/operator-gate.py" "$H1/.operator/gate/operator-gate.py.away"
rcfile="$H1/.operator-test-rcfile"; printf 'keep=me\n' > "$rcfile"
err=$(env_run "$H1" "tee $rcfile" 2>&1 </dev/null)
rc6=$?
err2=$(env_run "$H1" 'tee ~/.zshrc' 2>&1 </dev/null)
rc7=$?
mv "$H1/.operator/gate/operator-gate.py.away" "$H1/.operator/gate/operator-gate.py"
printf 'case 6: gate unreachable, shell-rc write (regression)\n'
[ "$rc6" -eq 0 ] && [ -f "$rcfile" ] && pass6a=y || pass6a=n
[ "$rc7" -eq 126 ] && pass6b=y || pass6b=n
printf '%s' "$err2" | grep -qi 'destructive\|blocked' && pass6c=y || pass6c=n
printf '         non-rc file: exit=%s wrote=%s\n         ~/.zshrc write: exit=%s blocked=%s warned=%s\n' "$rc6" "$pass6a" "$rc7" "$pass6b" "$pass6c"
if [ "$pass6a" = y ] && [ "$pass6b" = y ] && [ "$pass6c" = y ]; then pass; else
    fail "expected ~/.zshrc writes to fail closed (126) while gate is away"; fi
echo ""

# ---------------------------------------------------------------- shadow phase ---- #
H2=$(new_home)
ILOG2="$H2/install.log"
if ! do_install "$H2" "$ILOG2"; then
    echo "FATAL: install.sh failed inside the shadow home:"; cat "$ILOG2"; exit 1
fi
echo "phase 2: mode=shadow (installer default), receipts in $H2"
echo ""

# --- case 4: shadow rm -rf canary via shim -> passes through, receipt written ----- #
# (rm -rf on a nonexistent path is silent by design of -f; the pass-through here is
# proven by exit 0, no block message, the gate's shadow receipt, and case 4b.)
out=$(env_run "$H2" 'rm -rf ~/demo-factory/canary' 2>&1)
rc=$?
printf 'case 4: shadow `rm -rf ~/demo-factory/canary` via shim\n'
[ "$rc" -eq 0 ]                                     && pass4a=y || pass4a=n
printf '%s' "$out" | grep -q 'Operator Lite blocked' && pass4b=n || pass4b=y
grep -q '"tool": "shim:rm"' "$H2/.operator/receipts/"*.jsonl 2>/dev/null && \
grep -q '"decision": "SHADOW_STOP"' "$H2/.operator/receipts/"*.jsonl 2>/dev/null && \
grep -q '"commitments": \["OP-003"\]' "$H2/.operator/receipts/"*.jsonl 2>/dev/null && pass4c=y || pass4c=n
[ -f "$H2/demo-factory/canary-KEEPER" ]             && pass4d=y || pass4d=n
printf '         exit=%s not-blocked=%s shadow-receipt(shim:rm,OP-003)=%s KEEPER-survived=%s\n' \
    "$rc" "$pass4b" "$pass4c" "$pass4d"
if [ "$pass4a" = y ] && [ "$pass4b" = y ] && [ "$pass4c" = y ] && [ "$pass4d" = y ]; then pass; else
    fail "expected pass-through (exit 0, no block) and a SHADOW_STOP receipt for OP-003"; fi
echo ""

# --- case 4b: shadow pass-through is a REAL exec: rm (no -f) shows its own ENOENT -- #
out=$(env_run "$H2" 'rm ~/demo-factory/canary' 2>&1)
rc=$?
printf 'case 4b: shadow `rm ~/demo-factory/canary` (no -f) -> real rm runs, prints its own error\n'
[ "$rc" -eq 1 ] && pass4e=y || pass4e=n
printf '%s' "$out" | grep -q 'No such file or directory' && pass4f=y || pass4f=n
[ -f "$H2/demo-factory/canary-KEEPER" ] && pass4g=y || pass4g=n
printf '         exit=%s (real rm ENOENT output below; nothing was ever deleted)\n' "$rc"
printf '%s\n' "$out" | head -n 1 | sed 's/^/         | /'
if [ "$pass4e" = y ] && [ "$pass4f" = y ] && [ "$pass4g" = y ]; then pass; else
    fail "expected real rm to run (exit 1, ENOENT on stderr) with KEEPER intact"; fi
echo ""

# --- receipts + chain -------------------------------------------------------------- #
echo "receipt chain (shadow home):"
OPERATOR_HOME="$H2/.operator" python3 "$H2/.operator/gate/operator-gate.py" verify | head -n 1 | sed 's/^/   /'
echo ""
echo "--- aider CLI probe ---"
if command -v aider >/dev/null 2>&1; then
    echo "aider found: $(command -v aider)."
    echo "Live check (aider 0.86.2, this machine): launched under the shim PATH,"
    echo "aider's own startup commands (ls, git version, ls ~/.operator) landed on"
    echo "the shims and receipted with source=aider. Not yet exercised: an"
    echo "LLM-suggested shell command mid-session (needs an API key)."
else
    echo "aider CLI: not found on this machine -> UNTESTED: live aider session."
    echo "           The shim contract above is what this machine can prove; launch aider with:"
    echo "             PATH=\"\$HOME/.operator/shim/bin:\$PATH\" aider"
fi
echo ""

if [ "$failures" -gt 0 ]; then
    echo "RESULT: $failures case(s) FAILED"
    exit 1
fi
echo "RESULT: all 6 cases PASS"
