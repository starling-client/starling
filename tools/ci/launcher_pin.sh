#!/usr/bin/env bash
# Prints the sunrise-launcher commit pinned in tools/ci/launcher-pin, or refuses anything but a full lowercase SHA-1.
set -euo pipefail
pin="$(tr -d '[:space:]' < "$(dirname "$0")/launcher-pin")"
if ! [[ "$pin" =~ ^[0-9a-f]{40}$ ]]; then
  echo "::error::tools/ci/launcher-pin must be a full 40-character lowercase commit SHA" >&2
  exit 1
fi
printf '%s\n' "$pin"
