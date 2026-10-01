#!/bin/sh
# Operator Lite -- Windsurf (Cascade) port installer. POSIX sh, stdlib-only chain.
#
#   sh install.sh                # user-level: every workspace (~/.codeium/windsurf/hooks.json)
#   sh install.sh <workspace>    # workspace-level: <workspace>/.windsurf/hooks.json
#                                # (or <workspace>/.devin/hooks.json when one exists)
#
# Does four things, idempotently:
#   1. installs the gate to ~/.operator/gate/operator-gate.py
#      (downloads from GitHub unless an existing copy is same-or-newer;
#      never overwrites a newer gate)
#   2. installs the Windsurf adapter next to it (windsurf-wrapper.py)
#   3. sets mode to shadow if no mode file exists yet
#   4. merges the pre_run_command / pre_write_code / pre_mcp_tool_use hooks
#      into hooks.json (existing hooks are preserved; the original is backed
#      up first; an invalid existing file is left untouched and reported)
#
# Windsurf needs an adapter (unlike Crush): its hook payload on stdin carries
# tool_info.command_line / tool_info.cwd -- not the gate's
# tool_input.command / cwd contract -- so the hook command is the wrapper,
# which reshapes and hands the gate its own contract. Exit 2 with stderr as
# the reason blocks the tool call and Cascade shows the agent the reason.
#
# Installs are run by the owner in their own terminal -- same guard as the gate.
set -u

PORT_DIR=$(cd "$(dirname "$0")" && pwd)
GATE_URL="https://raw.githubusercontent.com/samvallad33/vestige/main/operator-lite/operator-gate.py"
REPO_GATE="$PORT_DIR/../../operator-gate.py"
WRAPPER_SRC="$PORT_DIR/windsurf-wrapper.py"

OP_HOME="${OPERATOR_HOME:-$HOME/.operator}"
GATE_DIR="$OP_HOME/gate"
GATE_DST="$GATE_DIR/operator-gate.py"
WRAPPER_DST="$GATE_DIR/windsurf-wrapper.py"
HOOK_CMD='python3 "$HOME/.operator/gate/windsurf-wrapper.py"'

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

# refuse to wire a gate that does not parse
python3 -c 'import sys; compile(open(sys.argv[1], encoding="utf-8").read(), sys.argv[1], "exec")' \
    "$GATE_DST" 2>/dev/null || die "$GATE_DST does not parse; not wiring a broken gate"

# --- install the adapter ---------------------------------------------------- #
[ -f "$WRAPPER_SRC" ] || die "windsurf-wrapper.py missing next to install.sh"
cp "$WRAPPER_SRC" "$WRAPPER_DST"
chmod 755 "$WRAPPER_DST"
say "windsurf: adapter installed -> $WRAPPER_DST"

# --- mode: shadow unless the owner already chose ----------------------------- #
MODE_DST="$OP_HOME/mode"
if [ ! -f "$MODE_DST" ]; then
    printf 'shadow\n' > "$MODE_DST"
    say "operator-gate: mode=shadow (log-only). Flip with: echo enforce > $MODE_DST"
else
    say "operator-gate: mode=$(cat "$MODE_DST") (existing mode kept)"
fi

# --- pick the hooks.json target ---------------------------------------------- #
# User level (default): ~/.codeium/windsurf/hooks.json covers every workspace.
# Workspace level (arg): the docs name .devin/hooks.json with the legacy
# .windsurf/hooks.json used when the .devin file is absent (or defines no
# hooks), so an existing .devin/hooks.json is merged into -- never shadowed.
WS_DIR="${1:-}"
if [ "$WS_DIR" != "" ]; then
    WS_DIR=$(cd "$WS_DIR" 2>/dev/null && pwd) || die "workspace dir not found: $1"
    if [ -f "$WS_DIR/.devin/hooks.json" ]; then
        TARGET="$WS_DIR/.devin/hooks.json"
    else
        TARGET="$WS_DIR/.windsurf/hooks.json"
    fi
else
    TARGET="$HOME/.codeium/windsurf/hooks.json"
