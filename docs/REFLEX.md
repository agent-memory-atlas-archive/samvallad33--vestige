# Reflex

Reflex is the Vestige tool that agent hooks call directly. Every hook firing reaches the server that owns
your store, and every firing leaves a line in a hash-chained Hook Ledger.

Its first job is the Operator stop. The Operator gate decides and blocks. Reflex reads what the gate
decided, saves each stop as a memory linked to its rule, tells the stopped agent once, and remembers how
each stop was answered. The gate stays the only thing that can block a call, and Reflex never allows,
retries or routes around a stop.

## What happens on a hook firing

On every firing Reflex reads the gate's day files under `~/.operator/receipts` from where it last
stopped. `$OPERATOR_HOME` moves that folder. Each new line is checked against the digest chain the gate
writes: it must name the digest of the line before it, and its own digest must match its content. A line
that breaks the chain is counted and never becomes a memory.

| Gate receipt | What Reflex records |
|---|---|
| `STOP` | An `operator-stop` memory tagged `op-receipt-<16 hex>`, `derived_from` the rule's memory, which is tagged `op-rule-<rule>`. These are the same tags and text the older `operator-stop-memory.py` hook writes, so `recall(handle='op-receipt-…')` works either way. |
| `PERMIT_GRANTED` | An `operator-resolution` memory with a `closes` edge to the stop it answers, found by the permit's `stop_receipt`. |
| `ALLOW` by a law's own compliant path, or by a used permit | An `operator-resolution` memory with a `closes` edge to the newest open stop of that rule or action in the same session. |

A stop already in memory, written by any other hook, is reused and never written twice.

Only the calling session's rows are written before the hook answers. Every other row, including the
history Reflex finds the first time it runs, drains in one background task per store, so a backlog never
slows a tool call.

## What the agent is told

| Event | What Reflex adds to the agent's context | Budget |
|---|---|---|
| `PreToolUse` | A stop of this session the agent has not heard about yet, then a warning when this exact command was stopped before | 600 characters |
| `PostToolUse`, `PostToolUseFailure`, `PostToolBatch` | Stops of this session the agent has not heard about yet | 600 characters |
| `UserPromptSubmit` | The same, and the turn budget resets | 2,000 characters |
| `SubagentStart` | The stops already told in this session, which hold for the subagent too | 1,500 characters |
| `SessionStart` | The newest stops on this machine from the last seven days, each with the handle to recall it; after compaction, the stops this session was already told | 2,000 characters |
| `Stop` and every other event | Nothing; Reflex records only | 0 |

Each stop is told once per session. Across every event, Reflex adds at most 4,000 characters to a turn.

When a later stop hits a rule that was answered before, the notice names that answer and its memory. A
command's fingerprint is the first 16 hex characters of the SHA-256 of the command with its whitespace
collapsed, so the exact command is recognized without storing it again.

## What the stop memory holds

Besides the gate's own fields, a stop memory names the memories that were in the agent's context at that
exact tool call. Reflex learns them from the agent's own Vestige reads on the same MCP connection, and
takes the snapshot at `PreToolUse`. The gate's receipt names that call by its `tool_use_id`.

## The Hook Ledger

Every firing appends one line to `<data dir>/reflex/ledger.jsonl` with the event, the session, the
characters added, the receipts told, the memories written and the time it took. Each line carries
`prev`, the digest of the line before it, and `digest`, the SHA-256 of the line's compact JSON with sorted
keys and without `digest`. Any edit to a line breaks the chain from that point on. This is a reference
digest that detects edits, not a signature.

To read it:

```bash
vestige reflex --since 24h
```

It reads only `<data dir>/reflex/`, never the log, so it works while a server owns the store. It reports
firings by event, characters told, stops and resolutions recorded, whether the ledger chain is intact, and
the receipt reader's own counters.

## Installing

`npx @vestige/init` registers Reflex when the installed `vestige-mcp` is 6.0.0 or newer.

- **Claude Code** calls Reflex from `mcp_tool` hooks on `PreToolUse`, `PostToolUse`,
  `PostToolUseFailure`, `UserPromptSubmit`, `SubagentStart` and `Stop`. `SessionStart` at launch fires
  before MCP servers connect, so it uses the command shim `~/.vestige/memory/hooks/reflex.cjs`.
- **Codex, Gemini CLI and Cursor** use the same command shim on their matching events.

None of these hooks can block a call. The Operator gate stays a separate command hook that fails closed.

While a Reflex server is active it refreshes `<data dir>/reflex/heartbeat`, and the older
`operator-stop-memory.py` hook steps aside so a stop is never handled twice. With an older server, or
none, that hook works exactly as before.

## Files

| Path | Holds |
|---|---|
| `<data dir>/reflex/state.json` | Read positions in the gate's day files, the last digest read, recorded stops, rules, open stops and the background queue |
| `<data dir>/reflex/ledger.jsonl` | The Hook Ledger; it rotates to `ledger.jsonl.1` past 8 MB |
| `<data dir>/reflex/heartbeat` | The time of the last firing |
