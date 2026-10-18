# operator-lite

A deterministic pre-tool gate for coding agents. One command, the same on macOS, Linux and Windows:

    npx operator-lite

It installs the gate to `~/.operator/gate`, wires Claude Code, then wires every other agent it
finds on the machine (Cursor, Gemini CLI, Windsurf, Cline, Goose, opencode, Amazon Q). It starts in
enforce mode: from your agent's next tool call, every command a STOP rule matches is stopped before
it runs, and every verdict gets a hash-chained receipt. An existing `~/.operator/mode` file is kept, except the `shadow` every install before 0.3.9 wrote, which becomes `enforce` once; a shadow you choose with `operator-gate mode shadow` stays.
To record without blocking:

    npx operator-lite mode shadow

To check it on your own machine before you trust it:

    npx operator-lite test       # prints: corpus=guardfall cases=46 passed=46 failed=0
    npx operator-lite status     # the mode, whether the hook is registered, what it stopped
    npx operator-lite verify     # the receipt chain, in order

`test` replays the 46-case GuardFall bypass corpus this package carries, with the gate it
carries; it classifies every command and executes none.

The STOP rules cover edits to the gate itself, wiping the Vestige memory store, deleting a
workspace, home or `/`, recursive deletes outside the project,
force-pushes and `.git` removal, unreviewed publishes, production deploys and infrastructure
teardown, destructive SQL and database resets, shell-init writes, reverse shells, cloud metadata
endpoints, discarded uncommitted git work, shredded or emptied files, raw-disk and recursive
permission wrecks, fork bombs, and a download piped or handed straight to a shell.

Run it in your own terminal. The installer refuses to run in a shell that Claude Code's agent
started (it reads `CLAUDECODE` and `CLAUDE_CODE_ENTRYPOINT`), or in any shell that sets
`OPERATOR_AGENT_SESSION`. Other agents' shells are not recognized, so do not hand it to an agent.

It never replaces a gate that outranks it. When `~/.operator/gate/operator-gate.py` is the paid
Operator gate, or a newer version, it prints both versions and the path, changes nothing, and exits 4.
`npx operator-lite --force` replaces it anyway: it warns, keeps the replaced gate in
`~/.operator/install-backups/`, and prints the command that puts it back.

Requires Python 3.9 or newer (the gate is one stdlib-only Python file, carried in this package).
On Windows, Claude Code is wired; the other hosts are listed with a link to their manual steps.

Other commands pass through to the gate: `status`, `replay`, `mode`, `approve`, `verify`, `uninstall`.
They run the installed gate when there is one (which may be the paid Operator), else the gate in this
package, and each first prints which gate ran, with its version and path, on stderr.

Full documentation: https://github.com/samvallad33/vestige/tree/main/operator-lite
