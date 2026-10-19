#!/bin/sh
# Operator Lite -- Continue port installer. POSIX sh, stdlib-only chain.
#
#   sh install.sh
#
# Does three things, idempotently:
#   1. installs the gate to ~/.operator/gate/operator-gate.py
#      (downloads from GitHub unless an existing copy is same-or-newer;
#      never overwrites a newer gate)
#   2. sets mode to enforce if no mode file exists yet; a shadow an install
#      before 0.3.9 wrote becomes enforce, once
#   3. merges a preToolUse hook into ~/.continue/hooks.json
#      (user scope; a timestamped backup is written before the first change;
#      existing hooks and settings are preserved; an invalid existing file
#      is left untouched and manual steps are printed)
#
# No adapter ships with this port. The hook command is the gate itself:
# stdin is the gate contract {"tool_name","tool_input","cwd","session_id"?},
# exit 0 allows, exit 2 blocks with the reason on stderr. The command is
# guarded: `[ -f <gate> ] && python3 <gate> hook --source continue`. A missing
# gate file makes the shell test exit 1, so a removed gate is not reported as
# a policy STOP (exit 2).
#
# This installer writes ~/.continue/hooks.json (Cursor-shaped preToolUse) so
# the gate is registered once.
#
# Installs are run by the owner in their own terminal -- same guard as the gate.
set -u

PORT_NAME="continue"
PORT_DIR=$(cd "$(dirname "$0")" && pwd)
GATE_URL="https://raw.githubusercontent.com/samvallad33/vestige/main/operator-lite/operator-gate.py"
REPO_GATE="$PORT_DIR/../../operator-gate.py"

OP_HOME="${OPERATOR_HOME:-$HOME/.operator}"
GATE_DIR="$OP_HOME/gate"
GATE_DST="$GATE_DIR/operator-gate.py"
CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
HOOKS_TARGET="$HOME/.continue/hooks.json"

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
# the checkout's own gate wins when it is newer than main's (a branch being tested, a release not yet merged)
if [ -f "$REPO_GATE" ] && grep -q '^VERSION = ' "$REPO_GATE" && \
    { [ "$SRC" = "" ] || ! version_ge "$(gate_version "$SRC")" "$(gate_version "$REPO_GATE")"; }; then
    [ "$SRC" = "$TMP_GATE" ] && rm -f "$TMP_GATE"
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

# --- mode: enforce unless the owner already chose ----------------------------- #
MODE_DST="$OP_HOME/mode"
NEW_MODE=""
[ -f "$MODE_DST" ] || { printf 'enforce\n' > "$MODE_DST"; NEW_MODE=1; }
# Installs before 0.3.9 wrote shadow, which blocks nothing. The gate settles the mode once (a shadow the
# owner chose with its mode command stays) and records that it did, so a later `echo shadow` is kept.
SETTLED=""
if grep -q '^EDITION = "lite"' "$GATE_DST" 2>/dev/null; then
    SETTLED=$(python3 -I "$GATE_DST" mode --settle 2>/dev/null) || SETTLED=""
fi
case "$SETTLED" in
    *"("*) say "operator-gate: $SETTLED. To record without blocking: echo shadow > $MODE_DST" ;;
    *) if [ -n "$NEW_MODE" ]; then
           say "operator-gate: mode=enforce (blocking). To record without blocking: echo shadow > $MODE_DST"
       else
           say "operator-gate: mode=$(cat "$MODE_DST") (existing mode kept)"
       fi ;;
esac

# --- wire preToolUse into the user hooks.json -------------------------------- #
# Absolute path, shell-quoted, guarded. The test fails closed to exit 1 when
# the gate file is gone, and the gate's own exit 2 passes through when it is
# there. Hosts run this string through a shell.
mkdir -p "$(dirname "$HOOKS_TARGET")" || die "cannot create $(dirname "$HOOKS_TARGET")"
PYQ=$(python3 -c 'import shlex,sys; print(shlex.quote(sys.argv[1]))' "$GATE_DST") || die "could not quote the gate path"
HOOK_CMD="[ -f $PYQ ] && python3 $PYQ hook --source ${PORT_NAME}"

