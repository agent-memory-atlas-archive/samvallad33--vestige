#!/bin/sh
# Vestige Operator Lite -- Cline installer (POSIX sh).
#
#   1. ensures the gate at ~/.operator/gate/operator-gate.py
#      (downloaded from the Vestige repo if missing; never overwritten)
#   2. sets shadow mode (log-only) if no mode file exists yet
#   3. installs the PreToolUse hook to ~/.cline/hooks/PreToolUse
#      (an existing different file is backed up, never clobbered)
#   4. if a `cline` CLI is present, also attempts the SDK plugin install;
#      otherwise prints the exact manual steps
#
# Run it from your own terminal. It never enables enforce mode for you.

set -u

REPO_RAW="https://raw.githubusercontent.com/samvallad33/vestige/main/operator-lite"
GATE_URL="$REPO_RAW/operator-gate.py"
HOOK_URL="$REPO_RAW/ports/cline/PreToolUse"
PLUGIN_URL="$REPO_RAW/ports/cline/operator-gate.plugin.mjs"

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
GATE_ROOT="${OPERATOR_HOME:-$HOME/.operator}"
GATE_DIR="$GATE_ROOT/gate"
GATE="$GATE_DIR/operator-gate.py"
HOOK_DIR="$HOME/.cline/hooks"
HOOK="$HOOK_DIR/PreToolUse"

note() { printf '%s\n' "$*"; }
die()  { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

fetch() { # fetch <url> <dest>
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL --retry 2 -o "$2" "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$2" "$1"
    else
        return 1
    fi
}

command -v python3 >/dev/null 2>&1 \
    || die "python3 is required (the gate is stdlib-only Python 3.9+)."

# --- 1. gate -----------------------------------------------------------------
REPO_GATE="$SCRIPT_DIR/../../operator-gate.py"
if [ -f "$GATE" ]; then
    note "gate:      already installed at $GATE"
elif [ -f "$REPO_GATE" ]; then
    mkdir -p "$GATE_DIR" || die "cannot create $GATE_DIR"
    cp "$REPO_GATE" "$GATE" && chmod 755 "$GATE"
    note "gate:      installed from checkout $REPO_GATE"
else
    mkdir -p "$GATE_DIR" || die "cannot create $GATE_DIR"
    TMPG="$GATE_DIR/.operator-gate.py.tmp.$$"
    fetch "$GATE_URL" "$TMPG" || die "cannot download the gate from $GATE_URL"
    [ -s "$TMPG" ] || die "downloaded gate is empty"
    grep -q "operator-gate" "$TMPG" || { rm -f "$TMPG"; die "downloaded file does not look like the gate"; }
    chmod 755 "$TMPG"
    mv "$TMPG" "$GATE"
    note "gate:      installed $GATE"
fi

# --- 2. mode (shadow first, always) ------------------------------------------
if [ -f "$GATE_ROOT/mode" ]; then
    note "mode:      $(cat "$GATE_ROOT/mode") (existing file left untouched)"
else
    mkdir -p "$GATE_ROOT" 2>/dev/null
    echo shadow > "$GATE_ROOT/mode"
    note "mode:      shadow (log-only). Flip later with: echo enforce > $GATE_ROOT/mode"
fi

# --- 3. PreToolUse hook --------------------------------------------------------
mkdir -p "$HOOK_DIR" || die "cannot create $HOOK_DIR"
SRC="$SCRIPT_DIR/PreToolUse"
TMPH="$HOOK_DIR/.PreToolUse.tmp.$$"
if [ -f "$SRC" ]; then
    cp "$SRC" "$TMPH"
else
    fetch "$HOOK_URL" "$TMPH" || die "cannot find ./PreToolUse next to install.sh and cannot download it"
fi
if [ -f "$HOOK" ] && ! cmp -s "$TMPH" "$HOOK"; then
    BAK="$HOOK.bak.$(date +%Y%m%d%H%M%S)"
    cp "$HOOK" "$BAK"
    note "hook:      existing $HOOK backed up to $BAK"
fi
chmod 755 "$TMPH"
mv "$TMPH" "$HOOK"
note "hook:      installed $HOOK"

# --- 4. plugin route (best effort) ---------------------------------------------
if command -v node >/dev/null 2>&1; then
    note "node:      $(node --version)"
else
    note "node:      NOT FOUND -- the PreToolUse hook needs node (brew install node)"
fi

if command -v cline >/dev/null 2>&1; then
    PLUG="$SCRIPT_DIR/operator-gate.plugin.mjs"
    if [ ! -f "$PLUG" ]; then
        PLUG="$HOOK_DIR/.operator-gate.plugin.tmp.$$.mjs"
        fetch "$PLUGIN_URL" "$PLUG" || PLUG=""
    fi
    if [ -n "$PLUG" ]; then
        note "cline CLI: found -- attempting 'cline plugin install' (SDK/CLI route)..."
        cline plugin install "$PLUG" && note "plugin:    installed via cline CLI" \
            || note "plugin:    'cline plugin install' failed -- the file-based hook above still covers the VS Code extension"
        case "$PLUG" in *.tmp.*.mjs) rm -f "$PLUG" ;; esac
    fi
else
    note ""
    note "cline CLI: not found on PATH -- skipping the SDK plugin route."
    note "           The VS Code extension uses the file-based hook above, which is already installed."
fi

note ""
note "Last step (required, VS Code extension):"
note "  open Cline Settings -> Features and enable Hooks, then restart the task."
note "  Hooks are macOS/Linux only in Cline (no Windows support yet)."
note ""
note "Verify:"
note "  sh test.sh                 # gate contract, Cline-shaped payloads"
note "  echo 'OPERATOR_CANARY_STOP_7f3a' as a task command in shadow mode"
note "  tail -f $GATE_ROOT/receipts/$(date +%Y-%m-%d).jsonl"
note ""
note "When the shadow log looks right:  echo enforce > $GATE_ROOT/mode"
