#!/bin/sh
# Operator Lite -- Crush port installer. POSIX sh, stdlib-only chain.
#
#   sh install.sh [project-dir]      (default: current directory)
#
# Does four things, idempotently:
#   1. installs the gate to ~/.operator/gate/operator-gate.py
#      (downloads from GitHub unless an existing copy is same-or-newer;
#      never overwrites a newer gate)
#   2. sets mode to shadow if no mode file exists yet
#   3. merges the PreToolUse hook into <project-dir>/crush.json
#      (existing keys are preserved; the original is backed up first;
#      an invalid existing file is left untouched and reported)
#   4. prints next steps
#
# Crush needs no adapter here: its hook payload on stdin is
# {"event","session_id","cwd","tool_name","tool_input"} -- byte-for-byte the
# gate's own contract -- and exit 2 with stderr as the reason blocks the tool
# call. The hook command is the gate invoked directly.
#
# Installs are run by the owner in their own terminal -- same guard as the gate.
set -u

PORT_DIR=$(cd "$(dirname "$0")" && pwd)
GATE_URL="https://raw.githubusercontent.com/samvallad33/vestige/main/operator-lite/operator-gate.py"
REPO_GATE="$PORT_DIR/../../operator-gate.py"

OP_HOME="${OPERATOR_HOME:-$HOME/.operator}"
GATE_DIR="$OP_HOME/gate"
GATE_DST="$GATE_DIR/operator-gate.py"
HOOK_CMD='python3 "$HOME/.operator/gate/operator-gate.py" hook --source crush'

say()  { printf '%s\n' "$*"; }
die()  { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

[ "${OPERATOR_AGENT_SESSION:-}" = "" ] || \
    die "refusing: installs are run by the owner in their own terminal."
command -v python3 >/dev/null 2>&1 || die "python3 is required (the gate is stdlib-only Python)."

PROJECT_DIR="${1:-$PWD}"
mkdir -p "$PROJECT_DIR" 2>/dev/null || die "cannot create project dir $PROJECT_DIR"
mkdir -p "$GATE_DIR"                 || die "cannot create $GATE_DIR"

# --- version helpers ------------------------------------------------------- #
gate_version() {   # gate_version <file> -> "0.3.1" or ""
    sed -n 's/^VERSION = *"\([^"]*\)".*/\1/p' "$1" 2>/dev/null | head -n 1
}
version_ge() {     # version_ge A B -> true when A >= B (dotted numeric)
    awk -v a="$1" -v b="$2" 'BEGIN {
        if (a == "") exit 1
        na = split(a, A, "."); nb = split(b, B, ".")
        n = na > nb ? na : nb
        for (i = 1; i <= n; i++) {
            x = A[i] + 0; y = B[i] + 0
            if (x > y) exit 0
            if (x < y) exit 1
        }
        exit 0
    }'
}

# --- obtain a source gate (download first, sibling repo copy as fallback) --- #
SRC=""
TMP_GATE="$GATE_DIR/.download.operator-gate.py"
if command -v curl >/dev/null 2>&1; then
    curl -fsSL --max-time 30 -o "$TMP_GATE" "$GATE_URL" 2>/dev/null || rm -f "$TMP_GATE"
elif command -v wget >/dev/null 2>&1; then
    wget -q -T 30 -O "$TMP_GATE" "$GATE_URL" 2>/dev/null || rm -f "$TMP_GATE"
else
    python3 -c "import urllib.request,sys; urllib.request.urlretrieve('$GATE_URL', sys.argv[1])" \
        "$TMP_GATE" 2>/dev/null || rm -f "$TMP_GATE"
fi
if [ -s "$TMP_GATE" ] && grep -q '^VERSION = ' "$TMP_GATE"; then
    SRC="$TMP_GATE"
else
    rm -f "$TMP_GATE"
fi
if [ "$SRC" = "" ] && [ -f "$REPO_GATE" ] && grep -q '^VERSION = ' "$REPO_GATE"; then
    SRC="$REPO_GATE"
fi

# --- install the gate (never overwrite a same-or-newer copy) ---------------- #
if [ "$SRC" = "" ]; then
    if [ -f "$GATE_DST" ]; then
        say "operator-gate: no source reachable and none needed -- keeping existing $(gate_version "$GATE_DST" 2>/dev/null || echo unknown)"
    else
        die "could not download $GATE_URL and no sibling gate at $REPO_GATE"
    fi
else
    SRC_VER=$(gate_version "$SRC")
    DST_VER=$(gate_version "$GATE_DST" 2>/dev/null)
    if [ -f "$GATE_DST" ] && version_ge "$DST_VER" "$SRC_VER"; then
        say "operator-gate: existing $DST_VER >= $SRC_VER -- not overwritten"
        [ "$SRC" = "$TMP_GATE" ] && rm -f "$TMP_GATE"
    else
        if [ "$SRC" = "$TMP_GATE" ]; then
            mv "$SRC" "$GATE_DST"          # downloaded file: consumed by the move
        else
            cp "$SRC" "$GATE_DST"          # sibling repo file: the repo keeps its copy
        fi
        chmod 755 "$GATE_DST"
        say "operator-gate: installed v$SRC_VER -> $GATE_DST"
    fi
