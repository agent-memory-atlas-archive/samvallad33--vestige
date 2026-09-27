#!/usr/bin/env python3
"""End-to-end manual verification for `source_sync` through the REAL
vestige-mcp binary: a local Redmine mock (issue removal -> reconcile ->
tombstone -> recall with source_status filters).

READ-ONLY with respect to the outside world: the "upstream" is a local HTTP
mock; nothing on GitHub or any real Redmine is touched. The store is a
throwaway VESTIGE_DATA_DIR.

Usage:
    python3 scripts/source-sync-redmine-e2e.py /path/to/vestige-mcp

Steps proved:
  1. source_sync (redmine, max_pages=1) creates cite-backed memories.
  2. recall with source_status=valid + sourceSystem=redmine finds them.
  3. Simulated upstream deletion (mock stops serving issue 2) + reconcile=true
     tombstones EXACTLY issue 2.
  4. recall source_status=valid no longer returns issue 2;
     recall source_status=tombstoned DOES return it.
  5. Error path: source_sync against a dead port fails LOUDLY with the exact
     failing call named (no silent empty result).
"""

import json
import os
import socket
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

BIN = sys.argv[1] if len(sys.argv) > 1 else "vestige-mcp"

# ---------------------------------------------------------------- Redmine mock

ISSUES_LIST_TEMPLATE = {
    "issues": [
        {
            "id": 1,
            "subject": "Disk full on db-primary",
            "description": "df -h shows 100% on /var",
            "status": {"id": 1, "name": "New"},
            "tracker": {"id": 1, "name": "Bug"},
            "priority": {"id": 2, "name": "Normal"},
            "author": {"id": 7, "name": "Jane Dev"},
            "done_ratio": 0,
            "updated_on": "2026-09-20T10:00:00Z",
        },
        {
            "id": 2,
            "subject": "Backup cron silent failure",
            "description": "cron exited 0 but no dump produced",
            "status": {"id": 2, "name": "In Progress"},
            "tracker": {"id": 2, "name": "Feature"},
            "priority": {"id": 3, "name": "High"},
            "author": {"id": 8, "name": "Bob Ops"},
            "done_ratio": 10,
            "updated_on": "2026-09-20T09:00:00Z",
        },
    ],
    "total_count": 2,
}

ISSUE_1 = dict(ISSUES_LIST_TEMPLATE["issues"][0], journals=[])
ISSUE_2 = dict(ISSUES_LIST_TEMPLATE["issues"][1], journals=[])

state = {"serve_issue_2": True}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        if self.path.startswith("/issues.json"):
            body = dict(ISSUES_LIST_TEMPLATE)
            if not state["serve_issue_2"]:
                body = {
                    "issues": [ISSUES_LIST_TEMPLATE["issues"][0]],
                    "total_count": 1,
                }
            self._json(200, body)
        elif self.path.startswith("/issues/1.json"):
            self._json(200, {"issue": ISSUE_1})
        elif self.path.startswith("/issues/2.json"):
            if state["serve_issue_2"]:
                self._json(200, {"issue": ISSUE_2})
            else:
                self._json(404, {"errors": ["not found"]})
        else:
            self._json(404, {"errors": ["no route"]})

    def _json(self, status, obj):
        raw = json.dumps(obj).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


port = free_port()
server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
print(f"[mock] Redmine mock on 127.0.0.1:{port}")

# ---------------------------------------------------------------- MCP harness

data_dir = tempfile.mkdtemp(prefix="vestige-sync-e2e-")
env = dict(
    os.environ,
    VESTIGE_DATA_DIR=data_dir,
    VESTIGE_DASHBOARD_ENABLED="false",
    VESTIGE_HTTP_ENABLED="0",
    VESTIGE_AUTOPILOT_ENABLED="0",
    REDMINE_URL=f"http://127.0.0.1:{port}",
    VESTIGE_ALLOW_PRIVATE_CONNECTOR_HOSTS="1",
)
env.pop("RUST_LOG", None)

proc = subprocess.Popen(
    [BIN],
    env=env,
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.DEVNULL,
    text=True,
    bufsize=1,
)

next_id = 0


def rpc(method, params=None, notification=False):
    global next_id
    msg = {"jsonrpc": "2.0", "method": method}
    if params is not None:
        msg["params"] = params
    if not notification:
        next_id += 1
        msg["id"] = next_id
    proc.stdin.write(json.dumps(msg) + "\n")
    proc.stdin.flush()
    if notification:
        return None
    while True:
        line = proc.stdout.readline()
        if not line:
            raise RuntimeError("server closed stdout")
        resp = json.loads(line)
        if resp.get("id") == msg["id"]:
            return resp


rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "0"}})
rpc("notifications/initialized", notification=True)
print("[mcp] initialized")

failures = []


def check(label, ok, detail=""):
    print(f"  {'PASS' if ok else 'FAIL'}  {label}" + (f"  -- {detail}" if detail and not ok else ""))
    if not ok:
        failures.append(label)


# 1. First sync: two issues created.
r = rpc("tools/call", {"name": "source_sync", "arguments": {"source": "redmine", "project": "infra", "max_pages": 1}})
out = json.loads(r["result"]["content"][0]["text"])
print(f"[sync1] {out['summary']}")
check("sync 1 created 2", out["created"] == 2, json.dumps(out))
check("sync 1 ok", out["ok"] is True)
check("sync 1 cursor set", bool(out["cursor"]))

# 2. recall valid finds the issues with citation fields.
def recall_valid(query):
    r = rpc("tools/call", {"name": "recall", "arguments": {
        "query": query,
        "limit": 10,
        "sourceSystem": "redmine",
        "sourceProject": "infra",
        "sourceStatus": "valid",
    }})
    return json.dumps(r)


valid_text = recall_valid("disk full db-primary")
check("recall valid finds issue 1", "Disk full" in valid_text, valid_text[:400])
valid_text2 = recall_valid("backup cron silent failure")
check("recall valid finds issue 2", "Backup cron" in valid_text2, valid_text2[:400])

# 3. Simulate upstream deletion of issue 2, then reconcile.
state["serve_issue_2"] = False
r = rpc("tools/call", {"name": "source_sync", "arguments": {
    "source": "redmine", "project": "infra", "max_pages": 1, "reconcile": True,
}})
out2 = json.loads(r["result"]["content"][0]["text"])
print(f"[sync2] {out2['summary']}")
check("reconcile ran", out2["reconciled"] is True, json.dumps(out2))
check("exactly issue 2 tombstoned", out2["tombstoned"] == 1, json.dumps(out2))

# 4. source_status filters now disagree — issue 2 only under tombstoned.
r = rpc("tools/call", {"name": "recall", "arguments": {
    "query": "backup cron silent failure",
    "limit": 10,
    "sourceSystem": "redmine",
    "sourceStatus": "valid",
}})
valid_after = json.dumps(r)
check("valid recall no longer returns issue 2", "Backup cron" not in valid_after, valid_after[:400])

r = rpc("tools/call", {"name": "recall", "arguments": {
    "query": "backup cron silent failure",
    "limit": 10,
    "sourceSystem": "redmine",
    "sourceStatus": "tombstoned",
}})
tomb_text = json.dumps(r)
check("tombstoned recall returns issue 2", "Backup cron" in tomb_text, tomb_text[:400])

# 5. Error path: connection refused must fail loudly naming the call.
dead_port = free_port()
r = rpc("tools/call", {"name": "source_sync", "arguments": {
    "source": "redmine", "project": "infra",
}})
# reuse of live mock; instead point env is fixed, so use an unknown project? No:
# for the loud-error check we spawn a second binary against a dead port.

proc.kill()

env_bad = dict(env, REDMINE_URL=f"http://127.0.0.1:{dead_port}")
proc2 = subprocess.Popen(
    [BIN], env=env_bad, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
    stderr=subprocess.DEVNULL, text=True, bufsize=1,
)


def rpc2(method, params=None, notification=False):
    global proc2
    msg = {"jsonrpc": "2.0", "method": method}
    if params is not None:
        msg["params"] = params
    if not notification:
        msg["id"] = 999
    proc2.stdin.write(json.dumps(msg) + "\n")
    proc2.stdin.flush()
    if notification:
        return None
    while True:
        line = proc2.stdout.readline()
        if not line:
            raise RuntimeError("server2 closed stdout")
        resp = json.loads(line)
        if resp.get("id") == 999:
            return resp


rpc2("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "0"}})
rpc2("notifications/initialized", notification=True)
r = rpc2("tools/call", {"name": "source_sync", "arguments": {"source": "redmine", "project": "infra"}})
err_text = json.dumps(r)
check(
    "dead upstream fails loudly naming the call",
    "GET http://127.0.0.1" in err_text and ("error" in err_text or "isError" in err_text or "sync failed" in err_text),
    err_text[:500],
)
proc2.kill()
server.shutdown()

print()
if failures:
    print(f"E2E RESULT: {len(failures)} FAILURE(S): {failures}")
    sys.exit(1)
print("E2E RESULT: all checks passed")
