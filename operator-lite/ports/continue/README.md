# Operator Lite — Continue port

The Operator gate ([../../operator-gate.py](../../operator-gate.py)) wired into
Continue at user scope. One stdlib-only Python file does all analysis. This
port is the installer: it contains no rule logic and no payload adapter.

## Install

```
git clone https://github.com/samvallad33/vestige && cd vestige/operator-lite   # skip if already cloned
sh ports/continue/install.sh
```

`install.sh` is idempotent:

- installs the gate to `~/.operator/gate/operator-gate.py` (downloads from
  GitHub, or uses this checkout's copy when it is newer; never overwrites an
  existing same-or-newer copy)
- sets `~/.operator/mode` to `enforce` if no mode file exists yet, and switches
  the `shadow` an install before 0.3.9 wrote to `enforce`, once
- merges one `preToolUse` hook into `~/.continue/hooks.json` (a timestamped
  backup is written before the first change; existing keys are preserved; a
  repeat run does not add a second entry and does not rewrite the file; an
  invalid existing file is left untouched)

Requires `python3` on PATH. `npx operator-lite` runs this installer when
`~/.continue` already exists.

## The hook

```json
{
  "version": 1,
  "hooks": {
    "preToolUse": [
      {
        "command": "[ -f '<gate>' ] && python3 '<gate>' hook --source continue",
        "timeout": 10,
        "failClosed": true,
        "matcher": "*"
      }
    ]
  }
}
```

`<gate>` is the absolute path of `~/.operator/gate/operator-gate.py`, quoted at
install time. The command is a shell command:

- gate file missing → the `[ -f ]` test exits 1 (not a policy STOP)
- gate exits 0 → allow
- gate exits 2 → block; stderr is the reason

Stdin is the gate contract: `{"tool_name", "tool_input", "cwd", "session_id"?}`.
`timeout` is seconds, matching Cursor's hooks schema.

## Enforce and shadow

```
echo shadow > ~/.operator/mode
echo enforce > ~/.operator/mode
```

## Honest limitations

- **Host wiring is not live-verified inside Continue.** The file written is
  the user-level Cursor-shaped `hooks.json`.
- **No payload adapter.** Shell and edit tools whose JSON already matches the
  gate contract are judged. A host-specific envelope needs a wrapper; this
  port does not ship one, and it does not modify the gate.
- The hook registration file itself is outside the gate's protected list.
  Receipts show a later edit; they do not stop it.

## Back to the main repo

[operator-lite](../../README.md) · [ports matrix](../README.md) ·
[Vestige](https://github.com/samvallad33/vestige)
