#!/bin/sh
# Operator Lite -- Amazon Q Developer CLI (Kiro CLI) port installer.
# POSIX sh, stdlib-only chain.
#
#   sh install.sh
#
# Does four things, idempotently:
#   1. installs the gate to ~/.operator/gate/operator-gate.py
#      (downloads from GitHub unless an existing copy is same-or-newer;
#      never overwrites a newer gate)
#   2. installs the adapter next to it (q-wrapper.py)
#   3. sets mode to shadow if no mode file exists yet
#   4. wires the preToolUse hook into Q's agent config:
#        - your configured default agent's file, if `chat.defaultAgent`
#          points at an agent file that exists, else
#        - ~/.aws/amazonq/cli-agents/q_cli_default.json (created mirroring
#          Q's built-in default if it does not exist yet)
#      (a timestamped backup is written first; existing fields and
#      existing hooks are preserved; an invalid existing file is left
#      untouched and manual steps are printed)
#
# Installs are run by the owner in their own terminal -- same guard as the gate.
set -u

PORT_DIR=$(cd "$(dirname "$0")" && pwd)
GATE_URL="https://raw.githubusercontent.com/samvallad33/vestige/main/operator-lite/operator-gate.py"
REPO_GATE="$PORT_DIR/../../operator-gate.py"
WRAPPER_SRC="$PORT_DIR/q-wrapper.py"

OP_HOME="${OPERATOR_HOME:-$HOME/.operator}"
GATE_DIR="$OP_HOME/gate"
GATE_DST="$GATE_DIR/operator-gate.py"
WRAPPER_DST="$GATE_DIR/q-wrapper.py"

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
[ -f "$WRAPPER_SRC" ] || die "q-wrapper.py missing next to install.sh"
cp "$WRAPPER_SRC" "$WRAPPER_DST"
chmod 755 "$WRAPPER_DST"
say "amazon-q: adapter installed -> $WRAPPER_DST"

# --- mode: shadow unless the owner already chose ----------------------------- #
MODE_DST="$OP_HOME/mode"
if [ ! -f "$MODE_DST" ]; then
    printf 'shadow\n' > "$MODE_DST"
    say "operator-gate: mode=shadow (log-only). Flip with: echo enforce > $MODE_DST"
else
    say "operator-gate: mode=$(cat "$MODE_DST") (existing mode kept)"
fi

# --- wire the hook into Q's agent config ------------------------------------- #
AGENTS_DIR="$HOME/.aws/amazonq/cli-agents"
mkdir -p "$AGENTS_DIR" || die "cannot create $AGENTS_DIR"
FRAGMENT="$PORT_DIR/agent-hook.json"

# The merge runs in python (stdlib): it picks the target agent file, backs it
# up, merges the preToolUse entry, and reports what it did.
WIRING_LOG="$GATE_DIR/.wiring.log"
python3 - "$AGENTS_DIR" "$FRAGMENT" >"$WIRING_LOG" 2>&1 <<'PYEOF'
import json
import os
import sys
import time

agents_dir, fragment_path = sys.argv[1], sys.argv[2]
home = os.path.expanduser("~")

# Q stores settings at data_local_dir/amazon-q/settings.json with the key
# "chat.defaultAgent" (crates/chat-cli/src/util/paths.rs, database/settings.rs).
if sys.platform == "darwin":
    base = os.path.join(home, "Library", "Application Support")
else:
    base = os.path.join(home, ".local", "share")
settings_path = os.path.join(base, "amazon-q", "settings.json")

entry = {
    "command": "python3 \"$HOME/.operator/gate/q-wrapper.py\"",
    "timeout_ms": 10000,
}

# 1. pick the target agent file
default_agent = ""
try:
    with open(settings_path) as f:
        default_agent = str((json.load(f) or {}).get("chat.defaultAgent") or "")
except Exception:
    pass

target = ""
origin = ""
if default_agent:
    cand = os.path.join(agents_dir, default_agent + ".json")
    if os.path.isfile(cand):
        target, origin = cand, "chat.defaultAgent=%r" % default_agent
if not target:
    target = os.path.join(agents_dir, "q_cli_default.json")
    origin = "q_cli_default (overrides Q's built-in default for plain `q chat`)"

data = None
if os.path.isfile(target):
    try:
        with open(target) as f:
            data = json.load(f)
        if not isinstance(data, dict):
            raise ValueError("top level is not an object")
    except Exception as exc:
        print("INVALID_JSON %s" % target)
        print("REASON %s" % exc)
        sys.exit(7)

