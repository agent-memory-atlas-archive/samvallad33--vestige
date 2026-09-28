#!/usr/bin/env python3
"""Backfill top-3 evaluation over closed GitHub issues with known fixing commits.

Ground truth: for each entry in corpus.json, the "causing" commit is derived
SZZ-style from the fixing commit — for every file the fix touched, the last
commit that touched that file BEFORE the issue was filed. A run HITS when any
causing commit appears in the backfill's top 3. Nothing is hand-scored: every
SHA comes from git, every ranking comes from the engine, and every corpus
entry runs — no cherry-picking. The `vestige` binary must be the one built
from this branch.

Usage: python3 scripts/backfill-eval/eval.py [--workdir DIR] [--vestige BIN]
"""

import argparse
import json
import re
import subprocess
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

CORPUS = Path(__file__).parent / "corpus.json"
WINDOW_DAYS = 365          # commit window ingested before the issue
LOOKBACK_DAYS = 365        # backfill reach (the tool clamps at 365)
SHALLOW_MARGIN_DAYS = 60   # extra history fetched so the window is fully covered


def sh(args, cwd=None):
    return subprocess.run(args, cwd=cwd, capture_output=True, text=True)


def must(args, cwd=None, what=""):
    r = sh(args, cwd=cwd)
    if r.returncode != 0:
        raise RuntimeError(f"{what or args[0]} failed: {r.stderr.strip()[:300]}")
    return r.stdout


def parse_args():
    ap = argparse.ArgumentParser()
    ap.add_argument("--workdir", default="/tmp/vestige-backfill-eval")
    ap.add_argument("--vestige", default=None,
                    help="path to the vestige binary built from this branch")
    return ap.parse_args()


def main():
    args = parse_args()
    workdir = Path(args.workdir)
    repos = workdir / "repos"
    stores = workdir / "stores"
    repos.mkdir(parents=True, exist_ok=True)
    stores.mkdir(parents=True, exist_ok=True)

    script = Path(__file__).resolve()
    default_bin = script.parents[2] / "target" / "release" / "vestige"
    vestige = args.vestige or str(default_bin)
    if not Path(vestige).exists():
        sys.exit(f"vestige binary not found at {vestige} — build with "
                 "`cargo build --release --bin vestige` or pass --vestige")

    entries = json.loads(CORPUS.read_text())
    rows = []
    for entry in entries:
        rows.append(run_entry(entry, repos, stores, vestige))
    write_report(rows, workdir)


def run_entry(entry, repos, stores, vestige):
    repo_slug = entry["repo"].split("/")[-1]
    issue_date = datetime.fromisoformat(entry["created_at"].replace("Z", "+00:00"))
    row = {
        "repo": f"{entry['repo']}#{entry['issue_number']}",
        "note": entry.get("note", ""),
    }
    try:
        clone = clone_repo(entry, repos, repo_slug, issue_date)
        causing = causing_commits(clone, entry, issue_date)
        if not causing:
            row["result"] = "ground truth unreachable"
            row["detail"] = "no commit found touching the fix files before the issue"
            return row
        row["causing"] = [s[:12] for s in sorted(causing)]

        store = stores / f"{repo_slug}-{entry['issue_number']}"
        if store.exists():
            subprocess.run(["rm", "-rf", str(store)], check=True)
        ingest_commits(vestige, clone, store, issue_date)
        failure_id = ingest_issue(vestige, store, entry)
        out = run_backfill(vestige, store, failure_id)
        row.update(score(out, causing, row))
    except Exception as e:  # report the failure, never substitute a result
        row["result"] = "error"
        row["detail"] = str(e)[:300]
    return row


def clone_repo(entry, repos, repo_slug, issue_date):
    clone = repos / repo_slug
    since = (issue_date - timedelta(days=WINDOW_DAYS + SHALLOW_MARGIN_DAYS)).strftime("%Y-%m-%d")
    if not (clone / ".git").exists():
        must(["git", "clone", "--shallow-since=" + since, "--single-branch",
              f"https://github.com/{entry['repo']}", str(clone)],
             what="git clone")
    return clone


def causing_commits(clone, entry, issue_date):
    """SZZ-lite: per fixed file, the last commit before the issue that touched it."""
    before = issue_date.strftime("%Y-%m-%dT%H:%M:%S%z")
    shas = set()
    for f in entry["fix_files"]:
        out = sh(["git", "log", f"--before={before}", "-n", "1", "--format=%H", "--", f],
                 cwd=clone)
        if out.returncode == 0 and out.stdout.strip():
            shas.add(out.stdout.strip().lower())
    return shas


