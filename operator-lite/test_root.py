import importlib.util, os, sys, json, tempfile
home = tempfile.mkdtemp(prefix="rootcheck-", dir=os.path.expanduser("~"))
os.environ["HOME"] = home; os.environ["OPERATOR_HOME"] = os.path.join(home, ".operator")
os.environ.pop("OPERATOR_AGENT_SESSION", None)
spec = importlib.util.spec_from_file_location("gate", (sys.argv[1] if len(sys.argv) > 1 and not sys.argv[1].startswith("-") else os.path.join(os.path.dirname(os.path.abspath(__file__)), "operator-gate.py"))); g = importlib.util.module_from_spec(spec); spec.loader.exec_module(g)
cfg = g.load_config(); work = os.path.join(home, "work"); os.makedirs(work)

def rules(cmd, background=False, tool="Bash"):
    ti = {"command": cmd}
    if background:
        ti["run_in_background"] = True
    return [h[0] for h in g.classify({"tool_name": tool, "tool_input": ti, "cwd": work}, cfg)[2]]

# (command, started as a background task)
STOP = [
    # the root job from anthropics/claude-code issue 99768, and its relatives
    ('sudo BUILD_TARGET=image ./build-image.sh rpi5 > "$LOG" 2>&1', True),
    ("sudo ./build.sh", True),
    ("sudo make install &", False),
    ("sudo -E ./build.sh > build.log 2>&1 &", False),
    ("nohup sudo ./build.sh > build.log", False),
    ("sudo nohup ./build.sh", False),
    ("sudo setsid ./serve.sh", False),
    ("doas ./build.sh &", False),
    ("cd infra && sudo ./provision.sh &", False),
    # the kill from the same issue: a process group, as root
    ("sudo /usr/bin/kill -TERM -1785862", False),
    ("sudo kill -TERM -1785862", False),
    ("sudo kill -9 -4242", False),
    ("sudo kill -s KILL -- -4242", False),
    ("doas kill -TERM -77", False),
    # every process, root or not
    ("kill -9 -1", False),
    ("kill -TERM -1", False),
    ("kill -s TERM -1", False),
    ("kill -- -1", False),
    ("sudo kill -KILL -1", False),
    ("/bin/kill -15 -1", False),
    ("killall5 -9", False),
    ("sudo killall5", False),
]
ALLOW = [
    ("sudo apt-get update", False),
    ("sudo ./build.sh", False),
    ("sudo systemctl restart nginx", False),
    ("./build.sh &", False),
    ("./build.sh", True),
    ("nohup ./serve.sh > serve.log 2>&1 &", False),
    ("make -j8 && sudo make install", False),
    ("sudo make install 2>&1 | tee install.log", False),
    ("sudo sh -c 'echo a && echo b'", False),
    ("kill 4242", False),
    ("kill -9 4242", False),
    ("kill -TERM 4242 4243", False),
    ("kill -0 4242", False),
    ("kill -1 4242", False),
    ("kill -HUP 1", False),
    ("kill %1", False),
    ("kill -l", False),
    ("kill -TERM -4242", False),
    ("kill -- -4242", False),
    ("sudo kill 4242", False),
    ("sudo kill -9 4242", False),
    ("pkill -f vite", False),
    ("echo 'sudo ./build.sh &'", False),
    ("git commit -m 'kill -9 -1 is every process'", False),
    ("ls -la && cargo check", False),
]

bad = 0
for c, bg in STOP:
    r = rules(c, bg)
    if "OP-013" not in r:
        bad += 1; print("MISS   %-70s %s" % (c[:70], r))
for c, bg in ALLOW:
    r = rules(c, bg)
    if "OP-013" in r:
        bad += 1; print("FALSE  %-70s %s" % (c[:70], r))
ps = [("sudo ./build.sh", True), ("kill -9 -1", False)]
for c, bg in ps:
    r = rules(c, bg, "PowerShell")
    if "OP-013" in r:
        bad += 1; print("PS FALSE %-60s %s" % (c, r))
print("stop cases %d, allow cases %d, powershell %d, wrong %d" % (len(STOP), len(ALLOW), len(ps), bad))
import shutil; shutil.rmtree(home, ignore_errors=True)
sys.exit(1 if bad else 0)
