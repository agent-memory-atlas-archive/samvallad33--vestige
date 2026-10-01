#!/bin/sh
# Operator Lite -- CrewAI port installer. POSIX sh, stdlib-only chain.
#
#   sh install.sh
#
# Does four things, idempotently:
#   1. installs the gate to ~/.operator/gate/operator-gate.py
#      (downloads from GitHub unless an existing copy is same-or-newer;
#      never overwrites a newer gate)
#   2. copies this port's adapter to ~/.operator/ports/crewai/operator_hooks.py
#      (any existing copy is backed up first)
#   3. sets mode to shadow if no mode file exists yet
#   4. prints next steps
#
# CrewAI has no hooks.json to merge: hooks are execution hooks registered from
# Python. Wiring is one import -- `import operator_hooks` -- after putting
# ~/.operator/ports/crewai on sys.path (see the next steps this prints).
#
# Installs are run by the owner in their own terminal -- same guard as the gate.
set -u

PORT_DIR=$(cd "$(dirname "$0")" && pwd)
GATE_URL="https://raw.githubusercontent.com/samvallad33/vestige/main/operator-lite/operator-gate.py"
REPO_GATE="$PORT_DIR/../../operator-gate.py"
REPO_MODULE="$PORT_DIR/operator_hooks.py"

OP_HOME="${OPERATOR_HOME:-$HOME/.operator}"
GATE_DIR="$OP_HOME/gate"
GATE_DST="$GATE_DIR/operator-gate.py"
PORTS_CREWAI_DIR="$OP_HOME/ports/crewai"
MODULE_DST="$PORTS_CREWAI_DIR/operator_hooks.py"

say()  { printf '%s\n' "$*"; }
die()  { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

[ "${OPERATOR_AGENT_SESSION:-}" = "" ] || \
    die "refusing: installs are run by the owner in their own terminal."
command -v python3 >/dev/null 2>&1 || die "python3 is required (the gate and the adapter are stdlib-only Python)."
[ -f "$REPO_MODULE" ] || die "operator_hooks.py not found next to install.sh ($PORT_DIR)"

mkdir -p "$GATE_DIR"        || die "cannot create $GATE_DIR"
mkdir -p "$PORTS_CREWAI_DIR" || die "cannot create $PORTS_CREWAI_DIR"

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

# --- install the adapter module (backup any existing copy first) ------------ #
if [ -f "$MODULE_DST" ]; then
    BACKUP="$MODULE_DST.bak-$(date +%Y%m%d-%H%M%S)"
    cp "$MODULE_DST" "$BACKUP" || die "could not back up $MODULE_DST"
    say "operator_hooks: previous copy saved -> $BACKUP"
fi
cp "$REPO_MODULE" "$MODULE_DST" || die "could not copy $REPO_MODULE -> $MODULE_DST"
chmod 644 "$MODULE_DST"
python3 -c 'import sys; compile(open(sys.argv[1], encoding="utf-8").read(), sys.argv[1], "exec")' \
    "$MODULE_DST" 2>/dev/null || die "$MODULE_DST does not parse; installation is broken"
say "operator_hooks: adapter installed -> $MODULE_DST"

# --- mode: shadow unless the owner already chose ----------------------------- #
MODE_DST="$OP_HOME/mode"
if [ ! -f "$MODE_DST" ]; then
    printf 'shadow\n' > "$MODE_DST"
    say "operator-gate: mode=shadow (log-only). Flip with: echo enforce > $MODE_DST"
else
    say "operator-gate: mode=$(cat "$MODE_DST") (existing mode kept)"
fi

# --- smoke: the adapter resolves the gate and reports its wiring -------------- #
python3 - "$MODULE_DST" "$GATE_DST" <<'PYEOF' || die "adapter smoke check failed"
import importlib.util, os, sys
module_path, gate_path = sys.argv[1], sys.argv[2]
spec = importlib.util.spec_from_file_location("operator_hooks_check", module_path)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)
if mod.resolve_gate() != gate_path:
    sys.stderr.write("adapter resolves gate to %r, expected %r\n" % (mod.resolve_gate(), gate_path))
    sys.exit(1)
print(mod.status())
PYEOF

# --- next steps -------------------------------------------------------------- #
say ""
say "Done. Next steps:"
say "  1. Wire your crew (one import before Crew(agents=[...]) is built):"
say "       import sys, os"
say "       sys.path.insert(0, os.path.expanduser(\"$PORTS_CREWAI_DIR\"))"
say "       import operator_hooks   # registers the global PRE_TOOL_CALL gate hook"
say "  2. Prove it: sh $PORT_DIR/test.sh"
say "  3. Review receipts: python3 $GATE_DST status ; python3 $GATE_DST verify"
say "  4. Inside a crew (shadow mode logs the STOP without blocking), ask it to run:"
say "       echo OPERATOR_CANARY_STOP_7f3a"
say "     then check $OP_HOME/receipts/ for the OP-CANARY receipt."
say "  5. When the shadow log looks right, flip to blocking:"
say "        echo enforce > $MODE_DST"
say "  6. Unwire: remove the import (or set OPERATOR_CREWAI_AUTOINSTALL=0);"
say "     delete $MODULE_DST to uninstall the adapter."
