#!/bin/sh
# Operator Lite -- Aider port installer. POSIX sh, stdlib-only chain.
#
#   sh install.sh
#
# Aider has no hook or plugin API (verified against the options reference, the
# config reference and HISTORY.md -- see README.md), but its agent runs real
# shell commands through the user's PATH. So this port ships PATH shims: tiny
# POSIX sh wrappers for the binaries the gate's rules cover. Each shim pipes
# the standard gate payload to the real gate and:
#   gate exit 0    -> exec the real binary (absolute path baked in at install)
#   gate exit 2    -> print "Operator Lite blocked: <reason>", exit 126
#   gate unreachable -> fail-open unless the text looks destructive
#
# Does five things, idempotently:
#   1. installs the gate to ~/.operator/gate/operator-gate.py
#      (downloads from GitHub unless the existing copy is same-or-newer;
#      never overwrites a newer gate)
#   2. installs the shim dispatcher to ~/.operator/shim/lib/
#   3. generates shims into ~/.operator/shim/bin/ with absolute real-binary
#      paths baked in (binaries missing on this machine are skipped and noted)
#   4. sets mode to shadow if no mode file exists yet
#   5. prints how to run aider under the shims
#
# Installs are run by the owner in their own terminal -- same guard as the gate.
set -u

PORT_DIR=$(cd "$(dirname "$0")" && pwd)
GATE_URL="https://raw.githubusercontent.com/samvallad33/vestige/main/operator-lite/operator-gate.py"
REPO_GATE="$PORT_DIR/../../operator-gate.py"
DISPATCH_SRC="$PORT_DIR/operator-dispatch.py"

OP_HOME="${OPERATOR_HOME:-$HOME/.operator}"
GATE_DIR="$OP_HOME/gate"
GATE_DST="$GATE_DIR/operator-gate.py"
SHIM_BIN="$OP_HOME/shim/bin"
SHIM_LIB="$OP_HOME/shim/lib"
DISPATCH_DST="$SHIM_LIB/operator-dispatch.py"
MODE_DST="$OP_HOME/mode"

# Binaries whose invocations the gate's rules judge (rm -rf, force pushes,
# publishes, deploys, destructive SQL, reverse shells, init-file and
# credential writes). Binaries missing on this machine are skipped and
# reported; re-run install.sh after installing new tooling to cover them.
SHIM_BINS="ls rm rmdir unlink shred srm mv cp ln install rsync scp dd truncate chmod chown chflags xattr mkfifo tee sed find git gh npm pnpm yarn cargo twine docker fly flyctl stripe vercel wrangler supabase psql mysql mariadb duckdb mongosh redis-cli sqlite3 curl wget nc ncat base64 mkfs crontab at launchctl systemctl tmux screen"

# ERE translation of openclaw-plugin/index.js DESTRUCTIVE_LIKE (POSIX ERE
# has no \b). Baked into every shim as the unreachable-gate fallback filter.
OP_DESTRUCTIVE_ERE='(^|[^A-Za-z0-9_])rm[[:space:]]+-[a-zA-Z]*[rR]|(^|[^A-Za-z0-9_])push[[:space:]].*--force|DROP[[:space:]]+TABLE|vestige\.db|fly[[:space:]]+deploy|mkfs|(^|[^A-Za-z0-9_])dd[[:space:]]+if='

say() { printf '%s\n' "$*"; }
die() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

# --- owner guard (test installs only inside disposable .operator-test homes) --- #
TEST_HOME=0
case "$HOME" in *.operator-test-*) TEST_HOME=1 ;; esac
if [ "${OPERATOR_AGENT_SESSION:-}" != "" ]; then
    if [ "$TEST_HOME" = 1 ] && [ "${OPERATOR_ALLOW_TEST_INSTALL:-}" = "1" ]; then
        : # sandboxed install into a disposable test home: allowed (used by test.sh)
    else
        die "refusing: installs are run by the owner in their own terminal."
    fi
fi
command -v python3 >/dev/null 2>&1 || die "python3 is required (the gate is stdlib-only Python)."
command -v grep >/dev/null 2>&1 || die "grep is required (the shim fallback filter)."