fi
TARGET_DIR=$(dirname "$TARGET")
mkdir -p "$TARGET_DIR" || die "cannot create $TARGET_DIR"

# --- wire the three pre-hooks into hooks.json -------------------------------- #
# Merge is done in python3 (hooks.json is JSON; POSIX sh is the wrong tool).
# The original file is backed up before the first write; a file that is not
# valid JSON is left untouched and the manual merge instructions are printed.
python3 - "$TARGET" "$HOOK_CMD" <<'PYEOF'
import json, os, sys, time

path, hook_cmd = sys.argv[1], sys.argv[2]
EVENTS = ("pre_run_command", "pre_write_code", "pre_mcp_tool_use")
entry = {"command": hook_cmd}          # Windsurf hook schema is flat: command,
                                       # optional powershell/show_output/working_directory.

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
    print('  "hooks": { "pre_run_command": [ { "command": "%s" } ], ...' % hook_cmd)
    print('             (same entry under pre_write_code and pre_mcp_tool_use) }')
    sys.exit(7)

hooks = data.setdefault("hooks", {})
if not isinstance(hooks, dict):
    print("install.sh: existing \"hooks\" is not an object (%r); not touching it." % hooks)
    print("Merge the three pre-hook entries from ports/windsurf/hooks.json by hand.")
    sys.exit(7)

# The idempotency marker is the wrapper filename (the schema has no name field),
# with the gate filename accepted too for hand-written merges.
marker = "windsurf-wrapper"
blob = json.dumps(hooks)
already = marker in blob or "operator-gate" in blob
if already:
    print("windsurf: gate hook already present in %s" % path)
    sys.exit(0)

for ev in EVENTS:
    lst = hooks.setdefault(ev, [])
    if not isinstance(lst, list):
        print("install.sh: existing hooks.%s is not a list (%r); not touching it." % (ev, lst))
        print("Merge the %s entry from ports/windsurf/hooks.json by hand." % ev)
        sys.exit(7)
    lst.append(dict(entry))

backup = None
if raw is not None:
    backup = path + ".bak-" + time.strftime("%Y%m%d-%H%M%S")
    try:
        with open(backup, "w") as b:
            b.write(raw)
    except Exception as exc:
        print("install.sh: could not write backup %s (%s); target left untouched." % (backup, exc))
        sys.exit(8)
    print("windsurf: original saved -> %s" % backup)

with open(path, "w") as f:
    json.dump(data, f, indent=2)
    f.write("\n")
print("windsurf: pre_run_command / pre_write_code / pre_mcp_tool_use hooks written -> %s" % path)
PYEOF
rc=$?
if [ "$rc" -eq 7 ]; then
    die "existing $TARGET is invalid JSON or has an unexpected shape; fix it, then rerun."
elif [ "$rc" -eq 8 ]; then
    die "could not back up $TARGET; nothing was modified."
elif [ "$rc" -ne 0 ]; then
    die "could not write $TARGET"
fi

# --- next steps -------------------------------------------------------------- #
say ""
say "Done. Next steps:"
say "  1. Start a NEW Cascade session in Windsurf, then prove the wiring from inside:"
say "       ask Cascade to run:  echo OPERATOR_CANARY_STOP_7f3a"
say "     then check $OP_HOME/receipts/ for the OP-CANARY receipt."
say "  2. Or prove it offline: sh $PORT_DIR/test.sh"
say "  3. Review receipts: python3 $GATE_DST status ; python3 $GATE_DST verify"
say "  4. When the shadow log looks right, flip to blocking:"
say "        echo enforce > $MODE_DST"
say "  5. Uninstall: delete the operator-gate entries from $TARGET."
if [ "$WS_DIR" != "" ]; then
    say ""
    say "Note: workspace hook target is $TARGET."
    say "Windsurf reads .devin/hooks.json first; the legacy .windsurf/hooks.json is"
    say "used only when no .devin/hooks.json exists (or defines no hooks). If you"
    say "later add a .devin/hooks.json with hooks of its own, rerun this installer"
    say "so the gate entries land there too."
fi
