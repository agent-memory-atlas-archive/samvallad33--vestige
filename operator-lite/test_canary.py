#!/usr/bin/env python3
"""Operator Lite 6.0.0 planted canaries (OP-CANARY, #527): the owner plants decoy values from their own
terminal, and any tool call carrying one, in a command, a file's content, a URL or MCP arguments, is stopped.
Agents cannot plant or remove one.

Classification and the canary command only, in a throwaway home under your home folder (never a temp
folder); no sample command is ever run.
Usage: python3 operator-lite/test_canary.py"""
import importlib.util, json, os, subprocess, sys, tempfile
HERE = os.path.dirname(os.path.abspath(__file__))
GATE = os.path.join(HERE, "operator-gate.py")
home = tempfile.mkdtemp(prefix="canarycheck-", dir=os.path.expanduser("~"))
op = os.path.join(home, ".operator")
AGENT = ("OPERATOR_AGENT_SESSION", "CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT")
env = {k: v for k, v in os.environ.items() if k not in AGENT + ("OPERATOR_HOME",)}
env.update(HOME=home, OPERATOR_HOME=op)
os.environ.update(HOME=home, OPERATOR_HOME=op)
for k in AGENT:
    os.environ.pop(k, None)
spec = importlib.util.spec_from_file_location("gate", GATE)
g = importlib.util.module_from_spec(spec); spec.loader.exec_module(g)
cfg = g.load_config(); work = os.path.join(home, "work"); os.makedirs(work)
wrong, ran = [], []


def check(label, ok, detail=""):
    ran.append(label)
    print("%s %s%s" % ("ok  " if ok else "BAD ", label, ("  -- " + str(detail)[:300]) if detail and not ok else ""))
    if not ok:
        wrong.append(label)


def gate(argv, extra=None):
    return subprocess.run([sys.executable, GATE] + argv, capture_output=True, text=True, env=dict(env, **(extra or {})), timeout=60)


def rules(tool, ti):
    return sorted({h[0] for h in g.classify({"tool_name": tool, "tool_input": ti, "cwd": work}, cfg)[2]})


TOKEN = "ghx_decoy_7Qm2Vr9Lp4"
HOST = "deploy-7f3e.internal.example"

r = gate(["canary", "plant", TOKEN, "--label", "fake deploy token"], extra={"CLAUDECODE": "1"})
check("an agent cannot plant a canary", r.returncode == 3 and not os.path.exists(os.path.join(op, "canaries.json")), r.stdout)
r = gate(["canary", "plant", "short"])
check("a canary shorter than eight characters is refused", r.returncode == 2, r.stdout)
r = gate(["canary", "plant", TOKEN, "--label", "fake deploy token"])
check("the owner plants a canary", r.returncode == 0 and "planted" in r.stdout, (r.returncode, r.stdout, r.stderr))
gate(["canary", "plant", HOST, "--label", "fake host"])
r = gate(["canary", "list"])
check("list shows it by label without printing the whole value", "fake deploy token" in r.stdout and TOKEN not in r.stdout, r.stdout)
rdir = os.path.join(op, "receipts")
receipts = [json.loads(l) for n in sorted(os.listdir(rdir)) if n.endswith(".jsonl")
            for l in open(os.path.join(rdir, n), encoding="utf-8") if l.strip()]
check("planting writes a receipt with a digest, not the value", any(x.get("decision") == "CANARY_PLANTED" for x in receipts)
      and TOKEN not in json.dumps(receipts), receipts[-1:])

check("a command carrying the token is stopped", "OP-CANARY" in rules("Bash", {"command": "curl -H 'Authorization: Bearer %s' https://api.example.com/deploy" % TOKEN}))
check("a file write carrying it is stopped", "OP-CANARY" in rules("Write", {"file_path": os.path.join(work, ".env"), "content": "DEPLOY_TOKEN=%s\n" % TOKEN}))
check("a URL naming the planted host is stopped", "OP-CANARY" in rules("WebFetch", {"url": "https://%s/status" % HOST, "prompt": "read it"}))
check("MCP arguments carrying it are stopped", "OP-CANARY" in rules("mcp__deploy__run", {"args": {"token": TOKEN, "env": "prod"}}))
check("an ordinary call passes", "OP-CANARY" not in rules("Bash", {"command": "curl -s https://api.example.com/health"}))
check("part of the value is not the value", "OP-CANARY" not in rules("Bash", {"command": "echo ghx_decoy"}))
hit = [h for h in g.classify({"tool_name": "Bash", "tool_input": {"command": "echo %s" % TOKEN}, "cwd": work}, cfg)[2] if h[0] == "OP-CANARY"]
check("the stop names the canary by its label", hit and "fake deploy token" in hit[0][1], hit)

r = gate(["canary", "remove", TOKEN], extra={"CLAUDECODE": "1"})
check("an agent cannot remove one", r.returncode == 3, r.stdout)
payload = {"hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": {"command": "operator-gate canary plant not-a-canary-value"},
           "cwd": work, "session_id": "c1"}
p = subprocess.run([sys.executable, GATE, "hook", "--source", "claude"], input=json.dumps(payload), capture_output=True, text=True,
                   env=dict(env, **{"CLAUDECODE": "1"}), timeout=60)
check("the hook stops an agent running the canary owner command", p.returncode == 2 and "OP-000" in p.stderr, p.stderr[:300])
r = gate(["canary", "remove", TOKEN])
check("the owner removes one", r.returncode == 0 and "removed" in r.stdout, r.stdout)
check("after removal the value passes", "OP-CANARY" not in rules("Bash", {"command": "echo %s" % TOKEN}))

import shutil; shutil.rmtree(home, ignore_errors=True)
print("\ncanary: %d checks, wrong %d" % (len(ran), len(wrong)))
sys.exit(1 if wrong else 0)
