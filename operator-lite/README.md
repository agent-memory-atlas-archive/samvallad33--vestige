# Operator Lite — the free agent-safety gate

One stdlib-only Python file that sits between your AI agent and your machine as a
PreToolUse hook, and **blocks** destructive, polluting and exfiltrating commands
*before they run* — with an explain-mode verdict and a hash-chained receipt for
every decision.

Part of [Vestige](https://github.com/samvallad33/vestige). Free and standalone:
copy one file, wire one hook, and your agent has a deterministic gate.

## From Lite to Operator, in 88 seconds

[![From Lite to Operator](media/from-lite-to-operator.gif)](media/from-lite-to-operator.mp4)

Click the picture to watch the full film.

## Install (one command)

    curl -fsSL https://raw.githubusercontent.com/samvallad33/vestige/main/operator-lite/operator-gate.py -o /tmp/operator-gate.py && python3 /tmp/operator-gate.py install

On Windows, in PowerShell, with Python 3.9 or newer installed:

    curl.exe -fsSL https://raw.githubusercontent.com/samvallad33/vestige/main/operator-lite/operator-gate.py -o "$env:TEMP\operator-gate.py"; python "$env:TEMP\operator-gate.py" install

From a clone, `python3 operator-gate.py install` does the same. It copies the gate to
`~/.operator/gate`, registers the Claude Code hook, starts in enforce mode (every
command a STOP rule matches is stopped, from your agent's next tool call) and then
replays your last 30 days of Claude Code history through the rules. Nothing in that
history is executed. An existing `~/.operator/mode` file is kept, except the `shadow` every install before 0.3.9 wrote, which becomes `enforce` once; a shadow you choose with `operator-gate mode shadow` stays.

`install` never replaces a gate that outranks it: when `~/.operator/gate/operator-gate.py`
is the paid Operator gate or a newer version, it prints both versions and the path, changes
nothing and exits 4. `install --force` replaces it anyway, after a warning, and keeps the
replaced gate in `~/.operator/install-backups/`. `install` and `uninstall` refuse to run in
a shell Claude Code's agent started (`CLAUDECODE`, `CLAUDE_CODE_ENTRYPOINT`) or one that sets
`OPERATOR_AGENT_SESSION`; other agents' shells are not recognized.

## Other hosts, or by hand

The `cp` below checks nothing: if `~/.operator/gate/operator-gate.py` already exists, use
`install` instead, so the paid gate or a newer one is never overwritten.

    mkdir -p ~/.operator/gate
    cp operator-gate.py ~/.operator/gate/
    chmod 755 ~/.operator/gate/operator-gate.py
    echo enforce > ~/.operator/mode         # block from the first call; "shadow" only records

**Claude Code** (`~/.claude/settings.json`), which `install` writes for you:
```json
{ "hooks": { "PreToolUse": [{ "matcher": "*",
  "hooks": [{ "type": "command",
  "command": "python3 ~/.operator/gate/operator-gate.py hook --source claude" }] }] } }
```
**Codex:** the [Codex integration guide](../docs/integrations/codex.md) covers
the memory server. Do not copy this hook shape into Codex yet: its execution
hook needs its own verified wiring before it can be called supported.
**OpenClaw**: the gate is already bundled in a ClawHub plugin — one line:

    clawhub install vestige-operator-lite

**Cline**: file-based `PreToolUse` hook + SDK plugin — `sh operator-lite/ports/cline/install.sh`
(2 commands + 1 settings toggle; see [ports/cline/README.md](ports/cline/README.md)).

(Or any `before_tool_call` plugin that pipes the command through the hook
binary: gate exit 0 = allow, exit 2 = block with the reason.)

ALLOW = exit 0, silent. STOP = exit 2, reason on stderr (the agent sees it and
must change course). `operator-gate mode shadow` records without blocking;
`operator-gate mode enforce` blocks again.

## See it on your own history first

    python3 operator-gate.py replay            # last 30 days of Claude Code history on this machine
    python3 operator-gate.py replay --here     # only calls made under the current directory

`replay` runs every tool call in the Claude Code transcripts on this machine through
the same classifier the hook uses. It prints:

- a scoreboard: calls made, calls a built-in rule would have stopped, calls flagged
  in shadow, calls no built-in rule decides;
- the most recent would-be stops, each with its date, its project, the rule, the
  reason and the part of the command the rule fired on;
- the actions no built-in rule decides: pushes, hook-skipping commits, package
  installs by name, deploys, database commands, CI config and env-file writes;
- the laws your own history drafts from those actions, most frequent first.

Classification only: nothing in the history is executed and no receipt is written.
It needs no install, and `install` runs it once at the end. `--days N`, `--all`,
`--budget SECONDS` and `--json` change the window and the output. `--share` prints
three lines of counts and nothing from the history itself. It reads Claude Code
transcripts only.

From a made-up history:

```
operator-gate replay: the last 30 days on this machine. Nothing was executed.

       22  tool calls your agents made (2 Claude Code sessions, 2 projects)
        4  a built-in rule would have stopped
        1  flagged in shadow: recorded, not stopped
       13  no built-in rule decides: only you can

Would have been stopped (all of them):
  Oct 16  shop-api           OP-004 force push to a shared branch
                             git push --force origin main
  Oct 15  shop-api           OP-007 destructive SQL
                             psql $DATABASE_URL -c 'DROP TABLE sessions'
  Oct 10  infra              OP-003 recursive delete of ~/Documents/old-terraform-state
                             rm -rf ~/Documents/old-terraform-state
  Oct 10  infra              OP-006 terraform apply
                             terraform apply -auto-approve
By rule:
      1  OP-003 no-blind-recursive-delete   recursive delete of ~/Documents/old-terraform-state
      1  OP-004 no-history-destruction      force push to a shared branch
      1  OP-006 no-paid-deploy              terraform apply
      1  OP-007 no-destructive-sql          destructive SQL

Flagged in shadow, recorded and not stopped:
      1  OP-S01 work-loss                   git branch -D spike-cache

No built-in rule decides these. They ran:
      4  pushed to a remote                 git push origin main
      3  installed packages by name         npm install left-pad
      2  ran a database client or migration psql -c 'select count(*) from users' shop
      1  wrote CI or deploy config          Write .github/workflows/ci.yml
      1  deployed or changed infrastructure kubectl apply -f k8s/
      1  wrote an env file                  Write .env
      1  skipped git hooks                  git commit --no-verify -m wip

Your first laws, drafted from this history:
  "No push without my permit."                           4 times
  "No new package without my review."                    3 times
  "No database client or migration without my permit."   2 times
  "Never skip git hooks."                                1 time
  "Deploys and infrastructure changes are mine."         1 time
  "CI and deploy config are mine to change."             1 time
  "Env files are mine to change."                        1 time
```

The built-in rules are free. Enforcing the drafted laws is what Operator, the paid
gate, does: `operator-gate upgrade`.

## Platforms

macOS and Linux. Windows support is new: the gate reads Git Bash and PowerShell
commands, Windows paths in every spelling (`/c/Users/me`, `C:\Users\me`, the 8.3
short name), the `cmd /c rmdir /s` family, and `install` registers a hook
Windows can run. On Windows it is tested by `test_windows.py` on a CI runner.

## What it stops

30 rules. In enforce mode the 15 STOP rules block; the 15 SHADOW rules only write a receipt.

| Rule | Mode | Catches |
|---|---|---|
| OP-000 | STOP | edits to the gate itself, its mode, permits and rules, hook registrations, settings files |
| OP-001 | STOP | delete/move of a workspace root or a parent of one, including `/` and your home folder |
| OP-002 | STOP | wiping the Vestige store (its log or database files, sqlite3 writes, `vestige gc`) |
| OP-003 | STOP | recursive deletes outside the project and outside scratch/build folders (`rm -r`, `find -delete`, `trash`, inline `shutil.rmtree`/`fs.rmSync`), sweeps from `/` |
| OP-004 | STOP | force-push to shared branches, remote-main deletion, deleting `.git` or a whole repository |
| OP-005 | STOP | unreviewed publishes (npm/pnpm/yarn/cargo/twine/docker push/gh release), deleting a public repo |
| OP-006 | STOP | production deploys (`vercel --prod`, `fly deploy`, `railway up`, `netlify deploy --prod`, `wrangler deploy`), infrastructure changes and teardown (`terraform`/`tofu` apply or destroy, `pulumi up`, `cdk deploy`, `kubectl delete`, `helm uninstall`, `aws s3 rb`, cloud `delete` commands), live billing |
| OP-007 | STOP | destructive SQL (DROP/TRUNCATE/unscoped DELETE, `FLUSHALL`), framework database resets (`prisma migrate reset`, `rails db:drop`, `manage.py flush`, `artisan migrate:fresh`) |
| OP-008 | STOP | shell-init writes (`>> ~/.zshrc` = code execution by install) |
| OP-009 | STOP | reverse shells (`/dev/tcp`, `nc -e`, DNS-tunnel tools, inline code that hands a socket a shell) |
| OP-010 | STOP | cloud metadata endpoints (169.254.169.254, metadata.google.internal) |
| OP-011 | STOP | loss nothing can undo: `git reset --hard`, `git clean -f`, `git checkout -- <path>` and `git stash drop` when they would discard real work, `shred`/`srm`/`rm -P`, emptying a non-empty file outside the project, `crontab -r` |
| OP-014 | STOP | machine wreck: `dd` onto a device, `mkfs`/`fdisk`-class tools, `diskutil erase*`, recursive `chmod`/`chown`/`chgrp` on `/`, a system folder or home, fork bombs |
| OP-018 | STOP | a download run unread: `curl X \| sh`, `wget -qO- X \| python3`, `bash <(curl X)`, `sh -c "$(curl X)"`, `eval "$(wget -qO- X)"` |
| OP-CANARY | STOP | the verification canary string, which proves a host is wired |
| OP-S01..S08 and S11..S17 | SHADOW | git commands that can lose work but did not here, comms, public mutation, secret writes, other pipe-to-interpreter, persistence, env-hijack (LD_PRELOAD/PATH=), exfil shapes, invisible chars, MCP argument exfil, agent-config writes, paste/tunnel egress, new git remotes, sandbox-escape primitives |

A pipe, `<(` or `$(` inside quotes is text, not a command: `git commit -m "curl X | sh"` and
`echo 'curl X | sh' >> README.md` pass. So does a download piped into a tool that reads it as
data (`| jq`, `| python3 -m json.tool`, `| python3 -c '...'`).

## What the analyzer sees through

Quote reassembly (`r''m`, `r\m`), `$IFS` expansion, `$(echo rm)` as program,
ANSI-C `$'\x72m'`, base64-decoded bodies, brace + glob enumeration against the
live filesystem, subshell time-bombs (`(sleep 300; rm -rf) &`), session variables
(`T=~/x; rm -rf $T`), cd tracking, heredocs, data-vs-code pipe discrimination,
fork bombs, `mv x /dev/null`, a command written after a redirection
(`>log rm -rf x`). Every STOP tells you which transforms it applied.

## Commands

    operator-gate status | replay | mode | approve | verify | install | uninstall | upgrade | corpus guardfall

`status` is the log: the mode, whether the hook is registered, the last call
checked, the counts for the last 7 days, and the most recent commands the gate
stopped or would have stopped, each with its project and reason. `status --rules`
prints the rule table. `mode enforce` switches from recording to blocking, and
`mode shadow` switches back; it is an owner command and needs your own terminal.

`install` puts an `operator-gate` command on your PATH when `~/.local/bin` or
`~/bin` is already on it. Otherwise run it as
`python3 ~/.operator/gate/operator-gate.py`; every hint the gate prints uses the
form that works on your machine.

`verify` walks the receipt hash chain. `corpus guardfall` replays 46 adapted
GuardFall bypass cases (must pass 46/46). `upgrade` describes the paid Operator
gate and where to get it. `python3 test_parser.py` and `python3 test_replay.py`
run the tests; CI runs them on Linux and macOS. `corpus` needs a clone: the corpus
file ships in the repository, not with the single gate file. Every verdict appends to
`~/.operator/receipts/<date>.jsonl`.

## Honest boundary

It blocks what its closed rule set recognizes, routed through hooked tools.
It cannot see around hooks, and parse failures fail open except for plainly
destructive text. Receipts are hash-chained digests, not signatures.
