#!/bin/sh
# Operator Lite -- Cursor port installer. POSIX sh, stdlib-only chain.
#
#   sh install.sh
#
# Does four things, idempotently:
#   1. installs the gate to ~/.operator/gate/operator-gate.py
#      (downloads from GitHub unless an existing copy is same-or-newer;
#      never overwrites a newer gate)
#   2. installs the Cursor adapter next to it (cursor-hook.py)
#   3. sets mode to shadow if no mode file exists yet
#   4. merges the beforeShellExecution / beforeMCPExecution / preToolUse
#      hooks into ~/.cursor/hooks.json (user scope; a timestamped backup is
#      written first; existing hooks and settings are preserved; an invalid
#      existing file is left untouched and manual steps are printed)
#
# Installs are run by the owner in their own terminal -- same guard as the gate.
set -u

PORT_DIR=$(cd "$(dirname "$0")" && pwd)
GATE_URL="https://raw.githubusercontent.com/samvallad33/vestige/main/operator-lite/operator-gate.py"
REPO_GATE="$PORT_DIR/../../operator-gate.py"
WRAPPER_SRC="$PORT_DIR/cursor-hook.py"

OP_HOME="${OPERATOR_HOME:-$HOME/.operator}"
GATE_DIR="$OP_HOME/gate"
GATE_DST="$GATE_DIR/operator-gate.py"
WRAPPER_DST="$GATE_DIR/cursor-hook.py"

say()  { printf '%s\n' "$*"; }
die()  { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

[ "${OPERATOR_AGENT_SESSION:-}" = "" ] || \
    die "refusing: installs are run by the owner in their own terminal."
command -v python3 >/dev/null 2>&1 || die "python3 is required (the gate is stdlib-only Python)."

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

# --- install the adapter ---------------------------------------------------- #
[ -f "$WRAPPER_SRC" ] || die "cursor-hook.py missing next to install.sh"
cp "$WRAPPER_SRC" "$WRAPPER_DST"
chmod 755 "$WRAPPER_DST"
say "cursor: adapter installed -> $WRAPPER_DST"

# --- mode: shadow unless the owner already chose ----------------------------- #
MODE_DST="$OP_HOME/mode"
if [ ! -f "$MODE_DST" ]; then
    printf 'shadow\n' > "$MODE_DST"
    say "operator-gate: mode=shadow (log-only). Flip with: echo enforce > $MODE_DST"
else
    say "operator-gate: mode=$(cat "$MODE_DST") (existing mode kept)"
fi

# --- wire the hooks into ~/.cursor/hooks.json (user scope) ------------------- #
# Cursor reads hooks from <project>/.cursor/hooks.json and ~/.cursor/hooks.json
# (user scope covers every project). Relative hook commands resolve against the
# hooks.json directory, so the merged entries use the ADAPTER'S ABSOLUTE PATH --
# do not hand-write `~` here; Cursor's expansion of it is not documented.
HOOKS_TARGET="$HOME/.cursor/hooks.json"
mkdir -p "$HOME/.cursor" || die "cannot create $HOME/.cursor"
if [ -f "$HOOKS_TARGET" ]; then
    BACKUP="$HOOKS_TARGET.backup-operator-$(date +%Y%m%d-%H%M%S)"
    cp "$HOOKS_TARGET" "$BACKUP" || die "could not back up $HOOKS_TARGET"
    say "cursor: backed up existing hooks.json -> $BACKUP"
fi
python3 - "$HOOKS_TARGET" "$WRAPPER_DST" <<'PYEOF'
import json, sys

path, adapter = sys.argv[1], sys.argv[2]
cmd = "python3 %s" % adapter
entries = {
    # every agent shell command (matcher "*" is the docs' match-everything form)
    "beforeShellExecution": [{"command": cmd, "timeout": 10, "failClosed": True, "matcher": "*"}],
    # every MCP tool call (the docs list no matcher for this hook: all calls)
    "beforeMCPExecution": [{"command": cmd, "timeout": 10, "failClosed": True}],
    # native file tools only -- Shell and MCP are covered above; forwarding the
    # same action twice would consume a single-use owner permit on the first
    # invocation and block the second
    "preToolUse": [{"command": cmd, "timeout": 10, "failClosed": True,
                    "matcher": "^(Write|Edit|MultiEdit|Delete|NotebookEdit)$"}],
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
          "Add the three hook entries from ports/cursor/hooks.json manually, "
          "with REPLACE_WITH_HOME replaced by your home directory." % (path, exc))
    sys.exit(7)

hooks = data.setdefault("hooks", {})
if not isinstance(hooks, dict):
    hooks = data["hooks"] = {}
data.setdefault("version", 1)

added = []
for event, entries_list in entries.items():
    existing = hooks.setdefault(event, [])
    if not isinstance(existing, list):
        existing = hooks[event] = []
    if any("cursor-hook" in json.dumps(h) for h in existing if isinstance(h, dict)):
        continue
    existing.extend(entries_list)
    added.append(event)

if added:
    with open(path, "w") as f:
        json.dump(data, f, indent=2)
        f.write("\n")
    print("cursor: hooks written -> %s (%s)" % (path, ", ".join(added)))
else:
    print("cursor: operator-lite hooks already present in %s" % path)
PYEOF
rc=$?
if [ "$rc" -eq 7 ]; then
    say ""
    say "MANUAL STEPS NEEDED: $HOOKS_TARGET is not valid JSON, so it was left"
    say "untouched. Open it, fix or remove the broken content, then either rerun"
    say "this installer or merge the entries from $PORT_DIR/hooks.json into it,"
    say "replacing REPLACE_WITH_HOME with your home directory ($HOME):"
    say ""
    sed "s|REPLACE_WITH_HOME|$HOME|g" "$PORT_DIR/hooks.json" | sed 's/^/    /'
    say ""
    die "hooks.json invalid; see manual steps above."
elif [ "$rc" -ne 0 ]; then
    die "could not write $HOOKS_TARGET"
fi

# --- next steps -------------------------------------------------------------- #
say ""
say "Done. Next steps:"
say "  1. Cursor watches hooks config files and reloads them automatically;"
say "     if the hooks do not load, restart Cursor. (User scope: covers every"
say "     project. Add the same entries to a project .cursor/hooks.json to"
say "     override per project.)"
say "  2. Prove it: sh $PORT_DIR/test.sh"
say "  3. Inside Cursor, ask the agent to run:"
say "        echo OPERATOR_CANARY_STOP_7f3a"
say "     Shadow mode logs the STOP to ~/.operator/receipts/ without blocking."
say "  4. Review receipts: python3 $GATE_DST status ; python3 $GATE_DST verify"
say "  5. When the shadow log looks right, flip to blocking:"
say "        echo enforce > $MODE_DST"
say "  6. failClosed is true: if this adapter cannot run at all, Cursor blocks"
say "     the action instead of allowing it. Remove the \"failClosed\": true"
say "     lines in $HOOKS_TARGET to relax that."
say "  7. Uninstall: remove the operator-lite entries from $HOOKS_TARGET"
say "     (or delete the file if it only contains them)."