# --- keep this script off our own shims (re-runs from a shimmed shell) --------- #
strip_entry() {  # strip_entry <PATH> <dir> -> PATH without <dir> entries
    printf '%s' "$1" | awk -v drop="$2" '
        BEGIN { RS = ":" }
        $0 != drop && $0 != "" { if (out != "") out = out ":"; out = out $0 }
        END { print out }'
}
PATH=$(strip_entry "$PATH" "$OP_HOME/shim/bin")
PATH=$(strip_entry "$PATH" "$HOME/.operator/shim/bin")
export PATH

mkdir -p "$GATE_DIR" || die "cannot create $GATE_DIR"
mkdir -p "$SHIM_BIN" "$SHIM_LIB" || die "cannot create $SHIM_BIN"

# --- 1. the gate (download unless the existing copy is same-or-newer) ---------- #
gate_version() {   # gate_version <file> -> "0.3.1" or ""
    sed -n 's/^VERSION = *"\([^"]*\)".*/\1/p' "$1" 2>/dev/null | head -n 1
}
version_ge() {     # version_ge A B -> true when A >= B (dotted numeric)
    awk -v a="$1" -v b="$2" 'BEGIN {
        if (a == "") exit 1
        na = split(a, A, "."); nb = split(b, B, ".")
        n = na > nb ? na : nb
        for (i = 1; i <= n; i++) {
            x = A[i] + 0; y = B[i] + 0
            if (x > y) exit 0
            if (x < y) exit 1
        }
        exit 0
    }'
}

SRC=""
TMP_GATE="$GATE_DIR/.download.operator-gate.py"
if command -v curl >/dev/null 2>&1; then
    curl -fsSL --max-time 30 -o "$TMP_GATE" "$GATE_URL" 2>/dev/null || rm -f "$TMP_GATE"
elif command -v wget >/dev/null 2>&1; then
    wget -q -T 30 -O "$TMP_GATE" "$GATE_URL" 2>/dev/null || rm -f "$TMP_GATE"
else
    python3 -c "import urllib.request,sys; urllib.request.urlretrieve('$GATE_URL', sys.argv[1])" \
        "$TMP_GATE" 2>/dev/null || rm -f "$TMP_GATE"
fi
if [ -s "$TMP_GATE" ] && grep -q '^VERSION = ' "$TMP_GATE"; then
    SRC="$TMP_GATE"
else
    rm -f "$TMP_GATE"
fi
if [ "$SRC" = "" ] && [ -f "$REPO_GATE" ] && grep -q '^VERSION = ' "$REPO_GATE"; then
    SRC="$REPO_GATE"
fi

if [ "$SRC" = "" ]; then
    if [ -f "$GATE_DST" ]; then
        say "operator-gate: no source reachable and none needed -- keeping existing $(gate_version "$GATE_DST" 2>/dev/null || echo unknown)"
    else
        die "could not download $GATE_URL and no sibling gate at $REPO_GATE"
    fi
else
    SRC_VER=$(gate_version "$SRC")
    DST_VER=$(gate_version "$GATE_DST" 2>/dev/null)
    if [ -f "$GATE_DST" ] && version_ge "$DST_VER" "$SRC_VER"; then
        say "operator-gate: existing $DST_VER >= $SRC_VER -- not overwritten"
        [ "$SRC" = "$TMP_GATE" ] && rm -f "$TMP_GATE"
    else
        if [ "$SRC" = "$TMP_GATE" ]; then
            mv "$SRC" "$GATE_DST"          # downloaded file: consumed by the move
        else
            cp "$SRC" "$GATE_DST"          # sibling repo file: the repo keeps its copy
        fi
        chmod 755 "$GATE_DST"
        say "operator-gate: installed v$SRC_VER -> $GATE_DST"
    fi
fi

# --- 2. the shim dispatcher ----------------------------------------------------- #
[ -f "$DISPATCH_SRC" ] || die "operator-dispatch.py missing next to install.sh"
cp "$DISPATCH_SRC" "$DISPATCH_DST"
chmod 755 "$DISPATCH_DST"
say "operator-lite: dispatcher installed -> $DISPATCH_DST"

