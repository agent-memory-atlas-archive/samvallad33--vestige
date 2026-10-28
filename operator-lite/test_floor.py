#!/usr/bin/env python3
"""Checks for the floor (OP-012) and for measured loss, both new in 0.3.11.

    python3 test_floor.py

Every command here is only classified: nothing is run. The floor must stop a recursive delete
that reaches a filesystem or drive root, a home folder or a parent of the working directory,
however the target is spelled, and must leave ordinary cleanup alone. Measured loss must count
what is really on disk under a target.
"""
import importlib.util
import os
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("gate", os.path.join(HERE, "operator-gate.py"))
g = importlib.util.module_from_spec(spec)
spec.loader.exec_module(g)
cfg = g.load_config()
CWD = os.path.dirname(HERE)
D = "r" + "m -r" + "f "

# The first two are the commands from anthropics/claude-code issues 95426 and 99193.
MUST_STOP = [
    D + '"$(cygpath -u \'C:\\\')"',
    D + "C:/Users/SAMVAL~1",
    D + '"$TARGET/"',
    D + "$TARGET/*",
    D + '"$A/$B"',
    D + '"\\\\"',
    D + "..",
    D + "~",
    D + '"$HOME"',
    D + '"$(dirname "$PWD")"',
    D + '"$(pwd)/.."',
    D + '"`pwd`/.."',
    "D=; " + D + '"$D/"',
    D + "/mnt/c/",
    D + "C:/PROGRA~1",
    D + "/Applications" if os.path.isdir("/Applications") else D + "/",
    "sudo " + D + "/",
]
MUST_PASS = [
    D + "./build",
    D + "node_modules",
    D + "dist target",
    D + "src/__pycache__",
    D + '"${BUILD_DIR:?}/"',
    D + '"$BUILD_DIR"',
    D + '"$TMPDIR/x"',
    "D=./out; " + D + '"$D/"',
    D + '"$(git rev-parse --show-toplevel)/dist"',
    D + '"$(mktemp -d)"',
    D + "/tmp/scratch-x",
    "find . -name '*.pyc' -delete",
    "ls -la",
    "git status",
]


def check(ok, what):
    print(("PASS " if ok else "FAIL ") + what)
    return 0 if ok else 1


def floor(cmd, cwd=CWD):
    hits = g.classify({"tool_name": "Bash", "tool_input": {"command": cmd}, "cwd": cwd}, cfg)[2]
    return [h for h in hits if h[0] == "OP-012"]


def main():
    failures = 0
    for cmd in MUST_STOP:
        failures += check(bool(floor(cmd)), "stops   " + cmd)
    for cmd in MUST_PASS:
        failures += check(not floor(cmd), "passes  " + cmd)

    name, base, _why = g.RULES["OP-012"]
    failures += check(base == "STOP" and "OP-012" in g.FLOOR_RULES, "OP-012 is a stop rule and a floor rule")
    failures += check("${VAR:?}" in g.FLOOR_HINT, "the stop tells the agent the safe spelling")

    # measured loss: a folder under the home folder with three old files, one of them large
    with tempfile.TemporaryDirectory(prefix=".opgate-loss-", dir=g.HOME) as tmp:
        work = os.path.join(tmp, "notes")
        os.makedirs(os.path.join(work, "deep"))
        sizes = {"a.txt": 10, "b.txt": 2000, os.path.join("deep", "c.bin"): 300000}
        old = time.time() - 3 * 86400
        for rel, size in sizes.items():
            path = os.path.join(work, rel)
            with open(path, "wb") as f:
                f.write(b"x" * size)
            os.utime(path, (old, old))
        effects = g.analyze(D + work, tmp)
        loss = g.measure_loss(effects, tmp)
        failures += check(bool(loss) and loss["files"] == 3 and loss["bytes"] == sum(sizes.values()) and not loss["capped"],
                          "measured loss counts the files and bytes on disk")
        failures += check(bool(loss) and loss["unrecoverable"] == 3,
                          "outside any repository every old file is counted as unrecoverable")
        line = g.loss_line(loss)
        failures += check(line.startswith("3 files, ") and "3 of them exist nowhere else" in line,
                          "the sentence for the stop message reads: " + line)
        failures += check(g.measure_loss(g.analyze("ls -la", tmp), tmp) is None, "a command that removes nothing has no loss")
        failures += check(g.measure_loss(g.analyze(D + os.path.join(tmp, "missing"), tmp), tmp) is None,
                          "a target that does not exist has no loss")
    failures += check(g.loss_line(None) == "" and g.loss_line({"files": 0, "unrecoverable": 2}).startswith("2 uncommitted changes"),
                      "no loss gives no sentence; discarded changes are named as changes")
    failures += check(g.loss_line({"files": 20000, "bytes": 5 * 1024 ** 3, "unrecoverable": 0, "capped": True})
                      == "at least 20,000 files, 5.0 GB", "a capped count says at least")
    print("failures: %d" % failures)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
