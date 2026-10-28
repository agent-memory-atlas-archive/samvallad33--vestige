#!/usr/bin/env python3
"""Operator Lite 6.0.0 Brake (OP-024 runaway-brake): a session the gate stopped BRAKE_AT times inside
BRAKE_WINDOW_S is a runaway. Its next calls that change anything are held until it has gone
BRAKE_QUIET_S without a stop or the owner releases it; reads still pass, and other sessions are not
touched. Sessions without an id are never braked.

A throwaway home under this folder (never a temp folder); the gate runs as a hook in enforce mode.
Nothing is ever run.
Usage: python3 operator-lite/test_brake.py"""
import json, os, shutil, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
GATE = os.path.join(HERE, "operator-gate.py")
BOX = tempfile.mkdtemp(prefix="op-brake-", dir=HERE)
home, op = os.path.join(BOX, "home"), os.path.join(BOX, "op")
AGENT = ("OPERATOR_AGENT_SESSION", "CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT")
env = {k: v for k, v in os.environ.items() if k not in ("OPERATOR_HOME", "OPERATOR_GATE_MODE") + AGENT}
env.update(HOME=home, OPERATOR_HOME=op, OPERATOR_BRAKE_AT="8")
wrong, ran = [], []


def check(label, ok, detail=""):
    ran.append(label)
    print("%s %s%s" % ("ok  " if ok else "BAD ", label, ("  -- " + str(detail)[:240]) if detail and not ok else ""))
    if not ok:
        wrong.append(label)


def hook(cmd, session, tool="Bash", ti=None):
    payload = {"hook_event_name": "PreToolUse", "tool_name": tool, "tool_input": ti if ti is not None else {"command": cmd},
               "cwd": home}
    if session is not None:
        payload["session_id"] = session
    p = subprocess.run([sys.executable, GATE, "hook", "--source", "claude"], input=json.dumps(payload),
                       capture_output=True, text=True, env=env, cwd=home, timeout=60)
    return p.returncode, p.stderr


def gate(*a, extra=None):
    return subprocess.run([sys.executable, GATE] + list(a), capture_output=True, text=True,
                          env=dict(env, **(extra or {})), timeout=60)


for d in ("notes", "photos", "work"):
    os.makedirs(os.path.join(home, d))
    with open(os.path.join(home, d, "f.txt"), "w") as f:
        f.write(d + "\n")
with open(os.path.join(home, "readme.txt"), "w") as f:   # outside every folder the runaway is stopped on
    f.write("readme\n")
os.makedirs(op)
with open(os.path.join(op, "mode"), "w") as f:
    f.write("enforce\n")

TRIES = ["rm -rf ~/notes", "rm -rf ~/photos", "rm -rf ~/work", "rm -rf ~/notes/", "rm -rf ~/photos/",
         "rm -rf ~/work/", "rm -rf $HOME/notes", "rm -rf $HOME/photos"]
for i, cmd in enumerate(TRIES[:-1]):
    rc, err = hook(cmd, "runaway")
check("seven stops do not brake a session", "OP-024" not in err and hook("echo ok > ~/work/new.txt", "runaway")[0] == 0, err[:200])
rc, err = hook(TRIES[-1], "runaway")
check("the eighth stop inside the window trips the brake", rc == 2 and "brake" in err.lower(), err[:300])

rc, err = hook("echo ok > ~/work/new.txt", "runaway")
check("a braked session's next change is held (OP-024)", rc == 2 and "OP-024" in err and "release" in err, err[:300])
rc, err = hook("cat ~/work/f.txt", "runaway")
check("a braked session may still read", rc == 0, err[:200])
# reads a braked Codex session was held on (receipts of 2026-10-22): they pass now
for cmd in ["awk 'NR>=1 && NR<=40 {printf \"%6d\\t%s\\n\", NR, $0}' ~/readme.txt",
            "pwd\ngit status --short --branch\ngit rev-parse HEAD\ngit worktree list\nls -la ~/work\ncat ~/work/f.txt",
            "git diff-tree --no-commit-id --name-status -r HEAD", "git ls-remote origin refs/heads/main",
            "find ~/work -maxdepth 1 -type f -print -exec wc -l {} \\; && ls -la ~/notes 2>/dev/null || true"]:
    rc, err = hook(cmd, "runaway")
    check("a braked session may read: %s" % cmd.split()[0], rc == 0, (cmd, err[:200]))
for tool, ti in [("mcp__vestige__recall", {"handle": "brake"}), ("mcp__vestige__causal_walk", {"start_points": []}),
                 ("mcp__vestige__forgotten_lesson", {"failure_id": "mem-1"}),
                 ("mcp__vestige__codebase", {"action": "get_context", "codebase": "x"})]:
    rc, err = hook("", "runaway", tool=tool, ti=ti)
    check("a braked session may read its memory: %s" % tool, rc == 0, err[:200])
for cmd in ["awk '{print > \"out.txt\"}' ~/readme.txt", "git worktree add ../elsewhere", "git stash",
            "python3 -c 'print(1)'", "ls ~/work > ~/work/listing.txt"]:
    rc, err = hook(cmd, "runaway")
    check("a braked session's change is still held: %s" % cmd, rc == 2 and "OP-024" in err, (cmd, err[:200]))
for tool, ti in [("mcp__vestige__smart_ingest", {"content": "x"}), ("mcp__vestige__codebase", {"action": "remember_decision"}),
                 ("mcp__vestige__causal_walk", {"promote": True}), ("mcp__github__create_issue", {"title": "x"})]:
    rc, err = hook("", "runaway", tool=tool, ti=ti)
    check("a braked session's MCP change is still held: %s" % tool, rc == 2 and "OP-024" in err, err[:200])
rc, err = hook("echo ok > ~/work/new.txt", "other")
check("another session is not touched", rc == 0, err[:200])

for cmd in TRIES:
    hook(cmd, None)
rc, err = hook("echo ok > ~/work/new.txt", None)
check("calls without a session id are never braked", rc == 0, err[:200])

r = gate("brake")
check("brake lists the braked session", r.returncode == 0 and "runaway" in r.stdout, r.stdout)
r = gate("brake", "--release", "runaway", extra={"CLAUDECODE": "1"})
check("release refuses inside an agent session", r.returncode == 3 and "Refusing" in r.stdout, r.stdout)
rc, err = hook("python3 %s brake --release runaway" % GATE, "runaway")
check("an agent calling brake --release is stopped (OP-000)", rc == 2 and "OP-000" in err, err[:200])
r = gate("brake", "--release", "runaway")
rc, err = hook("echo ok > ~/work/new.txt", "runaway")
check("the owner releases it and the session works again", r.returncode == 0 and rc == 0, (r.stdout, rc, err[:120]))

# the brake also lets go on its own after a quiet stretch
for cmd in TRIES:
    hook(cmd, "runaway2")
path = os.path.join(op, "state", "brake.json")
state = json.load(open(path))
state["runaway2"]["last_stop"] -= 16 * 60
with open(path, "w") as f:
    json.dump(state, f)
rc, err = hook("echo ok > ~/work/new.txt", "runaway2")
check("after 15 quiet minutes the brake lets go", rc == 0, err[:200])

shutil.rmtree(BOX, ignore_errors=True)
print("\nbrake: %d checks, wrong %d" % (len(ran), len(wrong)))
sys.exit(1 if wrong else 0)
