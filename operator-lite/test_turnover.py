#!/usr/bin/env python3
"""Operator Lite 6.0.0 Watch turnover (OP-025 watch-turnover). At a compaction the PreCompact hook seals what the
session hands over, read from the gate's own receipts: the actions it was stopped from, what the owner allowed,
permits still open, copies Rewind kept, its brake, the laws and protected files. SessionStart (compact) hands it
to the session that continues, and until that session takes the watch by the turnover's id its changing calls
are held; reads and MCP calls go ahead. A hold says nothing about its target: it leaves no shared-ledger mark
and drafts no law (OP-024 brake holds too).

A throwaway home under this folder (never a temp folder); the gate runs as a hook in enforce mode. Nothing is
ever run but the gate itself.
Usage: python3 operator-lite/test_turnover.py"""
import json, os, shutil, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
GATE = os.path.join(HERE, "operator-gate.py")
REAL_SETTINGS = os.path.join(os.path.expanduser("~"), ".claude", "settings.json")
BOX = tempfile.mkdtemp(prefix="op-turnover-", dir=HERE)
home, op = os.path.join(BOX, "home"), os.path.join(BOX, "op")
AGENT = ("OPERATOR_AGENT_SESSION", "CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT")
env = {k: v for k, v in os.environ.items() if k not in ("OPERATOR_HOME", "OPERATOR_GATE_MODE", "OPERATOR_TURNOVER") + AGENT}
env.update(HOME=home, OPERATOR_HOME=op, OPERATOR_BRAKE_AT="0", GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_SYSTEM=os.devnull)
wrong, ran = [], []


def check(label, ok, detail=""):
    ran.append(label)
    print("%s %s%s" % ("ok  " if ok else "BAD ", label, ("  -- " + str(detail)[:300]) if detail and not ok else ""))
    if not ok:
        wrong.append(label)


def hook(cmd, session, tool="Bash", ti=None, extra=None):
    payload = {"hook_event_name": "PreToolUse", "tool_name": tool, "tool_input": ti if ti is not None else {"command": cmd},
               "cwd": home, "session_id": session, "tool_use_id": "toolu_%s_%d" % (session, len(ran))}
    p = subprocess.run([sys.executable, GATE, "hook", "--source", "claude"], input=json.dumps(payload),
                       capture_output=True, text=True, env=dict(env, **(extra or {})), cwd=home, timeout=60)
    return p.returncode, p.stderr


def gate(argv, stdin="", extra=None):
    return subprocess.run([sys.executable, GATE] + argv, input=stdin, capture_output=True, text=True,
                          env=dict(env, **(extra or {})), cwd=home, timeout=60)


def compact(session, trigger="auto", extra=None):
    return gate(["turnover", "--pre-compact"], json.dumps({"hook_event_name": "PreCompact", "session_id": session,
                                                           "trigger": trigger, "cwd": home, "custom_instructions": ""}), extra)


def session_start(session, source="compact"):
    r = gate(["turnover", "--session-start"], json.dumps({"hook_event_name": "SessionStart", "session_id": session, "source": source}))
    try:
        return json.loads(r.stdout)["hookSpecificOutput"]["additionalContext"]
    except (ValueError, KeyError, TypeError):
        return ""


def receipts():
    rdir, rows = os.path.join(op, "receipts"), []
    for n in sorted(x for x in os.listdir(rdir) if x.endswith(".jsonl")):
        rows += [json.loads(line) for line in open(os.path.join(rdir, n), encoding="utf-8") if line.strip()]
    return rows


def turnover_state(session):
    try:
        return json.load(open(os.path.join(op, "state", "turnover", session + ".json"), encoding="utf-8"))
    except OSError:
        return None


def ledger_rules():
    path = os.path.join(op, "state", "chain", "_machine.jsonl")
    return [json.loads(line).get("rule") for line in open(path, encoding="utf-8")] if os.path.isfile(path) else []


def drafts():
    path = os.path.join(op, "state", "law-drafts.json")
    return json.load(open(path, encoding="utf-8")) if os.path.isfile(path) else {}


for d in ("notes", "photos", "work", "music"):
    os.makedirs(os.path.join(home, d))
    with open(os.path.join(home, d, "f.txt"), "w") as f:
        f.write(d + "\n")
