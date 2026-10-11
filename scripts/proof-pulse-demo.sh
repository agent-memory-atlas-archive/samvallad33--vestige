#!/usr/bin/env sh
# Run a visual, isolated rehearsal of the Vestige setup wizard.
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
NODE_BIN="$(command -v node)"
DEMO_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/vestige-proof-pulse.XXXXXX")"
cleanup() { rm -rf "$DEMO_ROOT"; }
trap cleanup EXIT HUP INT TERM

mkdir -p "$DEMO_ROOT/bin" "$DEMO_ROOT/.config/opencode"
printf '#!/usr/bin/env sh\nexit 0\n' > "$DEMO_ROOT/bin/vestige-mcp"
chmod 755 "$DEMO_ROOT/bin/vestige-mcp"
cat > "$DEMO_ROOT/.config/opencode/opencode.json" <<'EOF'
{
  "mcpServers": {
    "another-local-tool": { "command": "/usr/local/bin/another-local-tool" }
  }
}
EOF

HOME="$DEMO_ROOT" \
PATH="$DEMO_ROOT/bin:/usr/bin:/bin" \
VESTIGE_MCP_PATH="$DEMO_ROOT/bin/vestige-mcp" \
"$NODE_BIN" "$ROOT/packages/vestige-init/bin/init.js" \
  --interactive --no-dashboard

printf '\n  Demo fixture only: the temporary local client config will now be removed.\n'
