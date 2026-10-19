"""Operator Lite: an upgrade from 0.3.7 blocks.

Every Lite install before 0.3.9 wrote `shadow` into ~/.operator/mode, and shadow blocks nothing. The
first 0.3.9 install, port install or hook call settles that once: an installer's shadow becomes
enforce, and a shadow the owner chose stays. Each case below installs a real pre-0.3.9 gate (fetched
from this repository's history) into its own throwaway home, upgrades it one way, and sends
`rm -rf ~/Documents` through the hook. Nothing is executed; the gate classifies.

usage: python3 operator-lite/test_upgrade.py   (homes go under $OPERATOR_UPGRADE_TEST_DIR, else ~)"""
import importlib.util
import json
import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
GATE = os.path.join(HERE, "operator-gate.py")
OLD = {"0.3.7": "dc67bb712b479363ff363bb4c51fc3aba00eccfb",        # the 0.3.7 release: no mode command
       "pre-0.3.9": "b44477a538af8cdafe3274a5622e2bc557ecc26a"}    # the last gate before 0.3.9: has one
# a temp folder is scratch to the gate, so the homes live outside every one of them
BOX = os.path.join(os.environ.get("OPERATOR_UPGRADE_TEST_DIR") or os.path.expanduser("~"),
                   "operator-lite-upgrade-%d" % os.getpid())
fails = []


def check(name, ok, detail=""):
    print("%s %s%s" % ("ok  " if ok else "FAIL", name, "" if ok else "  (%s)" % detail))
    if not ok:
        fails.append(name)


def old_gate(label):
    """The pre-0.3.9 gate at this label, as a file under BOX (fetched once when the checkout is shallow)."""
    dst = os.path.join(BOX, "old", label, "operator-gate.py")
    if not os.path.exists(dst):
        spec = "%s:operator-lite/operator-gate.py" % OLD[label]
        r = subprocess.run(["git", "-C", REPO, "show", spec], capture_output=True)
        if r.returncode != 0:
            subprocess.run(["git", "-C", REPO, "fetch", "-q", "--depth", "1", "origin", OLD[label]], check=True)
            r = subprocess.run(["git", "-C", REPO, "show", spec], capture_output=True, check=True)
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        with open(dst, "wb") as f:
            f.write(r.stdout)
    return dst


def home(name):
    h = os.path.join(BOX, name)
    os.makedirs(os.path.join(h, "proj"), exist_ok=True)
    os.makedirs(os.path.join(h, "Documents"), exist_ok=True)
    env = dict(os.environ, HOME=h, USERPROFILE=h)
    for k in ("OPERATOR_HOME", "OPERATOR_AGENT_SESSION", "OPERATOR_GATE_MODE", "OPERATOR_GATE",
              "CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT"):
        env.pop(k, None)
    return h, env


def install(gate, env, h):
    return subprocess.run([sys.executable, gate, "install", "--no-replay"], capture_output=True, text=True,
                          env=env, cwd=h, timeout=120)


def hook(gate, env, h, cmd="rm -rf ~/Documents"):
    payload = json.dumps({"hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": {"command": cmd},
                          "cwd": os.path.join(h, "proj"), "session_id": "upgrade-test"})
    return subprocess.run([sys.executable, gate, "hook", "--source", "claude"], input=payload,
                          capture_output=True, text=True, env=env, timeout=60).returncode


def mode_of(h):
    with open(os.path.join(h, ".operator", "mode"), encoding="utf-8") as f:
        return f.read().strip()


def receipts(h):
    out = []
    rdir = os.path.join(h, ".operator", "receipts")
    for n in sorted(os.listdir(rdir)) if os.path.isdir(rdir) else []:
        if n.endswith(".jsonl"):
            with open(os.path.join(rdir, n), encoding="utf-8") as f:
                out += [json.loads(l) for l in f if l.strip()]
    return out


def installed(h):
    return os.path.join(h, ".operator", "gate", "operator-gate.py")


# 1. A 0.3.7 install, then the 0.3.9 install (npx operator-lite and the README line both run it).
h, env = home("037-then-install")
install(old_gate("0.3.7"), env, h)
check("0.3.7 install leaves shadow", mode_of(h) == "shadow", mode_of(h))
check("and the 0.3.7 gate lets rm -rf ~/Documents run", hook(installed(h), env, h) == 0)
p = install(GATE, env, h)
check("0.3.9 install exits 0", p.returncode == 0, p.stdout[-400:] + p.stderr[-400:])
check("0.3.9 install switches the mode to enforce", mode_of(h) == "enforce", mode_of(h))
check("and says why", "before 0.3.9 installed in shadow mode" in p.stdout, p.stdout[-400:])
check("the upgraded gate stops rm -rf ~/Documents", hook(installed(h), env, h) == 2)
check("a MODE_SET receipt records the switch", any(r.get("decision") == "MODE_SET" and r.get("source") == "upgrade"
                                                   and r.get("mode") == "enforce" for r in receipts(h)))
