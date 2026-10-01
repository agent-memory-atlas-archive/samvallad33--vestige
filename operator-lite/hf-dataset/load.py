#!/usr/bin/env python3
"""load.py -- load the GuardFall bypass corpus and pretty-print its distributions.

Prefers the Hugging Face `datasets` library (the exact path the dataset card
documents: load_dataset("json", data_files=..., field="cases")) and falls back
to stdlib json when `datasets` is not installed. Both paths yield the same rows;
neither ever executes a case's `cmd` -- classification material is data only.

Usage:
    python3 load.py            # distributions (class + expected verdict)
    python3 load.py --ids      # also list case ids grouped by class
    python3 load.py path.json  # explicit corpus path (default: ./guardfall.json)
"""
import argparse
import json
import os
import sys
from collections import Counter

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_PATH = os.path.join(HERE, "guardfall.json")


def load_rows(path):
    """The 43 corpus rows, via `datasets` when available (hub-compatible
    loader, field="cases") or stdlib json otherwise."""
    try:
        from datasets import load_dataset  # type: ignore
    except ImportError:
        with open(path, encoding="utf-8") as f:
            corpus = json.load(f)
        return corpus["cases"], "stdlib json"

    ds = load_dataset("json", data_files=path, field="cases")["train"]
    rows = [dict(r) for r in ds]
    return rows, "datasets (json loader, field=cases)"


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("path", nargs="?", default=DEFAULT_PATH)
    ap.add_argument("--ids", action="store_true",
                    help="also list every case id grouped by class")
    args = ap.parse_args(argv)

    rows, backend = load_rows(args.path)
    classes = Counter(r["class"] for r in rows)
    expects = Counter(r["expect"] for r in rows)

    print("corpus : %s" % os.path.basename(args.path))
    print("loader : %s" % backend)
    print("cases  : %d (%d distinct classes)" % (len(rows), len(classes)))
    print()
    width = max(len(c) for c in classes)
    print("== class distribution ==")
    for cls, n in sorted(classes.items(), key=lambda kv: (-kv[1], kv[0])):
        ids = "   " + ", ".join(sorted(r["id"] for r in rows if r["class"] == cls)) \
            if args.ids else ""
        print("  %-*s %2d%s" % (width, cls, n, ids))
    print()
    print("== expected-verdict distribution ==")
    for exp, n in sorted(expects.items()):
        print("  %-9s %2d" % (exp, n))
    return 0


if __name__ == "__main__":
    sys.exit(main())
