#!/usr/bin/env python3
"""Operator Lite 6.0.0 OP-023 no-remote-delete: an HTTP DELETE, or a GraphQL mutation that deletes, destroys,
drops, purges or wipes, sent to a remote API removes something the owner may not get back (the PocketOS
case: a production volume and its backups deleted in one API call). It needs the owner's approval. Calls
to this machine and to development hosts, and reads, pass.

The gate is only asked; nothing is sent anywhere.
Usage: python3 operator-lite/test_remote_delete.py"""
import json, os, shutil, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
GATE = os.path.join(HERE, "operator-gate.py")
BOX = tempfile.mkdtemp(prefix="op-remote-delete-", dir=HERE)
home, op = os.path.join(BOX, "home"), os.path.join(BOX, "op")
AGENT = ("OPERATOR_AGENT_SESSION", "CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT")
env = {k: v for k, v in os.environ.items() if k not in ("OPERATOR_HOME", "OPERATOR_GATE_MODE") + AGENT}
env.update(HOME=home, OPERATOR_HOME=op)
os.makedirs(home)
os.makedirs(op)
with open(os.path.join(op, "mode"), "w") as f:
    f.write("enforce\n")
wrong, ran, n = [], [], [0]


def check(label, ok, detail=""):
    ran.append(label)
    print("%s %s%s" % ("ok  " if ok else "BAD ", label, ("  -- " + str(detail)[:220]) if detail and not ok else ""))
    if not ok:
        wrong.append(label)


def hook(cmd):
    n[0] += 1
    payload = {"hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": {"command": cmd}, "cwd": home,
               "session_id": "rd-%d" % n[0]}
    p = subprocess.run([sys.executable, GATE, "hook", "--source", "claude"], input=json.dumps(payload),
                       capture_output=True, text=True, env=env, cwd=home, timeout=60)
    return p.returncode, p.stderr


STOP = [
    'curl -X DELETE https://api.railway.app/v2/volumes/vol_123 -H "Authorization: Bearer $TOKEN"',
    "curl --request DELETE https://api.github.com/repos/acme/app",
    "curl -XDELETE https://api.example-cloud.com/v1/databases/prod",
    "curl -s -X 'DELETE' https://api.vercel.com/v9/projects/web",
    "http DELETE https://api.render.com/v1/services/srv-1",
    "xh delete https://api.fly.io/v1/apps/web",
    "wget --method=DELETE https://api.netlify.com/api/v1/sites/abc",
    "curl https://backboard.railway.app/graphql/v2 -H 'content-type: application/json' "
    "-d '{\"query\":\"mutation { serviceDelete(id: \\\"s1\\\") }\"}'",
    "curl https://api.linear.app/graphql -d '{\"query\":\"mutation { issueArchive(id: 1) projectDelete(id: 2) }\"}'",
    "curl https://api.example.com/graphql --data '{\"query\": \"mutation { purgeCache(all: true) }\"}'",
]
PASS = [
    "curl -X DELETE http://localhost:3000/api/items/1",
    "curl -X DELETE http://127.0.0.1:8080/v1/sessions/abc",
    "curl -X DELETE https://api.myapp.test/v1/items/1",
    "curl https://api.github.com/repos/acme/app",
    "curl -X POST https://api.example.com/v1/items -d '{\"name\":\"x\"}'",
    "curl https://api.example.com/graphql -d '{\"query\":\"query { deletedItems { id } }\"}'",
    "echo 'curl -X DELETE https://api.railway.app/v2/volumes/1' > notes.txt",
]
for cmd in STOP:
    rc, err = hook(cmd)
    check("stops: %s" % cmd[:72], rc == 2 and any(r in err for r in ("OP-023", "OP-006", "OP-005")), (rc, err[:200]))
for cmd in PASS:
    rc, err = hook(cmd)
    check("passes: %s" % cmd[:72], rc == 0, (rc, err[:200]))
rc, err = hook(STOP[0])
check("the stop says what would be removed and how to go ahead", "OP-023" in err and "api.railway.app" in err and "approve" in err, err[:300])

shutil.rmtree(BOX, ignore_errors=True)
print("\nremote delete: %d checks, wrong %d" % (len(ran), len(wrong)))
sys.exit(1 if wrong else 0)
