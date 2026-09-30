#!/usr/bin/env python3
"""Stdio contracts for the Strata-backed vestige-mcp binary.

Discovery stays exact. Writes go through the gate. Similarity (embeddings,
cosine, BM25, FTS, Jaccard, keyword or name match) is an error. A link
exists only when the log recorded an edge. No SQLite file is created.
"""
import argparse
import json
import os
from pathlib import Path
import select
import subprocess
import tempfile


def run(binary, output):
    transcript = []
    coverage = []
    with tempfile.TemporaryDirectory(prefix="vestige-tool-frontier-") as temp:
        root = Path(temp)
        store = root / "store"
        env = {k: v for k, v in os.environ.items() if not k.startswith(("VESTIGE_", "REDMINE_", "GITHUB_"))}
        env.update(VESTIGE_DASHBOARD_ENABLED="false", VESTIGE_HTTP_ENABLED="false", RUST_LOG="error")
        proc = None
        seq = 0

        def spawn():
            nonlocal proc
            proc = subprocess.Popen(
                [str(binary.resolve()), "--no-http", "--data-dir", str(store)],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL, env=env, text=True, bufsize=1,
            )

        def rpc(method, params):
            nonlocal seq
            seq += 1
            request = {"jsonrpc": "2.0", "id": seq, "method": method, "params": params}
            proc.stdin.write(json.dumps(request) + "\n")
            proc.stdin.flush()
            while True:
                if not select.select([proc.stdout], [], [], 60)[0]:
                    raise TimeoutError(method)
                line = proc.stdout.readline()
                if not line:
                    raise RuntimeError("MCP process exited")
                response = json.loads(line)
                if response.get("id") == seq:
                    transcript.append({"request": request, "response": response})
                    assert "error" not in response, response
                    return response["result"]

        def tool(name, args, error=False):
            result = rpc("tools/call", {"name": name, "arguments": args})
            assert bool(result.get("isError")) == error, result
            coverage.append({
                "tool": name,
                "selector": args.get("action", args.get("mode", args.get("view", "default"))),
                "outcome": "expected_error" if error else "success",
            })
            return result.get("structuredContent") or json.loads(result["content"][0]["text"])

        def typed(name, args, needle):
            body = tool(name, args, error=True)
            assert needle in body["error"], body
            return body

        def handshake():
            rpc("initialize", {
                "protocolVersion": "2025-11-25", "capabilities": {},
                "clientInfo": {"name": "tool-frontier-fixture", "version": "1"},
            })
            proc.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
            proc.stdin.flush()

        def passed(name):
            print("PASS", name, flush=True)

        def assert_no_sqlite():
            bad = []
            if store.exists():
                for path in store.rglob("*"):
                    name = path.name.lower()
                    if path.is_file() and (
                        name.endswith(".sqlite") or name.endswith(".sqlite3")
                        or name.endswith(".db") or name.endswith(".db-wal")
                        or name.endswith(".db-shm")
                    ):
                        bad.append(str(path))
            assert not bad, bad

        spawn()
        try:
            handshake()
            catalog = rpc("tools/list", {})["tools"]
            names = [x["name"] for x in catalog]
            assert len(names) == 16, names
            assert "source_sync" not in names, names
            assert "purge" not in names and "suppress" in names, names
            memory_actions = next(x for x in catalog if x["name"] == "memory")["inputSchema"]["properties"]["action"]["enum"]
            assert "purge" not in memory_actions and "delete" not in memory_actions, memory_actions
            guide = tool("memory_status", {"view": "tools"})["tools"]
            assert [x["name"] for x in guide] == names
            for entry, definition in zip(guide, catalog):
                for selector in ("action", "mode", "view"):
                    values = definition["inputSchema"].get("properties", {}).get(selector, {}).get("enum")
                    if values is not None:
                        assert entry["selectors"][selector]["values"] == values
                detail = tool("memory_status", {"view": "tools", "tool": entry["name"]})
                full = detail["tools"][0]["inputSchema"]
                assert full.get("type") == definition["inputSchema"].get("type")
                assert len(json.dumps(full)) >= len(json.dumps(definition["inputSchema"]))
                for selector in ("action", "mode", "view"):
                    compact_enum = definition["inputSchema"].get("properties", {}).get(selector, {}).get("enum")
                    if compact_enum is not None:
                        full_enum = full.get("properties", {}).get(selector, {}).get("enum")
                        assert full_enum == compact_enum, (
                            f"{entry['name']}: {selector} enum drifted under compaction"
                        )
            for invalid in ("search", "", 12):
                tool("memory_status", {"view": "tools", "tool": invalid}, error=True)
            annotations = {x["name"]: x["annotations"] for x in catalog}
            assert annotations["recall"]["readOnlyHint"] is False
            assert annotations["recall"]["idempotentHint"] is False
            passed("all installed tool and action definitions match progressive discovery")

            typed("maintain", {"action": "consolidate", "phase": "embeddings", "batchSize": 2},
                  "phase must be all, lifecycle or logs")
            tool("maintain", {"action": "consolidate", "batchSize": 2}, error=True)
            tool("maintain", {"action": "consolidate", "phase": "embeddings", "batchSize": 101}, error=True)
            passed("embedding maintenance is refused; it is not a Strata operation")
            for phase in ("lifecycle", "logs"):
                page = tool("maintain", {"action": "consolidate", "phase": phase, "batchSize": 2})
                assert page["dryRun"] is True and page["hasMore"] is False and page["selected"] == 0
            tool("maintain", {"action": "consolidate", "phase": "logs", "after": "invalid"}, error=True)
            passed("lifecycle/log maintenance previews empty pages and rejects invalid controls")

            marker = "STRATA_FIXTURE_EXACT_HANDLE"
            created = tool("smart_ingest", {"content": marker, "forceCreate": True, "tags": ["fixture-old"]})
            node_id = created["nodeId"]
            assert node_id.startswith("mem-") and created["success"] is True
            got = tool("memory", {"action": "get", "id": node_id})
            assert marker in json.dumps(got)
            tool("memory", {"action": "get_batch", "ids": [node_id]})
            tool("memory", {"action": "state", "id": node_id})
            receipt = tool("receipt", {"action": "get", "receipt_id": node_id})
            assert "receipt" in receipt
            typed("recall", {"query": marker}, "similarity_disabled")
            typed("recall", {"query": "which fixture handle did we store"}, "similarity_disabled")
            handle = tool("recall", {"handle": node_id})
            assert marker in json.dumps(handle) and handle["exact"] is True
            passed("ingest, exact get, write receipt, and handle recall; query recall is refused")

            created_intention = tool("intention", {
                "action": "set",
                "description": "Synthetic reminder",
                "trigger": {"type": "time", "at": "2020-01-01T00:00:00Z"},
            })
            intention_id = created_intention["intentionId"]
            assert created_intention["success"] is True
            assert created_intention["receiptId"].startswith("eff-")
            assert intention_id in created_intention["receipt"]["retrieved"]
            listed = tool("intention", {"action": "list"})
            assert any(row["id"] == intention_id for row in listed["intentions"])
            passed("intention set admits a receipt and lists the row")

            proc.terminate()
            proc.wait(timeout=10)
            assert_no_sqlite()
            spawn()
            handshake()
            again = tool("memory", {"action": "get", "id": node_id})
            assert marker in json.dumps(again)
            restarted = tool("intention", {"action": "list"})
            assert any(row["id"] == intention_id and row["description"] == "Synthetic reminder"
                       for row in restarted["intentions"])
            assert_no_sqlite()
            passed("empty-dir restart keeps the node and creates no sqlite file")

            typed("recall", {"mode": "reason", "query": marker}, "similarity_disabled")
            typed("recall", {"mode": "contradictions"}, "similarity_disabled")
            replay_args = {"action": "replay", "receipt_id": node_id, "withheld_slots": []}
            replayed = tool("receipt", replay_args)
            repeated = tool("receipt", replay_args)
            assert repeated == replayed
            assert replayed["kind"] == "strata" and replayed["matched"] is True
            assert replayed["mismatches"] == [] and replayed["readOnly"] is True
            assert replayed["nodeId"] == node_id and replayed["stateDigest"] == replayed["replayedDigest"]
            passed("receipt replay matches the log and repeats")
            promoted = tool("memory", {"action": "promote", "id": node_id, "reason": "fixture"})
            assert promoted["action"] == "promoted" and promoted["success"] is True
            promote_receipt = promoted["receiptId"]
            assert promote_receipt.startswith("eff-")
            proved = tool("receipt", {"action": "get", "receipt_id": promote_receipt})
            assert proved["attestation"]["verification"]["locallyVerified"] is True
            assert proved["receipt"]["mutations"][0]["kind"] == "promoted"
            assert node_id in proved["receipt"]["retrieved"]
            demoted = tool("memory", {"action": "demote", "id": node_id, "reason": "fixture"})
            assert demoted["action"] == "demoted" and "NOT deleted" in demoted["note"]
            assert demoted["receiptId"].startswith("eff-") and demoted["receiptId"] != promote_receipt
            edited = tool("memory", {"action": "edit", "id": node_id, "content": "edited fixture"})
            successor = edited["nodeId"]
            assert edited["action"] == "edit" and edited["embeddingStatus"] == "refused"
            assert successor != node_id and edited["rule"] == "edit" and edited["supersedes"] == node_id
            edit_receipt = tool("receipt", {"action": "get", "receipt_id": edited["receiptId"]})
            assert edit_receipt["attestation"]["verification"]["locallyVerified"] is True
            assert edit_receipt["receipt"]["mutations"][0]["kind"] == "edited"
            assert "rule=edit" in edit_receipt["receipt"]["mutations"][0]["note"]
            assert "edited fixture" in json.dumps(tool("memory", {"action": "get", "id": successor}))
            hidden_edit = tool("memory", {"action": "get", "id": node_id})
            assert hidden_edit["message"] == "retired, can't be retrieved"
            retired = tool("recall", {"handle": node_id})
            assert node_id not in json.dumps(retired.get("nodes", []))
            live = tool("recall", {"handle": successor})
            assert "edited fixture" in json.dumps(live)
            typed("memory", {"action": "promote", "id": "not-a-handle"}, "Invalid memory ID")
            typed("memory", {"action": "edit", "id": "mem-ffffffffffffffff", "content": "nope"}, "not found")
            doomed = tool("smart_ingest", {"content": "STRATA_PURGE_DOOMED", "forceCreate": True})
            doomed_id = doomed["nodeId"]
            # 4.0 withholds erasure on Strata: every route refuses, the node stays.
            typed("purge", {"id": doomed_id, "confirm": True}, "unavailable_in_4_0")
            typed("memory", {"action": "purge", "id": doomed_id, "confirm": True}, "unavailable_in_4_0")
            typed("memory", {"action": "delete", "id": doomed_id, "confirm": True}, "unavailable_in_4_0")
            typed("delete_knowledge", {"id": doomed_id, "confirm": True}, "unavailable_in_4_0")
            still = tool("memory", {"action": "get", "id": doomed_id})
            assert "STRATA_PURGE_DOOMED" in json.dumps(still)
            context = tool("codebase", {"action": "get_context", "codebase": "fixture"})
            assert marker not in json.dumps(context)
            project_preview = tool("project", {"action": "preview"})
            defaults = tool("project", {})
            project_again = tool("project", {"action": "preview"})
            assert defaults == project_preview == project_again
            assert project_preview["action"] == "preview" and project_preview["scope"] == "user"
            assert project_preview["itemCount"] == 0 and marker not in json.dumps(project_preview["region"])
            target_root = root / "projection"
            target_root.mkdir()
            write_args = {
                "action": "write",
                "path": "CLAUDE.md",
                "root": str(target_root),
                "confirm": True,
            }
            written = tool("project", write_args)
            assert written["action"] == "write" and written["written"] is True
            assert written.get("refused") is not True
            assert written["receipt"]["receiptId"].startswith("eff-")
            assert written["receipt"]["hash"]
            target = target_root / "CLAUDE.md"
            first_bytes = target.read_bytes()
            assert b"vestige:projection:begin" in first_bytes
            assert marker.encode() not in first_bytes
            second = tool("project", write_args)
            assert second["written"] is False and second["receipt"]["hash"] == written["receipt"]["hash"]
            assert target.read_bytes() == first_bytes
            after = tool("project", {})
            assert after["region"] == project_preview["region"] and after["itemCount"] == 0
            passed("project {}, preview, and write complete; an untagged fact is not projected")
            checked = tool("intention", {"action": "check", "context": {
                "current_time": "2020-01-02T00:00:00Z"}})
            assert any(row["id"] == intention_id for row in checked["triggered"])
            assert checked["receiptId"].startswith("eff-")
            updated = tool("intention", {"action": "update", "id": intention_id, "status": "complete"})
            assert updated["success"] is True and updated["receiptId"].startswith("eff-")
            fulfilled = tool("intention", {"action": "list", "filter_status": "fulfilled"})
            assert any(row["id"] == intention_id for row in fulfilled["intentions"])
            planned = tool("intention", {
                "action": "graph",
                "scope": "user",
                "at": "2026-10-01T09:00:00Z",
                "command": {
                    "action": "plan",
                    "id": "fixture-plan",
                    "description": "Synthetic graph plan",
                    "requirements": [],
                    "conflict_keys": [],
                },
            })
            assert isinstance(planned.get("journal_seq"), int) and planned["journal_seq"] >= 1
            graph_replay = tool("intention", {
                "action": "graph",
                "scope": "user",
                "command": {"action": "replay"},
            })
            assert graph_replay["matched"] is True and graph_replay["commands"] == 1
            explained = tool("intention", {
                "action": "graph",
                "scope": "user",
                "at": "2026-10-01T09:00:00Z",
                "command": {"action": "explain", "id": "fixture-plan"},
            })
            assert "Synthetic graph plan" in json.dumps(explained)
            passed("intention graph replays the recorded plan")
            typed("intention", {"action": "set", "description": "  "}, "empty")
            # connectors is off in a default build: source_sync is not a tool.
            seq += 1
            request = {
                "jsonrpc": "2.0", "id": seq, "method": "tools/call",
                "params": {"name": "source_sync", "arguments": {"source": "gitlab", "repo": "a/b"}},
            }
            proc.stdin.write(json.dumps(request) + "\n")
            proc.stdin.flush()
            while True:
                if not select.select([proc.stdout], [], [], 60)[0]:
                    raise TimeoutError("source_sync")
                line = proc.stdout.readline()
                if not line:
                    raise RuntimeError("MCP process exited")
                response = json.loads(line)
                if response.get("id") == seq:
                    transcript.append({"request": request, "response": response})
                    assert "result" not in response, response
                    assert response["error"]["code"] == -32602, response
                    assert "Unknown tool" in response["error"]["message"], response
                    assert "source_sync" in response["error"]["message"], response
                    break
            for view in ("health", "retention", "timeline", "changelog", "stats", "coverage"):
                tool("memory_status", {"view": view})
            score = tool("maintain", {"action": "importance_score", "content": "Synthetic fixture design decision"})
            assert isinstance(score.get("composite"), (int, float))
            gc = tool("maintain", {"action": "gc"})
            assert gc.get("dryRun", gc.get("dry_run")) is True
            assert gc.get("deleted", gc.get("candidateCount", 0)) == 0 or gc.get("candidateCount") == 0
            preview = tool("dedup", {"action": "tag_rename", "source_tag": "fixture-old", "target_tag": "fixture-new"})
            assert preview.get("wouldWrite") is False
            recent = tool("graph", {"action": "recent", "limit": 5})
            assert recent.get("events", recent.get("compositions", [])) == [] or recent.get("count", 0) == 0 or "events" in recent
            never = tool("graph", {"action": "never_composed", "limit": 5})
            assert never["scope"] == "user" and never["globalNoveltyVerified"] is False
            bridge = tool("ghostlink", {"mode": "propose", "limit": 5})
            assert bridge["lens"] == "bridge" and bridge["globalNoveltyVerified"] is False
            assert "admission" in bridge, bridge
            divergent = tool("ghostlink", {"mode": "propose", "lens": "divergent", "limit": 5})
            assert all(c["proof"]["noEdgeVerified"] is True for c in divergent["candidates"]), divergent
            typed("session_start", {"queries": [marker], "include_predictions": False, "include_intentions": False}, "similarity_disabled")
            doomed_suppress = tool("smart_ingest", {"content": "STRATA_SUPPRESS_DOOMED", "forceCreate": True})
            suppress_id = doomed_suppress["nodeId"]
            typed("blast_radius", {"action": "retire", "ids": [suppress_id], "reason": "fixture"}, "unavailable_in_4_0")
            passed("purge and delete are withheld on Strata and change nothing")
            assert annotations["suppress"]["destructiveHint"] is True
            suppressed = tool("suppress", {"id": suppress_id, "reason": "fixture"})
            assert suppressed["success"] is True and suppressed["rule"] == "suppress"
            assert str(suppressed["receiptId"]).startswith("eff-")
            assert "STRATA_SUPPRESS_DOOMED" not in json.dumps(suppressed)
            hidden_suppress = tool("memory", {"action": "get", "id": suppress_id})
            assert hidden_suppress["message"] == "retired, can't be retrieved"
            assert "STRATA_SUPPRESS_DOOMED" not in json.dumps(hidden_suppress)
            hidden_recall = tool("recall", {"handle": suppress_id})
            assert "STRATA_SUPPRESS_DOOMED" not in json.dumps(hidden_recall)
            typed("suppress", {"id": suppress_id, "reverse": True}, "unavailable_in_4_0")
            passed("suppress hides a memory from every read; reverse is refused on Strata")
            anchored_repo = root / "anchored-repo"
            (anchored_repo / "src").mkdir(parents=True)
            anchored_source = anchored_repo / "src" / "state.rs"
            anchored_source.write_text(
                "use std::fs;\n\npub fn load_config(path: &str) -> Config {\n"
                "    let raw = fs::read_to_string(path).unwrap();\n    parse(&raw)\n}\n"
            )
            anchored_files = ["src/state.rs#load_config"]
            saved_pattern = tool("codebase", {
                "action": "remember_pattern", "name": "Eager config read",
                "description": "load_config reads the whole file eagerly",
                "files": anchored_files, "repoPath": str(anchored_repo), "codebase": "anchored",
            })
            saved_anchors = saved_pattern["anchors"]
            assert saved_anchors["count"] == 1 and saved_anchors["verifiable"] == 1, saved_pattern
            assert saved_anchors["recorded"] == 1 and saved_anchors.get("error") is None, saved_pattern
            pattern_id = saved_pattern["nodeId"]
            verify_args = {"action": "verify", "codebase": "anchored", "repoPath": str(anchored_repo)}
            context_args = {"action": "get_context", "codebase": "anchored", "repoPath": str(anchored_repo)}
            fresh_report = tool("codebase", verify_args)
            assert fresh_report["checked"] == 1 and fresh_report["fresh"] == 1, fresh_report
            assert fresh_report["stale"] == 0, fresh_report
            current = tool("codebase", context_args)
            assert current["patterns"]["items"][0]["anchorStatus"] == "verified", current
            assert current["staleMemories"] == [], current
            anchored_source.write_text("pub fn load_config(path: &str) -> Config {\n    Config::from_env()\n}\n")
            drift_report = tool("codebase", verify_args)
            assert drift_report["stale"] == 1 and drift_report["fresh"] == 0, drift_report
            assert drift_report["staleMemories"][0]["id"] == pattern_id, drift_report
            assert drift_report["staleMemories"][0]["status"] == "drifted", drift_report
            stale_context = tool("codebase", context_args)
            assert stale_context["staleMemories"] == [pattern_id], stale_context
            assert stale_context["patterns"]["items"][0]["stale"] is True, stale_context
            reanchored = tool("codebase", {
                "action": "reanchor", "memoryId": pattern_id,
                "repoPath": str(anchored_repo), "files": anchored_files,
            })
            assert reanchored["anchorsReplaced"] == 1, reanchored
            assert reanchored["memoryContentChanged"] is False, reanchored
            reanchored_report = tool("codebase", verify_args)
            assert reanchored_report["fresh"] == 1 and reanchored_report["stale"] == 0, reanchored_report
            pattern_receipt = tool("receipt", {"action": "get", "receipt_id": pattern_id})
            assert pattern_receipt["receipt"]["mutations"][0]["kind"] == "created", pattern_receipt
            proc.terminate()
            proc.wait(timeout=10)
            spawn()
            handshake()
            replayed_report = tool("codebase", verify_args)
            assert replayed_report["fresh"] == 1 and replayed_report["stale"] == 0, replayed_report
            assert_no_sqlite()
            passed("codebase anchors record, verify fresh, flag drift, reanchor, and replay after restart")
            unanchored = tool("causal_walk", {"scope": "user"})
            assert unanchored["status"] == "completed" and unanchored["causes"] == []
            assert unanchored["needs_report"]["missing"] == ["node_id"], unanchored
            walked = tool("causal_walk", {"node_id": successor})
            assert walked["start"] == successor and walked["direction"] == "backward"
            assert walked["truncated"] is False and walked["needs_report"] is None
            selftest = tool("selftest", {})
            assert selftest["all_passed"] is True and selftest["deterministic"] is True
            assert selftest["checks_passed"] == selftest["checks_total"] > 0, selftest
            lessons = tool("forgotten_lesson", {"failure_id": successor})
            assert lessons["failure_id"] == successor and isinstance(lessons["forgotten_lessons"], list)
            passed("causal_walk, selftest and forgotten_lesson answer from recorded edges only")
            called = {row["tool"] for row in coverage}
            missing = [name for name in names if name not in called]
            assert not missing, missing
            assert_no_sqlite()
            passed(f"all {len(names)} tools answered on Strata: real writes, or a typed error")
        finally:
            if proc and proc.poll() is None:
                proc.terminate()
                proc.wait(timeout=10)
            if output:
                output.write_text(json.dumps({
                    "catalog": locals().get("catalog", []),
                    "coverage": coverage,
                    "transcript": transcript,
                }, indent=2) + "\n")
    print(f"PASS {len(coverage)} tool calls; coverage lists successful and expected-error paths explicitly")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, default=Path("target/debug/vestige-mcp"))
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    run(args.binary, args.output)