def ingest_commits(vestige, clone, store, issue_date):
    # bound the window on BOTH sides of the issue: an open upper bound lets
    # post-issue commits consume the max-commits budget (run-1 harness bug)
    since = (issue_date - timedelta(days=WINDOW_DAYS)).strftime("%Y-%m-%dT%H:%M:%S%z")
    until = (issue_date + timedelta(days=1)).strftime("%Y-%m-%dT%H:%M:%S%z")
    must([vestige, "--data-dir", str(store), "ingest-git", str(clone),
          "--since", since, "--until", until, "--max-commits", "2000", "--json"],
         what="ingest-git")


def ingest_issue(vestige, store, entry):
    content = f"{entry['title']}. {entry['body_excerpt']}"
    # `vestige ingest` has no --json output; the Node ID line carries the id
    out = must([vestige, "--data-dir", str(store), "ingest", content,
                "--node-type", "event", "--tags", "issue,eval",
                "--created-at", entry["created_at"]],
               what="ingest issue")
    match = re.search(r"^Node ID: (\S+)", out, re.MULTILINE)
    if not match:
        raise RuntimeError(f"no Node ID in ingest output: {out[:200]}")
    return match.group(1)


def run_backfill(vestige, store, failure_id):
    out = must([vestige, "--data-dir", str(store), "backfill", "--failure-id", failure_id,
                "--manual", "--no-promote", "--json",
                "--lookback-days", str(LOOKBACK_DAYS)],
               what="backfill")
    return json.loads(out)


def score(out, causing, row):
    causes = out.get("causes", [])[:3]
    hit_rank = None
    for i, c in enumerate(causes, 1):
        content = (c.get("content_preview") or "").lower()
        for sha in causing:
            if sha[:12] in content:
                hit_rank = i
                break
        if hit_rank:
            break
    row["rank"] = hit_rank
    row["result"] = "hit" if hit_rank else "miss"
    gap = out.get("gap")
    row["detail"] = (
        f"causes={len(causes)} rejected={len(out.get('rejected', []))} "
        f"gap={'yes: ' + (gap.get('note', '')[:120] if gap else '') if gap else 'no'}"
    )
    return row


def write_report(rows, workdir):
    hits = [r for r in rows if r["result"] == "hit"]
    misses = [r for r in rows if r["result"] == "miss"]
    other = [r for r in rows if r["result"] not in ("hit", "miss")]
    lines = [
        "# Backfill top-3 evaluation — 20 closed issues with known fixing commits",
        "",
        f"Run: {datetime.now(timezone.utc).isoformat()} · corpus: scripts/backfill-eval/corpus.json · "
        f"window: {WINDOW_DAYS}d · lookback: {LOOKBACK_DAYS}d · hits in top-3: "
        f"**{len(hits)}/{len(rows)}**",
        "",
        "| issue | result | rank | causing commit(s) | detail |",
        "|---|---|---|---|---|",
    ]
    for r in rows:
        lines.append(
            f"| {r['repo']} | {r['result']} | {r.get('rank') or '—'} "
            f"| {', '.join(r.get('causing', ['—']))} | {r.get('detail', '')} |"
        )
    lines += ["", "## Notes"]
    notes = [r for r in rows if r.get("note")]
    for r in notes:
        lines.append(f"- {r['repo']}: {r['note']}")
    lines += [
        "",
        "Ground truth is derived SZZ-style from the verified fixing commit: for each",
        "file the fix touched, the last commit touching that file before the issue was",
        "filed. Every corpus entry runs; results are reported as produced. The mandated",
        "OpenHands issue #16727 is OPEN with an unmerged fix PR, so it cannot anchor a",
        "known-fix row; the nearest-area OpenHands issue with a verified fix (#1187) is",
        "entry #1 and the deviation is disclosed here rather than silently swapped.",
        "",
        "Known limitations: issues whose causing commits predate the ingested window,",
        "fixes whose files were renamed, and issues whose bodies name no file at all",
        "are expected misses — the gap report is the designed output for those.",
    ]
    out_path = workdir / "results.md"
    out_path.write_text("\n".join(lines) + "\n")
    print(f"{len(hits)} hits / {len(misses)} misses / {len(other)} other — {out_path}")
    for r in rows:
        print(f"  {r['result']:28} {r['repo']}" + (f" rank={r['rank']}" if r.get("rank") else ""))


if __name__ == "__main__":
    main()
