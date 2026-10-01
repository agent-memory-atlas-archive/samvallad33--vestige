/**
 * Operator Lite — OpenCode plugin (official port)
 *
 * Carries the host-agnostic Operator gate contract to OpenCode:
 *
 *   gate:  python3 <operator-gate.py> hook --source opencode
 *   input: JSON on stdin  {"tool_name", "tool_input": {"command"}, "cwd", "session_id"}
 *   exit 0 -> allow (plugin returns normally; the gate already wrote its receipt)
 *   exit 2 -> block (plugin throws; OpenCode surfaces the Error message to the model)
 *
 * Verified against opencode 1.16.x:
 *   hook      "tool.execute.before" (input: { tool, sessionID, callID }, output: { args })
 *   blocking  a thrown Error aborts the tool call before execution; the Error's
 *             message text is the tool-error the model sees
 *   loading   project .opencode/plugin/ (or .opencode/plugins/, deprecated) and
 *             global ~/.config/opencode/plugin/ (or plugins/, deprecated).
 *             The loader iterates EVERY module export as a plugin function,
 *             so this module exports exactly one function.
 *
 * Gate path resolution order:
 *   $OPERATOR_GATE -> ~/.operator/gate/operator-gate.py -> bundled copy (./operator-gate.py)
 *
 * Failure posture: gate unreachable (spawn error, non-Python, 8s timeout) fails
 * OPEN unless the command plainly looks destructive (mirror of the gate's own
 * degraded-mode regex and the OpenClaw reference adapter).
 *
 * Node/bun builtins only — no npm dependencies.
 */
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const GATE_TIMEOUT_MS = 8000;
const BLOCK_PREFIX = "Operator Lite blocked:";

/**
 * "Plainly looks destructive" — a JS translation of the gate's own degraded_hit
 * regex (the gate's definition of fail-closed text) plus the OpenClaw reference
 * adapter's mkfs/dd terms. Used ONLY when the gate is unreachable.
 */
const DESTRUCTIVE_LIKE =
  /\brm\s+-[a-zA-Z]*[rR]|\bmv\s+\S*(vestige|Developer|\.zcode|\.claude)|\bpush\s+.*--force|\bDROP\s+TABLE|\.vestige\/vestige\.db|fly\s+deploy|OPERATOR_CANARY_STOP_7f3a|mkfs|\bdd\s+if=/i;

/** Resolve the gate script. Checked per call so tests can override via env. */
function gatePath() {
  const env = process.env.OPERATOR_GATE;
  if (env) return env;
  const installed = join(homedir(), ".operator", "gate", "operator-gate.py");
  if (existsSync(installed)) return installed;
  return fileURLToPath(new URL("./operator-gate.py", import.meta.url));
}

function extractCommand(args) {
  if (typeof args === "string") return args;
  if (!args || typeof args !== "object") return null;
  const c = args.command ?? args.cmd ?? args.script;
  return typeof c === "string" && c.trim() ? c : null;
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
      child = spawn(
        process.env.OPERATOR_PYTHON || "python3",
        [gatePath(), "hook", "--source", "opencode"],
        { env: { ...process.env, OPERATOR_AGENT_SESSION: "1" }, stdio: ["pipe", "ignore", "pipe"] },
      );
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
      done({ ok: false, err: `gate timeout after ${GATE_TIMEOUT_MS}ms` });
    }, GATE_TIMEOUT_MS);
    child.on("error", (err) => {
      clearTimeout(timer);
      done({ ok: false, err: String(err) });
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      const c = code ?? 1;
      // A genuine gate decision is: exit 0 silent, or exit 2 with an
      // "OPERATOR:" reason on stderr. Any other exit (missing interpreter,
      // unreadable gate file, crash) is "gate unreachable", not a block.
      const isStop = c === 2 && /(^|\n)\s*OPERATOR:/.test(stderr);
      if (c !== 0 && !isStop) {
        return done({ ok: false, err: stderr.trim() || `gate exited with code ${c}` });
      }
      done({ ok: true, code: c, reason: stderr.trim() });
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

export const OperatorLiteGate = async ({ client, directory, worktree }) => {
  const cwd = directory || worktree || process.cwd();
  const log = (level, message) => {
    try {
      client?.app
        ?.log?.({ service: "operator-lite", level, message })
        ?.catch?.(() => console.error(`operator-lite: ${message}`));
    } catch {
      console.error(`operator-lite: ${message}`);
    }
  };

  const gate = gatePath();
  log("info", `operator-lite: gate registered (${gate})`);

  return {
    "tool.execute.before": async (input, output) => {
      const tool = String(input?.tool ?? "").toLowerCase();
      const args = output?.args;
      const command = extractCommand(args);
      // Scope: shell/bash-type calls — anything carrying a shell command string
      // (OpenCode's shell tool id is "bash"). Other tools are not forwarded.
      if (command == null) return;

      const payload = {
        tool_name: tool || "bash",
        tool_input: { command },
        cwd,
        session_id: input?.sessionID,
      };

      const res = await runGate(payload);
      if (!res.ok) {
        // Gate unreachable: fail open unless the text plainly looks destructive.
        if (DESTRUCTIVE_LIKE.test(command)) {
          log("error", `gate unreachable and command looks destructive — blocking. ${res.err}`);
          throw new Error(
            `${BLOCK_PREFIX} gate unreachable and the command looks destructive; ask the owner. (${res.err})`,
          );
        }
        log("warn", `gate unreachable, failing open: ${res.err}`);
        return;
      }
      if (res.code === 2) {
        // Gate says STOP. The thrown message is what the model sees.
        throw new Error(`${BLOCK_PREFIX} ${res.reason || "OPERATOR: STOPPED"}`);
      }
      // exit 0: allowed (shadow/enforce receipts are written by the gate itself).
      return;
    },
  };
};
