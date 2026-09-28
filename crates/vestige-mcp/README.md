# Vestige MCP Server

Local cognitive memory for MCP-compatible AI agents.

This crate provides the `vestige-mcp` stdio MCP server plus the `vestige` CLI.
The cognitive engine lives in `vestige-core`; this crate owns protocol handling,
tool dispatch, optional dashboard serving, backups, restore, update, and
portable import/export commands.

## Install

For normal users, prefer the release package:

```bash
npm install -g vestige-mcp-server
```

For local development:

```bash
cargo build --release -p vestige-mcp
```

## Register With An MCP Client

Use the command `vestige-mcp` in any stdio MCP client:

```json
{
  "mcpServers": {
    "vestige": {
      "command": "vestige-mcp"
    }
  }
}
```

Examples:

```bash
claude mcp add vestige vestige-mcp -s user
codex mcp add vestige -- vestige-mcp
```

## Transports

- Default: JSON-RPC 2.0 over stdio.
- Optional: MCP-over-HTTP on `/mcp`, enabled only with `--http`,
  `--http-port`, or `VESTIGE_HTTP_ENABLED=1`.
- Dashboard: `vestige dashboard` or `VESTIGE_DASHBOARD_ENABLED=1`.

HTTP and dashboard bearer tokens are generated locally; see
[`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).

## Current Tool Surface

The server exposes the current unified MCP tools from
[`src/server.rs`](src/server.rs), including:

- `backfill`, `codebase`, `dedup`, `graph`, `intention`, `maintain`
- `memory`, `memory_status`, `project`, `purge`, `recall`
- `receipt` (inspect persisted retrieval receipts and run controlled evidence replay)
- `session_start`, `smart_ingest`, `source_sync`, `suppress`

For the receipt workflow, replay claim boundary, signature status, durability,
and privacy limits, see [`docs/DECISION_RECEIPTS.md`](../../docs/DECISION_RECEIPTS.md).
See the root [`README.md`](../../README.md) and
[`docs/AGENT-MEMORY-PROTOCOL.md`](../../docs/AGENT-MEMORY-PROTOCOL.md) for
agent instructions.

## License

AGPL-3.0-only
