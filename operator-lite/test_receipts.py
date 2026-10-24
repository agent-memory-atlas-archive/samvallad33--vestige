#!/usr/bin/env python3
"""The receipt chain links each receipt to the last complete line of the ledger, not to HEAD.

HEAD is only a cache. A stale, missing or unwritable HEAD (a second gate copy, a restore, a crash between
the append and the cache write) must not fork the chain, and an incomplete last line must stop the next
append instead of being skipped. No command is executed: receipts are written straight to a throwaway home.
Usage: python3 operator-lite/test_receipts.py [path/to/operator-gate.py]"""
import contextlib, importlib.util, io, json, os, shutil, sys, tempfile

GATE = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "operator-gate.py"))
fails = []


def check(name, cond, detail=""):
    print("%s  %s%s" % ("ok " if cond else "BAD", name, ("   -> " + str(detail)[:300]) if (detail and not cond) else ""))
    if not cond:
        fails.append(name)


home = tempfile.mkdtemp(prefix="oplite-receipts-home-")
os.environ["HOME"] = home
os.environ.pop("OPERATOR_HOME", None)
os.environ.pop("OPERATOR_AGENT_SESSION", None)
spec = importlib.util.spec_from_file_location("gate", GATE)
g = importlib.util.module_from_spec(spec)
spec.loader.exec_module(g)
RDIR = os.path.join(g.OP_HOME, "receipts")
HEAD = os.path.join(RDIR, "HEAD")


def receipt(n):
    return g.write_receipt({"decision": "PASS", "tool": "Bash", "action_preview": "echo %d" % n})


def ledger():
    rows = []
    for name in sorted(f for f in os.listdir(RDIR) if f.endswith(".jsonl")):
        with open(os.path.join(RDIR, name), encoding="utf-8") as f:
            rows += [json.loads(line) for line in f]
    return rows


def verify():
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        code = g.cmd_verify([])
    return code, out.getvalue()


try:
    # 1. a fresh ledger chains from zero, one receipt to the next
    first = [receipt(n) for n in range(3)]
    rows = ledger()
    check("three receipts written", all(first) and len(rows) == 3, first)
    check("the first receipt chains from zero", rows[0]["prev"] == "0" * 64, rows[0].get("prev"))
    check("each receipt chains to the one before", [r["prev"] for r in rows[1:]] == first[:2])
    code, out = verify()
    check("verify reads a fresh ledger as OK", code == 0 and "chain=OK" in out, out)

    # 2. a stale HEAD (an older digest written by another copy of the gate) no longer forks the chain
    with open(HEAD, "w", encoding="utf-8") as f:
        f.write(first[0])
    d = receipt(3)
    check("a stale HEAD: the new receipt chains to the ledger's last line", ledger()[-1]["prev"] == first[2], ledger()[-1].get("prev"))
    with open(HEAD, encoding="utf-8") as f:
        check("HEAD is refreshed after the append", f.read().strip() == d)
    code, out = verify()
    check("verify stays OK after a stale HEAD", code == 0 and "chain=OK" in out, out)

    # 3. a missing HEAD does not restart the chain from zero
    os.remove(HEAD)
    e = receipt(4)
    check("a missing HEAD: the new receipt chains to the ledger's last line", ledger()[-1]["prev"] == d, ledger()[-1].get("prev"))
    code, out = verify()
    check("verify stays OK after a missing HEAD", code == 0 and "chain=OK" in out, out)

    # 4. a HEAD that cannot be written does not lose the receipt that was already appended
    os.remove(HEAD)
    os.mkdir(HEAD)
    f5 = receipt(5)
    check("an unwritable HEAD: the receipt is appended and its digest returned", f5 and ledger()[-1]["digest"] == f5, f5)
    check("an unwritable HEAD: the receipt still chains", ledger()[-1]["prev"] == e, ledger()[-1].get("prev"))
    os.rmdir(HEAD)
    code, out = verify()
    check("verify stays OK after an unwritable HEAD", code == 0 and "chain=OK" in out, out)

    # 5. across day files the tail is the last line of the newest file
    today = [n for n in os.listdir(RDIR) if n.endswith(".jsonl")]
    os.rename(os.path.join(RDIR, today[0]), os.path.join(RDIR, "2000-01-01.jsonl"))
    with open(HEAD, "w", encoding="utf-8") as f:
        f.write("f" * 64)
    receipt(6)
    check("a new day file chains to the last line of the previous one", ledger()[-1]["prev"] == f5, ledger()[-1].get("prev"))
    code, out = verify()
    check("verify stays OK across day files", code == 0 and "chain=OK" in out, out)

    # 6. an incomplete last line is evidence: it stops the next append and stays exactly as it was
    newest = sorted(n for n in os.listdir(RDIR) if n.endswith(".jsonl"))[-1]
    path = os.path.join(RDIR, newest)
    with open(path, "a", encoding="utf-8") as f:
        f.write('{"decision": "PASS", "partial')
    with open(path, "rb") as f:
        before = f.read()
    check("an incomplete tail refuses the append", receipt(7) is None)
    with open(path, "rb") as f:
        check("an incomplete tail is left untouched", f.read() == before)

    # 7. a last line whose digest does not match its content also refuses the append
    with open(path, "wb") as f:
        f.write(before[:before.rfind(b"\n") + 1])
    rows = ledger()
    tampered = dict(rows[-1], action_preview="echo tampered")
    with open(path, "rb") as f:
        lines = f.read().splitlines(keepends=True)
    lines[-1] = (json.dumps(tampered, sort_keys=True, ensure_ascii=False) + "\n").encode("utf-8")
    with open(path, "wb") as f:
        f.write(b"".join(lines))
    check("a tail whose digest does not match refuses the append", receipt(8) is None)
finally:
    shutil.rmtree(home, ignore_errors=True)

print("\n%d failed" % len(fails) if fails else "\nall receipt-chain checks passed")
sys.exit(1 if fails else 0)
