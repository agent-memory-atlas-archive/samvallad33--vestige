#!/usr/bin/env bash
# Keep this entrypoint compatible with macOS Bash 3.2 and Git Bash.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
exec python3 "$root/scripts/check-release-no-reqwest.py"
