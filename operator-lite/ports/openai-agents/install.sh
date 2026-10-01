#!/bin/sh
# Operator Lite -- OpenAI Agents SDK (Python) port installer. POSIX sh.
#
#   sh install.sh
#
# Does four things, idempotently:
#   1. installs the gate to ~/.operator/gate/operator-gate.py
#      (downloads from GitHub unless an existing copy is same-or-newer;
#      never overwrites a newer gate)
#   2. installs the guardrail module to
#      ~/.operator/ports/openai-agents/operator_guard.py
#      (repo copy first -- it is version-matched to this installer -- raw
#      download as fallback; never overwrites a newer module)
#   3. sets mode to shadow if no mode file exists yet
#   4. prints next steps
#
# The port is code, not configuration: the SDK gets the gate as a tool
# input guardrail (see README.md). Nothing else is wired into your Python
# environment -- no site-packages, no dependency install.
#
# Installs are run by the owner in their own terminal -- same guard as the gate.
set -u

PORT_DIR=$(cd "$(dirname "$0")" && pwd)
GATE_URL="https://raw.githubusercontent.com/samvallad33/vestige/main/operator-lite/operator-gate.py"
MODULE_URL="https://raw.githubusercontent.com/samvallad33/vestige/main/operator-lite/ports/openai-agents/operator_guard.py"
REPO_GATE="$PORT_DIR/../../operator-gate.py"
REPO_MODULE="$PORT_DIR/operator_guard.py"

OP_HOME="${OPERATOR_HOME:-$HOME/.operator}"
GATE_DIR="$OP_HOME/gate"
GATE_DST="$GATE_DIR/operator-gate.py"
MOD_DST_DIR="$OP_HOME/ports/openai-agents"
MOD_DST="$MOD_DST_DIR/operator_guard.py"