created = False
if data is None:
    created = True
    # Fresh q_cli_default.json mirrors Q's in-memory default exactly
    # (crates/chat-cli/src/cli/agent/mod.rs, impl Default for Agent):
    # tools ["*"], the workspace resources + rules glob, legacy mcp.json on.
    # The Agent schema is deny_unknown_fields; every key below is a real field.
    data = {
        "$schema": "https://raw.githubusercontent.com/aws/amazon-q-developer-cli"
                   "/refs/heads/main/schemas/agent-v1.json",
        "name": os.path.splitext(os.path.basename(target))[0],
        "description": "Default agent",
        "tools": ["*"],
        "resources": ["file://AmazonQ.md", "file://AGENTS.md", "file://README.md",
                      "file://.amazonq/rules/**/*.md"],
        "hooks": {},
        "useLegacyMcpJson": True,
    }

# 2. back up an existing target before touching it
if not created:
    backup = "%s.backup-operator-%s" % (target, time.strftime("%Y%m%d-%H%M%S"))
    with open(target) as f:
        body = f.read()
    with open(backup, "w") as f:
        f.write(body)
    print("BACKUP %s" % backup)

# 3. merge the preToolUse entry (existing hooks preserved, dedupe on q-wrapper)
hooks = data.setdefault("hooks", {})
if not isinstance(hooks, dict):
    hooks = data["hooks"] = {}
pre = hooks.setdefault("preToolUse", [])
if not isinstance(pre, list):
    pre = hooks["preToolUse"] = []
already = any("q-wrapper" in json.dumps(h) for h in pre if isinstance(h, dict))
if not already:
    pre.append(entry)
    with open(target, "w") as f:
        json.dump(data, f, indent=2)
        f.write("\n")
    print("WROTE %s" % target)
    print("MODE %s" % ("created" if created else "merged"))
else:
    print("PRESENT %s" % target)

# 4. warn when a workspace agent file shadows the global one we wired
ws = os.path.join(os.getcwd(), ".amazonq", "cli-agents", os.path.basename(target))
if os.path.isfile(ws):
    print("SHADOWED %s" % ws)

print("ORIGIN %s" % origin)
PYEOF
rc=$?

if [ "$rc" -eq 7 ]; then
    TARGET=$(sed -n 's/^INVALID_JSON //p' "$WIRING_LOG" | head -n 1)
    say ""
    say "MANUAL STEPS NEEDED: ${TARGET:-the agent config} is not valid JSON, so it"
    say "was left untouched. Open it, fix or remove the broken content, then"
    say "either rerun this installer or merge this fragment into the agent file's"
    say "top-level object (combining with any existing \"hooks\" object):"
    say ""
    sed 's/^/    /' "$FRAGMENT"
    say ""
    die "agent config invalid; see manual steps above."
elif [ "$rc" -ne 0 ]; then
    cat "$WIRING_LOG" >&2
    die "could not wire the agent config"
fi

WROTE=$(sed -n 's/^WROTE //p' "$WIRING_LOG" | head -n 1)
PRESENT=$(sed -n 's/^PRESENT //p' "$WIRING_LOG" | head -n 1)
MODE=$(sed -n 's/^MODE //p' "$WIRING_LOG" | head -n 1)
BACKUP=$(sed -n 's/^BACKUP //p' "$WIRING_LOG" | head -n 1)
SHADOWED=$(sed -n 's/^SHADOWED //p' "$WIRING_LOG" | head -n 1)
ORIGIN=$(sed -n 's/^ORIGIN //p' "$WIRING_LOG" | head -n 1)
[ -n "$BACKUP" ] && say "amazon-q: backed up existing agent file -> $BACKUP"
if [ -n "$WROTE" ]; then
    say "amazon-q: preToolUse hook $MODE -> $WROTE  (target: $ORIGIN)"
elif [ -n "$PRESENT" ]; then
    say "amazon-q: gate hook already present in $PRESENT"
fi
if [ -n "$SHADOWED" ]; then
    say ""
    say "WARNING: $SHADOWED also exists and workspace agents override global"
    say "ones with the same name. The gate will NOT fire from that directory"
    say "unless you add the same hooks entry there (fragment: $FRAGMENT)."
fi
rm -f "$WIRING_LOG"

# --- next steps -------------------------------------------------------------- #
say ""
say "Done. Next steps:"
say "  1. Start a NEW q chat session (agent files are read at startup)."
say "     Plain 'q chat' uses the wired default agent; other agents need the"
say "     hooks entry added to their own file (fragment: $FRAGMENT)."
say "  2. Prove it: sh $PORT_DIR/test.sh"
say "  3. Review receipts: python3 $GATE_DST status ; python3 $GATE_DST verify"
say "  4. When the shadow log looks right, flip to blocking:"
say "        echo enforce > $MODE_DST"
say "  5. To disable without uninstalling: remove the preToolUse entry (or set"
say "     hooks: {}) from $AGENTS_DIR/q_cli_default.json -- a backup of the"
say "     pre-install file is kept next to it."
