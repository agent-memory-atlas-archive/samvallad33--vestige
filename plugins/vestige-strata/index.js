/**
 * Vestige Strata — OpenClaw memory plugin
 *
 * Gives the agent first-class tools over a local Strata log: the signed,
 * append-only causal log at the heart of Vestige 4.0. Every write passes the
 * log's gate, comes back with a replayable receipt, and recall is by exact
 * handle — no similarity search, no silent merges.
 *
 * Tools call the `vestige` CLI (Vestige 4.0+). Binary resolution order:
 *   1. $VESTIGE_BIN
 *   2. `vestige` on PATH
 *   3. ~/.local/bin/vestige, ~/.cargo/bin/vestige, /opt/homebrew/bin/vestige
 */
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir, platform } from "node:os";
import { join } from "node:path";
import { Type } from "typebox";

const FALLBACKS = [
  join(homedir(), ".local", "bin", "vestige"),
  join(homedir(), ".cargo", "bin", "vestige"),
  "/opt/homebrew/bin/vestige",
];

function resolveVestige() {
  if (process.env.VESTIGE_BIN) return process.env.VESTIGE_BIN;
  for (const p of FALLBACKS) {
    if (existsSync(p)) return p;
  }
  return "vestige";
}

const VESTIGE = resolveVestige();
const DEFAULT_TIMEOUT_MS = 30_000;

function defaultDataDir() {
  if (process.env.VESTIGE_DATA_DIR) return process.env.VESTIGE_DATA_DIR;
  if (platform() === "darwin") {
    return join(homedir(), "Library", "Application Support", "com.vestige.core");
  }
  return join(homedir(), ".local", "share", "vestige");
}

function run(args, { timeoutMs = DEFAULT_TIMEOUT_MS, stdin = null } = {}) {
  return new Promise((resolve) => {
    let child;
    try {
      child = spawn(VESTIGE, args, { stdio: [stdin != null ? "pipe" : "ignore", "pipe", "pipe"] });
    } catch (err) {
      return resolve({ ok: false, error: String(err) });
    }
    let stdout = "";
    let stderr = "";
    child.stdout?.on("data", (d) => (stdout += d.toString()));
    child.stderr?.on("data", (d) => (stderr += d.toString()));
    const timer = setTimeout(() => {
      try {
        child.kill("SIGKILL");
      } catch {}
      resolve({ ok: false, error: `vestige ${args[0]} timed out after ${timeoutMs}ms` });
    }, timeoutMs);
    child.on("error", (err) => {
      clearTimeout(timer);
      const missing = String(err?.code ?? "") === "ENOENT";
      resolve({
        ok: false,
        error: missing
          ? `The vestige CLI was not found (looked for $VESTIGE_BIN, PATH, ~/.local/bin, ~/.cargo/bin, Homebrew). Install Vestige 4.0: brew install samvallad33/tap/vestige — or grab the release archive: https://github.com/samvallad33/vestige#install`
          : String(err),
      });
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      resolve({ ok: code === 0, code: code ?? 1, stdout: stdout.trim(), stderr: stderr.trim() });
    });
    if (stdin != null) {
      try {
        child.stdin.write(stdin);
        child.stdin.end();
      } catch {}
    }
  });
}

function text(result, text) {
  return result({ content: [{ type: "text", text }] });
}

