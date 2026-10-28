"""The two moments Operator Lite points its owner at the paid gate, and the places it never does.

1. `status` at a terminal ends with the week's actions no built-in rule decided ("Last 7 days, no built-in
   rule decided: 2 pushes, 1 package install") and one law the owner could write for them.
2. `approve` at a terminal, on every third permit for the same rule within 30 days, says the owner could
   write that rule once in Operator instead of approving it each time.
Neither line reaches an agent: piped `status`, a stop message and the receipts carry no offer.
Commands are only judged, never executed. Run: /usr/bin/python3 test_upgrade_moments.py [path/to/operator-gate.py]
"""
import json, os, pty, select, subprocess, sys, tempfile, time

GATE = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "operator-gate.py"))
# The folder name must not contain "upgrade": receipts record the working directory.
ROOT = tempfile.mkdtemp(prefix="lite-offer-moments-", dir=os.path.expanduser("~"))
fails = []


def check(name, cond, detail=""):
    print("%s  %s%s" % ("ok " if cond else "BAD", name, ("   -> " + str(detail)[-400:]) if not cond else ""))
    if not cond:
        fails.append(name)


def new_home(name, mode):
    home = os.path.join(ROOT, name)
    os.makedirs(os.path.join(home, ".operator"))
    os.makedirs(os.path.join(home, "work", "app"))
    with open(os.path.join(home, ".operator", "mode"), "w") as f:
        f.write(mode + "\n")
    env = {k: v for k, v in os.environ.items() if k not in ("CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "OPERATOR_AGENT_SESSION")}
    env.update(HOME=home, OPERATOR_HOME=os.path.join(home, ".operator"), PATH="/usr/bin:/bin:/opt/homebrew/bin", NO_COLOR="1")
    return home, env


def hook(env, home, cmd):
    payload = {"tool_name": "Bash", "tool_input": {"command": cmd}, "cwd": os.path.join(home, "work", "app"),
               "session_id": "moments", "hook_event_name": "PreToolUse"}
    return subprocess.run([sys.executable, "-I", GATE, "hook", "--source", "claude"], input=json.dumps(payload),
                          capture_output=True, text=True, env=env, timeout=60)


def at_tty(env, args, typed=""):
    """Run the gate with a pseudo-terminal as stdin and stdout, as an owner would; return its output."""
    master, slave = pty.openpty()
    p = subprocess.Popen([sys.executable, "-I", GATE] + args, stdin=slave, stdout=slave, stderr=slave, env=env)
    os.close(slave)
    if typed:
        time.sleep(0.4)
        os.write(master, typed.encode())
    out, deadline = b"", time.time() + 30
    while time.time() < deadline:
        r, _, _ = select.select([master], [], [], 0.2)
        if r:
            try:
                chunk = os.read(master, 4096)
            except OSError:
                break
            if not chunk:
                break
            out += chunk
        elif p.poll() is not None:
            break
    p.wait(timeout=10)
    os.close(master)
    return out.decode("utf-8", "replace").replace("\r", "")


def receipts(home):
    rdir = os.path.join(home, ".operator", "receipts")
    rows = []
    for fn in sorted(os.listdir(rdir)):
        if fn.endswith(".jsonl"):
            rows += [json.loads(l) for l in open(os.path.join(rdir, fn))]
    return rows


# 1. the weekly line
home, env = new_home("weekly", "shadow")
for cmd in ("git push origin main", "git push origin feature-x", "npm install left-pad", "ls -la"):
    hook(env, home, cmd)
rows = receipts(home)
check("receipts name what no built-in rule decides", any(r.get("own") == ["push"] for r in rows)
      and any(r.get("own") == ["packages"] for r in rows), [r.get("own") for r in rows])
check("a plain call carries no such field", not any(r.get("own") for r in rows if r.get("action_preview") == "ls -la"))
out = at_tty(env, ["status"])
check("status at a terminal: the week's undecided actions", "Last 7 days, no built-in rule decided: 2 pushes, 1 package install" in out, out)
check("status at a terminal: a law the owner could write, and where", "No push without my permit." in out and "upgrade" in out, out)
piped = subprocess.run([sys.executable, "-I", GATE, "status"], capture_output=True, text=True, env=env, timeout=60).stdout
check("piped status (an agent or a script) carries no offer", "no built-in rule decided" not in piped and "upgrade" not in piped, piped)
v = subprocess.run([sys.executable, "-I", GATE, "verify"], capture_output=True, text=True, env=env, timeout=60).stdout
check("the receipt chain still verifies", "chain=OK" in v, v)

# 2. the third approval
home, env = new_home("approvals", "enforce")
stops = []
for cmd in ("git push --force origin main", "git push --force origin master", "git push --force upstream main"):
    p = hook(env, home, cmd)
    stops.append(p)
check("three force pushes are stopped", all(p.returncode == 2 for p in stops), [p.returncode for p in stops])
check("a stop message carries no offer", not any("upgrade" in p.stderr or "$149" in p.stderr or "payhip" in p.stderr
                                                 for p in stops), stops[0].stderr)
digests = [r["action_digest"] for r in receipts(home) if r.get("decision") == "STOP"]
outs = [at_tty(env, ["approve", d], typed="ALLOW ONCE\n") for d in digests]
check("each approval is granted", all("Permit granted for one use." in o for o in outs), outs)
check("the first and second approval make no offer", not any("Operator" in o for o in outs[:2]), outs[:2])
check("the third approval of the same rule makes the offer", "third time" in outs[2] and "OP-004" in outs[2]
      and "upgrade" in outs[2], outs[2])
grants = [r for r in receipts(home) if r.get("decision") == "PERMIT_GRANTED"]
check("permit receipts name the rule they allowed", [r.get("commitment") for r in grants] == ["OP-004"] * 3, grants)
check("no receipt carries the offer", not any("upgrade" in json.dumps(r) for r in receipts(home)))

print("\nfailures: %d" % len(fails))
sys.exit(1 if fails else 0)