two_hours_ago = time.time() - 7200            # older than FRESH_S, so Rewind counts it as work, not scratch
for p in (os.path.join(home, "music", "f.txt"), os.path.join(home, "music")):
    os.utime(p, (two_hours_ago, two_hours_ago))
os.makedirs(op)
with open(os.path.join(op, "mode"), "w") as f:
    f.write("enforce\n")
NEW =os.path.join(home, "work", "new.txt")
OLD = os.path.join(home, "work", "f.txt")
WRITE = "echo ok > %s" % NEW
OVERWRITE = "echo ok > %s" % OLD
settings_before = os.stat(REAL_SETTINGS).st_mtime if os.path.exists(REAL_SETTINGS) else None

# -- before any compaction nothing is held
rc, err = hook(WRITE, "s1")
check("before a compaction a session changes files freely", rc == 0, err)
rc, err = hook("rm -rf ~/notes", "s1")
check("the session is stopped deleting ~/notes", rc == 2 and "OP-003" in err, err)

# -- PreCompact seals the turnover
r = compact("s1")
st = turnover_state("s1")
check("PreCompact exits 0 and prints nothing", r.returncode == 0 and not r.stdout.strip() and not r.stderr.strip(), (r.returncode, r.stdout, r.stderr))
check("the turnover names the stopped action from the receipts", bool(st) and st["holds"] and
      [s["rule"] for s in st["chit"]["stops"]] == ["OP-003"] and "rm -rf" in st["chit"]["stops"][0]["action"], st)
tid = (st or {}).get("id", "")
sealed = [x for x in receipts() if x.get("decision") == "TURNOVER_SEALED" and x.get("session") == "s1"]
check("the seal is a receipt in the gate's chain", len(sealed) == 1 and sealed[0].get("chit") == tid and sealed[0].get("digest"), sealed)

# -- SessionStart (compact) hands it over; other starts and other sessions get nothing
ctx = session_start("s1")
check("SessionStart after the compaction hands over the turnover", ("Watch turnover %s" % tid) in ctx and "OP-003" in ctx
      and "rm -rf" in ctx and ("turnover --ack %s" % tid) in ctx, ctx)
check("a fresh start of the same session gets no turnover", "Watch turnover" not in session_start("s1", "startup"))
check("another session's compaction start gets nothing of this one", "Watch turnover" not in session_start("s9"))

# -- the hold: changing calls wait, reads and MCP calls go ahead, other sessions are untouched
rc, err = hook(WRITE, "s1")
check("a change before taking the watch is held (OP-025)", rc == 2 and "OP-025" in err and ("turnover --ack %s" % tid) in err, err)
check("the hold says to make the call again, not to give it up", "Do not retry" not in err and "make this call again" in err, err)
rc, err = hook("cat %s" % OLD, "s1")
check("a shell read goes ahead", rc == 0, err)
# reads a compacted Codex session was held on (receipts of 2026-10-22 to 24): they go ahead now
for cmd in ["awk 'NR>=1 && NR<=30 {printf \"%%6d  %%s\\n\", NR, $0}' %s" % OLD,
            "pwd\ngit status --short --branch\ngit rev-parse HEAD\ngit remote -v\ngit worktree list\nls -la %s\ncat %s" % (home, OLD),
            "git diff-tree --no-commit-id --name-status -r HEAD -- a.rs", "git ls-remote origin refs/heads/main",
            "ls -la %s 2>/dev/null || true" % home]:
    rc, err = hook(cmd, "s1")
    check("a shell read goes ahead: %s" % cmd.split()[0], rc == 0, (cmd, err))
for cmd in ["awk '{print > \"%s\"}' %s" % (NEW, OLD), "git worktree add ../elsewhere"]:
    rc, err = hook(cmd, "s1")
    check("a change dressed as a read is still held: %s" % cmd.split()[0], rc == 2 and "OP-025" in err, (cmd, err))
rc, err = hook("", "s1", tool="Read", ti={"file_path": OLD})
check("the Read tool goes ahead", rc == 0, err)
rc, err = hook("", "s1", tool="mcp__vestige__recall", ti={"handle": "turnover"})
check("an MCP call goes ahead, so the session can read its memory", rc == 0, err)
rc, err = hook(WRITE, "s2")
check("another session is not held", rc == 0, err)
rc, err = hook("python3 %s turnover --ack %s && %s" % (GATE, tid, WRITE), "s1")
check("taking the watch chained to a change is still held", rc == 2 and "OP-025" in err, err)

