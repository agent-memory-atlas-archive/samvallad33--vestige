#!/usr/bin/env python3
"""Reject reqwest in each distinct feature set used to release vestige-mcp."""
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def release_feature_sets(text):
    """Read the release matrix and explicit builds, including folded shell lines."""
    joined = []
    pending = ""
    for raw in text.splitlines():
        if raw.rstrip().endswith("\\"):
            pending += raw.rstrip()[:-1] + " "
        else:
            joined.append(pending + raw)
            pending = ""
    if pending:
        joined.append(pending)

    sets = []
    target = ""
    for line in joined:
        match = re.search(r"(?:^|\s)target:\s*([A-Za-z0-9_.:-]+)\s*$", line)
        if match:
            target = match.group(1)
        match = re.search(r'cargo_flags:\s*"([^"]*)"$', line)
        if match:
            sets.append((target or "cargo_flags", shlex.split(match.group(1))))

    for line in joined:
        if "vestige-mcp" not in line or "matrix.cargo_flags" in line:
            continue
        if not re.search(r"\bcargo\s+(?:ndk\b.*\s)?build\b", line):
            continue
        args = ["--no-default-features"] if "--no-default-features" in line else []
        features = re.search(r"--features\s+(\S+)", line)
        if features:
            args.extend(["--features", features.group(1).strip("\"'")])
        target_flag = re.search(r"--target\s+(\S+)", line)
        label = (target_flag.group(1) if target_flag else
                 "aarch64-linux-android" if "cargo ndk" in line or "linux-android" in line else
                 "vestige-mcp")
        sets.append((label, args))

    unique = {}
    for label, args in sets:
        unique.setdefault(tuple(args), (label, args))
    if not unique:
        raise ValueError("no release feature sets found in release.yml")
    return list(unique.values())


def check_feature_set(label, args, run=subprocess.run):
    print(f"== {label}" + (f" ({' '.join(args)})" if args else ""), flush=True)
    result = run(["cargo", "tree", "-p", "vestige-mcp", "--locked", "-e", "features",
                  "-i", "reqwest", *args], cwd=ROOT, text=True,
                 stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                 env=dict(os.environ, CARGO_TERM_COLOR="never"), check=False)
    output = result.stdout
    if result.returncode == 0 or re.search(r"reqwest v[0-9]", output):
        print(f"FAIL: {label} links reqwest\n{output}", file=sys.stderr)
        return False
    if "did not match any packages" not in output:
        print(f"FAIL: cargo tree did not prove reqwest absent for {label} "
              f"(exit {result.returncode})\n{output}", file=sys.stderr)
        return False
    print("ok: reqwest absent", flush=True)
    return True


def main():
    try:
        sets = release_feature_sets((ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8"))
        results = [check_feature_set(label, args) for label, args in sets]
        return 0 if all(results) else 1
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
