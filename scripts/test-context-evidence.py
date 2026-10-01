#!/usr/bin/env python3
"""Stdio checks for startup validation and exact code-context reads.

Strata does not verify source anchors by text overlap. This script checks
the argument gate, an exact get of a force-created node, an empty code
context, and that the data dir never grows a SQLite file.
"""
import argparse
import json
import os
from pathlib import Path
import select
import subprocess
import tempfile


def run(binary):
    results = []
    with tempfile.TemporaryDirectory(prefix="vestige-context-evidence-") as temp:
        root = Path(temp)
        env = dict(os.environ, VESTIGE_DASHBOARD_ENABLED="false", VESTIGE_HTTP_ENABLED="false", RUST_LOG="error")
        proc = subprocess.Popen(
            [str(binary.resolve()), "--no-http", "--data-dir", str(root / "store")],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            env=env, text=True, bufsize=1,
        )
        seq = 0

        def rpc(method, params):
            nonlocal seq
            seq += 1
            proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": seq, "method": method, "params": params}) + "\n")
            proc.stdin.flush()
            while True:
                if not select.select([proc.stdout], [], [], 40)[0]:
                    raise TimeoutError(method)
                line = proc.stdout.readline()
                if not line:
                    raise RuntimeError("MCP exited")
                response = json.loads(line)
                if response.get("id") == seq:
                    if "error" in response:
                        raise RuntimeError(response["error"])
                    return response["result"]

        def tool(name, args, error=False):
            result = rpc("tools/call", {"name": name, "arguments": args})
            if error:
                assert result.get("isError"), result
                return result.get("structuredContent") or json.loads(result["content"][0]["text"])
            assert not result.get("isError"), result
            return result.get("structuredContent") or json.loads(result["content"][0]["text"])

        def passed(case):
            results.append(case)
            print("PASS", case, flush=True)

        try:
            rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                               "clientInfo": {"name": "context-evidence-fixture", "version": "1"}})
            proc.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
            proc.stdin.flush()
            bounds = tool("session_start", {"queries": ["q"] * 17}, error=True)
            assert "16" in bounds["error"]
            blank = tool("session_start", {"queries": [], "scope": "   "}, error=True)
            assert "scope" in blank["error"]
            passed("startup validates query bounds and namespace before retrieval")

            created = tool("smart_ingest", {
                "content": "STRATA_CONTEXT_NODE exact handle",
                "forceCreate": True,
            })
            node_id = created["nodeId"]
            got = tool("memory", {"action": "get", "id": node_id})
            assert "STRATA_CONTEXT_NODE" in json.dumps(got)
            context = tool("codebase", {"action": "get_context", "codebase": "fixture"})
            assert node_id not in json.dumps(context)
            queried = tool("session_start", {"queries": ["STRATA_CONTEXT_NODE"], "include_predictions": False})
            assert queried["notices"][0].startswith("queries ignored (1)"), queried
            assert node_id not in json.dumps(queried), queried
            assert "STRATA_CONTEXT_NODE" not in queried["context"], queried
            passed("exact get works; query startup and untagged code context do not invent a hit")

            bad = [
                str(path) for path in (root / "store").rglob("*")
                if path.is_file() and path.name.lower().endswith((".db", ".sqlite", ".sqlite3"))
            ]
            assert not bad, bad
            passed("context session creates no sqlite file")
        finally:
            proc.terminate()
            proc.wait(timeout=10)
    print(f"PASS {len(results)} context-evidence checks")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, default=Path("target/debug/vestige-mcp"))
    args = parser.parse_args()
    run(args.binary)