# -- a hold is not a verdict on its target
for _ in range(3):
    hook(OVERWRITE, "s1")
check("three held overwrites of one file draft no law", not any("op-025" in k for k in drafts()), drafts())
check("held calls leave no mark on the shared ledger", "OP-025" not in ledger_rules(), ledger_rules())

# -- taking the watch
r = gate(["turnover", "--ack", "0" * 16])
check("a wrong turnover id is refused", r.returncode == 2 and "No turnover" in r.stdout, r.stdout)
rc, err = hook("python3 %s turnover --ack %s" % (GATE, tid), "s1")
check("the hook lets the exact take-the-watch command through", rc == 0, err)
rc, err = hook("operator-gate turnover --show %s" % tid, "s1")
check("and the launcher form of it", rc == 0, err)
r = gate(["turnover", "--ack", tid], extra={"CLAUDECODE": "1"})
check("the agent takes the watch: the record is printed", r.returncode == 0 and "Watch taken" in r.stdout and "OP-003" in r.stdout, r.stdout)
taken = [x for x in receipts() if x.get("decision") == "TURNOVER_TAKEN" and x.get("chit") == tid]
check("taking the watch is a receipt naming the agent", len(taken) == 1 and taken[0].get("source") == "agent" and taken[0].get("session") == "s1", taken)
rc, err = hook(WRITE, "s1")
check("after the watch is taken the session changes files again", rc == 0, err)
rc, err = hook("rm -rf ~/photos", "s1")
check("and every rule still judges it", rc == 2 and "OP-003" in err and "OP-025" not in err, err)
check("a taken turnover is not handed over again", "Watch turnover" not in session_start("s1"))

# -- the next compaction seals a new turnover with everything the session was stopped from
compact("s1", "manual")
st2 = turnover_state("s1")
check("the next compaction seals a new turnover that holds again", bool(st2) and st2["id"] != tid and st2["holds"] and not st2.get("acked")
      and {s["rule"] for s in st2["chit"]["stops"]} == {"OP-003"} and len(st2["chit"]["stops"]) == 2, st2)
r = gate(["turnover"])
check("`turnover` lists it as waiting", r.returncode == 0 and st2["id"] in r.stdout and "waiting" in r.stdout, r.stdout)

# -- nothing to hand over holds nothing
hook(WRITE, "s3")
compact("s3")
st3 = turnover_state("s3")
rc, err = hook(WRITE, "s3")
check("a session with nothing to hand over is not held", bool(st3) and not st3["holds"] and rc == 0, (st3, err))

# -- what the owner allowed, and the copy Rewind kept, are handed over
rc, err = hook("rm -rf ~/music", "s5")
rc, err = hook("rm -rf ~/music", "s5")         # the same call again keeps its action id
stop_row = [x for x in receipts() if x.get("decision") == "STOP" and x.get("session") == "s5"][-1]
os.makedirs(os.path.join(op, "permits"), exist_ok=True)
with open(os.path.join(op, "permits", stop_row["action_digest"] + ".json"), "w") as f:      # what `approve` writes
    json.dump({"action_digest": stop_row["action_digest"], "granted": time.time(), "expires": time.time() + 900,
               "single_use": True, "by": "owner-tty", "stop_receipt": stop_row["digest"], "session": "s5"}, f)
rc, err = hook("rm -rf ~/music", "s5")
compact("s5")
c5 = (turnover_state("s5") or {}).get("chit") or {}
check("the owner's allow-once is in the turnover", rc == 0 and len(c5.get("allowed") or []) == 1
      and len(c5.get("stops") or []) == 1
      and c5["stops"][0].get("times") == 2, (rc, err, c5))
txt = gate(["turnover", "--show", turnover_state("s5")["id"]]).stdout
check("and the session that continues reads it", "Allowed once by the owner" in txt, txt)
with open(os.path.join(op, "permits", "a" * 24 + ".json"), "w") as f:
    json.dump({"action_digest": "a" * 24, "expires": time.time() + 900, "session": "s4", "stop_receipt": "b" * 64}, f)
compact("s4")
c4 = turnover_state("s4") or {}
check("a permit still open is handed over and holds", c4.get("holds") and [p["action_id"] for p in c4["chit"]["permits"]] == ["a" * 24], c4)