const tools = [
  {
    name: "strata_ingest",
    description:
      "Store a memory in the local Strata log. Nothing is merged by similarity; the write passes the log's gate and is admitted with a receipt. Use for decisions, patterns, incidents, and lessons that future sessions should not re-derive.",
    parameters: Type.Object({
      content: Type.String({ description: "The memory content to store (one self-contained fact or lesson)." }),
      tags: Type.Optional(Type.String({ description: "Comma-separated tags. Tags are exact handles for recall." })),
      node_type: Type.Optional(
        Type.String({
          description: "One of: fact, concept, event, person, place, note, pattern, decision. Default: fact.",
        }),
      ),
      source: Type.Optional(Type.String({ description: "Source reference, e.g. a file path, issue, or commit." })),
    }),
    async execute(_id, params) {
      const args = ["ingest", params.content, "--json"];
      if (params.tags) args.push("--tags", params.tags);
      if (params.node_type) args.push("--node-type", params.node_type);
      if (params.source) args.push("--source", params.source);
      const res = await run(args);
      if (!res.ok) return text(res, `ingest failed: ${res.error ?? res.stderr}`);
      return text(res, res.stdout || "stored");
    },
  },
  {
    name: "strata_recall",
    description:
      "Recall from the local Strata log by exact handle: a memory id, a unique id prefix (8+ chars), or an exact tag. Prints the memory and its one-hop recorded causal edges. Strata does no similarity search — if you do not have a handle, list tags via strata_stats or recall by a tag you stored.",
    parameters: Type.Object({
      handle: Type.String({ description: "Memory id, unique id prefix (8+ chars), or exact tag." }),
    }),
    async execute(_id, params) {
      const res = await run(["recall", "--handle", params.handle, "--json"]);
      if (!res.ok) return text(res, `recall failed: ${res.error ?? res.stderr}`);
      return text(res, res.stdout || "no match");
    },
  },
  {
    name: "strata_causal_walk",
    description:
      "Investigate a failure from an explicit start point: a bounded backward walk over recorded causal edges (closed_by, derived_from, evidence_of, touched). Returns hypotheses anchored in what the log actually recorded — never a guess. Refuses with needs_report when the start point has no recorded trail.",
    parameters: Type.Object({
      logged_write: Type.String({ description: "Memory / tool-call record id whose recorded edges are walked." }),
      lookback_days: Type.Optional(Type.Number({ description: "Days back the walk may reach. Default: 30." })),
    }),
    async execute(_id, params) {
      const args = ["causal-walk", "--logged-write", params.logged_write, "--json"];
      if (params.lookback_days) args.push("--lookback-days", String(Math.floor(params.lookback_days)));
      const res = await run(args, { timeoutMs: 60_000 });
      if (!res.ok) return text(res, `causal-walk failed: ${res.error ?? res.stderr}`);
      return text(res, res.stdout || "no trail");
    },
  },
  {
    name: "strata_compose",
    description:
      "List NEVER-COMPOSED memory pairs from the local Strata log as composition leads: two live memories in one scope with no recorded edge between them. Plain leads, no scores — try joining them.",
    parameters: Type.Object({
      limit: Type.Optional(Type.Number({ description: "How many pairs to list. Default: 10." })),
    }),
    async execute(_id, params) {
      const args = ["compose", "--json"];
      if (params.limit) args.push("--limit", String(Math.floor(params.limit)));
      const res = await run(args);
      if (!res.ok) return text(res, `compose failed: ${res.error ?? res.stderr}`);
      return text(res, res.stdout || "no pairs");
    },
  },
  {
    name: "strata_verify",
    description:
      "Verify the local Strata directory: replay the log, check the receipt chain, and report the signing-key fingerprint. Read-only. Note: the log has a single writer — stop any running Vestige server (or use the MCP tools) if this refuses.",
    parameters: Type.Object({
      dir: Type.Optional(
        Type.String({ description: "Strata directory to verify. Default: the active Vestige data directory." }),
      ),
    }),
    async execute(_id, params) {
      const dir = params.dir || defaultDataDir();
      const res = await run(["strata-verify", dir]);
      if (!res.ok) return text(res, `verify failed: ${res.error ?? res.stderr}`);
      return text(res, res.stdout || "ok");
    },
  },
  {
    name: "strata_stats",
    description: "Show statistics for the local Vestige store: memory counts by type, retention, and store health.",
    parameters: Type.Object({}),
    async execute(_id, _params) {
      const res = await run(["stats"]);
      if (!res.ok) return text(res, `stats failed: ${res.error ?? res.stderr}`);
      return text(res, res.stdout || "empty");
    },
  },
];

const plugin = {
  id: "vestige-strata",
  name: "Vestige Strata",
  description:
    "Local-first memory for your OpenClaw agent: a signed append-only causal log with gated writes, replayable receipts, FSRS-scheduled retention, exact-handle recall, and causal root-cause walks. Requires the vestige CLI (4.0+).",
  kind: "memory",

  register(api) {
    api.logger.info(`vestige-strata: registered (vestige=${VESTIGE})`);
    if (typeof api.registerTool !== "function") {
      api.logger.warn(
        "vestige-strata: this OpenClaw version does not expose api.registerTool; the plugin stays inactive.",
      );
      return;
    }
    for (const tool of tools) {
      api.registerTool({
        name: tool.name,
        description: tool.description,
        parameters: tool.parameters,
        async execute(id, params) {
          return tool.execute(id, params);
        },
      });
    }
  },
};

export default plugin;