python3 - "$HOOKS_TARGET" "$HOOK_CMD" "$PORT_NAME" <<'PYEOF'
import json, os, sys, time

path, hook_cmd, port = sys.argv[1], sys.argv[2], sys.argv[3]
entry = {
    "command": hook_cmd,
    "timeout": 10,
    "failClosed": True,
    "matcher": "*",
}
marker = "hook --source %s" % port

def ours(obj):
    try:
        blob = json.dumps(obj)
    except Exception:
        return False
    return "operator-gate.py" in blob and marker in blob

try:
    with open(path) as f:
        raw = f.read()
except FileNotFoundError:
    raw = None
except Exception as exc:
    print("install.sh: cannot read %s (%s); not touching it." % (path, exc))
    sys.exit(8)

if raw is None:
    data = {}
else:
    try:
        data = json.loads(raw)
        if not isinstance(data, dict):
            raise ValueError("top level is not an object")
    except Exception as exc:
        print("install.sh: %s exists but is not valid JSON (%s); not touching it." % (path, exc))
        print("Add a preToolUse entry whose command is:")
        print("  %s" % hook_cmd)
        sys.exit(7)

if ours(data):
    print("%s: operator-lite hook already present in %s" % (port, path))
    sys.exit(0)

hooks = data.get("hooks", {})
if hooks is None:
    hooks = {}
if not isinstance(hooks, dict):
    print('install.sh: existing "hooks" is not an object; not touching %s.' % path)
    sys.exit(7)
pre = hooks.get("preToolUse", [])
if not isinstance(pre, list):
    print("install.sh: existing hooks.preToolUse is not a list; not touching %s." % path)
    sys.exit(7)

if raw is not None:
    backup = "%s.backup-operator-%s" % (path, time.strftime("%Y%m%d-%H%M%S"))
    try:
        with open(backup, "w") as b:
            b.write(raw)
    except Exception as exc:
        print("install.sh: could not write backup %s (%s); target left untouched." % (backup, exc))
        sys.exit(8)
    print("%s: backed up existing hooks.json -> %s" % (port, backup))

pre.append(entry)
hooks["preToolUse"] = pre
data["hooks"] = hooks
if "version" not in data:
    data["version"] = 1

tmp = path + ".tmp-operator"
try:
    with open(tmp, "w") as f:
        json.dump(data, f, indent=2)
        f.write("\n")
    os.replace(tmp, path)
except Exception as exc:
    try:
        os.remove(tmp)
    except OSError:
        pass
    print("install.sh: could not write %s (%s)" % (path, exc))
    sys.exit(8)
print("%s: hooks written -> %s (preToolUse)" % (port, path))
PYEOF
rc=$?
if [ "$rc" -eq 7 ]; then
    say ""
    say "MANUAL STEPS NEEDED: $HOOKS_TARGET is not valid JSON (or its hooks"
    say "object is the wrong shape), so it was left untouched. Open it, fix"
    say "or remove the broken content, then rerun this installer."
    die "hooks.json invalid; see manual steps above."
elif [ "$rc" -ne 0 ]; then
    die "could not write $HOOKS_TARGET"
fi

# --- next steps -------------------------------------------------------------- #
say ""
say "Done. Next steps:"
say "  1. Restart $PORT_NAME so it reloads $HOOKS_TARGET."
say "  2. The hook command is guarded: a missing gate file exits 1, not 2."
say "  3. Prove the gate: python3 $GATE_DST corpus guardfall"
say "  4. Inside the agent, ask it to run the gate canary. The gate stops it"
say "     and writes a receipt to $OP_HOME/receipts/."
say "  5. Review receipts: python3 $GATE_DST status ; python3 $GATE_DST verify"
say "  6. To record without blocking instead:"
say "        echo shadow > $MODE_DST"
say "  7. Uninstall: remove the operator-lite preToolUse entry from"
say "     $HOOKS_TARGET (or delete the file if it only contains that entry)."
exit 0
