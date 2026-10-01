#!/bin/sh
# Operator Lite -- LangChain / LangGraph port installer. POSIX sh, stdlib-only chain.
#
#   sh install.sh
#
# Does four things, idempotently:
#   1. installs the gate to ~/.operator/gate/operator-gate.py
#      (downloads from GitHub unless an existing copy is same-or-newer;
#      never overwrites a newer gate)
#   2. installs the middleware module to ~/.operator/ports/langchain/operator_middleware.py
#   3. sets mode to shadow if no mode file exists yet
#   4. prints the pip-style usage and next steps
#
# LangChain needs no config-file wiring (unlike Claude Code's settings.json or
# Windsurf's hooks.json): the middleware is attached in Python, in your agent
# code, via create_agent(..., middleware=[operator_middleware()]). So there is
# nothing to merge -- the deliverable is the module plus the snippet.
#
# Installs are run by the owner in their own terminal -- same guard as the gate.
set -u

PORT_DIR=$(cd "$(dirname "$0")" && pwd)
GATE_URL="https://raw.githubusercontent.com/samvallad33/vestige/main/operator-lite/operator-gate.py"
REPO_GATE="$PORT_DIR/../../operator-gate.py"
MIDDLEWARE_SRC="$PORT_DIR/operator_middleware.py"

OP_HOME="${OPERATOR_HOME:-$HOME/.operator}"
GATE_DIR="$OP_HOME/gate"
GATE_DST="$GATE_DIR/operator-gate.py"
PORTS_DIR="$OP_HOME/ports/langchain"
MIDDLEWARE_DST="$PORTS_DIR/operator_middleware.py"

say()  { printf '%s\n' "$*"; }
die()  { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

[ "${OPERATOR_AGENT_SESSION:-}" = "" ] || \
    die "refusing: installs are run by the owner in their own terminal."
command -v python3 >/dev/null 2>&1 || die "python3 is required (the gate and the adapter are stdlib-only Python)."

mkdir -p "$GATE_DIR" || die "cannot create $GATE_DIR"

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
python3 -c 'import sys; compile(open(sys.argv[1], encoding="utf-8").read(), sys.argv[1], "exec")' \
    "$GATE_DST" 2>/dev/null || die "$GATE_DST does not parse; not wiring a broken gate"

# --- install the middleware module ------------------------------------------ #
[ -f "$MIDDLEWARE_SRC" ] || die "operator_middleware.py missing next to install.sh"
mkdir -p "$PORTS_DIR" || die "cannot create $PORTS_DIR"
cp "$MIDDLEWARE_SRC" "$MIDDLEWARE_DST" || die "could not copy the middleware to $MIDDLEWARE_DST"
chmod 644 "$MIDDLEWARE_DST"
# refuse to ship an adapter that does not parse
python3 -c 'import sys; compile(open(sys.argv[1], encoding="utf-8").read(), sys.argv[1], "exec")' \
    "$MIDDLEWARE_DST" 2>/dev/null || die "$MIDDLEWARE_DST does not parse"
say "langchain: middleware installed -> $MIDDLEWARE_DST"

# --- mode: shadow unless the owner already chose ----------------------------- #
MODE_DST="$OP_HOME/mode"
if [ ! -f "$MODE_DST" ]; then
    printf 'shadow\n' > "$MODE_DST"
    say "operator-gate: mode=shadow (log-only). Flip with: echo enforce > $MODE_DST"
else
    say "operator-gate: mode=$(cat "$MODE_DST") (existing mode kept)"
fi

# --- next steps -------------------------------------------------------------- #
say ""
say "Done. Usage (add to your agent code -- LangChain v1):"
say "  import sys, os"
say "  sys.path.insert(0, os.path.expanduser(\"$PORTS_DIR\"))"
say ""
say "  from langchain.agents import create_agent"
say "  from operator_middleware import operator_middleware"
say ""
say "  agent = create_agent("
say "      \"<your-model>\","
say "      tools=[<your shell tool>],        # any tool whose args carry \"command\""
say "      middleware=[operator_middleware()],"
say "  )"
say ""
say "Next steps:"
say "  1. Prove the chain offline: sh $PORT_DIR/test.sh"
say "  2. Review receipts: python3 $GATE_DST status ; python3 $GATE_DST verify"
say "  3. Inside your agent (shadow mode logs the STOP without blocking), ask it to run:"
say "       echo OPERATOR_CANARY_STOP_7f3a"
say "     then check $OP_HOME/receipts/ for the OP-CANARY receipt."
say "  4. When the shadow log looks right, flip to blocking:"
say "        echo enforce > $MODE_DST"
say "  5. Uninstall: rm $MIDDLEWARE_DST"
say "     and remove operator_middleware() from your agent's middleware list."