fi

# refuse to wire a gate that does not parse
if command -v python3 >/dev/null 2>&1; then
    python3 -c 'import sys; compile(open(sys.argv[1], encoding="utf-8").read(), sys.argv[1], "exec")' \
        "$GATE_DST" 2>/dev/null || die "$GATE_DST does not parse; not wiring a broken gate"
fi

# --- mode: shadow unless the owner already chose ----------------------------- #
MODE_DST="$OP_HOME/mode"
if [ ! -f "$MODE_DST" ]; then
    printf 'shadow\n' > "$MODE_DST"
    say "operator-gate: mode=shadow (log-only). Flip with: echo enforce > $MODE_DST"
else
    say "operator-gate: mode=$(cat "$MODE_DST") (existing mode kept)"
fi

# --- wire the hook into the project's crush.json ------------------------------ #
# Merge is done in python3 (crush.json is JSON; POSIX sh is the wrong tool).
# The original file is backed up before the first write; a file that is not
# valid JSON is left untouched and the manual merge instructions are printed.
CRUSH_TARGET="$PROJECT_DIR/crush.json"
python3 - "$CRUSH_TARGET" "$HOOK_CMD" <<'PYEOF'
import json, os, sys, time

path, hook_cmd = sys.argv[1], sys.argv[2]
entry = {"name": "operator-lite", "command": hook_cmd, "timeout": 10}

try:
    with open(path) as f:
        raw = f.read()
    data = json.loads(raw)
    if not isinstance(data, dict):
        raise ValueError("top level is not a JSON object")
except FileNotFoundError:
    data, raw = {}, None
except Exception as exc:
    print("install.sh: %s exists but is not valid JSON (%s); not touching it." % (path, exc))
    print("Merge this by hand once the file parses:")
    print('  "hooks": { "PreToolUse": [ { "name": "operator-lite", "timeout": 10,')
    print('      "command": "%s" } ] }' % hook_cmd)
    sys.exit(7)

hooks = data.setdefault("hooks", {})
if not isinstance(hooks, dict):
    print("install.sh: existing \"hooks\" is not an object (%r); not touching it." % hooks)
    print("Merge the PreToolUse entry from ports/crush/crush.json by hand.")
    sys.exit(7)
pre = hooks.setdefault("PreToolUse", [])
if not isinstance(pre, list):
    print("install.sh: existing hooks.PreToolUse is not a list (%r); not touching it." % pre)
    print("Merge the PreToolUse entry from ports/crush/crush.json by hand.")
    sys.exit(7)

already = [h for h in pre if isinstance(h, dict) and
           (h.get("name") == "operator-lite" or "operator-gate" in json.dumps(h))]
if already:
    print("crush: gate hook already present in %s" % path)
    sys.exit(0)

backup = None
if raw is not None:
    backup = path + ".bak-" + time.strftime("%Y%m%d-%H%M%S")
    try:
        with open(backup, "w") as b:
            b.write(raw)
    except Exception as exc:
        print("install.sh: could not write backup %s (%s); target left untouched." % (backup, exc))
        sys.exit(8)
    print("crush: original saved -> %s" % backup)

pre.append(entry)
with open(path, "w") as f:
    json.dump(data, f, indent=2)
    f.write("\n")
print("crush: PreToolUse hook written -> %s" % path)
PYEOF
rc=$?
if [ "$rc" -eq 7 ]; then
    die "existing $CRUSH_TARGET is invalid JSON; fix or remove it, then rerun."
elif [ "$rc" -eq 8 ]; then
    die "could not back up $CRUSH_TARGET; nothing was modified."
elif [ "$rc" -ne 0 ]; then
    die "could not write $CRUSH_TARGET"
fi

# --- next steps -------------------------------------------------------------- #
say ""
say "Done. Next steps:"
say "  1. Restart crush in $PROJECT_DIR -- hooks load at startup."
say "  2. Prove it: sh $PORT_DIR/test.sh"
say "  3. Review receipts: python3 $GATE_DST status ; python3 $GATE_DST verify"
say "  4. Inside crush (shadow mode logs the STOP without blocking), ask it to run:"
say "       echo OPERATOR_CANARY_STOP_7f3a"
say "     then check $OP_HOME/receipts/ for the OP-CANARY receipt."
say "  5. When the shadow log looks right, flip to blocking:"
say "        echo enforce > $MODE_DST"
say "  6. Uninstall: crush hook remove PreToolUse --name operator-lite"
say "     (or delete the \"operator-lite\" entry from $CRUSH_TARGET)."