# --- 3. generate the shims ------------------------------------------------------- #
generated=0
removed=0
skipped=""
for b in $SHIM_BINS; do
    real=$(PATH="$PATH" command -v "$b" 2>/dev/null) || real=""
    case "$real" in
        /*) : ;;
        *)
            if [ -f "$SHIM_BIN/$b" ]; then
                rm -f "$SHIM_BIN/$b" && removed=$((removed + 1))
            fi
            skipped="$skipped $b"
            continue ;;
    esac
    if [ ! -x "$real" ]; then
        skipped="$skipped $b"
        continue
    fi
    case "$real" in
        "$OP_HOME"/shim/*|"$HOME"/.operator/shim/*)
            skipped="$skipped $b(shim-loop)"
            continue ;;
    esac
    cat > "$SHIM_BIN/$b" <<EOF
#!/bin/sh
# operator-lite shim for '$b' -> real: $real
# Generated by operator-lite/ports/aider/install.sh -- edits are overwritten on
# reinstall. Contract: gate 0 -> exec real; gate 2 -> blocked, exit 126;
# gate unreachable -> fail-open unless the text plainly looks destructive.
OP_BIN="$b"
OP_REAL="$real"
OPH="\${OPERATOR_HOME:-\$HOME/.operator}"
OP_DESTRUCTIVE='$OP_DESTRUCTIVE_ERE'

case "\$OP_REAL" in "\$OPH"/shim/*)
    printf 'operator-lite: shim loop for %s (real path inside shim dir)\n' "\$OP_BIN" >&2
    exit 127 ;;
esac
if [ -f "\$OPH/shim/lib/operator-dispatch.py" ] && [ -f "\$OPH/gate/operator-gate.py" ] \\
        && command -v python3 >/dev/null 2>&1; then
    python3 "\$OPH/shim/lib/operator-dispatch.py" "\$OP_BIN" "\$@"
    rc=\$?
    [ "\$rc" -eq 0 ] && exec "\$OP_REAL" "\$@"
    [ "\$rc" -eq 126 ] && exit 126
    printf 'operator-lite: dispatcher failed (rc=%s); applying unreachable policy\n' "\$rc" >&2
else
    [ "\${OPERATOR_LITE_QUIET:-}" = "1" ] || \\
        printf 'operator-lite: gate unreachable for %s; applying fail-open policy\n' "\$OP_BIN" >&2
fi
if printf '%s\n' "\$OP_BIN \$*" | grep -Eqi -- "\$OP_DESTRUCTIVE"; then
    printf 'Operator Lite blocked: gate unreachable and the command looks destructive (%s %s).\n' "\$OP_BIN" "\$*" >&2
    exit 126
fi
exec "\$OP_REAL" "\$@"
EOF
    chmod 755 "$SHIM_BIN/$b"
    generated=$((generated + 1))
done
say "operator-lite: shims installed -> $SHIM_BIN ($generated generated, $removed stale removed, skipped:${skipped:- none})"

# --- smoke test: one shimmed call through the real gate -------------------------- #
if PATH="$SHIM_BIN:$PATH" ls >/dev/null 2>&1; then
    say "operator-lite: smoke test OK (shim -> gate -> real ls)"
else
    say "operator-lite: WARNING -- shimmed ls did not run cleanly; inspect $SHIM_BIN/ls"
fi

# --- 4. mode: shadow unless the owner already chose ------------------------------- #
if [ ! -f "$MODE_DST" ]; then
    printf 'shadow\n' > "$MODE_DST"
    MODE_MSG="mode=shadow (log-only). Flip with: echo enforce > $MODE_DST"
else
    MODE_MSG="mode=$(cat "$MODE_DST") (existing mode kept)"
fi
say "operator-gate: $MODE_MSG"

# --- 5. how to run aider under the gate ------------------------------------------- #
if [ "$OP_HOME" = "$HOME/.operator" ]; then
    SHIM_DISPLAY='$HOME/.operator/shim/bin'
else
    SHIM_DISPLAY="$SHIM_BIN"
fi
say ""
say "Done. Run aider under the gate (prefix PATH for that invocation only):"
say "  PATH=\"$SHIM_DISPLAY:\$PATH\" aider"
say "or define an alias -- in your own rc file, by hand:"
say "  alias aider='PATH=\"$SHIM_DISPLAY:\$PATH\" aider'"
say ""
say "Coverage boundary, stated honestly: shims catch bare-name invocations of"
say "the covered binaries at any shell depth; absolute-path calls (/bin/rm),"
say "sudo, shell builtins and redirects from unshimmed primaries bypass them."
say "Next steps:"
say "  1. Prove it:  sh $PORT_DIR/test.sh"
say "  2. Receipts:  python3 $GATE_DST verify ; python3 $GATE_DST status"
say "  3. When the shadow log looks right, flip to blocking:  echo enforce > $MODE_DST"
