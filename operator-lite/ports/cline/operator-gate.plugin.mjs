/**
 * Vestige Operator Lite -- Cline plugin (SDK/CLI route)
 *
 * For the Cline CLI and SDK, plugins install with:
 *
 *     cline plugin install /path/to/operator-gate.plugin.mjs
 *     cline plugin install https://raw.githubusercontent.com/samvallad33/vestige/main/operator-lite/ports/cline/operator-gate.plugin.mjs
 *
 * (Command syntax per https://docs.cline.bot/sdk/guides/writing-plugins and
 * the official example sdk/examples/plugins/env-blocker.ts in cline/cline.
 * NOTE: this route is documented-but-untested locally -- the file-based
 * PreToolUse hook in this directory is the verified, primary install.)
 *
 * Contract (verified against sdk/examples/plugins/env-blocker.ts and
 * sdk/packages/agents/src/agent-runtime.ts):
 *   beforeTool runs BEFORE tool policies and BEFORE user approval.
 *   return { skip: true, reason }  -> the tool call never runs; `reason`
 *                                     becomes the tool result the model reads.
 *   return undefined               -> normal flow.
 *
 * The gate (one stdlib-only Python file) classifies; it never executes what it
 * inspects. Node builtins only -- no npm dependencies.
 */

import { spawn } from "node:child_process";
import { homedir } from "node:os";
import { join } from "node:path";

const GATE_TIMEOUT_MS = 8000;
const MAX_REASON = 2000;

const GATE =
  process.env.OPERATOR_GATE_PY ||
  join(
    process.env.OPERATOR_HOME || join(homedir(), ".operator"),
    "gate",
    "operator-gate.py",
  );

const DESTRUCTIVE_LIKE =
  /\brm\s+-[a-zA-Z]*[rR]|\bmv\s+\S*(vestige|Developer|\.zcode|\.claude)|\bpush\b.*--force|\bDROP\s+TABLE|\.vestige[/\\]vestige\.db|fly\s+deploy|mkfs|\bdd\s+if=|OPERATOR_CANARY_STOP_7f3a/i;

const TOOL_MAP = {
  write_to_file: "write_file",
  apply_diff: "edit",
  replace_in_file: "edit",
  search_and_replace: "edit",
  insert_content: "edit",
  use_mcp_tool: "mcp__use_mcp_tool",
};

function extractCommands(input) {
  if (input == null) return [];
  if (typeof input === "string") return [input];
  if (Array.isArray(input)) return input.filter((c) => typeof c === "string");
  if (typeof input === "object") {
    const v = input.command ?? input.commands ?? input.cmd;
    if (typeof v === "string") return [v];
    if (Array.isArray(v)) return v.filter((c) => typeof c === "string");
  }
  return [];
}

function runGate(payload) {
  return new Promise((resolve) => {
    let settled = false;
    const done = (r) => {
      if (!settled) {
        settled = true;
        resolve(r);
      }
    };
    let child;
    try {
      child = spawn(process.env.OPERATOR_PYTHON || "python3", [GATE, "hook", "--source", "cline"], {
        env: { ...process.env, OPERATOR_AGENT_SESSION: "1" },
        stdio: ["pipe", "ignore", "pipe"],
      });
    } catch (err) {
      return done({ ok: false, err: String(err) });
    }
    let stderr = "";
    child.stderr?.on("data", (d) => {
      stderr += d.toString();
    });
    const timer = setTimeout(() => {
      try {
        child.kill("SIGKILL");
      } catch {}
      done({ ok: false, err: "gate timeout" });
    }, GATE_TIMEOUT_MS);
    child.on("error", (err) => {
      clearTimeout(timer);
      done({ ok: false, err: String(err) });
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      done({ ok: true, code: code ?? 1, reason: stderr.trim() });
    });
    try {
      child.stdin.write(JSON.stringify(payload));
      child.stdin.end();
    } catch (err) {
      clearTimeout(timer);
      done({ ok: false, err: String(err) });
    }
  });
}

async function beforeTool({ toolCall, input }) {
  const toolRaw = String(toolCall?.toolName ?? "").trim();
  if (!toolRaw) return undefined;

  const mapped = TOOL_MAP[toolRaw];
  const isShell = !mapped && extractCommands(input).length > 0;
  if (!mapped && !isShell) return undefined;

  const gateTool = isShell ? toolRaw : mapped;
  let toolInput;
  if (isShell) {
    toolInput = { command: extractCommands(input).join("\n") };
  } else if (input && typeof input === "object" && !Array.isArray(input)) {
    toolInput = input;
  } else {
    toolInput = { command: String(input ?? "") };
  }

  const payload = {
    tool_name: gateTool,
    tool_input: toolInput,
    cwd: process.cwd(),
    session_id: undefined,
  };

  const res = await runGate(payload);
  if (res.ok && res.code === 0) return undefined;
  if (res.ok && res.code === 2) {
    return { skip: true, reason: (res.reason || "OPERATOR: STOPPED").slice(0, MAX_REASON) };
  }

  if (DESTRUCTIVE_LIKE.test(JSON.stringify(toolInput))) {
    return {
      skip: true,
      reason:
        "OPERATOR: STOPPED (degraded) -- the gate is unreachable and this call looks destructive. " +
        `Ask the owner. (${res.err})`,
    };
  }
  return undefined; // gate unreachable, non-destructive-looking: fail open
}

const plugin = {
  name: "vestige-operator-gate",
  manifest: {
    capabilities: ["hooks"],
  },
  hooks: {
    beforeTool,
  },
};

export { plugin };
export default plugin;
