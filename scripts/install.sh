#!/usr/bin/env sh
# Install the latest signed Vestige release without npm.
# Usage: curl -fsSL https://raw.githubusercontent.com/samvallad33/vestige/main/scripts/install.sh | sh
set -eu

REPO="samvallad33/vestige"
INSTALL_DIR="${VESTIGE_INSTALL_DIR:-$HOME/.local/bin}"

fail() { printf '%s\n' "vestige install: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || fail "requires $1"; }

need curl
need tar
case "$(uname -s)" in
  Darwin)
    case "$(uname -m)" in
      arm64|aarch64) target="aarch64-apple-darwin" ;;
      x86_64) target="x86_64-apple-darwin" ;;
      *) fail "unsupported macOS architecture: $(uname -m)" ;;
    esac
    ;;
  Linux)
    case "$(uname -m)" in
      x86_64|amd64) target="x86_64-unknown-linux-gnu" ;;
      aarch64|arm64) target="aarch64-unknown-linux-gnu" ;;
      *) fail "unsupported Linux architecture: $(uname -m)" ;;
    esac
    ;;
  *) fail "supported systems are macOS and Linux" ;;
esac

archive="vestige-mcp-${target}.tar.gz"
base="https://github.com/${REPO}/releases/latest/download/${archive}"
workdir="$(mktemp -d "${TMPDIR:-/tmp}/vestige-install.XXXXXX")"
cleanup() { rm -rf "$workdir"; }
trap cleanup EXIT HUP INT TERM

printf '%s\n' "Downloading the latest signed Vestige release for ${target}..."
curl --fail --location --silent --show-error "$base" --output "$workdir/$archive"
curl --fail --location --silent --show-error "${base}.sha256" --output "$workdir/${archive}.sha256"

(
  cd "$workdir"
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -c "${archive}.sha256"
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c "${archive}.sha256"
  else
    fail "requires shasum or sha256sum to verify the release"
  fi
)

stage="$workdir/stage"
mkdir -p "$stage" "$INSTALL_DIR"
tar -xzf "$workdir/$archive" -C "$stage"
for binary in vestige-mcp vestige vestige-restore vestige-upgrade; do
  [ -f "$stage/$binary" ] || fail "release archive is missing $binary"
  install -m 755 "$stage/$binary" "$INSTALL_DIR/$binary"
done

"$INSTALL_DIR/vestige-mcp" --version
printf '%s\n' "Installed Vestige in $INSTALL_DIR"
printf '%s\n' "Connect Claude Code: claude mcp add vestige $INSTALL_DIR/vestige-mcp -s user"
printf '%s\n' "Connect Codex:       codex mcp add vestige -- $INSTALL_DIR/vestige-mcp"
