#!/bin/sh
# Operator Lite -- OpenHands port installer. POSIX sh, stdlib-only chain.
#
#   sh install.sh [project-dir]      (default: current directory)
#
# Does four things, idempotently:
#   1. installs the gate to ~/.operator/gate/operator-gate.py
#      (downloads from GitHub unless an existing copy is same-or-newer;
#      never overwrites a newer gate)
#   2. installs the OpenHands adapter next to it (openhands-wrapper.py)
#   3. sets mode to shadow if no mode file exists yet
#   4. merges the gate hook into <project-dir>/.openhands/hooks.json
#
# Installs are run by the owner in their own terminal -- same guard as the gate.
set -u

PORT_DIR=$(cd "$(dirname "$0")" && pwd)
GATE_URL="https://raw.githubusercontent.com/samvallad33/vestige/main/operator-lite/operator-gate.py"
REPO_GATE="$PORT_DIR/../../operator-gate.py"
WRAPPER_SRC="$PORT_DIR/openhands-wrapper.py"

OP_HOME="${OPERATOR_HOME:-$HOME/.operator}"
GATE_DIR="$OP_HOME/gate"
GATE_DST="$GATE_DIR/operator-gate.py"
WRAPPER_DST="$GATE_DIR/openhands-wrapper.py"

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

# --- install the adapter ---------------------------------------------------- #
[ -f "$WRAPPER_SRC" ] || die "openhands-wrapper.py missing next to install.sh"
cp "$WRAPPER_SRC" "$WRAPPER_DST"
chmod 755 "$WRAPPER_DST"
say "openhands: adapter installed -> $WRAPPER_DST"

# --- mode: shadow unless the owner already chose ----------------------------- #
MODE_DST="$OP_HOME/mode"
if [ ! -f "$MODE_DST" ]; then
    printf 'shadow\n' > "$MODE_DST"
    say "operator-gate: mode=shadow (log-only). Flip with: echo enforce > $MODE_DST"
else
    say "operator-gate: mode=$(cat "$MODE_DST") (existing mode kept)"
fi

# --- wire the hook into the project's .openhands/hooks.json ------------------ #
HOOKS_TARGET="$PROJECT_DIR/.openhands/hooks.json"
mkdir -p "$PROJECT_DIR/.openhands" || die "cannot create $PROJECT_DIR/.openhands"
python3 - "$HOOKS_TARGET" <<'PYEOF'
import json, sys

path = sys.argv[1]
entry = {
    "matcher": "*",
    "hooks": [
        {"type": "command",
         "command": "python3 ~/.operator/gate/openhands-wrapper.py",
         "timeout": 10}
    ],
}
try:
    with open(path) as f:
        data = json.load(f)
    if not isinstance(data, dict):
        raise ValueError("top level is not an object")
except FileNotFoundError:
    data = {}
except Exception as exc:
    print("install.sh: %s exists but is not valid JSON (%s); not touching it. "
          "Add the pre_tool_use entry from ports/openhands/hooks.json manually." % (path, exc))
    sys.exit(7)

hooks = data.setdefault("pre_tool_use", [])
if not isinstance(hooks, list):
    hooks = data["pre_tool_use"] = []
already = any("openhands-wrapper" in json.dumps(h) for h in hooks if isinstance(h, dict))
if not already:
    hooks.append(entry)
    with open(path, "w") as f:
        json.dump(data, f, indent=2)
        f.write("\n")
    print("openhands: gate hook written -> %s" % path)
else:
    print("openhands: gate hook already present in %s" % path)
PYEOF
rc=$?
if [ "$rc" -eq 7 ]; then
    die "existing $HOOKS_TARGET is invalid JSON; fix or remove it, then rerun."
elif [ "$rc" -ne 0 ]; then
    die "could not write $HOOKS_TARGET"
fi

# --- next steps -------------------------------------------------------------- #
say ""
say "Done. Next steps:"
say "  1. Start (or restart) an OpenHands conversation in $PROJECT_DIR --"
say "     hooks load at conversation start. CLI: /skills lists loaded hooks."
say "  2. Prove it: sh $PORT_DIR/test.sh"
say "  3. Review receipts: python3 $GATE_DST status ; python3 $GATE_DST verify"
say "  4. When the shadow log looks right, flip to blocking:"
say "        echo enforce > $MODE_DST"
say "  5. Commit .openhands/hooks.json with the project if teammates should share the gate."