p = install(GATE, env, h)
check("a second install says nothing more", "before 0.3.9" not in p.stdout and mode_of(h) == "enforce", p.stdout[-300:])

# 2. The last pre-0.3.9 gate, where the owner chose shadow with its own mode command: kept.
h, env = home("owner-chose-shadow")
install(old_gate("pre-0.3.9"), env, h)
saved = {k: os.environ.get(k) for k in ("HOME", "USERPROFILE", "OPERATOR_HOME")}
os.environ.update(HOME=h, USERPROFILE=h)
os.environ.pop("OPERATOR_HOME", None)
spec = importlib.util.spec_from_file_location("gate_pre_039", old_gate("pre-0.3.9"))
old = importlib.util.module_from_spec(spec)
spec.loader.exec_module(old)                     # module level only: constants and functions, no main()
old.write_receipt({"decision": "MODE_SET", "mode": "shadow", "source": "owner-tty"})   # what its `mode shadow` writes
for k, v in saved.items():
    if v is None:
        os.environ.pop(k, None)
    else:
        os.environ[k] = v
p = install(GATE, env, h)
check("an owner's shadow survives the 0.3.9 install", mode_of(h) == "shadow" and "before 0.3.9" not in p.stdout,
      mode_of(h))
check("and the gate records without blocking", hook(installed(h), env, h) == 0 and
      receipts(h)[-1].get("decision") == "SHADOW_STOP")

# 3. A 0.3.7 install, then only a 0.3.9 hook runs (the OpenClaw plugin's own gate, or a copied file).
h, env = home("037-then-hook")
install(old_gate("0.3.7"), env, h)
check("the first 0.3.9 hook call stops rm -rf ~/Documents", hook(GATE, env, h) == 2)
check("and leaves the mode on enforce", mode_of(h) == "enforce", mode_of(h))

# 4. No installer ever ran here; the owner wrote shadow by hand: kept.
h, env = home("hand-shadow-no-install")
os.makedirs(os.path.join(h, ".operator"), exist_ok=True)
with open(os.path.join(h, ".operator", "mode"), "w", encoding="utf-8") as f:
    f.write("shadow\n")
check("a hand-written shadow with no installer is kept", hook(GATE, env, h) == 0 and mode_of(h) == "shadow",
      mode_of(h))

# 5. A fresh 0.3.9 install, then the owner writes shadow by hand: kept, by the hook and by a reinstall.
h, env = home("fresh-then-hand-shadow")
install(GATE, env, h)
check("a fresh install blocks", mode_of(h) == "enforce" and hook(installed(h), env, h) == 2, mode_of(h))
with open(os.path.join(h, ".operator", "mode"), "w", encoding="utf-8") as f:
    f.write("shadow\n")
check("a later echo shadow is kept by the hook", hook(installed(h), env, h) == 0 and mode_of(h) == "shadow")
install(GATE, env, h)
check("and by a second install", mode_of(h) == "shadow", mode_of(h))

# 6. Where the paid gate is installed it owns the mode: nothing is switched.
h, env = home("paid-gate-owns-mode")
os.makedirs(os.path.join(h, ".operator", "gate"), exist_ok=True)
with open(installed(h), "w", encoding="utf-8") as f:
    f.write('VERSION = "0.3.13"\nEDITION = "operator"\n')
with open(os.path.join(h, ".operator", "mode"), "w", encoding="utf-8") as f:
    f.write("shadow\n")
hook(GATE, env, h)
check("a paid gate's shadow is left alone", mode_of(h) == "shadow", mode_of(h))

# 7. A 0.3.7 install, then a port installer (they share the mode file): the Cursor port.
if os.name != "nt" and shutil.which("sh"):
    h, env = home("037-then-cursor-port")
    install(old_gate("0.3.7"), env, h)
    p = subprocess.run(["sh", os.path.join(HERE, "ports", "cursor", "install.sh")], capture_output=True, text=True,
                       env=env, cwd=h, timeout=120)
    check("the Cursor port's install switches 0.3.7's shadow to enforce", mode_of(h) == "enforce",
          mode_of(h) + " " + p.stdout[-400:] + p.stderr[-300:])
    check("and says why", "before 0.3.9 installed in shadow mode" in p.stdout, p.stdout[-400:])
    check("the hook stops rm -rf ~/Documents", hook(installed(h), env, h) == 2)
    with open(os.path.join(h, ".operator", "mode"), "w", encoding="utf-8") as f:
        f.write("shadow\n")
    subprocess.run(["sh", os.path.join(HERE, "ports", "cursor", "install.sh")], capture_output=True, text=True,
                   env=env, cwd=h, timeout=120)
    check("a shadow chosen after that survives a port reinstall", mode_of(h) == "shadow", mode_of(h))

print("\n%d failed" % len(fails) if fails else "\nall upgrade checks passed")
sys.exit(1 if fails else 0)