say()  { printf '%s\n' "$*"; }
die()  { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

[ "${OPERATOR_AGENT_SESSION:-}" = "" ] || \
    die "refusing: installs are run by the owner in their own terminal."
command -v python3 >/dev/null 2>&1 || die "python3 is required (the gate and the guardrail module are stdlib-only Python)."

mkdir -p "$GATE_DIR"     || die "cannot create $GATE_DIR"
mkdir -p "$MOD_DST_DIR"  || die "cannot create $MOD_DST_DIR"

# --- version helpers ------------------------------------------------------- #
file_version() {    # file_version <file> <marker> -> "0.3.1" / "0.1.0" or ""
    sed -n "s/^$2 = *\"\([^\"]*\)\".*/\1/p" "$1" 2>/dev/null | head -n 1
}
version_ge() {      # version_ge A B -> true when A >= B (dotted numeric)
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

fetch() {  # fetch <url> <dst-file>
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL --max-time 30 -o "$2" "$1" 2>/dev/null
    elif command -v wget >/dev/null 2>/dev/null; then
        wget -q -T 30 -O "$2" "$1" 2>/dev/null
    else
        python3 -c "import urllib.request,sys; urllib.request.urlretrieve(sys.argv[1], sys.argv[2])" "$1" "$2" 2>/dev/null
    fi
}

# --- 1. install the gate (download first, sibling repo copy as fallback) ---- #
SRC=""
TMP_GATE="$GATE_DIR/.download.operator-gate.py"
fetch "$GATE_URL" "$TMP_GATE" || rm -f "$TMP_GATE"
if [ -s "$TMP_GATE" ] && grep -q '^VERSION = ' "$TMP_GATE"; then
    SRC="$TMP_GATE"
else
    rm -f "$TMP_GATE"
fi
if [ "$SRC" = "" ] && [ -f "$REPO_GATE" ] && grep -q '^VERSION = ' "$REPO_GATE"; then
    SRC="$REPO_GATE"
fi
if [ "$SRC" = "" ]; then
    if [ -f "$GATE_DST" ]; then
        say "operator-gate: no source reachable and none needed -- keeping existing $(file_version "$GATE_DST" 'VERSION' 2>/dev/null || echo unknown)"
    else
        die "could not download $GATE_URL and no sibling gate at $REPO_GATE"
    fi
else
    SRC_VER=$(file_version "$SRC" 'VERSION')
    DST_VER=$(file_version "$GATE_DST" 'VERSION' 2>/dev/null)
    if [ -f "$GATE_DST" ] && version_ge "$DST_VER" "$SRC_VER"; then
        say "operator-gate: existing v$DST_VER >= v$SRC_VER -- not overwritten"
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
python3 -c 'import sys; compile(open(sys.argv[1], encoding="utf-8").read(), sys.argv[1], "exec")' \
    "$GATE_DST" 2>/dev/null || die "$GATE_DST does not parse; not wiring a broken gate"

# --- 2. install the guardrail module ---------------------------------------- #
MSRC=""
if [ -f "$REPO_MODULE" ] && grep -q '^__version__' "$REPO_MODULE"; then
    MSRC="$REPO_MODULE"                      # version-matched to this installer
else
    TMP_MOD="$MOD_DST_DIR/.download.operator_guard.py"
    fetch "$MODULE_URL" "$TMP_MOD" || rm -f "$TMP_MOD"
    if [ -s "$TMP_MOD" ] && grep -q '^__version__' "$TMP_MOD"; then
        MSRC="$TMP_MOD"
    else
        rm -f "$TMP_MOD"
    fi
fi
[ "$MSRC" != "" ] || \
    { [ -f "$MOD_DST" ] && say "operator_guard: no source reachable -- keeping existing v$(file_version "$MOD_DST" '__version__')"; } \
    || die "could not obtain operator_guard.py ($MODULE_URL or $REPO_MODULE)"
if [ "$MSRC" != "" ]; then
    MSRC_VER=$(file_version "$MSRC" '__version__')
    MDST_VER=$(file_version "$MOD_DST" '__version__' 2>/dev/null)
    if [ -f "$MOD_DST" ] && version_ge "$MDST_VER" "$MSRC_VER"; then
        say "operator_guard: existing v$MDST_VER >= v$MSRC_VER -- not overwritten"
        [ "$MSRC" = "$MOD_DST_DIR/.download.operator_guard.py" ] && rm -f "$MSRC"
    else
        if [ "$MSRC" = "$MOD_DST_DIR/.download.operator_guard.py" ]; then
            mv "$MSRC" "$MOD_DST"
        else
            cp "$MSRC" "$MOD_DST"
        fi
        chmod 644 "$MOD_DST"
        say "operator_guard: installed v$MSRC_VER -> $MOD_DST"
    fi
fi

# refuse to ship a module that does not parse or does not import without the SDK
python3 -c 'import sys; compile(open(sys.argv[1], encoding="utf-8").read(), sys.argv[1], "exec")' \
    "$MOD_DST" 2>/dev/null || die "$MOD_DST does not parse"
# (import + resolve_gate + describe exercise the whole stdlib-only surface;
#  this must succeed whether or not the openai-agents package is installed)
python3 -c 'import sys; sys.path.insert(0, sys.argv[1]); import operator_guard as g; g.describe()' \
    "$MOD_DST_DIR" 2>/dev/null || die "$MOD_DST_DIR is not importable without the openai-agents package; refusing to ship it"

# --- 3. mode: shadow unless the owner already chose -------------------------- #
MODE_DST="$OP_HOME/mode"
if [ ! -f "$MODE_DST" ]; then
    printf 'shadow\n' > "$MODE_DST"
    say "operator-gate: mode=shadow (log-only). Flip with: echo enforce > $MODE_DST"
else
    say "operator-gate: mode=$(cat "$MODE_DST") (existing mode kept)"
fi

# --- 4. next steps ------------------------------------------------------------ #
say ""
say "Done. Next steps:"
say "  1. Point your code at the module (copy it beside your script, or):"
say "       export PYTHONPATH=\"\$PYTHONPATH:$MOD_DST_DIR\""
say "  2. Wrap your shell tool:  from operator_guard import gated_shell_tool"
say "     (minimal snippet in README.md, next to this installer)"
say "  3. Prove it:  sh $PORT_DIR/test.sh"
say "  4. Review receipts: python3 $GATE_DST status ; python3 $GATE_DST verify"
say "  5. Shadow mode logs every STOP without blocking. When the log looks"
say "     right, flip to blocking:"
say "        echo enforce > $MODE_DST"