# -- shadow records, the switch turns it off, a week-old turnover lets go
hook("rm -rf ~/notes", "s6")
compact("s6")
with open(os.path.join(op, "mode"), "w") as f:
    f.write("shadow\n")
rc, err = hook(WRITE, "s6")
last = receipts()[-1]
check("in shadow mode the hold is recorded and the call goes ahead", rc == 0 and last.get("decision") == "SHADOW_STOP"
      and "OP-025" in (last.get("commitments") or []), (rc, last))
with open(os.path.join(op, "mode"), "w") as f:
    f.write("enforce\n")
hook("rm -rf ~/notes", "s7")
compact("s7", extra={"OPERATOR_TURNOVER": "0"})
rc, err = hook(WRITE, "s6", extra={"OPERATOR_TURNOVER": "0"})
check("OPERATOR_TURNOVER=0 seals nothing and holds nothing", turnover_state("s7") is None and rc == 0, err)
path = os.path.join(op, "state", "turnover", "s1.json")
old = json.load(open(path, encoding="utf-8"))
old["sealed_t"] -= 8 * 24 * 3600
with open(path, "w", encoding="utf-8") as f:
    json.dump(old, f)
rc, err = hook(WRITE, "s1")
check("a turnover never taken lets go after a week", rc == 0, err)

# -- the session that continues is handed the watch in its host's shape
hook("rm -rf ~/notes", "s10")
compact("s10")
ctx = session_start("s10")
check("SessionStart hands the turnover over with the command that takes the watch",
      "Watch turnover" in ctx and "turnover --ack" in ctx, ctx[:300])
r = gate(["turnover", "--session-start", "--host", "cursor"], json.dumps({"session_id": "s10", "source": "compact"}))
check("Cursor reads it as additional_context", "additional_context" in r.stdout and "Watch turnover" in r.stdout, r.stdout[:200])

# -- OP-024 brake holds also leave no ledger mark and draft no law
braked = dict(OPERATOR_BRAKE_AT="3")
for cmd in ("rm -rf ~/notes", "rm -rf ~/photos", "rm -rf ~/music"):
    hook(cmd, "b1", extra=braked)
before = ledger_rules().count("OP-024")
outs = [hook(OVERWRITE, "b1", extra=braked) for _ in range(3)]
check("a braked session's held overwrites are OP-024", all(rc == 2 and "OP-024" in e for rc, e in outs), outs[-1])
check("brake holds draft no law and leave no ledger mark", not any("op-024" in k for k in drafts())
      and ledger_rules().count("OP-024") == before, (drafts(), ledger_rules()))

# -- a broken payload never blocks a compaction
r = gate(["turnover", "--pre-compact"], "{not json")
check("PreCompact with a broken payload exits 0 and prints nothing", r.returncode == 0 and not r.stdout.strip(), (r.returncode, r.stdout))

# -- install wires PreCompact, uninstall takes it out (throwaway home only)
op2 = os.path.join(BOX, "op-install")
os.makedirs(op2)
r = gate(["install"], extra={"OPERATOR_HOME": op2})
sp = os.path.join(home, ".claude", "settings.json")
cfg = json.load(open(sp, encoding="utf-8")) if os.path.isfile(sp) else {}
pre = [hh.get("command", "") for h in cfg.get("hooks", {}).get("PreCompact", []) for hh in h.get("hooks", [])]
check("install registers the PreCompact turnover hook", r.returncode == 0 and any("turnover --pre-compact" in c for c in pre)
      and "PreCompact hook" in r.stdout, (r.stdout[-400:], pre))
gate(["uninstall"], extra={"OPERATOR_HOME": op2})
cfg = json.load(open(sp, encoding="utf-8"))
pre = [hh.get("command", "") for h in cfg.get("hooks", {}).get("PreCompact", []) for hh in h.get("hooks", [])]
check("uninstall removes it", not any("operator-gate" in c for c in pre), pre)
check("the real ~/.claude/settings.json was not touched",
      settings_before == (os.stat(REAL_SETTINGS).st_mtime if os.path.exists(REAL_SETTINGS) else None))

shutil.rmtree(BOX, ignore_errors=True)
print("\nturnover: %d checks, wrong %d" % (len(ran), len(wrong)))
sys.exit(1 if wrong else 0)
