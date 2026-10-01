#!/bin/sh
# Operator Lite — OpenCode port installer.
#
# Owner-side script: installs the gate to ~/.operator/gate/, copies the plugin
# into OpenCode's plugin directory (global by default, --project for project
# scope), sets shadow mode if unset, and prints next steps.
#
# POSIX sh. No arguments required. `--project` installs into ./.opencode/plugin/
# of the current directory instead of the global config.
set -eu

if [ -n "${OPERATOR_AGENT_SESSION:-}" ]; then
  echo "Refusing: installs are run by the owner in their own terminal."
  exit 3
fi

REPO_RAW="https://raw.githubusercontent.com/samvallad33/vestige/main/operator-lite/operator-gate.py"

SRC_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
OP_HOME="${OPERATOR_HOME:-$HOME/.operator}"
GATE_DIR="$OP_HOME/gate"
GATE="$GATE_DIR/operator-gate.py"
CONFIG_ROOT="${XDG_CONFIG_HOME:-$HOME/.config}"

SCOPE=global
for arg in ${1+"$@"}; do
  case "$arg" in
    --project) SCOPE=project ;;
    *)
      echo "usage: sh install.sh [--project]" >&2
      exit 2
      ;;
  esac
done

if [ "$SCOPE" = project ]; then
  PLUGIN_DST="$PWD/.opencode/plugin/operator-lite.js"
  PLUGIN_ALT="global: $CONFIG_ROOT/opencode/plugin/operator-lite.js (applies to every project)"
else
  PLUGIN_DST="$CONFIG_ROOT/opencode/plugin/operator-lite.js"
  PLUGIN_ALT="project: ./.opencode/plugin/operator-lite.js (this project only)"
fi

# ---------------------------------------------------------------- gate -------
# Never overwrites an existing gate; upgrades are the owner's call.
if [ -f "$GATE" ]; then
  echo "gate:  already installed at $GATE (left untouched)"
else
  mkdir -p "$GATE_DIR"
  if [ -f "$SRC_DIR/operator-gate.py" ]; then
    cp "$SRC_DIR/operator-gate.py" "$GATE"
    echo "gate:  installed bundled copy -> $GATE"
  else
    TMP="$GATE_DIR/.operator-gate.py.$$"
    if command -v curl >/dev/null 2>&1; then
      curl -fsSL "$REPO_RAW" -o "$TMP"
    elif command -v wget >/dev/null 2>&1; then
      wget -qO "$TMP" "$REPO_RAW"
    else
      echo "error: need curl or wget (or the bundled operator-gate.py) to install the gate" >&2
      rm -f "$TMP"
      exit 1
    fi
    mv "$TMP" "$GATE"
    echo "gate:  downloaded -> $GATE"
  fi
  chmod 755 "$GATE"
  if command -v python3 >/dev/null 2>&1; then
    # syntax check; refuses to wire a broken gate
    python3 -c 'import sys; compile(open(sys.argv[1], encoding="utf-8").read(), sys.argv[1], "exec")' "$GATE"
  else
    echo "warning: python3 not found on PATH; OpenCode will fail-open on every shell call" >&2
  fi
fi

# ------------------------------------------------------------ mode file ------
mkdir -p "$OP_HOME"
if [ ! -f "$OP_HOME/mode" ]; then
  echo shadow > "$OP_HOME/mode"
  echo "mode:  shadow (log-only) written to $OP_HOME/mode"
else
  echo "mode:  existing $(cat "$OP_HOME/mode") left untouched ($OP_HOME/mode)"
fi

# --------------------------------------------------------------- plugin ------
mkdir -p "$(dirname -- "$PLUGIN_DST")"
cp "$SRC_DIR/index.js" "$PLUGIN_DST"
echo "plugin: installed -> $PLUGIN_DST"
echo "        alternative scope: $PLUGIN_ALT"

cat <<EOF

Next steps:
  1. Restart OpenCode (plugins load at startup).
  2. Verify the wiring:
       cd $(dirname -- "$0") && sh test.sh
     or end-to-end inside OpenCode: ask it to run
       echo OPERATOR_CANARY_STOP_7f3a
     Shadow mode logs the STOP to ~/.operator/receipts/ without blocking.
  3. When the log looks right, flip to enforce:
       echo enforce > $OP_HOME/mode
EOF
